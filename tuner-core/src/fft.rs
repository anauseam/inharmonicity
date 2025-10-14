//! # Fast Fourier Transform (FFT) Module
//! 
//! This module provides high-performance FFT processing for real-time audio analysis.
//! It handles frequency domain transformations, windowing functions, and spectrum
//! magnitude calculations for piano tuning applications.
//! 
//! ## Features
//! - High-performance FFT using RustFFT
//! - Hann windowing for reduced spectral leakage
//! - DC offset removal for accurate analysis
//! - Optimized for real-time processing

use rustfft::{num_complex::Complex, FftPlanner};
use crate::audio::BUFFER_SIZE;

/// Removes the DC offset from a signal by making its average value zero.
/// 
/// DC offset can cause issues in frequency analysis by introducing
/// a large component at 0 Hz. This function centers the signal
/// around zero for more accurate frequency analysis.
/// 
/// # Arguments
/// * `signal` - Audio signal to process (modified in-place)
fn remove_dc_offset(signal: &mut [f32]) {
    let len = signal.len();
    if len == 0 { return; }
    let avg = signal.iter().sum::<f32>() / len as f32;
    if avg.abs() > 1e-6 {
        for sample in signal.iter_mut() {
            *sample -= avg;
        }
    }
}

/// Applies a Hann window to the input buffer to reduce spectral leakage.
/// 
/// The Hann window reduces spectral leakage by tapering the signal
/// to zero at the edges. This is essential for accurate frequency
/// analysis of piano tones.
/// 
/// # Arguments
/// * `buffer` - Audio buffer to window (modified in-place)
fn apply_hann_window(buffer: &mut [f32]) {
    let n = buffer.len();
    if n == 0 { return; }
    let n_minus_1 = (n - 1) as f32;
    for (i, sample) in buffer.iter_mut().enumerate() {
        let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n_minus_1).cos());
        *sample *= multiplier;
    }
}

/// Performs a forward FFT on a signal and returns the complex spectrum.
/// 
/// This is the primary FFT function for the application. It processes
/// the input signal through the following steps:
/// 1. DC offset removal
/// 2. Hann windowing
/// 3. Forward FFT transformation
/// 
/// # Arguments
/// * `signal` - Input audio signal (must be exactly BUFFER_SIZE samples)
/// 
/// # Returns
/// * `Vec<Complex<f32>>` - Complex frequency spectrum
/// 
/// # Panics
/// * If signal length is not equal to BUFFER_SIZE
pub fn perform_fft(signal: &[f32]) -> Vec<Complex<f32>> {
    if signal.len() != BUFFER_SIZE {
        panic!("Input frame size must be equal to BUFFER_SIZE");
    }

    let mut processed_signal = signal.to_vec();
    remove_dc_offset(&mut processed_signal);
    apply_hann_window(&mut processed_signal);

    let mut planner = FftPlanner::new();
    let fft = planner.plan_fft_forward(BUFFER_SIZE);

    let mut buffer: Vec<Complex<f32>> = processed_signal
        .into_iter()
        .map(|sample| Complex { re: sample, im: 0.0 })
        .collect();

    fft.process(&mut buffer);
    buffer
}

/// Calculates the magnitude vector from a complex spectrum for spectrogram display.
/// 
/// This function extracts the magnitude (amplitude) information from the
/// complex FFT results. Due to the Nyquist theorem, we only need the first
/// half of the spectrum (up to the Nyquist frequency).
/// 
/// # Arguments
/// * `spectrum` - Complex frequency spectrum from FFT
/// 
/// # Returns
/// * `Vec<f32>` - Magnitude spectrum for visualization
pub fn spectrum_to_magnitudes(spectrum: &[Complex<f32>]) -> Vec<f32> {
    spectrum
        .iter()
        .take(BUFFER_SIZE / 2)
        .map(|c| c.norm()) // .norm() is sqrt(re^2 + im^2)
        .collect()
}