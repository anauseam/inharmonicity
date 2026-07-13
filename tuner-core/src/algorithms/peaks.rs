//! # Spectral Peak Extraction
//!
//! Stateless DSP module for extracting sub-bin accurate spectral peaks
//! from magnitude spectra.

use rustfft::num_complex::Complex;

use crate::algorithms::spectral;
use crate::models::SpectralPeak;

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
/// * `magnitudes` — Linear magnitude spectrum (output of `magnitude_spectrum`).
/// * `complex_spectrum` — Complex frequency spectrum from the RFFT.
/// * `sample_rate` — Audio sample rate in Hz.
/// * `fft_size` — FFT window size (e.g. 8192).
/// * `min_magnitude` — Absolute minimum linear magnitude threshold for a peak to be
///   considered. (Discovery passes a Neyman–Pearson AWGN false-alarm threshold
///   computed per frame — see the Kay 1998 derivation in `engine.rs`.)
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

    let mut temp_peaks = [SpectralPeak::default(); 128];
    let mut num_found = 0;

    // Walk magnitudes to find local maxima (avoid boundaries)
    for i in 1..(magnitudes.len() - 1) {
        let mag = magnitudes[i];

        if mag > noise_floor && mag > magnitudes[i - 1] && mag > magnitudes[i + 1] {
            // Sub-bin refinement via the complex-domain Jacobsen estimator (Candan 2015).
            let frequency = spectral::jacobsen(complex_spectrum, i, fft_size, sample_rate);

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

/// ── Peak Masking & Dynamic-Range Gate (OURS — empirically validated) ─────
/// Filters out acoustic side-lobes, sympathetic resonance, and intermodulation
/// distortion that cause TWM to sub-harmonically false-lock.
///
/// # Provenance (faithfulness-audit-04)
/// This is the codebase's own heuristic, NOT a paper port — validated on real
/// captures in ADR 0002 (2026-05-28: replaced the failed geometric gate; 8/8
/// keys, zero false locks; known limitation: environments with SNR ≲ 30 dB).
/// * The **global dynamic-range gate** adapts Cano (1998) §4.3, which accepts
///   only peaks "less than 40 dB below the highest peak"; we ship the stricter
///   −30 dB that ADR 0002 validated.
/// * The **dominance masking** (a louder peak suppresses smaller peaks within
///   a proportional band) is ours; the 20 % bandwidth matches the textbook
///   critical-band approximation (CB ≈ 0.2·f above ~500 Hz) — inspiration,
///   not a port. No masking procedure exists in Gómez (2006) or Cano (1998);
///   do not re-cite them for it (see faithfulness-audit-04).
///
/// # Preconditions
/// The `peaks` slice must contain no more than 64 elements. If it is larger,
/// it will be artificially truncated to 64 to fit the internal tracking array.
///
/// # Reference
/// 1. ADR 0002 (`docs/adr/0002-twm-peak-masking-validation.md`) — the
///    empirical basis for the mechanism and the −30 dB values.
/// 2. Cano, P. (1998). "Fundamental Frequency Estimation in the SMS Analysis."
///    DAFx-98, §4.3 — the dynamic-range rule the global gate adapts.
///
/// # Algorithm
/// Peaks are evaluated in descending amplitude order. First, any peak more
/// than 30 dB below the global maximum is discarded (Cano's 40 dB rule,
/// tightened per ADR 0002). Then, a dominant peak masks any smaller peak that
/// falls within its proportional critical band if the smaller peak is below a
/// relative masking threshold.
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

    // OUR constants, ADR 0002-validated (not from Gómez/Cano — see doc-comment).
    const GLOBAL_THRESHOLD_DB: f32 = 0.0316; // −30 dB from global max (Cano §4.3 proposes 40 dB; ADR 0002 validated 30)
    const MASK_THRESHOLD_DB: f32 = 0.0316; // −30 dB relative to masker
    const MASK_BANDWIDTH_PROPORTION: f32 = 0.20; // ≈ textbook critical band (CB ≈ 0.2·f above ~500 Hz)

    for i in 0..k {
        if masked[i] {
            continue;
        }

        // Global dynamic-range gate: −30 dB from the frame's maximum (ADR 0002).
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
