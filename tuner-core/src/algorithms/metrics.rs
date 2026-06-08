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
//! | [`calculate_nhwrsf`] | Gatekeeper States 1–2 | Detects transients (hammer strikes) |
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
///   RMS = √ [ (1/N) × ∑ (x_i)² ]
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
///   EMA_current = α × x_current + (1 - α) × EMA_previous
///
/// # Citation
/// Giannoulis, D., Massberg, M., and Reiss, J. D. (2012). "Digital Dynamic Range Compressor Design—
/// A Tutorial and Analysis." *Journal of the Audio Engineering Society*, 60(6), 399-408.
pub fn calculate_ema(current_val: f32, previous_ema: f32, alpha: f32) -> f32 {
    if previous_ema == 0.0 {
        current_val
    } else {
        (current_val * alpha) + (previous_ema * (1.0 - alpha))
    }
}

/// Calculates the Normalized Half-Wave Rectified Spectral Flux (NHWRSF).
///
/// This measures the increase in transient energy between two frames by summing
/// the positive magnitude differences across a specific frequency band (roughly 50Hz to 10kHz),
/// then normalizing it against the total signal energy of the current frame.
///
/// # Arguments
/// * `current_spectrum` — The complex frequency spectrum of the current frame.
/// * `prev_spectrum_mags` — Mutable slice of the previous frame's magnitudes.
///   This is updated in-place to prime it for the next frame.
///
/// # Returns
/// A normalized, dimensionless float representing the transient flux.
pub fn calculate_nhwrsf(
    current_spectrum: &[rustfft::num_complex::Complex<f32>],
    prev_spectrum_mags: &mut [f32],
) -> f32 {
    // 2048-window FFT at 44100 Hz = ~21.533 Hz per bin.
    const START_BIN: usize = 2; // ~43 Hz
    const END_BIN: usize = 464; // ~9991 Hz

    let mut total_flux = 0.0;
    let mut current_energy = 0.0;

    // Ensure we don't panic if buffers are small for some reason
    let limit = current_spectrum
        .len()
        .min(prev_spectrum_mags.len())
        .min(END_BIN + 1);

    let start = START_BIN.min(limit);

    for k in start..limit {
        let c = current_spectrum[k];
        let mag = (c.re * c.re + c.im * c.im).sqrt();

        current_energy += mag;

        let diff = mag - prev_spectrum_mags[k];
        if diff > 0.0 {
            total_flux += diff;
        }

        // Buffer maintenance for the next frame
        prev_spectrum_mags[k] = mag;
    }

    // Secure against division-by-zero
    total_flux / (current_energy + 1e-6)
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
///   NINOS2 = N × (∑ |X_k|²) / (∑ |X_k|)²
///
/// # Citation
/// Mounir, M., Karsmakers, P., and van Waterschoot, T. (2021). "Musical note onset detection based on
/// a spectral sparsity measure." *EURASIP Journal on Audio, Speech, and Music Processing*, 2021(30).
///
/// *Note: This implements the ℓ₁/ℓ₂ variant (Eqs. 14-15), which is computationally cheaper
/// than the original 2016 ℓ₂/ℓ₄ formulation.*
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
