//! # Phantom Partial Mask
//!
//! Implements the predictive exclusion filter for longitudinal intermodulation products in the bass
//! register. This stage runs between Stage 1 (Dot-Product Correlation) and Stage 2
//! (Sub-Bin Localization), zeroing targeted FFT bins before any partial extraction occurs.

/// The (m, n) index pairs for tracked intermodulation products.
/// Applied only in the bass register where longitudinal mode energy is significant.
const PHANTOM_PAIRS: [(u32, u32); 5] = [(2, 3), (3, 4), (4, 5), (2, 5), (3, 6)];

/// Dimensionless scaling constant representing geometric strain limits of wound string material.
const SMEAR_SCALE: f32 = 0.085;

/// Applies the Phantom Partial Mask to a mutable magnitude spectrum.
///
/// For each of the 5 tracked intermodulation pairs, this function:
///   1. Computes the expected center frequency using the inharmonic partial formula.
///   2. Calculates a dynamic smearing bandwidth proportional to β and parent partial indices.
///   3. Converts the bandwidth to discrete FFT bins.
///   4. Zeros the center bin and its smearing radius in-place.
///
/// This is a zero-allocation, in-place operation safe to call on the DSP thread.
///
/// # Arguments
/// * `magnitudes`   — Mutable magnitude spectrum. Modified in-place.
/// * `f0`           — Fundamental frequency seed from Stage 1 (Hz).
/// * `beta`         — Nominal inharmonicity coefficient from Stage 1.
/// * `sample_rate`  — Audio sample rate in Hz.
/// * `window_size`  — FFT window size (number of bins × 2).
pub fn apply_phantom_mask(
    magnitudes: &mut [f32],
    f0: f32,
    beta: f32,
    sample_rate: u32,
    window_size: usize,
) {
    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let n_bins = magnitudes.len();

    // Prevent divide by zero or extreme cases.
    if hz_per_bin <= 0.0 || n_bins == 0 {
        return;
    }

    for &(m, n) in &PHANTOM_PAIRS {
        let m_f = m as f32;
        let n_f = n as f32;

        // Inharmonic partial frequencies for parent indices m and n
        let f_m = m_f * f0 * (1.0 + beta * m_f * m_f).sqrt();
        let f_n = n_f * f0 * (1.0 + beta * n_f * n_f).sqrt();

        // Predicted phantom center frequency
        let f_center = f_m + f_n;

        // Dynamic smearing bandwidth (Hz), scaled by β and parent index energy
        let bandwidth = f_center * beta * (m_f * m_f + n_f * n_f) * SMEAR_SCALE;

        // Convert bandwidth to bin radius (ceiling to ensure full coverage)
        let radius = (bandwidth / hz_per_bin).ceil() as usize;

        // Center bin of the phantom
        let center_bin = (f_center / hz_per_bin).round() as usize;

        // Zero out center bin ± radius (bounds-checked)
        let lo = center_bin.saturating_sub(radius);
        let hi = (center_bin + radius + 1).min(n_bins);
        for bin in lo..hi {
            magnitudes[bin] = 0.0;
        }
    }
}
