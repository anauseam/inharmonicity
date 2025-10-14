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

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SupportedStreamConfigRange;
use crossbeam_channel::Sender;
use anyhow::{Result, anyhow};

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
/// * `sender` - Channel sender for streaming audio data to the analysis thread
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
pub fn start_audio_capture(sender: Sender<Vec<f32>>) -> Result<(cpal::Stream, u32)> {
    // ... (device and config selection code is the same)
    let host = cpal::default_host();
    let device = host.default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?;

    println!("Using audio input device: {}", device.name()?);

    let configs = device.supported_input_configs()?.collect::<Vec<_>>();
    let supported_config = find_supported_config(configs, 44100)
        .ok_or_else(|| anyhow!("No suitable f32 input format found"))?;

    let sample_rate = cpal::SampleRate(44100);
    let config = supported_config.with_sample_rate(sample_rate);
    
    let sample_rate_val = config.sample_rate().0;
    let config: cpal::StreamConfig = config.into();

    println!("Selected sample rate: {} Hz", sample_rate_val);

    let err_fn = |err| eprintln!("An error occurred on the audio stream: {}", err);

    // This buffer will accumulate audio data from the callback.
    let mut audio_buffer = Vec::with_capacity(BUFFER_SIZE * 2);

    let stream = device.build_input_stream(
        &config,
        move |data: &[f32], _: &cpal::InputCallbackInfo| {
            // Append new data to our buffer.
            audio_buffer.extend_from_slice(data);

            // While we have enough data for a full frame, process it.
            while audio_buffer.len() >= BUFFER_SIZE {
                // Take the first BUFFER_SIZE samples for processing.
                let frame_to_send = audio_buffer[..BUFFER_SIZE].to_vec();

                // Send the frame, ignoring errors if the channel is full.
                let _ = sender.try_send(frame_to_send);

                // Remove the processed samples from the front of the buffer.
                audio_buffer.drain(..BUFFER_SIZE);
            }
        },
        err_fn,
        None
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
            let min_diff = (c.min_sample_rate().0 as i32 - target_rate as i32).abs();
            let max_diff = (c.max_sample_rate().0 as i32 - target_rate as i32).abs();
            min_diff.min(max_diff)
        })
}