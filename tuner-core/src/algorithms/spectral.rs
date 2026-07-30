//! # Fast Fourier Transform (FFT) Module
//!
//! This module provides high-performance FFT processing for real-time audio analysis.
//! It handles frequency domain transformations, windowing functions, and spectrum
//! magnitude calculations for piano tuning applications.
//!
//! ## Features
//! - Highly-optimized Real-to-Complex FFT (RFFT) using `realfft`
//! - Hann windowing (zero at frame boundaries; COLA at 50% overlap)
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
/// 1. Hann windowing (zero at frame boundaries; satisfies COLA at 50% overlap)
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
pub fn fft(
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
/// (peak picking and sub-bin refinement).
///
/// # Arguments
/// * `spectrum` — Complex frequency spectrum from the RFFT.
/// * `window_size` — The FFT window size (2048 or 8192). Determines how many bins to process.
/// * `out` — Pre-allocated output slice. Must be at least `window_size / 2` elements.
///
/// # Panics
/// * If `out.len() < window_size / 2`.
pub fn magnitude_spectrum(spectrum: &[Complex<f32>], window_size: usize, out: &mut [f32]) {
    let count = window_size / 2;
    for (o, c) in out[..count].iter_mut().zip(spectrum.iter().take(count)) {
        *o = c.norm();
    }
}

/// Complex Spectral Phase Evolution (CSPE) — super-resolution per-bin frequency estimation.
///
/// Reassigns every DFT bin to the true frequency of the component dominating it, by
/// comparing the phase of the spectrum against that of the same frame advanced by one
/// sample. For a component at angular frequency $\omega$, advancing the signal one sample
/// multiplies its spectrum by $e^{j\omega}$, so
///
/// ```text
///   spectrum · conj(spectrum_shifted) = |spectrum|² · e^{−jω}
///   f_bin = −∠(spectrum · conj(spectrum_shifted)) · sample_rate / (2π)
/// ```
///
/// The estimate is independent of the bin index and far more accurate than the DFT grid or
/// parabolic interpolation. It is exact under any analysis window applied identically to
/// both frames — the window's phase cancels in the conjugate product (M·M* = ‖M‖², paper
/// Eq. 37) — provided the window confines leakage so the ±frequency interaction terms stay
/// negligible (Eq. 36); Hann qualifies.
///
/// # Arguments
/// * `spectrum` — Complex spectrum $F(s_0)$ of the analysis frame (`fft` output).
/// * `spectrum_shifted` — Complex spectrum $F(s_1)$ of the *same* frame advanced by one
///   sample (and windowed identically).
/// * `window_size` — FFT window size; exactly `window_size / 2` bins are written.
/// * `sample_rate` — Audio sample rate in Hz.
/// * `out` — Per-bin refined frequency in Hz (parallel to `magnitude_spectrum`). Bins
///   whose phase product yields a non-physical (≤ 0 / non-finite) frequency fall back to the
///   bin-centre frequency.
///
/// # Panics
/// * If `spectrum`, `spectrum_shifted`, or `out` is shorter than `window_size / 2`
///   (same size contract as [`fft`] — a silent truncation would mask a caller bug).
///
/// # Reference
/// Short, K. M. & Garcia, R. A. (2006). "Signal Analysis Using the Complex Spectral Phase
/// Evolution (CSPE) Method." AES 120th Convention, Paris. Paper 6645. (Eqs. 7, 38.)
/// As applied to inharmonic analysis in Hodgkinson et al., DAFx-09 §2.3, Eqs. 18–19.
pub fn cspe(
    spectrum: &[Complex<f32>],
    spectrum_shifted: &[Complex<f32>],
    window_size: usize,
    sample_rate: u32,
    out: &mut [f32],
) {
    let count = window_size / 2;
    if spectrum.len() < count || spectrum_shifted.len() < count {
        panic!("CSPE input spectra must be at least window_size / 2 bins long");
    }
    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let scale = sample_rate as f32 / (2.0 * std::f32::consts::PI);

    // Slice-to-count up front: one bounds check here, none in the hot loop,
    // and an undersized `out` panics instead of silently truncating the map.
    let out = &mut out[..count];
    for (bin, (o, (s0, s1))) in out
        .iter_mut()
        .zip(spectrum.iter().zip(spectrum_shifted.iter()))
        .enumerate()
    {
        // ∠(F(s0) · conj(F(s1))) = −ω for a component at angular frequency ω.
        let product = s0 * s1.conj();
        let freq = -product.arg() * scale;
        *o = if freq.is_finite() && freq > 0.0 {
            freq
        } else {
            // Phase product degenerate (no coherent component) — keep the bin centre.
            bin as f32 * hz_per_bin
        };
    }
}

/// Jacobsen sub-bin frequency estimator with Candan's window bias correction —
/// complex-domain, single-peak refinement.
///
/// Given a spectral peak at integer bin `bin`, estimates the true frequency of the
/// underlying tone from the *raw* complex DFT values of the peak and its two immediate
/// neighbours (Candan 2015, Eq. 1):
///
/// ```text
///   δ = c_N · Re( (X[m-1] − X[m+1]) / (2·X[m] − X[m-1] − X[m+1]) )
/// ```
///
/// and returns the refined frequency `(bin + δ) · sample_rate / window_size` in Hz.
/// The bins are consumed exactly as the windowed DFT produces them: the estimator is
/// derived for raw (causal) bins and needs no phase correction — the neighbours' sign
/// alternation is intrinsic to the formula. The window's effect is absorbed entirely by
/// the bias-correction factor `c_N` (Eq. 12), precomputed offline for the pipeline's
/// Hann window in [`candan_bias_correction`].
///
/// Used in Discovery by [`crate::algorithms::peaks::extract_peaks`], per detected peak.
///
/// Falls back to the plain bin-centre frequency for boundary bins (no neighbour) or a
/// degenerate (near-zero) denominator (ours — the paper is silent on degenerate input).
///
/// # Reference
/// Candan, Ç. (2015). "Fine resolution frequency estimation from three DFT samples:
/// Case of windowed data." Signal Processing, 114, pp. 245–250.
/// DOI: 10.1016/j.sigpro.2015.03.009 (Eqs. 1, 12.)
/// Confirmed by Keyta & Dilaveroğlu (2025), Elektronika ir Elektrotechnika 31(3),
/// whose whole-interval least-squares c_N (their Eq. 20) matches Eq. 12 to five
/// decimals for the Hann window at these sizes.
pub fn jacobsen(
    complex_spectrum: &[Complex<f32>],
    bin: usize,
    window_size: usize,
    sample_rate: u32,
) -> f32 {
    let hz_per_bin = sample_rate as f32 / window_size as f32;

    if bin == 0 || bin + 1 >= complex_spectrum.len() {
        return bin as f32 * hz_per_bin;
    }

    let x_prev = complex_spectrum[bin - 1];
    let x_peak = complex_spectrum[bin];
    let x_next = complex_spectrum[bin + 1];

    let numerator = x_prev - x_next;
    let denominator = Complex::new(2.0, 0.0) * x_peak - x_prev - x_next;

    let delta = if denominator.norm_sqr() > 1e-12 {
        candan_bias_correction(window_size) * (numerator / denominator).re
    } else {
        0.0
    };

    (bin as f32 + delta) * hz_per_bin
}

/// Candan 2015 Eq. 12 bias-correction factor `c_N` for the pipeline's Hann window
/// (defined over `[0, N-1]`), precomputed offline by evaluating Eq. 12 with the
/// window transform f_w and its derivative at 0, ±1 (derivation and numbers:
/// docs/audits/faithfulness-audit-03-jacobsen.md). For Hann, c_N → 2 exactly as
/// N → ∞; the finite-N values differ only in the fourth decimal.
#[inline]
fn candan_bias_correction(window_size: usize) -> f32 {
    match window_size {
        2048 => 2.001_329,
        8192 => 2.000_332,
        // Hann asymptotic limit — within 1.4e-3 of exact for any N ≥ 1024.
        _ => 2.0,
    }
}

/// Signature shared by the fixed-length Goertzel evaluators ([`goertzel`],
/// [`goertzel_bass`]): `(samples, sample_rate, target_hz) → (amplitude, phase)`.
/// Callers that select a window length at runtime (engine tracker, strobe
/// bank) hold one of these per register.
pub type GoertzelFn = fn(&[f32], u32, f32) -> (f32, f32);

/// The Neyman–Pearson amplitude threshold coefficient for an `n`-sample
/// Hann-windowed Goertzel: `T_amp = noise_floor · K(n)` with
/// `K(n) = (4/n)·√(0.375·n·ln(1/P_fa))`, P_fa = 0.001 — the Kay 1998
/// (*Detection Theory*, Ch. 9; Rayleigh tail) magnitude threshold scaled by
/// the [`goertzel`] `4/n` physical-units normalization and the unnormalized
/// Hann window energy `Σw² = 0.375·n`. At n = 1024 this reproduces the
/// engine's historical `NEYMAN_PEARSON_K = 0.201184` exactly (pinned by
/// test in `strobe.rs`); K ∝ 1/√n, so the 4096-sample window's threshold is
/// half that — the processing gain a longer window buys.
pub fn neyman_pearson_k(n: usize) -> f32 {
    (4.0 / n as f32) * (0.375 * n as f32 * 1000f32.ln()).sqrt()
}

/// Precomputed Hann window for the 1024-sample Goertzel hop.
static HANN_1024: Lazy<[f32; 1024]> = Lazy::new(hann::<1024>);

/// Precomputed Hann window for the long deep-bass strobe Goertzel (R3):
/// main-lobe half-width 2·fs/N ≈ ±21.5 Hz at 44.1 kHz, below A0's ≈27.5 Hz
/// partial spacing, so a neighboring partial no longer sits inside the lobe.
static HANN_4096: Lazy<[f32; 4096]> = Lazy::new(hann::<4096>);

/// Periodic-form Hann coefficients for a length-`N` analysis window.
fn hann<const N: usize>() -> [f32; N] {
    let mut window = [0.0; N];
    for (i, w) in window.iter_mut().enumerate() {
        *w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (N as f32 - 1.0)).cos());
    }
    window
}

/// Hann-windowed non-integer Goertzel algorithm.
///
/// Evaluates the DFT at an arbitrary `target_hz` (not restricted to FFT bin centers).
/// Applies a precomputed window (e.g., `HANN_1024`) to the first 1024 samples.
///
/// Returns `(amplitude, phase)` where the amplitude is normalized by `4/N`
/// (Hann coherent gain = 0.5, ×2 for single-sided) to match physical time-domain units.
///
/// The phase carries a constant `ω(N−1)` offset relative to the DTFT phase (the
/// standard Goertzel finalization; it vanishes only at integer bins). The offset is
/// fixed per target frequency, so **hop-to-hop phase differences are exact** — the
/// engine's phase-vocoder use — but the absolute phase is not the DTFT's.
///
/// # Reference
/// Goertzel, G. (1958). "An Algorithm for the Evaluation of Finite Trigonometric
/// Series." American Mathematical Monthly 65(1). Non-integer-frequency evaluation
/// per Sysel & Rajmic (2012), EURASIP J. Adv. Signal Process. 2012:56. The Hann
/// window and 4/N physical-units normalization are ours.
pub fn goertzel(samples: &[f32], sample_rate: u32, target_hz: f32) -> (f32, f32) {
    goertzel_windowed(samples, sample_rate, target_hz, &*HANN_1024)
}

/// [`goertzel`] over the last `window.len()` samples of `samples` with an
/// arbitrary precomputed window — the strobe bank's deep-bass path evaluates
/// a 4096-sample window ([`goertzel_bass`]) where the 1024 main lobe would
/// swallow the neighboring partial (R3). Window length ≠ hop: callers still
/// evaluate every hop, so the update rate is unchanged. Same normalization
/// and phase contract as [`goertzel`].
///
/// `pub` so the offline window-length diagnostics (`examples/pitch_ground_truth.rs`)
/// can sweep lengths the shipping code does not instantiate; shipping callers
/// use the two fixed-length wrappers.
pub fn goertzel_windowed(
    samples: &[f32],
    sample_rate: u32,
    target_hz: f32,
    window: &[f32],
) -> (f32, f32) {
    let n = window.len();
    if samples.len() < n {
        return (0.0, 0.0);
    }

    let k = (n as f32 * target_hz) / sample_rate as f32;
    let omega = (2.0 * std::f32::consts::PI * k) / n as f32;
    let cosine = omega.cos();
    let sine = omega.sin();
    let coeff = 2.0 * cosine;

    let mut q1 = 0.0_f32;
    let mut q2 = 0.0_f32;

    // The freshest `n` samples — for the engine's 1024-hop slice this is the
    // whole slice (its caller already passes exactly one hop).
    let start = samples.len() - n;
    for (&sample, &w) in samples[start..].iter().zip(window.iter()) {
        let q0 = coeff * q1 - q2 + (sample * w);
        q2 = q1;
        q1 = q0;
    }

    let real = q1 - q2 * cosine;
    let imag = q2 * sine;

    let magnitude = (real * real + imag * imag).sqrt();
    let phase = imag.atan2(real);

    // Normalize by 4/N to correct for windowing and single-sided spectrum.
    let amplitude = magnitude * 4.0 / n as f32;

    (amplitude, phase)
}

/// [`goertzel`] with the 4096-sample Hann window — the deep-bass strobe
/// resolution path (R3). Evaluates the freshest 4096 samples of `samples`.
/// `pub` so the offline strobe-replay diagnostic (`examples/strobe_replay.rs`)
/// can A/B the two window lengths on captured bass audio; the shipping caller
/// is [`crate::strobe::Strobe`].
pub fn goertzel_bass(samples: &[f32], sample_rate: u32, target_hz: f32) -> (f32, f32) {
    goertzel_windowed(samples, sample_rate, target_hz, &*HANN_4096)
}
