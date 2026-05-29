//! # Pitch Detection Module
//!
//! This module implements advanced, precision pitch detection algorithms optimized for piano tuning.
//! It serves as the primary refinement layer to exact sub-cent precision after a coarse pitch
//! neighborhood is established.
//!
//! ## Features
//! - **XQIFFT (Exponential-weighted QIFFT)**: Hann-window bias elimination for sub-cent spectral accuracy
//! - **QIFFT (Quadratic Interpolated FFT)**: Fast sub-bin parabolic peak resolution
//! - **DPLL (Digital Phase-Locked Loop)**: High-resolution time-domain phase tracking
//! - **Quinn**: Magnitude-only estimator for noise-resistant bass fundamental tracking

/// Quinn's Second Estimator for frequency refinement.
///
/// A magnitude-only estimator that uses the true peak bin and its two immediate neighbors
/// to estimate the true sub-bin frequency. Well-suited for cleanly separating
/// dense bass clusters without requiring complex FFT phase retention.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum.
/// * `sample_rate` — Audio sample rate in Hz.
/// * `seed_hz` — Coarse F0. Restricts peak search to narrow band around seed.
pub fn quinn_second_estimator(magnitudes: &[f32], sample_rate: u32, seed_hz: f32) -> Option<f32> {
    if magnitudes.len() < 3 || seed_hz <= 0.0 {
        return None;
    }

    let buffer_size = magnitudes.len() * 2;
    let target_bin = (seed_hz * buffer_size as f32) / sample_rate as f32;

    // Dynamic search window of ±1 semitone (at least 3 bins)
    let semitone_ratio = 1.059463_f32; // 2^(1/12)
    let search_radius = (target_bin * (semitone_ratio - 1.0)).max(3.0);

    let start_bin = (target_bin - search_radius).max(1.0) as usize;
    let end_bin = (target_bin + search_radius).min((magnitudes.len() - 2) as f32) as usize;

    if start_bin >= end_bin {
        return None;
    }

    let mut peak_bin = 0;
    let mut max_mag = -1.0;

    for i in start_bin..=end_bin {
        let mag = magnitudes[i];
        if mag > max_mag {
            max_mag = mag;
            peak_bin = i;
        }
    }

    if max_mag <= 1e-6 || peak_bin == 0 {
        return None;
    }

    let ap = magnitudes[peak_bin + 1] / magnitudes[peak_bin];
    let am = magnitudes[peak_bin - 1] / magnitudes[peak_bin];

    let dp = -ap / (1.0 - ap);
    let dm = am / (1.0 - am);

    let delta = if dp > 0.0 && dm > 0.0 { dp } else { dp + dm };

    let interpolated_bin = peak_bin as f32 + delta;
    let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

    if final_freq.is_finite() && final_freq > 0.0 {
        Some(final_freq)
    } else {
        None
    }
}

/// Sub-cent frequency refinement using exponentially-weighted QIFFT.
///
/// Raises the magnitude of the seed peak and its neighbors to power `p`
/// before parabolic interpolation, minimizing interpolation bias for
/// Hann-windowed spectra.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum (output of `spectrum_to_magnitudes`).
/// * `sample_rate` — Audio sample rate in Hz.
/// * `seed_hz` — Coarse F0 from TWM. Restricts peak search to ±`search_bins` around seed.
/// * `p` — Exponential weighting factor. Use `2.0` for Hann window, 50% overlap.
pub fn detect_pitch_xqifft_seeded(
    magnitudes: &[f32],
    sample_rate: u32,
    seed_hz: f32,
    p: f32,
) -> Option<f32> {
    if magnitudes.len() < 3 || seed_hz <= 0.0 {
        return None;
    }

    let buffer_size = magnitudes.len() * 2;
    let target_bin = (seed_hz * buffer_size as f32) / sample_rate as f32;

    // Dynamic search window of ±1 semitone (at least 3 bins)
    let semitone_ratio = 1.059463_f32; // 2^(1/12)
    let search_radius = (target_bin * (semitone_ratio - 1.0)).max(3.0);

    // Calculate valid bin indices, ignoring DC (0) and Nyquist (len-1) boundaries to allow for interpolation
    let start_bin = (target_bin - search_radius).max(1.0) as usize;
    let end_bin = (target_bin + search_radius).min((magnitudes.len() - 2) as f32) as usize;

    if start_bin >= end_bin {
        return None; // No valid search area
    }

    // Find the actual peak bin near the rough estimate
    let mut peak_bin = 0;
    let mut max_mag = -1.0;

    for i in start_bin..=end_bin {
        let mag = magnitudes[i];
        if mag > max_mag {
            max_mag = mag;
            peak_bin = i;
        }
    }

    // If the spectrum is essentially empty/silent in this band
    if max_mag <= 1e-6 || peak_bin == 0 {
        return None;
    }

    // XQIFFT: Raise magnitude of the peak and its neighbors to power `p`
    let m_left = magnitudes[peak_bin - 1].powf(p);
    let m_peak = magnitudes[peak_bin].powf(p);
    let m_right = magnitudes[peak_bin + 1].powf(p);

    if let Some(offset) =
        crate::algorithms::pitch::parabolic_interpolation_offset(m_left, m_peak, m_right)
    {
        let interpolated_bin = peak_bin as f32 + offset;
        let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

        if final_freq.is_finite() && final_freq > 0.0 {
            Some(final_freq)
        } else {
            None
        }
    } else {
        None
    }
}

/// Calculates the offset of a parabola's vertex from a center point.
///
/// Given three equidistant points (y_left, y_center, y_right), this function
/// fits a parabola to them and returns the fractional offset of the true
/// extremum (peak or trough) from the center point's index.
///
/// # Arguments
/// * `y_left` - The value of the point to the left of the center.
/// * `y_center` - The value of the center point (the detected peak/trough).
/// * `y_right` - The value of the point to the right of the center.
///
/// # Returns
/// * `Some(offset)` - The calculated offset, which can be added to the center index.
/// * `None` - If the points form a straight line (denominator is zero).
pub(crate) fn parabolic_interpolation_offset(
    y_left: f32,
    y_center: f32,
    y_right: f32,
) -> Option<f32> {
    let denominator = y_left - 2.0 * y_center + y_right;

    if denominator.abs() < 1e-6 {
        // The points are collinear; no parabola can be fit.
        return None;
    }

    let offset = (y_left - y_right) / (2.0 * denominator);
    Some(offset)
}
