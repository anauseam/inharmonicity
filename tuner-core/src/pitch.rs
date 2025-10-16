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

/// Finds the partials (overtones) of a note from its magnitude spectrum.
///
/// This function uses a guided search, looking for spectral peaks near the expected
/// integer multiples of the fundamental frequency. This makes it robust against
/// picking up unrelated noise. Each found peak's frequency is then refined
/// using parabolic interpolation for maximum accuracy.
///
/// # Arguments
/// * `spectrum_magnitudes` - Magnitude spectrum from an FFT.
/// * `fundamental_freq` - The fundamental frequency ($f_0$) of the note, used to guide the search.
/// * `sample_rate` - The sample rate of the original audio.
/// * `max_partials` - The maximum number of partials (overtones) to search for.
///
/// # Returns
/// * `Vec<f32>` - A vector containing the refined frequencies of the detected partials.
pub fn find_partials(
    spectrum_magnitudes: &[f32],
    fundamental_freq: f32,
    sample_rate: u32,
    max_partials: u32,
) -> Vec<f32> {
    if fundamental_freq <= 0.0 {
        return vec![];
    }

    let mut partial_freqs = Vec::new();
    let buffer_size = spectrum_magnitudes.len() * 2;

    // A relative threshold to ignore noise. A peak must be at least 5% of the
    // magnitude of the fundamental's peak to be considered a partial.
    let fundamental_bin = (fundamental_freq * buffer_size as f32) / sample_rate as f32;
    let peak_threshold = if let Some(mag) = spectrum_magnitudes.get(fundamental_bin.round() as usize) {
        mag * 0.05
    } else {
        0.0 // No fundamental found, so we can't find partials
    };

    if peak_threshold == 0.0 { return vec![]; }

    // Start the loop at n=2 to find the first overtone (2nd harmonic) and go up from there.
    // To still find `max_partials` number of overtones, we loop to `max_partials + 1`.
    for n in 2..=(max_partials + 1) {
        let expected_freq = fundamental_freq * n as f32;
        
        // Stop if we go past the Nyquist frequency
        if expected_freq > sample_rate as f32 / 2.0 {
            break;
        }

        // Define a search window in Hz around the expected frequency.
        // A wider window is needed for higher, more inharmonic partials.
        let search_width_hz = fundamental_freq * 0.5;

        // Convert frequency window to bin indices
        let target_bin = (expected_freq * buffer_size as f32) / sample_rate as f32;
        let bin_width = (search_width_hz * buffer_size as f32) / sample_rate as f32;
        let start_bin = ((target_bin - bin_width / 2.0).max(0.0) as usize)
            .min(spectrum_magnitudes.len() -1);
        let end_bin = ((target_bin + bin_width / 2.0)
            .min((spectrum_magnitudes.len() - 1) as f32) as usize)
            .max(start_bin);

        if start_bin >= end_bin { continue; }

        // Find the bin with the highest magnitude within our search window
        // FIX: The closure in `max_by` now correctly handles the Option returned by `partial_cmp`.
        // We provide a fallback ordering (`Less`) in case of NaN values, which is safe.
        let peak_in_window = spectrum_magnitudes[start_bin..=end_bin]
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Less));

        if let Some((offset, &magnitude)) = peak_in_window {
            // Check if the peak is strong enough to be considered a partial
            if magnitude > peak_threshold {
                let peak_bin = start_bin + offset;
                if let Some(refined_freq) = interpolate_peak_frequency(spectrum_magnitudes, peak_bin, sample_rate) {
                    partial_freqs.push(refined_freq);
                }
            }
        }
    }

    partial_freqs
}

/// Refines a frequency estimate using parabolic interpolation on the FFT spectrum.
///
/// This is a private helper function used by `refine_from_spectrum` and `find_partials`.
/// Given a peak bin, it uses the magnitude of that bin and its neighbors to
/// estimate the true peak location with sub-bin accuracy.
///
/// # Arguments
/// * `spectrum_magnitudes` - Magnitude spectrum from an FFT.
/// * `peak_bin` - The index of the peak bin to be refined.
/// * `sample_rate` - The sample rate of the original audio.
///
/// # Returns
/// * `Some(refined_freq)` if successful, otherwise `None`.
fn interpolate_peak_frequency(
    spectrum_magnitudes: &[f32],
    peak_bin: usize,
    sample_rate: u32,
) -> Option<f32> {
    // Ensure we have neighbors for interpolation
    if peak_bin == 0 || peak_bin >= spectrum_magnitudes.len() - 1 {
        return None;
    }

    let y1 = spectrum_magnitudes[peak_bin - 1].ln();
    let y2 = spectrum_magnitudes[peak_bin].ln();
    let y3 = spectrum_magnitudes[peak_bin + 1].ln();

    // Avoid division by zero or NaN results from non-finite log values
    if !y1.is_finite() || !y2.is_finite() || !y3.is_finite() {
        return None;
    }

    let denominator = 2.0 * y2 - y1 - y3;
    if denominator.abs() < 1e-6 {
        return None;
    }
    
    let peak_shift = (y3 - y1) / (2.0 * denominator);
    let interpolated_bin = peak_bin as f32 + peak_shift;
    let buffer_size = spectrum_magnitudes.len() * 2;
    let final_freq = (interpolated_bin * sample_rate as f32) / buffer_size as f32;

    if final_freq.is_finite() && final_freq > 0.0 {
        Some(final_freq)
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
    if rough_freq <= 0.0 { return Some(rough_freq); }
    let buffer_size = spectrum_magnitudes.len() * 2;
    let target_bin = (rough_freq * buffer_size as f32) / sample_rate as f32;
    
    // Search a very small radius since our rough_freq should be close
    let search_radius = 2.0;
    let start_bin = (target_bin - search_radius).max(0.0) as usize;
    let end_bin = (target_bin + search_radius).min((spectrum_magnitudes.len() - 1) as f32) as usize;
    if start_bin >= end_bin { return Some(rough_freq); }

    // Find the actual peak bin near the rough estimate
    // FIX: The closure in `max_by` now correctly handles the Option returned by `partial_cmp`.
    // We provide a fallback ordering (`Less`) in case of NaN values, which is safe.
    let peak_bin_result = spectrum_magnitudes[start_bin..=end_bin]
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Less));
        
    let peak_bin = if let Some((offset, _)) = peak_bin_result {
        start_bin + offset
    } else {
        return Some(rough_freq); // No peak found, return original
    };

    // Use our new helper for the final interpolation
    interpolate_peak_frequency(spectrum_magnitudes, peak_bin, sample_rate)
        .or(Some(rough_freq)) // If interpolation fails, fall back to the rough frequency
}