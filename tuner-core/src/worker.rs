//! # Background Worker (Thread 3) — Heavy Offline DSP
//!
//! The "Heavy Lifter" of the pipeline. A single dedicated background thread
//! that receives filled 1.5-second audio captures from the Gatekeeper (via the
//! [`AudioPool`](crate::pipeline::AudioPool)) and performs computationally
//! expensive offline DSP, such as extracting up to 32 partials and calculating
//! the inharmonicity constant ($B$).
//!
//! ## Why a Single Thread?
//!
//! Captures are infrequent (one stable note at a time, triggered by the Gatekeeper's
//! State 4 RELEASE). The MAT / ICF algorithms are fast enough to complete well before
//! the next capture could arrive, so a single dedicated thread avoids the overhead
//! of a full thread pool.
//!
//! ## Implementation
//!
//! The `WorkerManager` spawns a single background thread at pipeline startup.
//! The thread blocks on a crossbeam receiver and processes payloads as they arrive:
//!
//! 1. Receive a `CapturePayload` (a `Box<[f32; 66150]>` buffer + metadata,
//!    including the already-identified `target_note`)
//! 2. Perform a high-resolution FFT on the captured audio + a one-sample-shifted
//!    frame, and derive a CSPE super-resolution frequency map
//! 3. Take the note identity from the payload (the Engine's discovery lock in
//!    Auto mode, the user selection in Manual mode) — the worker does not
//!    re-identify the note
//! 4. Run MAT to extract partials and jointly refine ($f_0$, $B$)
//! 5. Write diagnostic files (audio.raw + analysis.json) to disk
//! 6. Send a `KeyMeasurement` result to the UI via crossbeam SPSC channel
//! 7. Recycle the buffer back to the `AudioPool`

use crate::algorithms::curves::{self, BALANCED_INTERVALS, CurveParams, PURE_TWELFTHS_INTERVALS};
use crate::algorithms::{
    mat::{self, MAX_PARTIALS, MatOrder},
    spectral,
};
use crate::audio::BASS_WINDOW_SIZE;
use crate::models::{self, CurveInput, KeyMeasurement, NOTES, Partial, TuningCurve};
use crate::pipeline::{AudioPool, CapturePayload, CaptureState, PipelineAtomics};
use crossbeam_channel::{Receiver, Sender, select};
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Relative band around the named key's ET frequency inside which the live
/// tracker's seed is trusted for MAT. **Ours, from validation**: the joint
/// (f0, B) association recovers known B to < 1 % only when seeded within
/// ±10 % of the true fundamental (`examples/mat_b_recovery.rs`); honest
/// mistuning of the named key is far smaller (a whole semitone is 5.9 %),
/// so anything outside the basin is tracker garbage, not signal.
pub const MAT_SEED_TOLERANCE: f32 = 0.10;

/// A UI → Worker request to (re)compute the tuning-curve bundle from a
/// trust-filtered profile snapshot (crossing #6, UI → Worker).
///
/// The UI sends the already-filtered [`CurveInput`] — **not** the raw profile —
/// so the worker stays free of the trust/provenance policy (ADR 0006 item 3
/// lives on the UI side, where the profile does). `generation` is a
/// monotonic counter the UI stamps on every job; the returned [`CurveBundle`]
/// echoes it so the UI can drop a bundle superseded by a newer edit
/// (latest-wins — a stale curve is worthless).
#[derive(Debug, Clone)]
pub struct CurveJob {
    pub generation: u64,
    pub input: CurveInput,
}

/// Everything the UI can ask the Worker to do on the job channel (crossing #6,
/// UI → Worker) — the mirror of [`WorkerOutput`] on the return path. One input
/// stream, a sum type of every background job the worker accepts. Curve
/// recompute is the only kind today; a new kind is a new variant, so the
/// channel type (and every signature carrying it) never changes to add one.
#[derive(Debug, Clone)]
pub enum WorkerJob {
    Curve(CurveJob),
}

/// All tuning-curve engines computed from one [`CurveJob`], echoed back to the
/// UI. Every engine is computed so the (deferred) comparison UI can switch
/// between them with no worker round-trip; the manual-mode default view is
/// [`multi_balanced`](Self::multi_balanced). Derived data — never persisted
/// (`TuningCurve` has no `Serialize`; design note §9).
#[derive(Debug, Clone)]
pub struct CurveBundle {
    pub generation: u64,
    /// (a) Rigaud-pure.
    pub rigaud_pure: TuningCurve,
    /// (b) per-key + Whittaker smoothing.
    pub per_key_smoothed: TuningCurve,
    /// (c) Giordano sensory-dissonance-calibrated octave type.
    pub giordano: TuningCurve,
    /// (d) weighted multi-interval least squares, BALANCED preset — the
    /// manual-mode default.
    pub multi_balanced: TuningCurve,
    /// (d) weighted multi-interval least squares, PURE_TWELFTHS preset.
    pub multi_pure_twelfths: TuningCurve,
    /// Per-key displayed strobe partial `n*` (strobe design §6, R5/R8):
    /// carried in the bundle so it locks with the curve. Filled by the
    /// amplitude-informed [`curves::select_display_partials`] (CyberTuner
    /// "Smart Partials", §6.3), which falls back per key to the
    /// [`curves::default_display_partials`] register table where no
    /// measurement exists.
    pub display_partials: [u8; 88],
}

impl CurveBundle {
    /// Runs every engine at [`CurveParams::default()`] on the job's input.
    /// Cold path (~1.4 s, dominated by (c)'s Giordano scans) — worker thread
    /// only, never the DSP hot path. The ρ Low/Mean/High preset variants of
    /// (c) are deferred with the comparison UI (they need (c)'s calibration
    /// factored out of the per-preset path to avoid re-running the scan).
    pub fn compute(job: &CurveJob) -> Self {
        let input = &job.input;
        let params = CurveParams::default();
        Self {
            generation: job.generation,
            rigaud_pure: curves::rigaud_pure(input, &params),
            per_key_smoothed: curves::per_key_smoothed(input, &params),
            giordano: curves::giordano_calibrated(input, &params),
            multi_balanced: curves::multi_interval(input, &params, BALANCED_INTERVALS, None),
            multi_pure_twelfths: curves::multi_interval(
                input,
                &params,
                PURE_TWELFTHS_INTERVALS,
                None,
            ),
            display_partials: curves::select_display_partials(input),
        }
    }
}

/// Everything the Worker sends back to the UI on the single result channel
/// (crossing #5, Worker → UI). One output stream, a sum type of every result
/// the worker produces — the idiomatic actor pattern. The large `Curve`
/// variant is boxed so the common `Measurement` case stays small.
#[derive(Debug, Clone)]
pub enum WorkerOutput {
    Measurement(KeyMeasurement),
    Curve(Box<CurveBundle>),
}

/// Manages the lifecycle of the background worker thread.
///
/// The `WorkerManager` owns an `Arc<AudioPool>` so it can return processed buffers
/// back to the pool after the heavy DSP is complete. Currently a wireframe.
pub struct WorkerManager {
    audio_pool: Arc<AudioPool>,
    atomics: Arc<PipelineAtomics>,
    capture_rx: Receiver<CapturePayload>,
    /// UI → Worker background jobs (crossing #6). Serviced only when no
    /// capture (crossing #5) is pending — measurement latency is
    /// user-facing mid-session; a background job (curve recompute) is not.
    worker_job_rx: Receiver<WorkerJob>,
    result_tx: Sender<WorkerOutput>,
}

impl WorkerManager {
    pub fn new(
        audio_pool: Arc<AudioPool>,
        atomics: Arc<PipelineAtomics>,
        capture_rx: Receiver<CapturePayload>,
        worker_job_rx: Receiver<WorkerJob>,
        result_tx: Sender<WorkerOutput>,
    ) -> Self {
        Self {
            audio_pool,
            atomics,
            capture_rx,
            worker_job_rx,
            result_tx,
        }
    }

    pub fn start_workers(self) {
        std::thread::spawn(move || {
            let mut planner = realfft::RealFftPlanner::<f32>::new();
            // Pre-plan for max size
            let max_fft_size = BASS_WINDOW_SIZE * 8; // 65536
            let mut fft_instance = planner.plan_fft_forward(max_fft_size);

            // Scratch buffers
            let mut time_buffer = vec![0.0f32; max_fft_size];
            let mut frequency_buffer = vec![Complex { re: 0.0, im: 0.0 }; max_fft_size / 2 + 1];
            // Second spectrum of the one-sample-shifted frame, for CSPE phase comparison.
            let mut frequency_buffer_shifted =
                vec![Complex { re: 0.0, im: 0.0 }; max_fft_size / 2 + 1];
            let mut magnitude_buffer = vec![0.0f32; max_fft_size / 2];
            // CSPE super-resolution per-bin frequency map (parallel to magnitude_buffer).
            let mut cspe_buffer = vec![0.0f32; max_fft_size / 2];

            loop {
                // Captures first: measurement latency is user-facing mid-session,
                // a curve recompute is not. Drain every pending capture before
                // even looking at a curve job.
                let mut capture_disconnected = false;
                loop {
                    match self.capture_rx.try_recv() {
                        Ok(payload) => Self::process_payload(
                            payload,
                            &self.audio_pool,
                            &self.atomics,
                            &self.result_tx,
                            &mut planner,
                            &mut fft_instance,
                            &mut time_buffer,
                            &mut frequency_buffer,
                            &mut frequency_buffer_shifted,
                            &mut magnitude_buffer,
                            &mut cspe_buffer,
                        ),
                        Err(crossbeam_channel::TryRecvError::Empty) => break,
                        Err(crossbeam_channel::TryRecvError::Disconnected) => {
                            capture_disconnected = true;
                            break;
                        }
                    }
                }
                if capture_disconnected {
                    // Capture channel closed → pipeline shut down.
                    break;
                }

                // Nothing to process right now: block until either channel wakes
                // us. The capture arm re-loops (drained first above); the job arm
                // coalesces to the newest job and dispatches it.
                select! {
                    recv(self.capture_rx) -> msg => match msg {
                        Ok(payload) => Self::process_payload(
                            payload,
                            &self.audio_pool,
                            &self.atomics,
                            &self.result_tx,
                            &mut planner,
                            &mut fft_instance,
                            &mut time_buffer,
                            &mut frequency_buffer,
                            &mut frequency_buffer_shifted,
                            &mut magnitude_buffer,
                            &mut cspe_buffer,
                        ),
                        Err(_) => break, // capture channel closed → shutdown
                    },
                    // A job-channel disconnect (Err) is ignored: captures may
                    // still flow, so keep serving the loop.
                    recv(self.worker_job_rx) -> msg => if let Ok(mut job) = msg {
                        // Coalesce: keep only the newest queued job (latest-wins;
                        // an earlier one is already superseded). Only curve jobs
                        // exist today — when a second kind is added, the `match`
                        // below stops compiling, forcing a coalescing rethink.
                        while let Ok(newer) = self.worker_job_rx.try_recv() {
                            job = newer;
                        }
                        match job {
                            WorkerJob::Curve(curve_job) => {
                                let bundle = CurveBundle::compute(&curve_job);
                                // Drop on full: a superseded bundle is worthless, and
                                // the UI re-requests from its dirty flag. Never blocks
                                // captures.
                                let _ = self
                                    .result_tx
                                    .try_send(WorkerOutput::Curve(Box::new(bundle)));
                            }
                        }
                    },
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn process_payload(
        payload: CapturePayload,
        audio_pool: &Arc<AudioPool>,
        atomics: &Arc<PipelineAtomics>,
        result_tx: &Sender<WorkerOutput>,
        planner: &mut realfft::RealFftPlanner<f32>,
        fft_instance: &mut Arc<dyn RealToComplex<f32>>,
        time_buffer: &mut [f32],
        frequency_buffer: &mut [Complex<f32>],
        frequency_buffer_shifted: &mut [Complex<f32>],
        magnitude_buffer: &mut [f32],
        cspe_buffer: &mut [f32],
    ) {
        // Step 1: Calculate power-of-two size
        let sample_count = payload.stable_sample_count.max(2048);
        let fft_size = 1 << (usize::BITS - 1 - sample_count.leading_zeros());

        if fft_instance.len() != fft_size {
            *fft_instance = planner.plan_fft_forward(fft_size);
        }

        // Apply Hann window and copy to scratch
        spectral::fft(
            &payload.stable_buffer[..fft_size],
            &mut time_buffer[..fft_size],
            &mut frequency_buffer[..(fft_size / 2 + 1)],
            fft_instance,
            fft_size,
        );

        spectral::magnitude_spectrum(
            &frequency_buffer[..],
            fft_size,
            &mut magnitude_buffer[..(fft_size / 2)],
        );

        // CSPE: transform the SAME frame advanced by one sample, then derive the per-bin
        // super-resolution frequency map from the two spectra (DAFx-09 §2.3). The capture
        // buffer holds 66150 samples and fft_size ≤ 65536, so the one-sample shift is in
        // bounds; the Hann window zeroes the lone boundary sample regardless.
        spectral::fft(
            &payload.stable_buffer[1..fft_size + 1],
            &mut time_buffer[..fft_size],
            &mut frequency_buffer_shifted[..(fft_size / 2 + 1)],
            fft_instance,
            fft_size,
        );

        spectral::cspe(
            &frequency_buffer[..],
            &frequency_buffer_shifted[..],
            fft_size,
            payload.sample_rate,
            &mut cspe_buffer[..(fft_size / 2)],
        );

        let measured_key_index = payload.target_note;
        let hz_per_bin = payload.sample_rate as f32 / fft_size as f32;

        let f0_et = NOTES[measured_key_index as usize].frequency;
        let expected_beta = models::get_expected_beta(measured_key_index);

        // If the real-time Goertzel Engine successfully tracked the note, use its highly
        // accurate frequency as the seed. Otherwise, fall back to the mathematically
        // perfect Equal Temperament frequency for this key.
        //
        // Plausibility gate (MAT_SEED_TOLERANCE): the tracker seed is trusted
        // only within ±10 % of the named key's ET. MAT's joint (f0, B)
        // association is validated to recover B only when the seed lies
        // within ±10 % of the true fundamental (`examples/mat_b_recovery.rs`),
        // and a genuine strike of the named key deviates from ET by tuning
        // error only (a whole semitone is 5.9 %) — so a seed outside the
        // basin can only be tracker garbage, not a valid reading. Observed
        // 2026-07-10: the deep-bass tracker walked onto low-frequency rumble
        // and seeded A0 (27.5 Hz) captures at 5–16 Hz, mis-associating the
        // entire partial comb; the untrusted-seed case now falls back to ET
        // exactly as when tracking fails outright.
        let actual_seed = match payload.measured_f0 {
            Some(tracked) if (tracked / f0_et - 1.0).abs() <= MAT_SEED_TOLERANCE => tracked,
            Some(tracked) => {
                eprintln!(
                    "[WORKER] Tracker seed {tracked:.2} Hz implausible for key {measured_key_index} \
                     (ET {f0_et:.2} Hz) — seeding MAT from ET"
                );
                f0_et
            }
            None => f0_et,
        };

        // Step 3: Run the MAT adjustive trajectory, which jointly refines (f0, B) and
        // returns a measured B with a reliability score. It only fails (`None`) when the
        // capture yields fewer than two partials — no pair to solve. Partial frequencies are
        // read from the CSPE map, so MAT is register-agnostic (no bass/treble split).
        let mut partial_freqs_out = [0.0; MAX_PARTIALS];
        let mut partial_ns_out = [0u32; MAX_PARTIALS];

        let mat_res = mat::detect_pitch_mat(
            &magnitude_buffer[..(fft_size / 2)],
            &cspe_buffer[..(fft_size / 2)],
            payload.sample_rate,
            actual_seed, // Goertzel seed for the first prediction; MAT refines it
            // Serial growth (the paper's Fig. 3 order): uses more partials and, by the
            // goodness-of-fit check in `validate_mat`, explains the clean low partials as well
            // as Simultaneous while fitting the high partials it discards. Simultaneous remains
            // the conservative fallback (one flag flip). See `MatOrder`.
            MatOrder::Serial,
            &mut partial_freqs_out,
            &mut partial_ns_out,
        );

        // `calculated_b` carries the measured coefficient; `b_confidence` carries its
        // reliability. It is `None` only when MAT found no usable partials (a capture
        // failure). The Rigaud prior is never substituted for a measured value.
        let mut partials = Vec::new();
        let mut calculated_b: Option<f32> = None;
        let mut b_confidence = 0.0_f32;
        let mut mat_f0 = actual_seed;

        if let Some(est) = mat_res {
            calculated_b = Some(est.b);
            b_confidence = est.confidence;
            mat_f0 = est.f0;

            for i in 0..est.partial_count {
                let bin = (partial_freqs_out[i] / hz_per_bin).round() as usize;
                let amp = if bin < magnitude_buffer.len() {
                    magnitude_buffer[bin]
                } else {
                    0.0
                };

                partials.push(Partial {
                    number: partial_ns_out[i],
                    frequency: partial_freqs_out[i],
                    amplitude: amp,
                });
            }
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        // Build Measurement
        let measurement = KeyMeasurement {
            key_index: measured_key_index,
            measured_f0: actual_seed,
            partials,
            calculated_b,
            last_captured: format!("{}", now), // Basic string timestamp
            captured_in_auto: payload.captured_in_auto,
        };

        // Step 4: Write Diagnostic Dump
        Self::write_diagnostics(
            &payload,
            &measurement,
            fft_size,
            payload.sample_rate as f32 / fft_size as f32,
            expected_beta,
            b_confidence,
            mat_f0,
        );

        // Step 5: Clean up and send result
        let _ = result_tx.try_send(WorkerOutput::Measurement(measurement));

        // Reset capture state back to Idle
        atomics
            .capture_state
            .store(CaptureState::Idle as u8, Ordering::Relaxed);

        // Return boxed arrays to memory pool
        let _ = audio_pool.push(payload.stable_buffer);
        if let Some(dbuf) = payload.full_event_buffer {
            let _ = audio_pool.push(dbuf);
        }
    }

    fn write_diagnostics(
        payload: &CapturePayload,
        measurement: &KeyMeasurement,
        fft_size: usize,
        hz_per_bin: f32,
        expected_beta: f32,
        b_confidence: f32,
        mat_f0: f32,
    ) {
        let (key_name, _) = models::find_nearest_note_by_index(measurement.key_index);

        // Use measurement's organically resolved key_index. The capture
        // timestamp suffix makes every capture its own directory, so
        // repeat captures of one key are all retained (the repeat-capture
        // noise-decomposition experiment consumes them; a fixed per-key
        // name silently overwrote earlier dumps). Offline tools discover
        // dumps by the `key_` prefix and read the key identity from
        // analysis.json, so the suffix is transparent to them.
        let mut dir = PathBuf::from("diagnostics");
        dir.push(format!(
            "key_{:03}_{}_{}",
            measurement.key_index, key_name, measurement.last_captured
        ));

        if fs::create_dir_all(&dir).is_ok() {
            // Write audio.raw
            let mut file = dir.clone();
            file.push("audio.raw");
            if let Ok(mut f) = fs::File::create(file) {
                // write f32 bytes
                let slice = &payload.stable_buffer[..payload.stable_sample_count];
                let byte_slice: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        slice.as_ptr() as *const u8,
                        std::mem::size_of_val(slice),
                    )
                };
                let _ = f.write_all(byte_slice);
            }

            // Write audio_full_event.raw
            let mut file_full = dir.clone();
            file_full.push("audio_full_event.raw");
            if let Some(ref dbuf) = payload.full_event_buffer
                && let Ok(mut f_full) = fs::File::create(file_full)
            {
                let slice = &dbuf[..payload.full_event_sample_count];
                let byte_slice: &[u8] = unsafe {
                    std::slice::from_raw_parts(
                        slice.as_ptr() as *const u8,
                        std::mem::size_of_val(slice),
                    )
                };
                let _ = f_full.write_all(byte_slice);
            }

            // Write analysis.json
            let mut file2 = dir.clone();
            file2.push("analysis.json");
            if let Ok(mut f2) = fs::File::create(file2) {
                let json = serde_json::json!({
                    "key_index": payload.target_note,
                    "appVersion": "0.1",
                    "metadata": {
                        "key_index": measurement.key_index,
                        "sample_rate": payload.sample_rate,
                        "target_note_input": payload.target_note,
                        "measured_f0": measurement.measured_f0,
                        "f0_et": 27.5 * 2.0_f32.powf(measurement.key_index as f32 / 12.0),
                        "fft_size": fft_size,
                        "hz_per_bin": hz_per_bin,
                        "noise_floor": payload.noise_floor,
                        "calculated_b": measurement.calculated_b,
                        "expected_beta": expected_beta,
                        "b_confidence": b_confidence,
                        "mat_f0": mat_f0,
                        // Provenance: manual-mode captures are trusted by the
                        // curve layer (ADR 0006 item 3); auto-mode ones are not.
                        // Persist it so offline tools that rebuild a profile from
                        // diagnostics (regenerate_partials → curve_compare) keep
                        // the trust flag instead of defaulting it to untrusted.
                        "captured_in_auto": measurement.captured_in_auto,
                        "partials": measurement.partials,
                    }
                });
                let _ = f2.write_all(
                    serde_json::to_string_pretty(&json)
                        .unwrap_or_default()
                        .as_bytes(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The launch / no-captures state: an empty (prior-only) input must
    /// produce a full bundle without panicking, so the live curve widget can
    /// render the prior curve before any key is measured.
    #[test]
    fn bundle_from_empty_input_is_prior_only_and_anchored() {
        let job = CurveJob {
            generation: 7,
            input: CurveInput::default(), // 88 × None
        };
        let bundle = CurveBundle::compute(&job);

        // Generation echoes so the UI can drop superseded bundles.
        assert_eq!(bundle.generation, 7);

        // Every engine yields an A4-anchored 88-key curve (cents[48] == 0),
        // and no key is flagged as measured (there is no measurement).
        for curve in [
            &bundle.rigaud_pure,
            &bundle.per_key_smoothed,
            &bundle.giordano,
            &bundle.multi_balanced,
            &bundle.multi_pure_twelfths,
        ] {
            assert_eq!(curve.cents.len(), 88);
            assert!(curve.cents[48].abs() < 1e-3, "A4 not anchored to 0");
            assert!(curve.cents.iter().all(|c| c.is_finite()));
            assert!(curve.flags.iter().all(|f| !f.measured));
        }
    }
}
