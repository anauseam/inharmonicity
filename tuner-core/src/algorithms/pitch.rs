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

/// Coherence assessment for a single extracted partial.
pub struct CoherenceResult {
    /// Refined frequency in Hz from XQIFFT or Quinn's.
    pub frequency: f32,
    /// True if the spectral lobe passes both coherence checks.
    /// False indicates beating unison or false beat — DO NOT use this partial for β updates.
    pub is_coherent: bool,
}

/// Maximum allowed asymmetry residual between measured and theoretical Hann shoulder ratio.
/// If exceeded, the lobe is asymmetric — indicative of a beating unison.
/// Unit: dB.
const ASYMMETRY_THRESHOLD_DB: f32 = 1.85;

/// Maximum allowed lobe width as a fraction of the theoretical Hann half-power width.
/// If measured_width > THEORETICAL_WIDTH * this factor, the lobe is smeared.
const LOBE_WIDTH_TOLERANCE: f32 = 1.15;

/// Theoretical half-power lobe width of a Hann window, in bins.
const HANN_HALF_POWER_WIDTH_BINS: f32 = 2.0;

pub(crate) fn spectral_asymmetry_index(magnitudes: &[f32], peak_bin: usize, delta: f32) -> f32 {
    if peak_bin < 1 || peak_bin + 1 >= magnitudes.len() {
        return 0.0;
    }

    let m_left  = magnitudes[peak_bin - 1].max(1e-10);
    let _m_peak  = magnitudes[peak_bin].max(1e-10);
    let m_right = magnitudes[peak_bin + 1].max(1e-10);

    // Measured log-domain shoulder ratio
    let measured_ratio_db = 20.0 * (m_left / m_right).log10();

    let theoretical_ratio_db = -2.0 * delta * 6.0;

    (measured_ratio_db - theoretical_ratio_db).abs()
}

fn lobe_width_is_coherent(magnitudes: &[f32], peak_bin: usize) -> bool {
    let m_peak = magnitudes[peak_bin];
    let half_power = m_peak * std::f32::consts::FRAC_1_SQRT_2; // -3 dB threshold

    let mut left_width: Option<f32> = None;
    for offset in 1..=8 {
        if peak_bin < offset { break; }
        if magnitudes[peak_bin - offset] <= half_power {
            left_width = Some(offset as f32);
            break;
        }
    }

    let mut right_width: Option<f32> = None;
    for offset in 1..=8 {
        if peak_bin + offset >= magnitudes.len() { break; }
        if magnitudes[peak_bin + offset] <= half_power {
            right_width = Some(offset as f32);
            break;
        }
    }

    // If either side never crossed half-power, the lobe is too wide to measure — not coherent.
    let (left, right) = match (left_width, right_width) {
        (Some(l), Some(r)) => (l, r),
        _ => return false,
    };

    let measured_full_width = left + right;
    let theoretical_full_width = HANN_HALF_POWER_WIDTH_BINS * 2.0;

    measured_full_width <= theoretical_full_width * LOBE_WIDTH_TOLERANCE
}

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
pub fn quinn_second_estimator(
    magnitudes: &[f32],
    sample_rate: u32,
    seed_hz: f32,
) -> Option<CoherenceResult> {
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
    
    let delta = if dp > 0.0 && dm > 0.0 {
        dp
    } else {
        dp + dm
    };

    let interpolated_bin = peak_bin as f32 + delta;
    let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

    if final_freq.is_finite() && final_freq > 0.0 {
        let asymmetry_db = spectral_asymmetry_index(magnitudes, peak_bin, delta);
        let width_ok = lobe_width_is_coherent(magnitudes, peak_bin);
        let is_coherent = asymmetry_db < ASYMMETRY_THRESHOLD_DB && width_ok;
        Some(CoherenceResult { frequency: final_freq, is_coherent })
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
) -> Option<CoherenceResult> {
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

    if let Some(offset) = crate::algorithms::pitch::parabolic_interpolation_offset(m_left, m_peak, m_right) {
        let interpolated_bin = peak_bin as f32 + offset;
        let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

        if final_freq.is_finite() && final_freq > 0.0 {
            let asymmetry_db = spectral_asymmetry_index(magnitudes, peak_bin, offset);
            let width_ok = lobe_width_is_coherent(magnitudes, peak_bin);
            let is_coherent = asymmetry_db < ASYMMETRY_THRESHOLD_DB && width_ok;
            Some(CoherenceResult { frequency: final_freq, is_coherent })
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
pub(crate) fn parabolic_interpolation_offset(y_left: f32, y_center: f32, y_right: f32) -> Option<f32> {
    let denominator = y_left - 2.0 * y_center + y_right;

    if denominator.abs() < 1e-6 {
        // The points are collinear; no parabola can be fit.
        return None;
    }

    let offset = (y_left - y_right) / (2.0 * denominator);
    Some(offset)
}


