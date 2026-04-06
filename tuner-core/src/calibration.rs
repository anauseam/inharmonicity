//! # Calibration
//!
//! Standalone calibration routines that measure specific signal properties
//! and return structured results. These functions can either open their own
//! temporary CPAL audio stream (via [`AudioSource::Default`]) or accept an
//! externally provided audio consumer (via [`AudioSource::External`]).
//!
//! ## Functions
//!
//! | Function | Wizard Step | Measures | Returns |
//! |---|---|---|---|
//! | [`calibrate_noise_floor`] | 1 — Room Tone | Peak NHWRSF + RMS during silence | [`NoiseFloorResult`] |
//! | [`calibrate_minimum_strike`] | 2 — Softest Strike | Peak NHWRSF of a single transient | [`StrikeResult`] |
//!
//! ## Usage (from the GUI via `Task::perform`)
//!
//! ```text
//! // Step 1 — run at startup or on recalibrate:
//! let result = calibrate_noise_floor(AudioSource::Default, 1.0, 43, progress_arc)?;
//! // result.rms_threshold → ConfigState.silence_threshold
//! // result.nhwrsf_peak  → N_max (wizard lower bound)
//!
//! // Step 2 — run when the user is ready
//! let (tx, rx) = std::sync::mpsc::channel();
//! let result = calibrate_minimum_strike(AudioSource::Default, noise_peak, 5, progress_arc, tx)?;
//! // Poll rx for flux values to drive the seismograph
//! // result.nhwrsf_peak → S_min (wizard upper bound)
//! ```

use anyhow::Result;
use realfft::RealFftPlanner;
use ringbuf::traits::{Consumer, Observer};
use rustfft::num_complex::Complex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::algorithms::metrics::{calculate_ema, calculate_nhwrsf, calculate_rms};
use crate::audio::{AudioConsumer, AudioSource, WINDOW_SIZE};

// ─── Constants ────────────────────────────────────────────────────────────────

/// Default number of calibration frames (~2 seconds at 44.1 kHz / 2048 samples).
pub const DEFAULT_CALIBRATION_FRAMES: usize = 43;

/// Default multiplier applied to the measured ambient RMS noise.
/// Require the signal to be 1× louder than the room's background noise by default.
pub const DEFAULT_NOISE_MULTIPLIER: f32 = 1.0;

/// Default maximum seconds to wait for a key strike during strike calibration.
pub const DEFAULT_STRIKE_TIMEOUT_SECS: u64 = 5;

/// Number of frames to discard at stream open before measuring.
///
/// When a CPAL stream is first opened, the audio driver's AGC and the
/// hardware ADC need time to stabilize. Dropping these frames prevents
/// the warm-up transient from inflating the measured values.
const WARMUP_FRAMES: usize = 10;

/// Ring buffer capacity for calibration streams.
///
/// 4 × WINDOW_SIZE = 8,192 samples (~186 ms at 44.1 kHz).
/// Only needs to hold a few frames of headroom since calibration
/// processes frames as fast as they arrive.
const CALIBRATION_RING_CAPACITY: usize = WINDOW_SIZE * 4;

/// EMA smoothing factor used during calibration (same as the Gatekeeper's).
const CALIBRATION_EMA_ALPHA: f32 = 0.1;

// ─── Result Types ─────────────────────────────────────────────────────────────

/// Result of a noise-floor calibration pass (Wizard Step 1 — Room Tone).
#[derive(Debug, Clone, Copy)]
pub struct NoiseFloorResult {
    /// Raw ambient EMA-smoothed RMS average across all collected frames.
    /// The GUI uses this as a reference for computing the silence slider range.
    pub rms_baseline: f32,
    /// Silence threshold = `rms_baseline × multiplier`.
    /// Write to `ConfigState.silence_threshold`.
    pub rms_threshold: f32,
    /// Peak NHWRSF observed during the silent period ($N_{max}$).
    /// This is the lower bound of the transient threshold slider in the wizard.
    pub noise_floor_peak: f32,
}

/// Result of a minimum strike calibration pass (Wizard Step 2 — Softest Strike).
#[derive(Debug, Clone, Copy)]
pub struct StrikeResult {
    /// Peak NHWRSF observed during the transient event ($S_{min}$).
    /// This is the upper bound of the transient threshold slider in the wizard.
    pub nhwrsf_peak: f32,
}

// ─── Private Helpers ─────────────────────────────────────────────────────────

/// Resolves an [`AudioSource`] into a concrete consumer for calibration.
///
/// For `AudioSource::Default`, opens a temporary CPAL stream via
/// [`audio::open_input_stream()`] with a small calibration-sized ring buffer.
/// For `AudioSource::External`, uses the provided consumer directly.
///
/// # Returns
/// A tuple of:
/// - An optional CPAL stream (must be kept alive for the duration of calibration)
/// - The audio consumer to pop frames from
fn resolve_source(source: AudioSource) -> Result<(Option<cpal::Stream>, AudioConsumer)> {
    match source {
        AudioSource::Default => {
            let (stream, consumer, _sample_rate) =
                crate::audio::open_input_stream(CALIBRATION_RING_CAPACITY)?;
            Ok((Some(stream), consumer))
        }
        AudioSource::External {
            consumer,
            sample_rate: _,
        } => Ok((None, consumer)),
    }
}

/// Discards `WARMUP_FRAMES` frames from `consumer` to let the hardware/driver settle.
fn drain_warmup(consumer: &mut AudioConsumer, frame_buffer: &mut [f32]) {
    let mut done = 0;
    while done < WARMUP_FRAMES {
        if consumer.occupied_len() >= WINDOW_SIZE {
            consumer.pop_slice(frame_buffer);
            done += 1;
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

// ─── Public Calibration Functions ────────────────────────────────────────────

/// Wizard Step 1: Measures the ambient room noise during a period of silence.
///
/// Collects `num_frames` frames to calculate:
/// - The EMA-smoothed RMS average (→ silence gate threshold)
/// - The peak NHWRSF observed ($N_{max}$ — the noise ceiling for transient detection)
///
/// This is a **blocking** function (~2 seconds with default `num_frames = 43`).
/// Wrap it in `Task::perform` (or equivalent) on the GUI side.
///
/// # Arguments
/// * `source` — Where to get audio from. Use [`AudioSource::Default`] for standalone
///   apps, or [`AudioSource::External`] to feed pre-recorded/routed audio.
/// * `multiplier` — Safety multiplier applied to the ambient RMS
///   (e.g., `1.0` means threshold = ambient RMS exactly)
/// * `num_frames` — Number of audio frames to collect
/// * `progress` — Atomic counter incremented after each frame (GUI progress bar)
///
/// # Returns
/// [`NoiseFloorResult`] containing the RMS threshold and $N_{max}$ NHWRSF peak.
pub fn calibrate_noise_floor(
    source: AudioSource,
    multiplier: f32,
    num_frames: usize,
    progress: Arc<AtomicUsize>,
) -> Result<NoiseFloorResult> {
    eprintln!(
        "[CALIBRATION] Starting noise floor calibration ({} frames)...",
        num_frames
    );

    let (_stream, mut consumer) = resolve_source(source)?;

    // FFT setup — RFFT produces WINDOW_SIZE/2 + 1 complex bins
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    let mut freq_buf: Vec<Complex<f32>> = fft.make_output_vec();
    let mut prev_mags = vec![0.0f32; freq_buf.len()];

    let mut frame_buffer = vec![0.0f32; WINDOW_SIZE];

    drain_warmup(&mut consumer, &mut frame_buffer);

    // Measurement pass
    let mut rms_ema: f32 = 0.0;
    let mut rms_sum: f32 = 0.0;
    let mut noise_floor_peak: f32 = 0.0;
    let mut frames_collected: usize = 0;

    while frames_collected < num_frames {
        if consumer.occupied_len() >= WINDOW_SIZE {
            consumer.pop_slice(&mut frame_buffer);

            // RMS — silence gate
            let rms = calculate_rms(&frame_buffer);
            rms_ema = calculate_ema(rms, rms_ema, CALIBRATION_EMA_ALPHA);
            rms_sum += rms_ema;

            // FFT → NHWRSF noise ceiling
            let mut time_buf = frame_buffer.clone();
            fft.process(&mut time_buf, &mut freq_buf)
                .expect("FFT failed during noise floor calibration");
            let flux = calculate_nhwrsf(&freq_buf, &mut prev_mags);
            if flux > noise_floor_peak {
                noise_floor_peak = flux;
            }

            frames_collected += 1;
            progress.store(frames_collected, Ordering::Relaxed);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    let rms_baseline = rms_sum / num_frames as f32;
    let rms_threshold = rms_baseline * multiplier;

    eprintln!(
        "[CALIBRATION] Noise floor done. RMS baseline: {:.6}, threshold: {:.6} (×{:.1}), N_max NHWRSF: {:.4}",
        rms_baseline, rms_threshold, multiplier, noise_floor_peak
    );

    Ok(NoiseFloorResult {
        rms_baseline,
        rms_threshold,
        noise_floor_peak,
    })
}

/// Wizard Step 2: Captures the NHWRSF peak of the user's softest key strike.
///
/// Waits up to `timeout_secs` for a transient event whose NHWRSF value exceeds
/// `noise_ceiling` (the $N_{max}$ from Step 1). The first qualifying transient
/// is captured and returned as $S_{min}$.
///
/// Every frame, a [`StrikeCalibrationEvent::Tick`] is sent over the returned
/// channel so the GUI can update the real-time seismograph. When the transient
/// is detected, a [`StrikeCalibrationEvent::Completed`] is sent before the
/// function returns.
///
/// This is a **blocking** function. Wrap it in `Task::perform` on the GUI side.
///
/// # Arguments
/// * `source` — Where to get audio from. Use [`AudioSource::Default`] for standalone
///   apps, or [`AudioSource::External`] for routed/test audio.
/// * `noise_ceiling` — The $N_{max}$ NHWRSF peak from [`calibrate_noise_floor`].
///   A transient is considered valid only when it exceeds this value.
/// * `timeout_secs` — Maximum seconds to wait for a strike before giving up.
/// * `progress` — Atomic counter incremented each frame (GUI progress / timeout bar)
/// * `tick_tx` — Background MPSC channel to send live `f32` flux ticks to the GUI.
///
/// # Returns
/// - [`StrikeResult`] containing the peak NHWRSF of the captured strike ($S_{min}$)
///
/// # Errors
/// Returns an error if the audio stream cannot be opened or if the timeout
/// expires without a valid transient being detected.
pub fn calibrate_minimum_strike(
    source: AudioSource,
    noise_ceiling: f32,
    timeout_secs: u64,
    progress: Arc<AtomicUsize>,
    tick_tx: std::sync::mpsc::Sender<f32>,
) -> Result<StrikeResult> {
    eprintln!(
        "[CALIBRATION] Waiting for softest strike (noise ceiling: {:.4}, timeout: {}s)...",
        noise_ceiling, timeout_secs
    );

    let (_stream, mut consumer) = resolve_source(source)?;

    // FFT setup
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    let mut freq_buf: Vec<Complex<f32>> = fft.make_output_vec();
    let mut prev_mags = vec![0.0f32; freq_buf.len()];

    let mut frame_buffer = vec![0.0f32; WINDOW_SIZE];

    drain_warmup(&mut consumer, &mut frame_buffer);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let mut peak_nhwrsf: f32 = 0.0;
    let mut transient_detected = false;
    let mut frame_index: usize = 0;

    while std::time::Instant::now() < deadline {
        if consumer.occupied_len() >= WINDOW_SIZE {
            consumer.pop_slice(&mut frame_buffer);

            // FFT → NHWRSF
            let mut time_buf = frame_buffer.clone();
            fft.process(&mut time_buf, &mut freq_buf)
                .expect("FFT failed during strike calibration");
            let flux = calculate_nhwrsf(&freq_buf, &mut prev_mags);

            // Emit live tick for the seismograph
            let _ = tick_tx.send(flux);

            // Detect a valid transient (a spike above the noise ceiling)
            if flux > noise_ceiling && flux > peak_nhwrsf {
                peak_nhwrsf = flux;
                transient_detected = true;
            }

            // Once we've seen a spike and the flux has settled back below the
            // ceiling, the strike is complete — no need to keep capturing.
            if transient_detected && flux <= noise_ceiling {
                break;
            }

            frame_index += 1;
            progress.store(frame_index, Ordering::Relaxed);
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    if !transient_detected {
        return Err(anyhow::anyhow!(
            "Strike calibration timed out after {}s. No transient above noise ceiling ({:.4}) was detected.",
            timeout_secs,
            noise_ceiling
        ));
    }

    let result = StrikeResult {
        nhwrsf_peak: peak_nhwrsf,
    };
    eprintln!(
        "[CALIBRATION] Strike captured. S_min NHWRSF peak: {:.4}",
        peak_nhwrsf
    );

    Ok(result)
}
