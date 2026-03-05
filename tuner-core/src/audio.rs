//! # Audio Capture Module
//!
//! This module handles real-time audio capture using CPAL (Cross-Platform Audio Library).
//! It provides functions for setting up audio streams, selecting appropriate devices,
//! and streaming audio data to the analysis pipeline.
//!
//! ## Features
//! - Automatic audio device selection
//! - Configurable sample rates and formats
//! - Real-time audio streaming with buffering
//! - Error handling and device fallback

use anyhow::{Result, anyhow};
use cpal::SupportedStreamConfigRange;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use ringbuf::traits::Producer;

/// Audio buffer size for processing frames.
///
/// This constant defines the number of samples per audio frame.
/// Larger buffers provide more frequency resolution but increase latency.
pub const BUFFER_SIZE: usize = 2048;

/// Starts audio capture from the default input device.
///
/// This function:
/// 1. Selects the default audio input device
/// 2. Configures the audio stream for optimal piano tuning
/// 3. Sets up a callback to stream audio data to the analysis pipeline
///
/// # Arguments
/// * `mut producer` - Lock-free ring buffer producer for streaming audio data to the analysis thread
///
/// # Returns
/// * `Ok((stream, sample_rate))` - Audio stream handle and sample rate
/// * `Err(e)` - Error if audio setup fails
///
/// # Audio Configuration
/// - Sample Rate: 44.1 kHz (CD quality)
/// - Format: 32-bit float
/// - Channels: Mono (1 channel)
/// - Buffer Size: 2048 samples (~46ms at 44.1kHz)
pub fn start_audio_capture(
    mut producer: impl Producer<Item = f32> + Send + 'static,
) -> Result<(cpal::Stream, u32)> {
    // ... (device and config selection code is the same)
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?;

    println!("Using audio input device: {:?}", device.description()?);

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, 44100)
        .ok_or_else(|| anyhow!("No suitable f32 input format found"))?;

    let sample_rate = 44100;
    let config = supported_config.with_sample_rate(sample_rate);

    let sample_rate_val = config.sample_rate();
    let config: cpal::StreamConfig = config.into();

    println!("Selected sample rate: {} Hz", sample_rate_val);

    let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Push slice of floats lock-free into the ring buffer
            let _pushed = producer.push_slice(data);
            // In a real-time audio thread, if the buffer is full (PushError),
            // we have no choice but to drop samples since we cannot block.
            // A properly sized buffer like ours (e.g. 8x or 16x BUFFER_SIZE) avoids this.
        },
        err_fn,
        None,
    )?;

    stream.play()?;

    Ok((stream, sample_rate_val))
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
fn find_supported_config(
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
