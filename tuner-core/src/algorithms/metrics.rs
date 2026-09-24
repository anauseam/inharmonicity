//! # Signal metrics
//!
//! Power and spectral-shape measures of a frame: RMS, a smoothing step, spectral
//! flux and spectral sparsity.

/// Root-mean-square amplitude of a buffer, `√((1/N)·Σ x_i²)`; `0.0` for silence.
pub fn rms(buffer: &[f32]) -> f32 {
    let sum_sq: f32 = buffer.iter().map(|&x| x * x).sum();
    (sum_sq / buffer.len() as f32).sqrt()
}

/// One exponential-moving-average step, `α·x + (1 − α)·previous`, with `alpha` in
/// 0–1 (higher is more responsive). A `previous_ema` of exactly `0.0` counts as
/// uninitialised and returns `current_val`.
///
/// # References
/// Giannoulis, D., Massberg, M., and Reiss, J. D. (2012). "Digital Dynamic Range
/// Compressor Design—A Tutorial and Analysis." *Journal of the Audio Engineering
/// Society*, 60(6), 399–408. The same one-pole smoother as its level-detector
/// ballistics, with α on the input rather than the previous output (α ↔ 1 − α).
// The zero-means-uninitialised shortcut is ours, and relies on the Gatekeeper
// resetting its EMAs to 0.0 in silence.
pub fn ema(current_val: f32, previous_ema: f32, alpha: f32) -> f32 {
    if previous_ema == 0.0 {
        current_val
    } else {
        (current_val * alpha) + (previous_ema * (1.0 - alpha))
    }
}

/// Normalised half-wave rectified spectral flux: the rise in magnitude since the
/// previous frame over ≈ 43 Hz–10 kHz, divided by this frame's total magnitude,
/// so it is dimensionless.
///
/// `prev_spectrum_mags` holds the previous frame's magnitudes and is overwritten
/// with this frame's; `window_size` is the FFT length behind `current_spectrum`.
///
/// Half-wave-rectified L1 spectral flux, Σ H(|X_n| − |X_{n−1}|), is the canonical
/// onset-detection function. Two changes are ours: the band, which excludes rumble
/// below A0 and air noise above 10 kHz, and the normalisation by total magnitude,
/// which makes the flux independent of gain where the papers' flux is unnormalised.
///
/// # References
/// * Masri, P. (1996). PhD thesis, University of Bristol (origin).
/// * Bello, J.P. et al. (2005). "A Tutorial on Onset Detection in Music
///   Signals." IEEE Trans. Speech Audio Process. 13(5).
/// * Dixon, S. (2006). "Onset Detection Revisited." DAFx-06 (this HWR-L1 form).
pub fn nhwrsf(
    current_spectrum: &[rustfft::num_complex::Complex<f32>],
    prev_spectrum_mags: &mut [f32],
    window_size: usize,
    sample_rate: u32,
) -> f32 {
    // Analysis band, derived from the FFT config.
    const BAND_LOW_HZ: f32 = 43.0;
    const BAND_HIGH_HZ: f32 = 10_000.0;
    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let start_bin = (BAND_LOW_HZ / hz_per_bin).round() as usize; // 2 @ 2048/44.1k
    let end_bin = (BAND_HIGH_HZ / hz_per_bin) as usize; // 464 @ 2048/44.1k

    let mut total_flux = 0.0;
    let mut current_energy = 0.0;

    let limit = current_spectrum
        .len()
        .min(prev_spectrum_mags.len())
        .min(end_bin + 1);

    let start = start_bin.min(limit);

    for k in start..limit {
        let c = current_spectrum[k];
        let mag = (c.re * c.re + c.im * c.im).sqrt();

        current_energy += mag;

        let diff = mag - prev_spectrum_mags[k];
        if diff > 0.0 {
            total_flux += diff;
        }

        prev_spectrum_mags[k] = mag;
    }

    total_flux / (current_energy + 1e-6)
}

/// Inverse participation ratio of the magnitude spectrum, scaled by the bin
/// count — the Gatekeeper's tonality measure.
///
/// A tone concentrates energy in a few bins and reads toward N; white noise
/// spreads it evenly and reads ≈ 1.
///
/// ```text
///   S = N × (∑ |X_k|²) / (∑ |X_k|)²  =  N / N_eff
/// ```
///
/// N_eff = (∑ |X_k|)² / ∑ |X_k|² is the participation ratio (Bell & Dean 1970)
/// of the ℓ¹-normalized magnitudes — the effective number of occupied bins — so
/// S is its inverse scaled by N. Equivalently S = N·(ℓ²/ℓ¹)², the sparsity
/// measure Hurley & Rickard (2009) define, of which Hoyer's (2004) sparseness is
/// a monotone function. The DC bin is excluded from the sums but counted in N,
/// so a flat spectrum gives len/(len−1) ≈ 1.001.
///
/// The measure is established; applying it here is ours. The idea that spectral
/// sparsity separates a note's transient from its tonal steady state is Mounir
/// et al.'s (2021), whose NINOS² is an onset detection function. This gate is
/// not NINOS²: it takes linear magnitudes over all bins with no energy factor, so
/// it stays level-independent.
///
/// # References
/// * Bell, R. J. & Dean, P. (1970). "Atomic vibrations in vitreous silica."
///   Discuss. Faraday Soc. 50, 55–61.
/// * Hurley, N. & Rickard, S. (2009). "Comparing Measures of Sparsity."
///   IEEE Trans. Inf. Theory 55(10), 4723–4741.
/// * Hoyer, P. O. (2004). "Non-negative Matrix Factorization with Sparseness
///   Constraints." JMLR 5, 1457–1469.
/// * Mounir, M., Karsmakers, P. & van Waterschoot, T. (2021). EURASIP JASMP
///   2021:30.
// Measured head-to-head against the faithful Mounir variants (`cargo lab
// gatekeeper sparsity`): the two are complementary by register, so a swap is a
// real candidate. The A/B has run on one instrument; do not swap until it has
// run on a second.
// audit 05
pub fn inverse_participation_ratio(spectrum: &[rustfft::num_complex::Complex<f32>]) -> f32 {
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

    (sum_mag_sq * spectrum.len() as f32) / (sum_mag * sum_mag)
}
