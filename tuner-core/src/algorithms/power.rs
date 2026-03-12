//! # Power & Spectral Energy Algorithms
//!
//! Stateless functions for measuring signal power and spectral characteristics.
//! These are the core metrics used by the [`Gatekeeper`](crate::gatekeeper::Gatekeeper)
//! to evaluate signal stability and drive its 5-state machine.
//!
//! | Function | Used By | Purpose |
//! |---|---|---|
//! | [`calculate_rms`] | Gatekeeper State 0 | Silence gating — below threshold = IDLE |
//! | [`calculate_ema`] | Gatekeeper State 0 | Smooths RMS to ignore momentary dips |
//! | [`calculate_csd`] | Gatekeeper States 1–2 | Detects transients (hammer strikes) |
//! | [`calculate_ninos2`] | Gatekeeper State 3 | Measures spectral sparsity (tonal stability) |

/// Calculates the Root Mean Square (RMS) amplitude of an audio buffer.
///
/// RMS measures the "average loudness" of a signal. The Gatekeeper uses this
/// (after EMA smoothing) to determine if the signal is above the dynamic
/// silence threshold calibrated during startup.
///
/// # Arguments
/// * `buffer` — A slice of audio samples (typically one frame worth).
///
/// # Returns
/// The RMS amplitude as a non-negative `f32`. A silent signal returns `0.0`.
///
/// # Formula
/// $$\text{RMS} = \sqrt{\frac{1}{N} \sum_{i=0}^{N-1} x_i^2}$$
pub fn calculate_rms(buffer: &[f32]) -> f32 {
    let sum_sq: f32 = buffer.iter().map(|&x| x * x).sum();
    (sum_sq / buffer.len() as f32).sqrt()
}

/// Calculates an Exponential Moving Average (EMA) step.
///
/// EMA smooths a noisy signal by weighting the current value and the previous
/// average. The Gatekeeper applies this to RMS to prevent false Silence → Attack
/// transitions caused by momentary unison beating dips.
///
/// If `previous_ema` is `0.0` (cold start), the function initializes directly
/// to `current_val` to avoid a slow ramp-up from zero.
///
/// # Arguments
/// * `current_val` — The new raw sample (e.g., current frame RMS).
/// * `previous_ema` — The EMA value from the previous frame.
/// * `alpha` — Smoothing factor (0.0–1.0). Higher = more responsive, lower = smoother.
///
/// # Returns
/// The updated EMA value.
///
/// # Formula
/// $$\text{EMA}_t = \alpha \cdot x_t + (1 - \alpha) \cdot \text{EMA}_{t-1}$$
pub fn calculate_ema(current_val: f32, previous_ema: f32, alpha: f32) -> f32 {
    if previous_ema == 0.0 {
        current_val
    } else {
        (current_val * alpha) + (previous_ema * (1.0 - alpha))
    }
}

/// Calculates the Complex Spectral Difference (CSD) between two successive frames.
///
/// CSD measures the Euclidean distance between complex spectra. A large CSD
/// indicates a sudden spectral change — exactly what happens when a piano hammer
/// strikes a string (State 1: ATTACK). The Gatekeeper compares CSD against
/// `csd_attack_threshold` to trigger the Attack state.
///
/// # Arguments
/// * `prev` — Complex spectrum from the previous frame.
/// * `curr` — Complex spectrum from the current frame.
///
/// # Returns
/// The squared Euclidean distance between the two spectra (not square-rooted,
/// for performance — the Gatekeeper thresholds work on squared values).
///
/// # Formula
/// $$\text{CSD} = \sum_{k} \left[ (\text{Re}_k^{\text{curr}} - \text{Re}_k^{\text{prev}})^2 + (\text{Im}_k^{\text{curr}} - \text{Im}_k^{\text{prev}})^2 \right]$$
pub fn calculate_csd(
    prev: &[rustfft::num_complex::Complex<f32>],
    curr: &[rustfft::num_complex::Complex<f32>],
) -> f32 {
    let mut sum_sq = 0.0;
    for (p, c) in prev.iter().zip(curr.iter()) {
        let diff_re = c.re - p.re;
        let diff_im = c.im - p.im;
        sum_sq += diff_re * diff_re + diff_im * diff_im;
    }
    sum_sq
}

/// Normalized Identification of Note Onset based on Spectral Sparsity (NINOS2).
///
/// NINOS2 quantifies how "peaky" (tonal) vs. "flat" (noisy) a spectrum is.
/// A pure tone concentrates energy in a few bins → high NINOS2. White noise
/// spreads energy evenly → NINOS2 ≈ 1.0. The Gatekeeper uses this in
/// State 3 (HARMONIC DECAY) to identify the "Golden Window" where the spectrum
/// is sparse enough for a high-quality capture.
///
/// # Arguments
/// * `spectrum` — Complex frequency spectrum from an FFT. The DC bin (index 0) is skipped.
///
/// # Returns
/// A non-negative `f32`. Higher values indicate a sparser (more tonal) spectrum.
/// For white noise, the value approaches `1.0`.
///
/// # Formula
/// $$\text{NINOS2} = \frac{N \cdot \sum |X_k|^2}{\left(\sum |X_k|\right)^2}$$
pub fn calculate_ninos2(spectrum: &[rustfft::num_complex::Complex<f32>]) -> f32 {
    let mut sum_mag = 0.0;
    let mut sum_mag_sq = 0.0;

    // Skip DC bin
    for c in spectrum.iter().skip(1) {
        let mag_sq = c.re * c.re + c.im * c.im;
        let mag = mag_sq.sqrt();
        sum_mag += mag;
        sum_mag_sq += mag_sq;
    }

    if sum_mag == 0.0 {
        return 0.0;
    }

    // The fewer the peaks (more sparse), the closer this ratio gets to N.
    // For white noise, it approaches 1.
    (sum_mag_sq * spectrum.len() as f32) / (sum_mag * sum_mag)
}
