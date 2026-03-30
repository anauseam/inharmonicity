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

use anyhow::{Result, anyhow};
use cpal::SupportedStreamConfigRange;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::{HeapRb, traits::{Producer, Split}};

/// The standard analysis window size (samples).
/// Used by the Gatekeeper, Scout, and all Engine paths.
pub const WINDOW_SIZE: usize = 2048;

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
/// Returned by [`start_audio_capture`]. The analysis thread holds this end
/// and pops samples whenever it is ready to process a new frame.
/// Callers do not need to import `ringbuf` directly.
pub type AudioConsumer = ringbuf::HeapCons<f32>;

/// DC blocking filter coefficient.
///
/// α = 0.995 → ~3.5 Hz cutoff at 44.1 kHz — well below A0 (27.5 Hz).
const DC_BLOCK_ALPHA: f32 = 0.995;

/// Starts audio capture from the default input device.
///
/// This function:
/// 1. Selects the default audio input device.
/// 2. Configures the audio stream for optimal piano tuning.
/// 3. Creates a lock-free ring buffer internally (capacity [`RING_BUFFER_CAPACITY`]).
/// 4. Sets up a real-time callback to stream audio data to the analysis pipeline.
///
/// The producer half is moved into the CPAL real-time callback, which must
/// remain allocation-free. The consumer half is returned to the caller.
///
/// Every sample passes through a DC blocking high-pass filter
/// (y[n] = x[n] - x[n-1] + α·y[n-1]) before entering the ring buffer,
/// removing hardware-dependent DC offset regardless of device or driver.
///
/// # Returns
/// * `Ok((stream, consumer, sample_rate))` — Stream handle, ring buffer consumer,
///   and the negotiated sample rate.
/// * `Err(e)` — Error if audio setup fails.
///
/// # Audio Configuration
/// - Sample Rate: 44.1 kHz (CD quality)
/// - Format: 32-bit float
/// - Channels: Mono (1 channel)
pub fn start_audio_capture() -> Result<(cpal::Stream, AudioConsumer, u32)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?;

    println!("Using audio input device: {:?}", device.description()?);

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, SAMPLE_RATE)
        .ok_or_else(|| anyhow!("No suitable f32 input format found"))?;

    let config = supported_config.with_sample_rate(SAMPLE_RATE);

    let sample_rate_val = config.sample_rate();
    let config: cpal::StreamConfig = config.into();

    println!("Selected sample rate: {} Hz", sample_rate_val);

    let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

    // Create the lock-free ring buffer. The producer moves into the CPAL callback
    // (real-time thread); the consumer is returned to the analysis thread.
    let rb = HeapRb::<f32>::new(RING_BUFFER_CAPACITY);
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
