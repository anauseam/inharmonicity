//! # Audio Capture Module
//!
//! This module handles real-time audio capture using CPAL (Cross-Platform Audio Library).
//! It provides functions for setting up audio streams, selecting appropriate devices,
//! and streaming clean, DC-free audio data to the analysis pipeline.
//!
//! ## Features
//! - Automatic audio device selection
//! - Configurable sample rates and formats
//! - Real-time audio streaming with buffering
//! - Always-on DC offset removal
//! - Error handling and device fallback
//!
//! ## Standalone Host Extension
//!
//! The [`spawn_analysis_thread()`] function provides a turnkey solution for standalone
//! applications. It creates an [`AudioPipeline`], opens a CPAL stream (or accepts an
//! external audio consumer via [`AudioSource`]), spawns a dedicated analysis thread,
//! and returns a [`HostHandle`] with everything the frontend needs.
//!
//! This is **optional to use** — VST/plugin hosts that drive their own audio thread
//! can call [`AudioPipeline::push_audio()`] directly and ignore this module's
//! thread-spawning facilities entirely.

use anyhow::{Result, anyhow};
use cpal::SupportedStreamConfigRange;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use ringbuf::{
    HeapRb,
    traits::{Consumer, Observer, Producer, Split},
};
use std::thread::{self, JoinHandle};

use crate::AnalysisResult;
use crate::pipeline::{AudioPipeline, PipelineHandle};

/// The standard analysis window size (samples).
/// Used by the Gatekeeper, Scout, and all Engine paths.
pub const WINDOW_SIZE: usize = 2048;

/// The expanded analysis window size for extracting exact bass fundamental frequencies.
pub const BASS_WINDOW_SIZE: usize = WINDOW_SIZE * 4; // 8192 samples

/// Hop size for overlapping frame analysis (50% overlap of WINDOW_SIZE).
/// Each hop triggers a new FFT + pipeline frame.
pub const HOP_SIZE: usize = WINDOW_SIZE / 2; // 1024 samples

/// Capacity of the lock-free ring buffer between the CPAL capture thread and
/// the analysis thread, in samples.
///
/// 8 × WINDOW_SIZE = 16,384 samples (~371 ms at 44.1 kHz).
/// This headroom ensures the real-time callback never drops samples even if
/// the analysis thread hits a scheduling spike.
pub const RING_BUFFER_CAPACITY: usize = WINDOW_SIZE * 8;

/// The target sample rate for the application in Hz.
pub const SAMPLE_RATE: u32 = 44100;

/// Consumer half of the audio ring buffer.
///
/// Returned by [`open_input_stream`] and [`start_audio_capture`]. The analysis thread
/// holds this end and pops samples whenever it is ready to process a new frame.
/// Callers do not need to import `ringbuf` directly.
pub type AudioConsumer = ringbuf::HeapCons<f32>;

/// DC blocking filter coefficient.
///
/// α = 0.995 → ~3.5 Hz cutoff at 44.1 kHz — well below A0 (27.5 Hz).
const DC_BLOCK_ALPHA: f32 = 0.995;

// ─── Shared CPAL Stream Setup ────────────────────────────────────────────────

/// Opens a CPAL input stream with DC blocking and a ring buffer of `capacity` samples.
///
/// This is the single source of truth for CPAL device negotiation, DC-filtered
/// ring buffer construction, and stream activation. Both [`start_audio_capture()`]
/// and the calibration module call this helper instead of duplicating the logic.
///
/// The CPAL callback is allocation-free: it only applies the DC blocking filter
/// and pushes filtered samples into the ring buffer via `try_push`. If the buffer
/// is full, samples are silently dropped to avoid blocking the real-time thread.
///
/// # Arguments
/// * `capacity` — Ring buffer capacity in samples. Use [`RING_BUFFER_CAPACITY`]
///   for analysis streams, or a smaller value (e.g., `WINDOW_SIZE * 4`) for
///   short-lived calibration streams.
///
/// # Returns
/// * `Ok((stream, consumer, sample_rate))` — Active CPAL stream, ring buffer
///   consumer, and the negotiated sample rate in Hz.
/// * `Err(e)` — If no input device is available or config negotiation fails.
pub fn open_input_stream(capacity: usize) -> Result<(cpal::Stream, AudioConsumer, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?;

    eprintln!("Using audio input device: {:?}", device.description()?);

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, SAMPLE_RATE)
        .ok_or_else(|| anyhow!("No suitable f32 input format found"))?;

    let config = supported_config.with_sample_rate(SAMPLE_RATE);

    let sample_rate_val = config.sample_rate();
    let config: cpal::StreamConfig = config.into();

    eprintln!("Selected sample rate: {} Hz", sample_rate_val);

    let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

    // Create the lock-free ring buffer. The producer moves into the CPAL callback
    // (real-time thread); the consumer is returned to the caller.
    let rb = HeapRb::<f32>::new(capacity);
    let (mut producer, consumer) = rb.split();

    // DC blocking filter state — captured by the closure below.
    let mut dc_prev_x: f32 = 0.0;
    let mut dc_prev_y: f32 = 0.0;

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            for &sample in data {
                let filtered = dc_block(sample, &mut dc_prev_x, &mut dc_prev_y);

                // Push filtered sample lock-free into the ring buffer.
                // If the buffer is full, drop the sample — we cannot block
                // in a real-time audio callback.
                let _ = producer.try_push(filtered);
            }
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    Ok((stream, consumer, sample_rate_val))
}

// ─── Standalone Host Extension ───────────────────────────────────────────────

/// Determines where audio samples come from.
///
/// Use `Default` for standalone apps (opens CPAL internally via [`open_input_stream()`]).
/// Use `External` when the caller provides their own audio consumer
/// (e.g., a VST host feeding samples, or a test harness with pre-recorded audio).
pub enum AudioSource {
    /// Opens a CPAL stream internally with the standard ring buffer capacity.
    Default,
    /// Caller provides a pre-existing ring buffer consumer and the sample rate.
    External {
        /// The consumer end of a ring buffer already being fed by an external source.
        consumer: AudioConsumer,
        /// The sample rate of the external audio source in Hz.
        sample_rate: u32,
    },
}

/// Control handle returned by [`spawn_analysis_thread()`].
///
/// The frontend holds this handle to receive analysis results, access pipeline
/// shared state, and shut down the audio system cleanly.
///
/// When this handle is dropped, the CPAL stream (if any) is dropped with it,
/// which stops the hardware capture. Call [`stop()`](HostHandle::stop) first
/// to cleanly signal the analysis thread to exit.
pub struct HostHandle {
    /// Receive analysis results from the analysis thread.
    ///
    /// Currently uses `crossbeam_channel` — will migrate to `rtrb` + `triple_buffer`
    /// during the cross-thread communication architecture refactor.
    pub analysis_rx: Receiver<AnalysisResult>,

    /// Frontend-side handle to the pipeline's shared state.
    ///
    /// Use this to read `RuntimeState` (RMS, NHWRSF) and write `ConfigState`
    /// (silence threshold, key hint, etc.).
    pub pipeline_handle: PipelineHandle,

    /// Send a signal to shut down the analysis thread.
    shutdown_tx: Sender<()>,

    /// Keep the CPAL stream alive for the lifetime of the host.
    /// `None` when using `AudioSource::External`.
    _stream: Option<cpal::Stream>,

    /// Join handle for the analysis thread, used for clean shutdown.
    thread_handle: Option<JoinHandle<()>>,
}

impl HostHandle {
    /// Signals the analysis thread to stop and waits for it to finish.
    ///
    /// This must be called before dropping the handle to ensure the audio
    /// thread exits cleanly (preventing CPAL/ALSA segfaults on shutdown).
    pub fn stop(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(handle) = self.thread_handle.take() {
            eprintln!("[HOST] Waiting for analysis thread to finish...");
            let _ = handle.join();
            eprintln!("[HOST] Analysis thread finished.");
        }
    }
}

impl Drop for HostHandle {
    fn drop(&mut self) {
        // Ensure the analysis thread is stopped if the user forgot to call stop().
        if self.thread_handle.is_some() {
            self.stop();
        }
    }
}

// `HostHandle` holds a `JoinHandle` and `cpal::Stream`, which are not `Debug`.
impl std::fmt::Debug for HostHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostHandle")
            .field("has_stream", &self._stream.is_some())
            .field(
                "thread_alive",
                &self
                    .thread_handle
                    .as_ref()
                    .map_or(false, |h| !h.is_finished()),
            )
            .finish()
    }
}

/// Spawns a dedicated analysis thread that polls audio and feeds the pipeline.
///
/// This is the turnkey entry point for standalone applications. It:
///
/// 1. Creates a fresh [`AudioPipeline`] and its [`PipelineHandle`].
/// 2. Opens a CPAL stream (for [`AudioSource::Default`]) or accepts an external
///    audio consumer (for [`AudioSource::External`]).
/// 3. Spawns a dedicated analysis thread that polls the ring buffer consumer
///    and calls [`AudioPipeline::push_audio()`].
/// 4. Returns a [`HostHandle`] with the analysis result receiver, pipeline handle,
///    and shutdown control.
///
/// The CPAL callback thread remains **allocation-free** — it only pushes raw
/// samples into the ring buffer. All DSP work (FFT, Gatekeeper, Engine) runs
/// on the spawned analysis thread, which is a normal OS thread.
///
/// # Arguments
/// * `source` — Where to get audio samples from. Use [`AudioSource::Default`]
///   for standalone apps, or [`AudioSource::External`] for VST/plugin hosts.
///
/// # Returns
/// * `Ok(HostHandle)` — The control handle for the frontend.
/// * `Err(e)` — If audio device setup fails (only for `AudioSource::Default`).
///
/// # Example
/// ```no_run
/// use tuner_core::audio::{AudioSource, spawn_analysis_thread};
///
/// let mut handle = spawn_analysis_thread(AudioSource::Default).unwrap();
///
/// // Poll for results on the GUI thread
/// while let Ok(result) = handle.analysis_rx.try_recv() {
///     println!("Detected: {:?}", result.detected_frequency);
/// }
///
/// // Clean shutdown
/// handle.stop();
/// ```
pub fn spawn_analysis_thread(source: AudioSource) -> Result<HostHandle> {
    let (pipeline, pipeline_handle) = AudioPipeline::new();

    // Resolve the audio source — either open CPAL or use the provided consumer.
    let (stream, consumer, sample_rate) = match source {
        AudioSource::Default => {
            let (stream, consumer, sr) = open_input_stream(RING_BUFFER_CAPACITY)?;
            (Some(stream), consumer, sr)
        }
        AudioSource::External {
            consumer,
            sample_rate,
        } => (None, consumer, sample_rate),
    };

    // Crossbeam channels for analysis results and shutdown signaling.
    // TODO: Migrate to rtrb + triple_buffer per cross-thread communication rules.
    let (analysis_tx, analysis_rx) = crossbeam_channel::unbounded();
    let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);

    let thread_handle = thread::spawn(move || {
        eprintln!("[HOST] Analysis thread started.");

        let mut pipeline = pipeline;
        pipeline.engine.sample_rate = sample_rate;

        let mut consumer = consumer;

        // Fixed-size stack array — no heap allocation.
        // 512 × f32 = 2 KB, well within stack budget.
        let mut pop_buf = [0.0_f32; 512];

        // Track state for debug logging
        let mut last_logged_state = pipeline.gatekeeper.current_state.clone();

        // Add a small delay to let GUI initialize
        std::thread::sleep(std::time::Duration::from_millis(100));

        loop {
            // 1. Check for shutdown signal (non-blocking)
            if shutdown_rx.try_recv().is_ok() {
                eprintln!("[HOST] Received shutdown signal.");
                break;
            }

            // 2. Lock-free ring buffer polling
            let available = consumer.occupied_len().min(pop_buf.len());
            if available > 0 {
                consumer.pop_slice(&mut pop_buf[..available]);

                let (engine_result, spectrogram_data) =
                    if let Some(res) = pipeline.push_audio(&pop_buf[..available]) {
                        res
                    } else {
                        continue; // Hop boundary not reached yet
                    };

                // Build analysis result
                let (detected_frequency, confidence) = match engine_result {
                    Some((freq, conf)) => (Some(freq), conf),
                    None => (None, None),
                };

                let (cents_deviation, note_name) = if let Some(freq) = detected_frequency {
                    let (name, target_freq) = crate::models::find_nearest_note(freq);
                    let deviation =
                        crate::algorithms::tuning::calculate_cents_deviation(freq, target_freq);
                    (Some(deviation), Some(name))
                } else {
                    (None, None)
                };

                let result = AnalysisResult {
                    detected_frequency,
                    confidence,
                    cents_deviation,
                    note_name,
                    spectrogram_data,
                    partials: vec![],
                };

                // Log Gatekeeper state transitions
                if pipeline.gatekeeper.current_state != last_logged_state {
                    eprintln!(
                        "[GATEKEEPER] Transition: {:?} -> {:?} (CSD Dly: {}, Stable Cnt: {})",
                        last_logged_state,
                        pipeline.gatekeeper.current_state,
                        pipeline.gatekeeper.transient_delay_counter,
                        pipeline.gatekeeper.stable_counter
                    );
                    last_logged_state = pipeline.gatekeeper.current_state.clone();
                }

                // Send result to the frontend. If the receiver is dropped, exit.
                if analysis_tx.send(result).is_err() {
                    eprintln!("[HOST] Analysis receiver dropped — exiting.");
                    break;
                }
            } else {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }

        eprintln!("[HOST] Analysis thread exiting.");
    });

    Ok(HostHandle {
        analysis_rx,
        pipeline_handle,
        shutdown_tx,
        _stream: stream,
        thread_handle: Some(thread_handle),
    })
}

// ─── DSP Utilities ───────────────────────────────────────────────────────────

/// Applies one step of the DC blocking high-pass IIR filter.
///
/// Removes hardware-dependent DC offset from a single sample.
/// `prev_x` and `prev_y` are the filter's persistent state and must be
/// initialized to `0.0` before the first call.
///
/// Transfer function: `y[n] = x[n] - x[n-1] + α·y[n-1]`
pub(crate) fn dc_block(sample: f32, prev_x: &mut f32, prev_y: &mut f32) -> f32 {
    let y = sample - *prev_x + DC_BLOCK_ALPHA * *prev_y;
    *prev_x = sample;
    *prev_y = y;
    y
}

/// Finds the best supported audio configuration for the target sample rate.
///
/// This function searches through available audio configurations and selects
/// the one that best matches our requirements:
/// - Mono channel (1 channel)
/// - 32-bit float format
/// - Closest sample rate to target
///
/// # Arguments
/// * `configs` - List of supported audio configurations from the device
/// * `target_rate` - Desired sample rate in Hz
///
/// # Returns
/// * `Some(config)` - Best matching configuration
/// * `None` - No suitable configuration found
pub(crate) fn find_supported_config(
    configs: Vec<SupportedStreamConfigRange>,
    target_rate: u32,
) -> Option<SupportedStreamConfigRange> {
    configs
        .into_iter()
        .filter(|c| c.channels() == 1 && c.sample_format() == cpal::SampleFormat::F32)
        .min_by_key(|c| {
            let min_diff = (c.min_sample_rate() as i32 - target_rate as i32).abs();
            let max_diff = (c.max_sample_rate() as i32 - target_rate as i32).abs();
            min_diff.min(max_diff)
        })
}
