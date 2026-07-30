//! # Power & Spectral Energy Algorithms
//!
//! Stateless functions for measuring signal power and spectral characteristics.
//! These are the core metrics used by the [`Gatekeeper`](crate::gatekeeper::Gatekeeper)
//! to evaluate signal stability and drive its 5-state machine.
//!
//! | Function | Used By | Purpose |
//! |---|---|---|
//! | [`rms`] | Gatekeeper State 0 | Silence gating — below threshold = IDLE |
//! | [`ema`] | Gatekeeper State 0 | Smooths RMS to ignore momentary dips |
//! | [`nhwrsf`] | Gatekeeper States 1–2 | Detects transients (hammer strikes) |
//! | [`ninos2`] | Gatekeeper State 3 | Measures spectral sparsity (tonal stability) |

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
pub fn rms(buffer: &[f32]) -> f32 {
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
///
/// Faithfulness notes (faithfulness-audit-05): same one-pole smoother as the
/// paper's level-detector ballistics, with the opposite symbol convention
/// (their α multiplies the *previous* output; ours the current input — swap
/// α ↔ 1−α). The Gatekeeper's dynamic alpha (α = 1 while rising, slow α on
/// decay) is the paper's decoupled attack/release peak-detector pattern.
/// The `previous_ema == 0.0` cold-start shortcut is OURS: it treats exact
/// zero as "uninitialized", which is correct here because the Gatekeeper
/// resets the EMAs to 0.0 in Silence precisely so they re-seed instantly on
/// the next active frame.
pub fn ema(current_val: f32, previous_ema: f32, alpha: f32) -> f32 {
    if previous_ema == 0.0 {
        current_val
    } else {
        (current_val * alpha) + (previous_ema * (1.0 - alpha))
    }
}

/// Calculates the Normalized Half-Wave Rectified Spectral Flux (NHWRSF).
///
/// This measures the increase in transient energy between two frames by summing
/// the positive magnitude differences across a fixed frequency band (≈43 Hz to
/// 10 kHz), then normalizing it against the total signal energy of the current
/// frame.
///
/// # Provenance (faithfulness-audit-05)
/// The core — half-wave-rectified L1 spectral flux, Σ H(|X_n|−|X_{n−1}|) — is
/// the canonical onset-detection function:
/// * Masri, P. (1996). PhD thesis, University of Bristol (origin).
/// * Bello, J.P. et al. (2005). "A Tutorial on Onset Detection in Music
///   Signals." IEEE Trans. Speech Audio Process. 13(5).
/// * Dixon, S. (2006). "Onset Detection Revisited." DAFx-06 — this exact
///   HWR-L1 form. (Also restated as Eq. 1 in Mounir et al. 2021.)
///
/// Two modifications are OURS: (a) the analysis band (≈43 Hz–10 kHz — below
/// that is room rumble beneath A0, above is percussive/air noise), derived at
/// runtime from the FFT configuration; (b) the normalization by the current
/// frame's total magnitude, which makes the flux dimensionless and
/// scale-invariant (hardware/gain-agnostic) — the
/// papers' SF is unnormalized (LSF gets robustness from a log instead).
///
/// # Arguments
/// * `current_spectrum` — The complex frequency spectrum of the current frame.
/// * `prev_spectrum_mags` — Mutable slice of the previous frame's magnitudes.
///   This is updated in-place to prime it for the next frame.
/// * `window_size` — FFT window size that produced `current_spectrum`.
/// * `sample_rate` — Audio sample rate in Hz.
///
/// # Returns
/// A normalized, dimensionless float representing the transient flux.
pub fn nhwrsf(
    current_spectrum: &[rustfft::num_complex::Complex<f32>],
    prev_spectrum_mags: &mut [f32],
    window_size: usize,
    sample_rate: u32,
) -> f32 {
    // Analysis band, derived from the FFT config (was hardcoded bins 2/464
    // for 2048 @ 44.1 kHz; these formulas reproduce those bins exactly there).
    const BAND_LOW_HZ: f32 = 43.0;
    const BAND_HIGH_HZ: f32 = 10_000.0;
    let hz_per_bin = sample_rate as f32 / window_size as f32;
    let start_bin = (BAND_LOW_HZ / hz_per_bin).round() as usize; // 2 @ 2048/44.1k
    let end_bin = (BAND_HIGH_HZ / hz_per_bin) as usize; // 464 @ 2048/44.1k

    let mut total_flux = 0.0;
    let mut current_energy = 0.0;

    // Ensure we don't panic if buffers are small for some reason
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

        // Buffer maintenance for the next frame
        prev_spectrum_mags[k] = mag;
    }

    // Secure against division-by-zero
    total_flux / (current_energy + 1e-6)
}

/// Spectral sparsity ratio — the Gatekeeper's tonality gate.
///
/// Quantifies how "peaky" (tonal) vs. "flat" (noisy) a spectrum is. A pure
/// tone concentrates energy in a few bins → high value (→ N). White noise
/// spreads energy evenly → ≈ 1. The Gatekeeper uses this in State 3
/// (HARMONIC DECAY) to identify the "Golden Window" where the spectrum is
/// sparse enough for a high-quality capture.
///
/// # Formula and formal identity
///   S = N × (∑ |X_k|²) / (∑ |X_k|)²  =  N / N_eff  =  N × (ℓ²/ℓ¹)²
///
/// where ℓ²/ℓ¹ = ‖X‖₂/‖X‖₁ is the sparsity measure **defined** in Hurley &
/// Rickard (2009), Table I (restated in the proof of their Theorem 4.1),
/// and N_eff = (‖X‖₁/‖X‖₂)² is the **Cauchy–Schwarz effective support
/// size** (‖X‖₁² ≤ ‖X‖₀·‖X‖₂² ⇒ N_eff ≤ ‖X‖₀) — equivalently the
/// **participation ratio** (Bell & Dean 1970) of the ℓ¹-normalized
/// magnitude distribution, whose reciprocal is the
/// Herfindahl–Hirschman/Simpson concentration index. At fixed N, S is a
/// strictly increasing transform of ℓ²/ℓ¹. Against Hurley & Rickard's six
/// sparsity criteria, S satisfies **all six**: D1 Robin Hood, D2 Scaling,
/// P1 Bill Gates carry over from their Theorems 4.1/A.5 (strict monotone
/// transform at fixed N); the ×N factor makes S exactly clone-invariant
/// (D4) and strictly zero-padding-increasing (P2), the two criteria bare
/// ℓ²/ℓ¹ fails; D3 Rising Tide holds by direct derivation (their Table III
/// mis-marks ℓ²/ℓ¹ on D3 — proof in the faithfulness-audit-05 addendum).
/// Hoyer (2004, §3.1) sparseness, (√N − ‖X‖₁/‖X‖₂)/(√N − 1), is an affine
/// function of the same ℓ¹/ℓ² ratio (H&R: "a normalized version of the
/// ℓ²/ℓ¹ measure") and hence strictly monotone in S at fixed N — related,
/// but not affine in S. Endpoints: 1-sparse → N; flat → ≈ 1 (exactly
/// len/(len−1) ≈ 1.001, since the DC bin is excluded from the sums but
/// counted in N).
///
/// # Provenance (OURS, Mounir-inspired — faithfulness-audit-05)
/// The *idea* that spectral sparsity separates a note's transient from its
/// tonal steady state is due to Mounir et al.; the *measure* here is ours,
/// not their NINOS². The deviations are deliberate for a *tonality gate*
/// rather than an onset ODF: linear magnitudes over ALL bins, no energy
/// factor (the gate must be level-independent), high =
/// tonal. A/B against the faithful variants (`examples/sparsity_ab.rs`)
/// showed the two are complementary by register; any swap is gated on
/// instrument #2. (The function name is historical.)
///
/// # Arguments
/// * `spectrum` — Complex frequency spectrum from an FFT. The DC bin (index 0) is skipped.
///
/// # Returns
/// A non-negative `f32`. Higher values indicate a sparser (more tonal) spectrum.
/// For white noise, the value approaches `1.0`.
///
/// # References
/// * Hurley, N. & Rickard, S. (2009). "Comparing Measures of Sparsity."
///   IEEE Trans. Inf. Theory 55(10), 4723–4741. Table I (ℓ²/ℓ¹
///   definition), Theorems 4.1/A.5 (Robin Hood / Bill Gates), Table III
///   (criteria comparison). arXiv:0811.4706 in `resources/gatekeeper/`.
/// * Hoyer, P. O. (2004). "Non-negative Matrix Factorization with
///   Sparseness Constraints." JMLR 5, 1457–1469, §3.1 (the sparseness
///   definition — unnumbered display). PDF in `resources/curve/`.
/// * Bell, R. J. & Dean, P. (1970). "Atomic vibrations in vitreous
///   silica." Discuss. Faraday Soc. 50, 55–61. (Participation ratio.)
/// * Mounir, M., Karsmakers, P., van Waterschoot, T. (2021). EURASIP JASMP
///   2021:30. (Inspiration: sparsity-based note-phase segmentation.)
pub fn ninos2(spectrum: &[rustfft::num_complex::Complex<f32>]) -> f32 {
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
