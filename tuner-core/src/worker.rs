//! # Background worker
//!
//! The thread that measures captures and recomputes tuning curves. A capture is
//! analysed over its first [`CAPTURE_ANALYSIS_SAMPLES`], however long the record:
//! a CSPE frequency map, then MAT's joint (f₀, B) for the key the pipeline named.
//! The result returns as a [`KeyMeasurement`] and the audio goes to the dump.

use crate::algorithms::curves::{self, BALANCED_INTERVALS, CurveParams, PURE_TWELFTHS_INTERVALS};
use crate::algorithms::{
    mat::{self, MAX_PARTIALS, MatOrder},
    spectral,
};
use crate::audio::BASS_WINDOW_SIZE;
use crate::models::{self, CurveInput, KeyMeasurement, NOTES, Partial, TuningCurve};
use crate::pipeline::{AudioPool, CAPTURE_ANALYSIS_SAMPLES, CapturePayload, PipelineAtomics};
use crossbeam_channel::{Receiver, Sender, select};
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Relative band around the named key's ET frequency within which the live
/// tracker's seed is trusted for MAT; outside it, MAT seeds from ET.
// MAT recovers B to < 1 % only when seeded within ±10 % of the true f₀, and a
// whole semitone of mistuning is 5.9 %. Do not widen it: the deep-bass tracker
// can walk onto rumble and seed A0 at 5–16 Hz, mis-associating the whole comb.
// asserted: tests/mat_b_recovery.rs
pub const MAT_SEED_TOLERANCE: f32 = 0.10;

/// A request to recompute the curve bundle from a trust-filtered [`CurveInput`]
/// (crossing #6). The bundle echoes `generation`, so a superseded one can be
/// dropped.
#[derive(Debug, Clone)]
pub struct CurveJob {
    pub generation: u64,
    pub input: CurveInput,
}

/// A job for the Worker (crossing #6).
#[derive(Debug, Clone)]
pub enum WorkerJob {
    /// Recompute the curve bundle.
    Curve(CurveJob),
    /// Write later capture dumps under this directory, or none. Captures already
    /// queued go under the old one, the instrument they were taken on.
    SetDumpDir(Option<PathBuf>),
}

/// Every engine's curve from one [`CurveJob`], so a frontend switches engines
/// without a recompute. Derived, never persisted.
#[derive(Debug, Clone)]
pub struct CurveBundle {
    pub generation: u64,
    /// (a) Rigaud-pure.
    pub rigaud_pure: TuningCurve,
    /// (b) per-key + Whittaker smoothing.
    pub per_key_smoothed: TuningCurve,
    /// (c) Giordano sensory-dissonance-calibrated octave type.
    pub giordano: TuningCurve,
    /// (d) weighted multi-interval least squares, BALANCED preset.
    pub multi_balanced: TuningCurve,
    /// (d) weighted multi-interval least squares, PURE_TWELFTHS preset.
    pub multi_pure_twelfths: TuningCurve,
    /// Per-key displayed strobe partial n* ([`curves::select_display_partials`]),
    /// computed from the same input as the curves, so the two never disagree.
    pub display_partials: [u8; 88],
}

impl CurveBundle {
    /// The curve for `choice`.
    pub fn curve(&self, choice: models::EngineChoice) -> &TuningCurve {
        match choice {
            models::EngineChoice::RigaudPure => &self.rigaud_pure,
            models::EngineChoice::PerKeySmoothed => &self.per_key_smoothed,
            models::EngineChoice::GiordanoMean => &self.giordano,
            models::EngineChoice::MultiBalanced => &self.multi_balanced,
            models::EngineChoice::MultiPureTwelfths => &self.multi_pure_twelfths,
        }
    }

    /// Runs every engine at [`CurveParams::default()`]. A cold path: ≈ 1.4 s,
    /// most of it (c)'s Giordano scans.
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

/// A Worker result (crossing #5). `Curve` is boxed, so the common `Measurement`
/// stays small.
#[derive(Debug, Clone)]
pub enum WorkerOutput {
    Measurement(KeyMeasurement),
    Curve(Box<CurveBundle>),
}

/// The directory of one capture's dump under the dump root:
/// `key_<idx>_<note>_<epoch>`, unique per capture so repeats are all kept. It
/// takes the identity rather than the measurement, so a dump stays nameable
/// after its profile entry is gone.
// Offline tools find dumps by the `key_` prefix and read the key from
// `analysis.json`, so the suffix is free to change.
pub fn dump_dir_name(key_index: u8, epoch: &str) -> String {
    let (key_name, _) = models::find_nearest_note_by_index(key_index);
    format!("key_{key_index:03}_{key_name}_{epoch}")
}

/// The Worker thread's owner: its channels, the pool it returns buffers to, and
/// the dump root.
pub struct WorkerManager {
    audio_pool: Arc<AudioPool>,
    atomics: Arc<PipelineAtomics>,
    capture_rx: Receiver<CapturePayload>,
    /// UI → Worker jobs (crossing #6), serviced only when no capture is pending:
    /// capture latency is what the operator waits on.
    worker_job_rx: Receiver<WorkerJob>,
    result_tx: Sender<WorkerOutput>,
    /// Directory capture dumps are written under; `None` writes none.
    dump_dir: Option<PathBuf>,
}

impl WorkerManager {
    pub fn new(
        audio_pool: Arc<AudioPool>,
        atomics: Arc<PipelineAtomics>,
        capture_rx: Receiver<CapturePayload>,
        worker_job_rx: Receiver<WorkerJob>,
        result_tx: Sender<WorkerOutput>,
        dump_dir: Option<PathBuf>,
    ) -> Self {
        Self {
            audio_pool,
            atomics,
            capture_rx,
            worker_job_rx,
            result_tx,
            dump_dir,
        }
    }

    pub fn start_workers(self) {
        std::thread::spawn(move || {
            // The loop's own copy, which `WorkerJob::SetDumpDir` moves.
            let mut dump_dir = self.dump_dir;
            let mut planner = realfft::RealFftPlanner::<f32>::new();
            let max_fft_size = BASS_WINDOW_SIZE * 8; // 65536
            let mut fft_instance = planner.plan_fft_forward(max_fft_size);

            let mut time_buffer = vec![0.0f32; max_fft_size];
            let mut frequency_buffer = vec![Complex { re: 0.0, im: 0.0 }; max_fft_size / 2 + 1];
            // Second spectrum of the one-sample-shifted frame, for CSPE phase comparison.
            let mut frequency_buffer_shifted =
                vec![Complex { re: 0.0, im: 0.0 }; max_fft_size / 2 + 1];
            let mut magnitude_buffer = vec![0.0f32; max_fft_size / 2];
            // CSPE super-resolution per-bin frequency map (parallel to magnitude_buffer).
            let mut cspe_buffer = vec![0.0f32; max_fft_size / 2];

            loop {
                // Every pending capture before any job.
                let mut capture_disconnected = false;
                loop {
                    match self.capture_rx.try_recv() {
                        Ok(payload) => Self::process_payload(
                            payload,
                            &self.audio_pool,
                            &self.atomics,
                            &self.result_tx,
                            dump_dir.as_deref(),
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

                // Block until either channel has something.
                select! {
                    recv(self.capture_rx) -> msg => match msg {
                        Ok(payload) => Self::process_payload(
                            payload,
                            &self.audio_pool,
                            &self.atomics,
                            &self.result_tx,
                            dump_dir.as_deref(),
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
                    recv(self.worker_job_rx) -> msg => if let Ok(job) = msg {
                        // Coalesced per kind: only the newest curve job matters,
                        // and a directory change is never lost to a curve job.
                        let mut latest_curve = None;
                        let mut latest_dir = None;
                        let mut sort = |job| match job {
                            WorkerJob::Curve(curve_job) => latest_curve = Some(curve_job),
                            WorkerJob::SetDumpDir(dir) => latest_dir = Some(dir),
                        };
                        sort(job);
                        while let Ok(newer) = self.worker_job_rx.try_recv() {
                            sort(newer);
                        }
                        // Directory first: it decides where anything computed
                        // after this point is written.
                        if let Some(dir) = latest_dir {
                            dump_dir = dir;
                        }
                        if let Some(curve_job) = latest_curve {
                            let bundle = CurveBundle::compute(&curve_job);
                            // Dropped on a full channel rather than blocking captures.
                            let _ = self
                                .result_tx
                                .try_send(WorkerOutput::Curve(Box::new(bundle)));
                        }
                    },
                }
            }
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn process_payload(
        mut payload: CapturePayload,
        audio_pool: &Arc<AudioPool>,
        atomics: &Arc<PipelineAtomics>,
        result_tx: &Sender<WorkerOutput>,
        dump_dir: Option<&Path>,
        planner: &mut realfft::RealFftPlanner<f32>,
        fft_instance: &mut Arc<dyn RealToComplex<f32>>,
        time_buffer: &mut [f32],
        frequency_buffer: &mut [Complex<f32>],
        frequency_buffer_shifted: &mut [Complex<f32>],
        magnitude_buffer: &mut [f32],
        cspe_buffer: &mut [f32],
    ) {
        // The largest power of two within the analysis window, so a longer record
        // measures the same span.
        let sample_count = payload
            .stable_sample_count
            .clamp(2048, CAPTURE_ANALYSIS_SAMPLES);
        let fft_size = 1 << (usize::BITS - 1 - sample_count.leading_zeros());

        // Zero-pad a record shorter than the transform. Pool buffers are
        // reused, so the slack past `stable_sample_count` still holds the
        // previous capture's audio — analysing it would mix two notes. The
        // `+ 1` covers the CSPE frame's one-sample shift.
        let analysed = fft_size + 1;
        if payload.stable_sample_count < analysed {
            payload.stable_buffer[payload.stable_sample_count..analysed].fill(0.0);
        }

        if fft_instance.len() != fft_size {
            *fft_instance = planner.plan_fft_forward(fft_size);
        }

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

        // CSPE (DAFx-09 §2.3): the frame advanced by one sample gives every bin's
        // frequency. `fft_size + 1` stays inside the buffer, which outruns the
        // analysis window.
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

        // The tracker's frequency when it is plausible for the named key, else ET.
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

        // MAT's joint (f₀, B) refinement on the CSPE map; `None` only when fewer than
        // two partials were found.
        let mut partial_freqs_out = [0.0; MAX_PARTIALS];
        let mut partial_ns_out = [0u32; MAX_PARTIALS];

        let mat_res = mat::detect_pitch_mat(
            &magnitude_buffer[..(fft_size / 2)],
            &cspe_buffer[..(fft_size / 2)],
            payload.sample_rate,
            actual_seed, // Goertzel seed for the first prediction; MAT refines it
            // The paper's serial order; see `MatOrder`.
            MatOrder::Serial,
            &mut partial_freqs_out,
            &mut partial_ns_out,
        );

        // The Rigaud prior is never substituted for a measured B.
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

        let measurement = KeyMeasurement {
            key_index: measured_key_index,
            measured_f0: actual_seed,
            partials,
            calculated_b,
            last_captured: format!("{}", now),
            captured_in_auto: payload.captured_in_auto,
            sounding_strings: payload.sounding_strings,
        };

        Self::write_diagnostics(
            dump_dir,
            &payload,
            &measurement,
            fft_size,
            payload.sample_rate as f32 / fft_size as f32,
            expected_beta,
            b_confidence,
            mat_f0,
        );

        // In this order: the buffers first, so "not in flight" means the pipeline
        // can borrow them; then the flag, ending the lifecycle; the result last,
        // so a consumer that re-arms on it finds the lifecycle finished.
        let _ = audio_pool.push(payload.stable_buffer);
        if let Some(dbuf) = payload.full_event_buffer {
            let _ = audio_pool.push(dbuf);
        }

        atomics.capture_in_flight.store(false, Ordering::Relaxed);

        let _ = result_tx.try_send(WorkerOutput::Measurement(measurement));
    }

    #[allow(clippy::too_many_arguments)]
    fn write_diagnostics(
        dump_dir: Option<&Path>,
        payload: &CapturePayload,
        measurement: &KeyMeasurement,
        fft_size: usize,
        hz_per_bin: f32,
        expected_beta: f32,
        b_confidence: f32,
        mat_f0: f32,
    ) {
        let Some(dump_dir) = dump_dir else {
            return;
        };
        let dir = dump_dir.join(dump_dir_name(
            measurement.key_index,
            &measurement.last_captured,
        ));

        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!(
                "[WORKER] Cannot write diagnostics to {}: {e}",
                dir.display()
            );
            return;
        }
        {
            Self::write_raw(
                &dir.join("audio.raw"),
                &payload.stable_buffer[..payload.stable_sample_count],
            );
            if let Some(ref dbuf) = payload.full_event_buffer {
                Self::write_raw(
                    &dir.join("audio_full_event.raw"),
                    &dbuf[..payload.full_event_sample_count],
                );
            }

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
                        // The gate's three thresholds as they stood, `noise_floor`
                        // holding the silence threshold: a replay needs all three.
                        "noise_floor": payload.noise_floor,
                        "nhwrsf_threshold": payload.nhwrsf_threshold,
                        "sustain_stability_threshold": payload.sustain_stability_threshold,
                        "calculated_b": measurement.calculated_b,
                        "expected_beta": expected_beta,
                        "b_confidence": b_confidence,
                        "mat_f0": mat_f0,
                        // So a profile rebuilt from dumps keeps the trust flag
                        // rather than defaulting to untrusted.
                        "captured_in_auto": measurement.captured_in_auto,
                        // `null` unless the operator declared one.
                        "sounding_strings": measurement.sounding_strings,
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

    /// Writes `samples` to `path` as raw native-endian `f32`.
    fn write_raw(path: &Path, samples: &[f32]) {
        if let Ok(mut f) = fs::File::create(path) {
            // SAFETY: `f32` has no padding and `u8` no alignment requirement, so
            // the samples' memory reads as `size_of_val(samples)` valid bytes.
            let bytes = unsafe {
                std::slice::from_raw_parts(samples.as_ptr() as *const u8, size_of_val(samples))
            };
            let _ = f.write_all(bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::CAPTURE_MAX_SAMPLES;

    /// The analysis window must stay inside the scratch buffers this thread
    /// allocates once at startup (`max_fft_size = BASS_WINDOW_SIZE * 8`).
    ///
    /// This is what lets the capture length be a runtime knob without touching
    /// the FFT ceiling: however long a record is, `fft_size` is the largest
    /// power of two inside [`CAPTURE_ANALYSIS_SAMPLES`], and the scratch holds
    /// it. Raise the analysis window past 2 × 65536 and the FFT plan outgrows
    /// the buffers it is written into.
    #[test]
    fn the_analysis_window_fits_the_worker_scratch() {
        let max_fft_size = BASS_WINDOW_SIZE * 8;
        let fft_size = 1 << (usize::BITS - 1 - CAPTURE_ANALYSIS_SAMPLES.leading_zeros());
        assert!(
            fft_size <= max_fft_size,
            "fft_size {fft_size} outgrew the {max_fft_size}-sample scratch"
        );
        // CSPE reads one sample past `fft_size`, from the allocated buffer
        // rather than the analysed span, so the ceiling has to clear it too.
        assert!(CAPTURE_MAX_SAMPLES > fft_size);
    }

    /// A record shorter than the transform is zero-padded, not filled with
    /// whatever the pooled buffer held before it. The buffers are reused, so the
    /// slack past `stable_sample_count` is the previous capture's audio.
    #[test]
    fn a_short_record_is_padded_with_silence_not_the_previous_capture() {
        // Buffer pre-loaded with a loud "previous capture", then 512 samples of
        // this one — under the 2048-sample floor, so the transform overruns it.
        let mut buffer = vec![1.0f32; CAPTURE_MAX_SAMPLES].into_boxed_slice();
        let recorded = 512usize;
        buffer[..recorded].fill(0.25);

        let sample_count = recorded.clamp(2048, CAPTURE_ANALYSIS_SAMPLES);
        let fft_size = 1 << (usize::BITS - 1 - sample_count.leading_zeros());
        let analysed = fft_size + 1;
        assert!(recorded < analysed, "precondition: the transform overruns");

        if recorded < analysed {
            buffer[recorded..analysed].fill(0.0);
        }
        assert!(
            buffer[recorded..analysed].iter().all(|s| *s == 0.0),
            "the slack the transform reads must be silence"
        );
        // …and nothing beyond the analysed span was disturbed.
        assert_eq!(buffer[analysed], 1.0);
    }

    /// The launch / no-captures state: an empty (prior-only) input must
    /// produce a full bundle without panicking, so the prior curve exists before
    /// any key is measured.
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
