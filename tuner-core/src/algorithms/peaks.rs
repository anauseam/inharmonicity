//! # Spectral Peak Extraction
//!
//! Stateless DSP module for extracting sub-bin accurate spectral peaks
//! from magnitude spectra.

use rustfft::num_complex::Complex;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SpectralPeak {
    /// True frequency in Hz (sub-bin interpolated via the Jacobsen estimator (Candan 2015)).
    pub frequency: f32,
    /// Linear magnitude at this peak.
    pub magnitude: f32,
}

/// Extracts all significant spectral peaks from a magnitude spectrum with sub-bin
/// interpolated frequencies using the complex-domain Jacobsen estimator.
///
/// # Algorithm
/// 1. Walk magnitudes to find local maxima (`mag[i] > mag[i-1]` AND `mag[i] > mag[i+1]`).
/// 2. Filter out peaks below `min_magnitude` (absolute threshold).
/// 3. For each surviving peak, apply the Jacobsen estimator on the `complex_spectrum`
///    for Hann-optimal sub-bin frequency interpolation.
/// 4. Sort peaks by magnitude descending. Store in `peaks_out`.
///
/// # Arguments
/// * `magnitudes` — Linear magnitude spectrum (output of `spectrum_to_magnitudes`).
/// * `complex_spectrum` — Complex frequency spectrum from the RFFT.
/// * `sample_rate` — Audio sample rate in Hz.
/// * `fft_size` — FFT window size (e.g. 8192).
/// * `min_magnitude` — Absolute minimum linear magnitude threshold for a peak to be considered.
/// * `peaks_out` — Mutable slice to write peaks into.
///
/// # Returns
/// The number of peaks extracted (up to `peaks_out.len()`).
pub fn extract_peaks(
    magnitudes: &[f32],
    complex_spectrum: &[Complex<f32>],
    sample_rate: u32,
    fft_size: usize,
    min_magnitude: f32,
    peaks_out: &mut [SpectralPeak],
) -> usize {
    if magnitudes.len() < 3 || peaks_out.is_empty() {
        return 0;
    }

    let noise_floor = min_magnitude;
    if noise_floor <= 0.0 {
        return 0; // Empty spectrum or invalid threshold
    }

    let hz_per_bin = sample_rate as f32 / fft_size as f32;

    let mut temp_peaks = [SpectralPeak::default(); 128];
    let mut num_found = 0;

    // Walk magnitudes to find local maxima (avoid boundaries)
    for i in 1..(magnitudes.len() - 1) {
        let mag = magnitudes[i];

        if mag > noise_floor && mag > magnitudes[i - 1] && mag > magnitudes[i + 1] {
            // Jacobsen estimator (Candan 2015 — optimal for Hann windows)
            // Citation: Candan, Ç. (2015). Signal Processing, 114, 245-250.
            // DOI: 10.1016/j.sigpro.2015.03.009
            // Our Hann window is defined from [0, N-1], which is a time-shift of N/2
            // relative to the zero-centered window [-N/2, N/2-1] assumed by the estimator.
            // By the Fourier Shift Theorem, we must multiply bin `m` by e^(j*pi*m) = (-1)^m
            // to correct the phase before applying the complex formula.
            let sign_prev = if (i - 1) % 2 == 0 { 1.0 } else { -1.0 };
            let sign_peak = if i % 2 == 0 { 1.0 } else { -1.0 };
            let sign_next = if (i + 1) % 2 == 0 { 1.0 } else { -1.0 };

            let x_prev = complex_spectrum[i - 1] * sign_prev;
            let x_peak = complex_spectrum[i] * sign_peak;
            let x_next = complex_spectrum[i + 1] * sign_next;

            let numerator = x_prev - x_next;
            let denominator = Complex::new(2.0, 0.0) * x_peak - x_prev - x_next;

            let delta = if denominator.norm_sqr() > 1e-12 {
                (numerator / denominator).re
            } else {
                0.0
            };

            let interpolated_bin = i as f32 + delta;
            let frequency = interpolated_bin * hz_per_bin;

            if frequency > 0.0 && num_found < temp_peaks.len() {
                temp_peaks[num_found] = SpectralPeak {
                    frequency,
                    magnitude: mag,
                };
                num_found += 1;
            }
        }
    }

    let valid_peaks = &mut temp_peaks[..num_found];
    // Sort temp_peaks by magnitude descending
    valid_peaks.sort_unstable_by(|a, b| {
        b.magnitude
            .partial_cmp(&a.magnitude)
            .unwrap_or(core::cmp::Ordering::Equal)
    });

    // Copy to peaks_out up to its capacity
    let count = num_found.min(peaks_out.len());
    peaks_out[..count].copy_from_slice(&valid_peaks[..count]);

    count
}

/// ── Gómez (2006) Peak Masking & SMS Dynamic Range ────────────────
/// Filters out acoustic side-lobes, sympathetic resonance, and intermodulation
/// distortion that cause TWM to sub-harmonically false-lock.
///
/// # Preconditions
/// The `peaks` slice must contain no more than 64 elements. If it is larger,
/// it will be artificially truncated to 64 to fit the internal tracking array.
///
/// # Reference
/// 1. Gómez, E. (2006). "Tonal Description of Music Audio Signals." PhD Thesis, MTG - Universitat Pompeu Fabra. Section 3.1.2.2.
/// 2. Cano, P. (1998). "Fundamental Frequency Estimation in the SMS Analysis". DAFX.
/// (Note: University theses and DAFx conference proceedings typically do not issue DOIs).
///
/// # Algorithm
/// Peaks are evaluated in descending amplitude order. First, any
/// peak outside the 40 dB dynamic range of the global maximum is discarded (Cano).
/// Then, a dominant peak masks any smaller peak that falls within its proportional
/// critical band if the smaller peak is below a relative masking threshold (Gómez).
pub fn mask_peaks(peaks: &mut [SpectralPeak]) -> usize {
    if peaks.is_empty() {
        return 0;
    }

    let k = peaks.len().min(64);
    let active_peaks = &mut peaks[..k];

    // 1. Sort by magnitude descending
    active_peaks.sort_unstable_by(|a, b| b.magnitude.partial_cmp(&a.magnitude).unwrap());

    let mut valid_count = 0;
    let mut masked = [false; 64];
    let global_max = active_peaks[0].magnitude;

    // Canonical Gómez/Essentia Defaults & SMS dynamic range
    const GLOBAL_THRESHOLD_DB: f32 = 0.0316; // -30 dB from global max
    const MASK_THRESHOLD_DB: f32 = 0.0316; // -30 dB relative to masker
    const MASK_BANDWIDTH_PROPORTION: f32 = 0.20; // 20% proportional bandwidth

    for i in 0..k {
        if masked[i] {
            continue;
        }

        // Absolute structural threshold (SMS Rule): -40 dB from the global maximum.
        // Prevents the engine from analyzing isolated microscopic acoustic room noise.
        if active_peaks[i].magnitude < global_max * GLOBAL_THRESHOLD_DB {
            continue;
        }

        let masker_freq = active_peaks[i].frequency;
        let masker_mag = active_peaks[i].magnitude;

        let mask_threshold = masker_mag * MASK_THRESHOLD_DB;
        let mask_bw = masker_freq * MASK_BANDWIDTH_PROPORTION;

        // Mask neighboring weaker peaks
        for j in (i + 1)..k {
            if !masked[j] {
                let target_freq = active_peaks[j].frequency;
                let target_mag = active_peaks[j].magnitude;

                if (target_freq - masker_freq).abs() < mask_bw && target_mag < mask_threshold {
                    masked[j] = true;
                }
            }
        }

        // Retain valid peak
        active_peaks[valid_count] = active_peaks[i];
        valid_count += 1;
    }

    // Sort by frequency ascending for O(N+K) two-pointer sweep in TWM
    active_peaks[..valid_count]
        .sort_unstable_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

    valid_count
}
