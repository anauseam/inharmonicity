//! # Fast Fourier Transform (FFT) Module
//!
//! This module provides high-performance FFT processing for real-time audio analysis.
//! It handles frequency domain transformations, windowing functions, and spectrum
//! magnitude calculations for piano tuning applications.
//!
//! ## Features
//! - Highly-optimized Real-to-Complex FFT (RFFT) using `realfft`
//! - Hamming windowing for zero-overlap transient preservation
//! - Optimized for real-time processing

use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::sync::Arc;

/// Performs an in-place forward RFFT on a real audio signal into a complex buffer.
///
/// This is the primary FFT function for the application. It leverages `realfft`
/// to process strictly real microphone data in roughly half the computational time
/// of a standard Complex-to-Complex FFT.
///
/// 1. Hamming windowing (preserves 8% amplitude at frame boundaries)
/// 2. Forward Real-to-Complex FFT transformation
///
/// DC offset removal is handled upstream by the audio stream's `dc_block` filter,
/// so all samples arriving here are already zero-mean.
///
/// # Arguments
/// * `signal` - Input audio signal (must be exactly WINDOW_SIZE samples, e.g., 2048)
/// * `time_buffer` - Pre-allocated mutable scratch space (must be at least WINDOW_SIZE).
///   The `realfft` algorithm performs its work in this buffer.
/// * `frequency_buffer` - Pre-allocated buffer for the FFT output. Must be at least `WINDOW_SIZE / 2 + 1` (e.g., 1025).
/// * `fft_instance` - A pre-planned Real FFT instance from `RealFftPlanner`
///
/// # Panics
/// * If array lengths are insufficient
pub fn perform_fft(
    signal: &[f32],
    time_buffer: &mut [f32],
    frequency_buffer: &mut [Complex<f32>],
    fft_instance: &Arc<dyn RealToComplex<f32>>,
    window_size: usize,
) {
    if signal.len() != window_size || time_buffer.len() < window_size {
        panic!("Input frame size and time scratch must be at least window_size");
    }

    // Real FFT of size N produces N/2 + 1 complex bins (0 to Nyquist)
    let expected_bins = window_size / 2 + 1;
    if frequency_buffer.len() < expected_bins {
        panic!("Frequency buffer must be at least window_size / 2 + 1 bins long");
    }

    let n_minus_1 = (window_size - 1) as f32;
    for (i, (&sample, real_val)) in signal.iter().zip(time_buffer.iter_mut()).enumerate() {
        // Hann window: 0.5 * (1 - cos(2π * n / (N - 1)))
        // Satisfies COLA at 50% overlap — no boundary artifacts.
        let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n_minus_1).cos());
        *real_val = sample * multiplier;
    }

    // The realfft crate modifies the input buffer in-place during calculation
    // and outputs the N/2 + 1 complex bins directly into our frequency_buffer.
    fft_instance
        .process(&mut time_buffer[..window_size], &mut frequency_buffer[..expected_bins])
        .expect("FFT Process Failed");
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
/// **Note:** This allocating version is used for the GUI spectrogram (which crosses
/// thread boundaries via `Vec<f32>`). Once the cross-thread communication structures
/// are migrated (see `update-communication-structures` workflow), this function will
/// likely become redundant — replaced entirely by [`spectrum_to_magnitudes_into`].
///
/// # Arguments
/// * `spectrum` - Complex frequency spectrum from the RFFT (1025 bins)
///
/// # Returns
/// * `Vec<f32>` - Magnitude spectrum
pub fn spectrum_to_magnitudes(spectrum: &[Complex<f32>], window_size: usize) -> Vec<f32> {
    spectrum
        .iter()
        .take(window_size / 2)
        .map(|c| c.norm()) // .norm() is sqrt(re^2 + im^2)
        .collect()
}

/// Zero-allocation magnitude extraction — writes directly into a pre-allocated slice.
///
/// Use this on the DSP hot path (Engine). The caller provides `out` which must be
/// at least `window_size / 2` elements. Only that many elements are written.
///
/// # Arguments
/// * `spectrum` — Complex frequency spectrum from the RFFT.
/// * `window_size` — The FFT window size (2048 or 8192). Determines how many bins to process.
/// * `out` — Pre-allocated output slice. Must be at least `window_size / 2` elements.
pub fn spectrum_to_magnitudes_into(
    spectrum: &[Complex<f32>],
    window_size: usize,
    out: &mut [f32],
) {
    let count = window_size / 2;
    for (o, c) in out[..count].iter_mut().zip(spectrum.iter().take(count)) {
        *o = c.norm();
    }
}
