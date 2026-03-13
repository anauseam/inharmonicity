//! # Noise Floor Calibration
//!
//! Standalone noise-floor calibration that opens its own temporary CPAL audio
//! stream, collects ~2 seconds of ambient room noise, and computes a silence
//! threshold. This module has **no dependency** on [`AudioPipeline`] or the
//! main audio processing thread.
//!
//! ## Usage
//!
//! ```text
//! // At startup (via iced Task::perform):
//! let result = calibrate_noise_floor(2.0, 43)?;
//!
//! // result.baseline  — raw ambient RMS average
//! // result.threshold — baseline × multiplier (the silence gate value)
//! ```
//!
//! The same function can be called again at any time from the GUI to
//! recalibrate (e.g., user moved to a different room).

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::algorithms::power::{calculate_ema, calculate_rms};
use crate::audio::{BUFFER_SIZE, dc_block, find_supported_config};

/// Default number of calibration frames (~2 seconds at 44.1 kHz / 2048 samples).
pub const DEFAULT_CALIBRATION_FRAMES: usize = 43;

/// Default multiplier applied to the measured ambient noise.
/// Require the signal to be 2× louder than the room's background noise.
pub const DEFAULT_NOISE_MULTIPLIER: f32 = 1.0;

/// Number of frames to discard at stream open before measuring.
///
/// When a CPAL stream is first opened, the audio driver's AGC and the
/// hardware ADC need time to stabilize. Dropping these frames prevents
/// the warm-up transient from inflating the measured noise floor.
const WARMUP_FRAMES: usize = 10;

/// EMA smoothing factor used during calibration (same as the Gatekeeper's).
const CALIBRATION_EMA_ALPHA: f32 = 0.1;

/// Result of a noise-floor calibration pass.
#[derive(Debug, Clone, Copy)]
pub struct CalibrationResult {
    /// Raw ambient RMS average (before multiplier).
    /// The GUI uses this as the baseline for computing the slider range.
    pub baseline: f32,
    /// Silence threshold = `baseline × multiplier`.
    /// Written to `ConfigState.silence_threshold`.
    pub threshold: f32,
}

/// Runs noise-floor calibration by opening a temporary CPAL audio stream,
/// collecting `num_frames` frames of ambient room noise, and computing
/// the silence threshold.
///
/// This is a **blocking** function that takes ~2 seconds with the default
/// `num_frames` of 43. It opens and closes its own audio stream, so it is
/// completely independent of the main audio processing thread.
///
/// # Arguments
/// * `multiplier` — Safety multiplier applied to the measured ambient noise
///   (e.g., 2.0 means "signal must be 2× louder than the room")
/// * `num_frames` — Number of audio frames to collect (each frame is
///   `BUFFER_SIZE` samples; 43 frames ≈ 2.0 seconds at 44.1 kHz)
/// * `progress` — Atomic counter incremented after each frame, allowing the
///   GUI to poll and display a live progress indicator.
///
/// # Returns
/// `Ok(CalibrationResult)` with the raw baseline and computed threshold,
/// or an error if the audio device cannot be opened.
pub fn calibrate_noise_floor(
    multiplier: f32,
    num_frames: usize,
    progress: Arc<AtomicUsize>,
) -> Result<CalibrationResult> {
    eprintln!(
        "[CALIBRATION] Starting noise floor calibration ({} frames)...",
        num_frames
    );

    // ── Open a temporary CPAL stream ─────────────────────────────────────
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available for calibration"))?;

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, 44100)
        .ok_or_else(|| anyhow!("No suitable audio config for calibration"))?;

    let config = supported_config.with_sample_rate(44100);
    let stream_config: cpal::StreamConfig = config.into();

    // Small ring buffer — only needs to hold a few frames
    let rb = ringbuf::HeapRb::<f32>::new(BUFFER_SIZE * 4);
    let (mut producer, mut consumer) = rb.split();

    let err_fn = |err| eprintln!("[CALIBRATION] Audio stream error: {}", err);

    // DC blocking filter state — same filter as the main audio stream.
    let mut dc_prev_x: f32 = 0.0;
    let mut dc_prev_y: f32 = 0.0;

    let stream = device.build_input_stream(
        &stream_config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                let _ = producer.try_push(dc_block(sample, &mut dc_prev_x, &mut dc_prev_y));
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    // ── Warm-up: discard initial frames so the driver/hardware can settle ──
    let mut frame_buffer = vec![0.0f32; BUFFER_SIZE];
    let mut warmup_done: usize = 0;

    while warmup_done < WARMUP_FRAMES {
        if consumer.occupied_len() >= BUFFER_SIZE {
            consumer.pop_slice(&mut frame_buffer);
            warmup_done += 1;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    // ── Collect frames and compute RMS ───────────────────────────────────
    let mut ema: f32 = 0.0;
    let mut rms_sum: f32 = 0.0;
    let mut frames_collected: usize = 0;

    while frames_collected < num_frames {
        if consumer.occupied_len() >= BUFFER_SIZE {
            consumer.pop_slice(&mut frame_buffer);

            let rms = calculate_rms(&frame_buffer);
            ema = calculate_ema(rms, ema, CALIBRATION_EMA_ALPHA);
            rms_sum += ema;
            frames_collected += 1;
            progress.store(frames_collected, Ordering::Relaxed);
        } else {
            // Don't spin — wait for the CPAL callback to fill the buffer
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    // ── Compute result ───────────────────────────────────────────────────
    // Explicitly drop and stop the stream before returning
    drop(stream);

    let baseline = rms_sum / num_frames as f32;
    let threshold = baseline * multiplier;

    eprintln!(
        "[CALIBRATION] Done. Ambient RMS: {:.6}, Threshold: {:.6} (×{:.1})",
        baseline, threshold, multiplier
    );

    Ok(CalibrationResult {
        baseline,
        threshold,
    })
}
