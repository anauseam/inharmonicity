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
//! 1. Receive a `CapturePayload` (a `Box<[f32; 66150]>` buffer + metadata)
//! 2. Perform a high-resolution FFT on the captured audio
//! 3. Run the 88-Key Template Matcher (Auto mode) or bounded peak search (Manual mode)
//! 4. Run MAT to extract partials and compute the $B$ coefficient
//! 5. Write diagnostic files (audio.raw + analysis.json) to disk
//! 6. Send a `KeyMeasurement` result to the UI via crossbeam SPSC channel
//! 7. Recycle the buffer back to the `AudioPool`

use crate::audio::BASS_WINDOW_SIZE;
use crate::models::{KeyMeasurement, Partial};
use crate::pipeline::{AudioPool, CapturePayload, CaptureState, PipelineAtomics};
use crossbeam_channel::{Receiver, Sender};
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::Ordering;

/// Manages the lifecycle of the background worker thread.
///
/// The `WorkerManager` owns an `Arc<AudioPool>` so it can return processed buffers
/// back to the pool after the heavy DSP is complete. Currently a wireframe.
pub struct WorkerManager {
    audio_pool: Arc<AudioPool>,
    atomics: Arc<PipelineAtomics>,
    capture_rx: Receiver<CapturePayload>,
    result_tx: Sender<KeyMeasurement>,
}

impl WorkerManager {
    pub fn new(
        audio_pool: Arc<AudioPool>,
        atomics: Arc<PipelineAtomics>,
        capture_rx: Receiver<CapturePayload>,
        result_tx: Sender<KeyMeasurement>,
    ) -> Self {
        Self {
            audio_pool,
            atomics,
            capture_rx,
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
            let mut magnitude_buffer = vec![0.0f32; max_fft_size / 2];

            loop {
                match self.capture_rx.recv() {
                    Ok(payload) => {
                        Self::process_payload(
                            payload,
                            &self.audio_pool,
                            &self.atomics,
                            &self.result_tx,
                            &mut planner,
                            &mut fft_instance,
                            &mut time_buffer,
                            &mut frequency_buffer,
                            &mut magnitude_buffer,
                        );
                    }
                    Err(_) => {
                        // Channel closed, pipeline shut down
                        break;
                    }
                }
            }
        });
    }

    fn process_payload(
        payload: CapturePayload,
        audio_pool: &Arc<AudioPool>,
        atomics: &Arc<PipelineAtomics>,
        result_tx: &Sender<KeyMeasurement>,
        planner: &mut realfft::RealFftPlanner<f32>,
        fft_instance: &mut Arc<dyn RealToComplex<f32>>,
        time_buffer: &mut [f32],
        frequency_buffer: &mut [Complex<f32>],
        magnitude_buffer: &mut [f32],
    ) {
        // Step 1: Calculate power-of-two size
        let sample_count = payload.stable_sample_count.max(2048);
        let fft_size = 1 << (usize::BITS - 1 - sample_count.leading_zeros());

        if fft_instance.len() != fft_size {
            *fft_instance = planner.plan_fft_forward(fft_size);
        }

        // Apply Hann window and copy to scratch
        crate::algorithms::spectral::perform_fft(
            &payload.stable_buffer[..fft_size],
            &mut time_buffer[..fft_size],
            &mut frequency_buffer[..(fft_size / 2 + 1)],
            fft_instance,
            fft_size,
        );

        crate::algorithms::spectral::spectrum_to_magnitudes(
            &frequency_buffer[..],
            fft_size,
            &mut magnitude_buffer[..(fft_size / 2)],
        );

        let measured_key_index = payload.target_note;
        let hz_per_bin = payload.sample_rate as f32 / fft_size as f32;

        let f0_et = crate::models::NOTES[measured_key_index as usize].frequency;
        let expected_beta = crate::models::get_expected_beta(measured_key_index);

        // If the real-time Goertzel Engine successfully tracked the note, use its highly
        // accurate frequency as the seed. Otherwise, fall back to the mathematically
        // perfect Equal Temperament frequency for this key.
        let actual_seed = match payload.measured_f0 {
            Some(tracked_f0) => tracked_f0,
            None => f0_et,
        };

        // Step 3: Run MAT via process_payload decoupled procedure
        let mut partial_freqs_out = [0.0; 12];
        let mut partial_ns_out = [0u32; 12];

        // This calculates beta naturally
        let mut partials = Vec::new();
        let mut calculated_b = expected_beta;

        // TODO: Remove this dynamic is_bass flag when we upgrade from Quinn to CSPE.
        // CSPE will handle high-res peak extraction universally regardless of register.
        let is_bass = payload.target_note < 40;

        let mat_res = crate::algorithms::mat::detect_pitch_mat(
            magnitude_buffer,
            payload.sample_rate,
            actual_seed, // Unified Goertzel Seed
            expected_beta,
            is_bass,
            &mut partial_freqs_out,
            &mut partial_ns_out,
        );

        if let Some((_, p_count)) = mat_res {
            // Because detect_pitch_mat doesn't currently return the paired-up Beta array nicely,
            // we will just use basic pairwise comparison ourselves here to find beta
            // Or just store the fallback expected_beta

            // To be accurate, let's just do a quick Beta combination from the extracted partial array:
            let mut b_sum = 0.0;
            let mut pairs = 0;
            for i in 0..p_count {
                for j in (i + 1)..p_count {
                    let f_m = partial_freqs_out[i];
                    let n_m = partial_ns_out[i];
                    let f_n = partial_freqs_out[j];
                    let n_n = partial_ns_out[j];

                    let k_m = (f_m / n_m as f32).powi(2);
                    let k_n = (f_n / n_n as f32).powi(2);
                    let denom = k_m * (n_n as f32).powi(2) - k_n * (n_m as f32).powi(2);
                    if denom.abs() > 1e-8 {
                        let b = (k_n - k_m) / denom;
                        if b > -0.001 && b < 0.01 {
                            b_sum += b;
                            pairs += 1;
                        }
                    }
                }
            }
            if pairs > 0 {
                calculated_b = b_sum / pairs as f32;
            }

            for i in 0..p_count {
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
            calculated_b: Some(calculated_b),
            last_captured: format!("{}", now), // Basic string timestamp
        };

        // Step 4: Write Diagnostic Dump
        Self::write_diagnostics(
            &payload,
            &measurement,
            fft_size,
            payload.sample_rate as f32 / fft_size as f32,
        );

        // Step 5: Clean up and send result
        let _ = result_tx.try_send(measurement);

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
    ) {
        let (key_name, _) = crate::models::find_nearest_note_by_index(measurement.key_index);

        // Use measurement's organically resolved key_index
        let mut dir = PathBuf::from("diagnostics");
        dir.push(format!("key_{:03}_{}", measurement.key_index, key_name));

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
                && let Ok(mut f_full) = fs::File::create(file_full) {
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
