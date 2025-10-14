//! # Pitch Detection Module
//! 
//! This module implements advanced pitch detection algorithms optimized for piano tuning.
//! It provides robust frequency detection using the YIN algorithm with enhancements
//! for musical instrument analysis.
//! 
//! ## Features
//! - YIN pitch detection algorithm with octave error prevention
//! - Noise rejection and clarity checking
//! - Parabolic interpolation for sub-sample accuracy
//! - Spectrum refinement for improved precision

/// A robust implementation of the YIN pitch detection algorithm.
/// 
/// This version is optimized for piano tuning with several enhancements:
/// - Octave error prevention through careful threshold selection
/// - Noise rejection using clarity checking
/// - Parabolic interpolation for sub-sample accuracy
/// - Amplitude gating to filter out silence
/// 
/// # Arguments
/// * `signal` - Input audio signal
/// * `sample_rate` - Sample rate in Hz
/// * `amplitude_threshold` - Minimum amplitude for pitch detection
/// 
/// # Returns
/// * `Some(frequency)` - Detected frequency in Hz
/// * `None` - No pitch detected (silence, noise, or invalid signal)
pub fn detect_pitch_yin(
    signal: &[f32],
    sample_rate: u32,
    amplitude_threshold: f32,
) -> Option<f32> {
    let frame_size = signal.len();
    let mut yin_buffer = vec![0.0; frame_size / 2];

    // --- Noise Gate: Calculate RMS to filter out silence/noise ---
    let rms = (signal.iter().map(|&s| s * s).sum::<f32>() / frame_size as f32).sqrt();
    if rms < amplitude_threshold {
        return None;
    }

    // --- Step 1 & 2: Difference function and squared difference ---
    for tau in 1..(frame_size / 2) {
        let mut diff = 0.0;
        for i in 0..(frame_size / 2) {
            let delta = signal[i] - signal[i + tau];
            diff += delta * delta;
        }
        yin_buffer[tau] = diff;
    }

    // --- Step 3: Cumulative mean normalized difference ---
    let mut running_sum = 0.0;
    yin_buffer[0] = 1.0;
    for tau in 1..(frame_size / 2) {
        running_sum += yin_buffer[tau];
        if running_sum != 0.0 {
            yin_buffer[tau] *= tau as f32 / running_sum;
        } else {
            yin_buffer[tau] = 1.0;
        }
    }

    // --- Step 4 & 5: Find the first significant dip to avoid octave errors ---
    let min_val = yin_buffer
        .iter()
        .skip(1) // Skip tau = 0
        .cloned()
        .fold(f32::INFINITY, f32::min);

    let mut period = 0;
    let threshold = min_val + 0.05;

    for tau in 2..(frame_size / 2) {
        if yin_buffer[tau] < threshold && yin_buffer[tau] < yin_buffer[tau - 1] {
            period = tau;
            break;
        }
    }

    // --- NEW: Step 5.5: Clarity Check to Reject Noise ---
    // If no period was found OR the found period is not "clear" enough,
    // it's likely noise. A clear tone will have a very low value in the YIN buffer.
    const CLARITY_THRESHOLD: f32 = 0.1;
    if period == 0 || yin_buffer[period] > CLARITY_THRESHOLD {
        return None;
    }

    // --- Step 6: Parabolic interpolation for better precision ---
    if period + 1 >= frame_size / 2 { // Bounds check for interpolation
        return None;
    }

    let y1 = yin_buffer[period - 1];
    let y2 = yin_buffer[period];
    let y3 = yin_buffer[period + 1];

    let period_float = if (y1 - 2.0 * y2 + y3) != 0.0 {
        let peak_shift = (y1 - y3) / (2.0 * (y1 - 2.0 * y2 + y3));
        period as f32 + peak_shift
    } else {
        period as f32
    };

    let frequency = sample_rate as f32 / period_float;

    // FIX: Add a final guard to ensure we only return valid, audible frequencies.
    if frequency.is_finite() && frequency > 20.0 {
        Some(frequency)
    } else {
        None
    }
}



/// Refines a frequency estimate using a pre-computed magnitude spectrum.
/// 
/// This function improves the accuracy of pitch detection by analyzing
/// the frequency spectrum around the initial estimate. It uses parabolic
/// interpolation to achieve sub-bin accuracy.
/// 
/// # Arguments
/// * `spectrum_magnitudes` - Magnitude spectrum from FFT
/// * `rough_freq` - Initial frequency estimate in Hz
/// * `sample_rate` - Sample rate in Hz
/// 
/// # Returns
/// * `Some(refined_freq)` - Refined frequency estimate
/// * `None` - Refinement failed, use original estimate
pub fn refine_from_spectrum(
    spectrum_magnitudes: &[f32],
    rough_freq: f32,
    sample_rate: u32,
) -> Option<f32> {
    if rough_freq <= 0.0 { return None; }
    let buffer_size = spectrum_magnitudes.len() * 2;
    let target_bin = (rough_freq * buffer_size as f32) / sample_rate as f32;
    let search_radius = 2.0;
    let start_bin = (target_bin - search_radius).max(0.0) as usize;
    let end_bin = (target_bin + search_radius).min((spectrum_magnitudes.len() - 1) as f32) as usize;
    if start_bin >= end_bin { return Some(rough_freq); }

    let peak_bin_result = spectrum_magnitudes[start_bin..=end_bin]
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal));
        
    let peak_bin = if let Some((offset, _)) = peak_bin_result {
        start_bin + offset
    } else {
        return Some(rough_freq);
    };

    if peak_bin == 0 || peak_bin >= spectrum_magnitudes.len() - 1 { return Some(rough_freq); }

    let y1 = spectrum_magnitudes[peak_bin - 1].ln();
    let y2 = spectrum_magnitudes[peak_bin].ln();
    let y3 = spectrum_magnitudes[peak_bin + 1].ln();

    if !y1.is_finite() || !y2.is_finite() || !y3.is_finite() { return Some(rough_freq); }

    let denominator = 2.0 * y2 - y1 - y3;
    if denominator.abs() < 1e-6 { return Some(rough_freq); }

    let peak_shift = (y3 - y1) / (2.0 * denominator);
    let interpolated_bin = peak_bin as f32 + peak_shift;
    let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

    if final_freq.is_finite() && final_freq > 0.0 {
        Some(final_freq)
    } else {
        Some(rough_freq)
    }
}