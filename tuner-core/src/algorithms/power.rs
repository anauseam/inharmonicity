//! Power and Spectral Energy algorithms used for Gatekeeper analysis and thresholding.

/// Calculates RMS (Root Mean Square) amplitude to detect silence or measure total energy.
pub fn calculate_rms(buffer: &[f32]) -> f32 {
    let sum_sq: f32 = buffer.iter().map(|&x| x * x).sum();
    (sum_sq / buffer.len() as f32).sqrt()
}

/// Calculates Exponential Moving Average (EMA).
/// If `previous_ema` is 0.0, it initializes to `current_val` to prevent slow ramp-up.
pub fn calculate_ema(current_val: f32, previous_ema: f32, alpha: f32) -> f32 {
    if previous_ema == 0.0 {
        current_val
    } else {
        (current_val * alpha) + (previous_ema * (1.0 - alpha))
    }
}

/// Calculus of Complex Spectral Difference (CSD).
/// Measures the Euclidean distance between successive complex spectra frames.
/// Highly useful for detecting transients (e.g., a hammer striking a piano string).
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
/// Defines a metric for the "peakiness" or harmonicity of the spectrum.
/// A lower value indicates noise, and a higher value indicates a sparse (tonal) spectrum.
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
