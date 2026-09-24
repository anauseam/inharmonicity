//! # Spectral analysis
//!
//! The windowed FFT and its magnitudes, and frequency estimates from spectra:
//! CSPE reassignment, Jacobsen–Candan peak refinement, and Goertzel evaluation at
//! a single frequency.

use once_cell::sync::Lazy;
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::sync::Arc;

/// Hann-windows `signal` into `time_buffer` and writes its forward real FFT into
/// `frequency_buffer`. `signal` must be exactly `window_size` samples and
/// `time_buffer` at least that long; `frequency_buffer` must hold at least
/// `window_size / 2 + 1` bins, and `fft_instance` is a plan for `window_size`.
///
/// # Panics
/// If any of those lengths is short.
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
        // Symmetric Hann: 0.5·(1 − cos(2πn / (N − 1))).
        let multiplier = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / n_minus_1).cos());
        *real_val = sample * multiplier;
    }

    fft_instance
        .process(
            &mut time_buffer[..window_size],
            &mut frequency_buffer[..expected_bins],
        )
        .expect("FFT Process Failed");
}

/// Writes the magnitudes of the first `window_size / 2` bins of `spectrum` into
/// `out`.
///
/// # Panics
/// If `out` is shorter than `window_size / 2`.
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
/// The estimate is independent of the bin index. It is exact under any window applied
/// identically to both frames, whose phase cancels in the conjugate product (M·M* = ‖M‖²,
/// paper Eq. 37), provided the window keeps the ±frequency interaction terms negligible
/// (Eq. 36); Hann does.
///
/// `spectrum` and `spectrum_shifted` are $F(s_0)$ and $F(s_1)$: the `fft` of the
/// frame and of the same frame advanced one sample. `out` receives
/// `window_size / 2` refined frequencies in Hz, parallel to `magnitude_spectrum`; a
/// bin whose phase product gives a non-physical frequency (≤ 0 or non-finite) falls
/// back to its bin centre.
///
/// # Panics
/// If `spectrum`, `spectrum_shifted` or `out` is shorter than `window_size / 2`.
///
/// # References
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

    // Sliced up front: one bounds check, and a short `out` panics rather than
    // truncating the map.
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
/// underlying tone from the raw complex DFT values of the peak and its two immediate
/// neighbours (Candan 2015, Eq. 1):
///
/// ```text
///   δ = c_N · Re( (X[m-1] − X[m+1]) / (2·X[m] − X[m-1] − X[m+1]) )
/// ```
///
/// and returns the refined frequency `(bin + δ) · sample_rate / window_size` in Hz. The
/// raw windowed bins need no phase correction; the window's effect is absorbed by the
/// bias-correction factor `c_N` (Eq. 12). A boundary bin or a near-zero denominator
/// returns the bin centre, a fallback of ours where the paper is silent.
///
/// # References
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

/// Candan 2015 Eq. 12 bias-correction factor `c_N` for the pipeline's Hann window,
/// tabulated for the two FFT sizes [`jacobsen`] runs at because the evaluation is
/// `O(N)` and `jacobsen` runs per peak. The values are [`candan_c_n`] at those sizes
/// (`candan_c_n_reproduces_the_jacobsen_table`); for Hann, c_N → 2 as N → ∞.
#[inline]
fn candan_bias_correction(window_size: usize) -> f32 {
    match window_size {
        2048 => 2.001_329,
        8192 => 2.000_332,
        // Hann asymptotic limit — within 1.4e-3 of exact for any N ≥ 1024.
        _ => 2.0,
    }
}

/// Candan 2015 Eq. 12 evaluated numerically for the project's Hann window
/// (symmetric over `[0, N−1]`, the window [`fft`] and `hann` both apply) with
/// no zero-padding (`N₂ = N`).
///
/// Eq. 12 is `c_N = B₀² / (A₁B₀ + A₀B₁)`, with the four real constants built
/// (his Eq. 10) from the window transform and its derivative sampled at
/// `α ∈ {−1, 0, 1}`:
///
/// ```text
///   f_w(α)  = Σ_n w[n]·e^{−j2πnα/N}                    (Eq. 6)
///   f_w'(α) = (−j2π/N)·Σ_n n·w[n]·e^{−j2πnα/N}
///   A₀ = Im{f_w(1) − f_w(−1)}        A₁ = f_w'(1) − f_w'(−1)
///   B₀ = 2f_w(0) − f_w(1) − f_w(−1)  B₁ = Im{2f_w'(0) − f_w'(1) − f_w'(−1)}
/// ```
///
/// The paper prescribes this numerical route for an arbitrary window, so this is the
/// port rather than a fit. `O(N)` in trigonometry: call it once per length, never per
/// peak. Returns the Hann asymptote 2.0 for a length too short for Eq. 10.
///
/// # References
/// Candan, Ç. (2015). "Fine resolution frequency estimation from three DFT
/// samples: Case of windowed data." Signal Processing 114, pp. 245–250.
/// (Eqs. 6, 10, 12.)
pub fn candan_c_n(window_size: usize) -> f32 {
    if window_size < 4 {
        return 2.0;
    }
    let n = window_size as f64;
    // Index 0/1/2 ↔ α = −1/0/+1.
    let mut f = [Complex::<f64>::new(0.0, 0.0); 3];
    let mut df = [Complex::<f64>::new(0.0, 0.0); 3];
    for k in 0..window_size {
        let kf = k as f64;
        let w = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * kf / (n - 1.0)).cos());
        for (slot, alpha) in [-1.0f64, 0.0, 1.0].into_iter().enumerate() {
            let angle = -2.0 * std::f64::consts::PI * kf * alpha / n;
            let e = Complex::new(angle.cos(), angle.sin());
            f[slot] += e * w;
            df[slot] += e * (w * kf);
        }
    }
    let scale = Complex::new(0.0, -2.0 * std::f64::consts::PI / n);
    let df = [df[0] * scale, df[1] * scale, df[2] * scale];

    let a0 = (f[2] - f[0]).im;
    let a1 = (df[2] - df[0]).re;
    let b0 = (f[1] * 2.0 - f[2] - f[0]).re;
    let b1 = (df[1] * 2.0 - df[2] - df[0]).im;
    let denominator = a1 * b0 + a0 * b1;
    if denominator.abs() < f64::EPSILON {
        return 2.0;
    }
    (b0 * b0 / denominator) as f32
}

/// Signature shared by the fixed-length Goertzel evaluators ([`goertzel`],
/// [`goertzel_bass`]): `(samples, sample_rate, target_hz) → (amplitude, phase)`.
pub type GoertzelFn = fn(&[f32], u32, f32) -> (f32, f32);

/// The Neyman–Pearson amplitude threshold coefficient for an `n`-sample Hann-windowed
/// Goertzel: `T_amp = σ · K(n)` for noise σ, with `K(n) = (4/n)·√(0.375·n·ln(1/P_fa))`
/// at P_fa = 0.001. That is Kay 1998's magnitude threshold (*Detection Theory*, Ch. 9;
/// Rayleigh tail) under [`goertzel`]'s `4/n` normalization and the Hann window energy
/// `Σw² = 0.375·n`. K ∝ 1/√n, so the 4096-sample window's threshold is half the
/// 1024-sample one's (0.201184, `test_neyman_pearson_k_matches_engine`).
pub fn neyman_pearson_k(n: usize) -> f32 {
    (4.0 / n as f32) * (0.375 * n as f32 * 1000f32.ln()).sqrt()
}

/// Precomputed Hann window for the 1024-sample Goertzel hop.
static HANN_1024: Lazy<[f32; 1024]> = Lazy::new(hann::<1024>);

/// Precomputed Hann window for the long deep-bass strobe Goertzel:
/// main-lobe half-width 2·fs/N ≈ ±21.5 Hz at 44.1 kHz, below A0's ≈27.5 Hz
/// partial spacing, so a neighboring partial no longer sits inside the lobe.
static HANN_4096: Lazy<[f32; 4096]> = Lazy::new(hann::<4096>);

/// Symmetric Hann coefficients over `[0, N − 1]` for a length-`N` window.
fn hann<const N: usize>() -> [f32; N] {
    let mut window = [0.0; N];
    for (i, w) in window.iter_mut().enumerate() {
        *w = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (N as f32 - 1.0)).cos());
    }
    window
}

/// Hann-windowed non-integer Goertzel algorithm.
///
/// Evaluates the DFT at an arbitrary `target_hz`, over the freshest 1024 samples.
///
/// Returns `(amplitude, phase)`, the amplitude normalized by `4/N` (Hann coherent gain
/// 0.5, ×2 for single-sided) to physical time-domain units. The phase carries a
/// constant `ω(N−1)` offset from the DTFT phase (the standard Goertzel finalization),
/// fixed per target frequency, so hop-to-hop phase differences are exact but the
/// absolute phase is not the DTFT's.
///
/// # References
/// Goertzel, G. (1958). "An Algorithm for the Evaluation of Finite Trigonometric
/// Series." American Mathematical Monthly 65(1). Non-integer-frequency evaluation
/// per Sysel & Rajmic (2012), EURASIP J. Adv. Signal Process. 2012:56. The Hann
/// window and 4/N physical-units normalization are ours.
pub fn goertzel(samples: &[f32], sample_rate: u32, target_hz: f32) -> (f32, f32) {
    goertzel_windowed(samples, sample_rate, target_hz, &*HANN_1024)
}

/// [`goertzel`] over the freshest `window.len()` samples with an arbitrary precomputed
/// window, under the same normalization and phase contract.
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

/// [`goertzel`] with the 4096-sample Hann window, over the freshest 4096 samples:
/// the deep-bass resolution path.
pub fn goertzel_bass(samples: &[f32], sample_rate: u32, target_hz: f32) -> (f32, f32) {
    goertzel_windowed(samples, sample_rate, target_hz, &*HANN_4096)
}
