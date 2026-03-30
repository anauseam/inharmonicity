//! # Calibration
//!
//! Standalone calibration routines that each open their own temporary CPAL audio
//! stream, measure a specific signal property, and return a result. These functions
//! have **no dependency** on [`AudioPipeline`] or the main audio processing thread.
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
//! let result = calibrate_noise_floor(1.0, 43, progress_arc)?;
//! // result.rms_threshold → ConfigState.silence_threshold
//! // result.nhwrsf_peak  → N_max (wizard lower bound)
//!
//! // Step 2 — run when the user is ready to play a note:
//! let (result, rx) = calibrate_minimum_strike(result.nhwrsf_peak, 5, progress_arc)?;
//! // Poll rx for StrikeCalibrationEvent::Tick(flux) to drive the seismograph
//! // StrikeCalibrationEvent::Completed is sent before the function returns
//! // result.nhwrsf_peak → S_min (wizard upper bound)
//! ```

use anyhow::{Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Sender, unbounded};
use realfft::RealFftPlanner;
use ringbuf::traits::{Consumer, Observer, Producer, Split};
use rustfft::num_complex::Complex;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::algorithms::metrics::{calculate_ema, calculate_nhwrsf, calculate_rms};
use crate::audio::{WINDOW_SIZE, dc_block, find_supported_config};

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

/// Real-time events emitted by [`calibrate_minimum_strike`] over its channel.
///
/// The GUI receives these on each tick to drive the seismograph preview
/// while the user is waiting to play their note.
#[derive(Debug, Clone)]
pub enum StrikeCalibrationEvent {
    /// Emitted every frame with the current NHWRSF flux value.
    /// Use this to update the real-time seismograph visualizer.
    Tick(f32),
    /// Emitted once when a transient spike above `noise_ceiling` is captured.
    /// The inner value is the peak NHWRSF of the detected strike.
    Completed(StrikeResult),
}

// ─── Private Stream Helper ────────────────────────────────────────────────────

/// Opens a temporary CPAL input stream at 44100 Hz with DC blocking applied.
///
/// Returns the active `Stream` (keep alive for the duration of calibration)
/// and a ring buffer `Consumer` to pop audio frames from.
///
/// # Errors
/// Returns an error if no input device is available or if the device does not
/// support 44100 Hz input at the required buffer size.
fn open_calibration_stream() -> Result<(cpal::Stream, impl ringbuf::traits::Consumer<Item = f32>)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available for calibration"))?;

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, 44100)
        .ok_or_else(|| anyhow!("No suitable 44100 Hz audio config found for calibration"))?;

    let config = supported_config.with_sample_rate(44100);
    let stream_config: cpal::StreamConfig = config.into();

    // Small ring buffer — only needs to hold a few frames of headroom
    let rb = ringbuf::HeapRb::<f32>::new(WINDOW_SIZE * 4);
    let (mut producer, consumer) = rb.split();

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
    Ok((stream, consumer))
}

/// Discards `WARMUP_FRAMES` frames from `consumer` to let the hardware/driver settle.
fn drain_warmup(
    consumer: &mut impl ringbuf::traits::Consumer<Item = f32>,
    frame_buffer: &mut [f32],
) {
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
/// Opens a temporary CPAL stream, discards warm-up frames, then collects
/// `num_frames` frames to calculate:
/// - The EMA-smoothed RMS average (→ silence gate threshold)
/// - The peak NHWRSF observed ($N_{max}$ — the noise ceiling for transient detection)
///
/// This is a **blocking** function (~2 seconds with default `num_frames = 43`).
/// Wrap it in `Task::perform` (or equivalent) on the GUI side.
///
/// # Arguments
/// * `multiplier` — Safety multiplier applied to the ambient RMS
///   (e.g., `1.0` means threshold = ambient RMS exactly)
/// * `num_frames` — Number of audio frames to collect
/// * `progress` — Atomic counter incremented after each frame (GUI progress bar)
///
/// # Returns
/// [`NoiseFloorResult`] containing the RMS threshold and $N_{max}$ NHWRSF peak.
pub fn calibrate_noise_floor(
    multiplier: f32,
    num_frames: usize,
    progress: Arc<AtomicUsize>,
) -> Result<NoiseFloorResult> {
    eprintln!(
        "[CALIBRATION] Starting noise floor calibration ({} frames)...",
        num_frames
    );

    let (stream, mut consumer) = open_calibration_stream()?;

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

    drop(stream);

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
/// Opens a temporary CPAL stream and waits up to `timeout_secs` for a transient
/// event whose NHWRSF value exceeds `noise_ceiling` (the $N_{max}$ from Step 1).
/// The first qualifying transient is captured and returned as $S_{min}$.
///
/// Every frame, a [`StrikeCalibrationEvent::Tick`] is sent over the returned
/// channel so the GUI can update the real-time seismograph. When the transient
/// is detected, a [`StrikeCalibrationEvent::Completed`] is sent before the
/// function returns.
///
/// This is a **blocking** function. Wrap it in `Task::perform` on the GUI side.
///
/// # Arguments
/// * `noise_ceiling` — The $N_{max}$ NHWRSF peak from [`calibrate_noise_floor`].
///   A transient is considered valid only when it exceeds this value.
/// * `timeout_secs` — Maximum seconds to wait for a strike before giving up.
/// * `progress` — Atomic counter incremented each frame (GUI progress / timeout bar)
///
/// # Returns
/// A tuple of:
/// - [`StrikeResult`] containing the peak NHWRSF of the captured strike ($S_{min}$)
/// - [`crossbeam_channel::Receiver<StrikeCalibrationEvent>`] for live seismograph ticks
///
/// # Errors
/// Returns an error if the audio stream cannot be opened or if the timeout
/// expires without a valid transient being detected.
pub fn calibrate_minimum_strike(
    noise_ceiling: f32,
    timeout_secs: u64,
    progress: Arc<AtomicUsize>,
) -> Result<(
    StrikeResult,
    crossbeam_channel::Receiver<StrikeCalibrationEvent>,
)> {
    eprintln!(
        "[CALIBRATION] Waiting for softest strike (noise ceiling: {:.4}, timeout: {}s)...",
        noise_ceiling, timeout_secs
    );

    let (tx, rx): (Sender<StrikeCalibrationEvent>, _) = unbounded();
    let (stream, mut consumer) = open_calibration_stream()?;

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
            let _ = tx.send(StrikeCalibrationEvent::Tick(flux));

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

    drop(stream);

    if !transient_detected {
        return Err(anyhow!(
            "Strike calibration timed out after {}s. No transient above noise ceiling ({:.4}) was detected.",
            timeout_secs,
            noise_ceiling
        ));
    }

    let result = StrikeResult {
        nhwrsf_peak: peak_nhwrsf,
    };

    let _ = tx.send(StrikeCalibrationEvent::Completed(result));

    eprintln!(
        "[CALIBRATION] Strike captured. S_min NHWRSF peak: {:.4}",
        peak_nhwrsf
    );

    Ok((result, rx))
}
