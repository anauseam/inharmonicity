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

use once_cell::sync::Lazy;
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
        .process(
            &mut time_buffer[..window_size],
            &mut frequency_buffer[..expected_bins],
        )
        .expect("FFT Process Failed");
}

/// Extracts magnitudes from a complex spectrum into a pre-allocated output slice.
///
/// Computes `sqrt(re² + im²)` for the first `window_size / 2` bins of the RFFT
/// output and writes them into `out`. This is zero-allocation and safe for the
/// DSP hot path.
///
/// The resulting magnitudes are used for spectrogram visualisation (via the
/// [`FrameOutput`](crate::FrameOutput) triple buffer) and for downstream DSP
/// (TWM peak picking, XQIFFT refinement).
///
/// # Arguments
/// * `spectrum` — Complex frequency spectrum from the RFFT.
/// * `window_size` — The FFT window size (2048 or 8192). Determines how many bins to process.
/// * `out` — Pre-allocated output slice. Must be at least `window_size / 2` elements.
///
/// # Panics
/// * If `out.len() < window_size / 2`.
pub fn spectrum_to_magnitudes(spectrum: &[Complex<f32>], window_size: usize, out: &mut [f32]) {
    let count = window_size / 2;
    for (o, c) in out[..count].iter_mut().zip(spectrum.iter().take(count)) {
        *o = c.norm();
    }
}

/// Precomputed Hann window for the 1024-sample Goertzel hop.
static HANN_1024: Lazy<[f32; 1024]> = Lazy::new(|| {
    let mut window = [0.0; 1024];
    for i in 0..1024 {
        window[i] = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / 1023.0).cos());
    }
    window
});

/// Hann-windowed non-integer Goertzel algorithm.
///
/// Evaluates the DFT at an arbitrary `target_hz` (not restricted to FFT bin centers).
/// Applies a precomputed window (e.g., `HANN_1024`) to the first 1024 samples.
///
/// Returns `(amplitude, phase)` where the amplitude is normalized by `4/N`
/// (Hann coherent gain = 0.5, ×2 for single-sided) to match physical time-domain units.
pub fn goertzel(samples: &[f32], sample_rate: u32, target_hz: f32) -> (f32, f32) {
    if samples.len() < 1024 {
        return (0.0, 0.0);
    }

    let k = (1024.0 * target_hz) / sample_rate as f32;
    let omega = (2.0 * std::f32::consts::PI * k) / 1024.0;
    let cosine = omega.cos();
    let sine = omega.sin();
    let coeff = 2.0 * cosine;

    let mut q1 = 0.0_f32;
    let mut q2 = 0.0_f32;

    for (&sample, &w) in samples.iter().take(1024).zip(HANN_1024.iter()) {
        let q0 = coeff * q1 - q2 + (sample * w);
        q2 = q1;
        q1 = q0;
    }

    let real = q1 - q2 * cosine;
    let imag = q2 * sine;

    let magnitude = (real * real + imag * imag).sqrt();
    let phase = imag.atan2(real);

    // Normalize by 4.0 / 1024.0 to correct for windowing and single-sided spectrum
    let amplitude = magnitude * 4.0 / 1024.0;

    (amplitude, phase)
}
