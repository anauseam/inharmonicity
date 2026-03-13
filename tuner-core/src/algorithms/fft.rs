//! # Fast Fourier Transform (FFT) Module
//!
//! This module provides high-performance FFT processing for real-time audio analysis.
//! It handles frequency domain transformations, windowing functions, and spectrum
//! magnitude calculations for piano tuning applications.
//!
//! ## Features
//! - High-performance FFT using RustFFT
//! - Hann windowing for reduced spectral leakage
//! - Optimized for real-time processing

use crate::audio::BUFFER_SIZE;
use rustfft::{Fft, num_complex::Complex};
use std::sync::Arc;

/// Performs an in-place forward FFT on a signal into a complex buffer.
///
/// This is the primary FFT function for the application. It processes
/// the input signal directly into the provided frequency buffer through:
/// 1. Hann windowing
/// 2. Forward FFT transformation
///
/// DC offset removal is handled upstream by the audio stream's `dc_block` filter,
/// so all samples arriving here are already zero-mean.
///
/// # Arguments
/// * `signal` - Input audio signal (must be exactly BUFFER_SIZE samples)
/// * `frequency_buffer` - Pre-allocated buffer for the FFT output (mutated in-place)
/// * `fft_instance` - A pre-planned FFT instance from `FftPlanner`
///
/// # Panics
/// * If signal length is not equal to BUFFER_SIZE
pub fn perform_fft(
    signal: &[f32],
    frequency_buffer: &mut [Complex<f32>],
    fft_instance: &Arc<dyn Fft<f32>>,
) {
    if signal.len() != BUFFER_SIZE || frequency_buffer.len() < BUFFER_SIZE {
        panic!("Input frame size and frequency buffer must be at least BUFFER_SIZE");
    }

    let n_minus_1 = (signal.len() - 1) as f32;

    for (i, (&sample, complex)) in signal.iter().zip(frequency_buffer.iter_mut()).enumerate() {
        let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n_minus_1).cos());
        complex.re = sample * multiplier;
        complex.im = 0.0;
    }

    fft_instance.process(frequency_buffer);
}

/// Calculates the magnitude vector from a complex spectrum.
///
/// This function extracts the magnitude (amplitude) information from the
/// complex FFT results. Due to the Nyquist theorem, we only need the first
/// half of the spectrum (up to the Nyquist frequency).
///
/// The resulting magnitudes are fundamental to the audio processing pipeline.
/// They are used both for visual rendering (e.g., spectrogram displays in the GUI)
/// and for downstream DSP algorithms (e.g., refining pitch estimates with
/// parabolic interpolation and tracking harmonic partials).
///
/// # Arguments
/// * `spectrum` - Complex frequency spectrum from FFT
///
/// # Returns
/// * `Vec<f32>` - Magnitude spectrum
pub fn spectrum_to_magnitudes(spectrum: &[Complex<f32>]) -> Vec<f32> {
    // Note: If you want to use the passed frame_size instead of BUFFER_SIZE, we can add it here.
    // The previous tuner-core version used BUFFER_SIZE. So we'll use BUFFER_SIZE.
    spectrum
        .iter()
        .take(BUFFER_SIZE / 2)
        .map(|c| c.norm()) // .norm() is sqrt(re^2 + im^2)
        .collect()
}
