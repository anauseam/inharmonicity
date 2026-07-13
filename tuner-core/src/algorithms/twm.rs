//! # Two-Way Mismatch (TWM)
//!
//! Continuous, sub-bin-precision fundamental-frequency scoring via the
//! Maher & Beauchamp (1994) distance-sum formulation. Stateless.

use crate::models::{KeyProfile, SpectralPeak};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwmConfig {
    pub p: f32,              // frequency weighting exponent
    pub q: f32,              // amplitude penalty scaling
    pub r: f32,              // reward constant
    pub rho: f32,            // reverse error weight
    pub lambda_penalty: f32, // Duan M→P error ceiling
    /// EXPERIMENT (test #1): when true, the forward error Err_{p→m} is SUMMED
    /// instead of averaged (no /N). Tests the deep-research claim that the /N
    /// normalization launders a dense candidate's many predicted-but-absent
    /// partials into a small average (the bass-attractor root-cause hypothesis).
    /// Default false ⇒ canonical M&B behavior, byte-identical.
    pub sum_forward: bool,
    /// EXPERIMENT (n-kernel): forward-error deadzone scaling. The predicted
    /// partial n carries a B-uncertainty `δf_n ≈ c·B·n²·f_n / (2(1+Bn²))` (the
    /// stiff-string law's ∂f_n/∂B propagated with relative σ_B absorbed into c).
    /// We forgive forward distance up to that deadzone: `eff_Δf = max(0, Δf − tol_n)`.
    /// Trades B-tolerance (wider = absorb per-note B scatter) against octave
    /// discrimination (wider = forgive the octave candidate's inharmonic
    /// divergence). c≈0.14 ≈ the Rigaud σ_B; c=0 ⇒ off (byte-identical default).
    pub b_deadzone: f32,
    /// EXPERIMENT (Duan non-peak): per-partial penalty charged for each predicted
    /// partial that falls in the OBSERVED ACTIVE BAND `[min_obs, max_obs]` with no
    /// peak within the match tolerance — a "hallucinated" harmonic. UN-normalized
    /// (a count), so it scales with how many partials a candidate predicts where
    /// none exist — the principled inverse of the /N laundering, charging dense
    /// (bass) impostors for the N_gap channel. Below the lowest peak (the
    /// missing-fundamental zone) it does NOT apply, sparing legitimately-absent
    /// bass fundamentals. 0 ⇒ off (byte-identical default). See
    /// `docs/design/duan-likelihood-design.md`.
    pub nonpeak_penalty: f32,
    /// EXPERIMENT (Emiya smoothness): penalize the amplitude INCOHERENCE of the
    /// partials that ARE matched — Σ of squared second-differences of their
    /// log-amplitude sequence. A true note's matched partials follow a smooth decay
    /// (2nd-diff ≈ 0); a dense impostor's coincidental matches have random
    /// amplitudes (jagged). Distinguishes via amplitude coherence, NOT gap count,
    /// so a sparse-but-coherent bass note is spared (gated to ≥3 matched partials)
    /// while an incoherent impostor is charged — the inverse of the non-peak count's
    /// bass-crushing failure. 0 ⇒ off (byte-identical default).
    pub smoothness_penalty: f32,
}

impl Default for TwmConfig {
    fn default() -> Self {
        // Conservative tuned constants (ADR 0006): MOBO-tuned amplitude terms
        // (q, r raised) with p and the λ ceiling held at canonical values for
        // robustness. Real-capture validation: 71/87 → 74/87, bass preserved.
        // The canonical M&B constants (q=1.4, r=0.5, ρ=0.33) are pinned in
        // `test_twm_regression` as the math-regression guard.
        Self {
            p: 0.5,
            q: 3.88,
            r: 1.426,
            rho: 0.298,
            lambda_penalty: 18.0,
            sum_forward: false,
            b_deadzone: 0.0,
            nonpeak_penalty: 0.0,
            smoothness_penalty: 0.0,
        }
    }
}

/// Scores a single key profile against observed peaks using the canonical
/// Maher & Beauchamp (1994) Two-Way Mismatch formulation.
///
/// Implements Equations (1)–(3) from:
///   Maher, R.C. & Beauchamp, J.W. (1994). JASA 95(4), pp. 2256–2257.
///   DOI: 10.1121/1.408685
///
/// Per-term error weighting function E_w (applied in both Err_{p-m} and Err_{m-p}):
///   E_w = Δf·(f^-p) + (a/A_max) × [q·Δf·(f^-p) − r]
///
/// Total error (Eq. 3):
///   Err_total = Err_{p-m}/N + ρ·Err_{m-p}/K
///
/// The paper's empirically calibrated values (NOT the shipped defaults — see
/// `TwmConfig::default` and ADR 0006):
///   p = 0.5  (frequency weighting exponent)
///   q = 1.4  (amplitude penalty scaling)
///   r = 0.5  (reward constant for aligned strong peaks)
///   ρ = 0.33 (reverse error combination weight)
///
/// Returns Err_total. Note on units: the paper's E_w mixes dimensions —
/// Δf·f^-p carries Hz^(1-p) (Hz^0.5 at p = 0.5) while the r reward is a pure
/// number — so the score is a figure of merit, not a quantity in Hz. The /N
/// and /K normalizations make scores comparable across candidates regardless
/// of the number of active partials or observed peaks.
///
/// # Noise Floor Boundary
/// To prevent unbounded error accumulation from low-frequency noise (which causes
/// high-treble ghost locks), we enforce a topological boundary on the
/// Measured-to-Predicted error, adapting the noise-floor asymptote shown by:
///
/// Duan, Z., Pardo, B., & Zhang, C. (2010). "Multiple Fundamental Frequency
/// Estimation by Modeling Spectral Peaks and Non-Peak Regions."
/// IEEE TASLP 18(8). DOI: 10.1109/TASL.2010.2042119
///
/// We apply a ceiling to the distance penalty to model the asymptote to the
/// uniform noise distribution for distant peaks. The hard ceiling is our
/// approximation — a deliberate adaptation, not a port of Duan's likelihood
/// (see docs/audits/faithfulness-audit-01-twm.md).
///
/// Note: Parameters come from TwmConfig. The shipped default is the MOBO-tuned
/// conservative config (ADR 0006); the paper's canonical values are pinned in
/// `test_twm_regression` as the math-regression guard.
pub fn score_candidate(
    peaks: &[SpectralPeak],
    profile: &KeyProfile,
    scale: f32,
    cfg: &TwmConfig,
) -> f32 {
    let valid_count = profile.valid_partial_count;
    if valid_count == 0 || peaks.is_empty() {
        return f32::MAX;
    }

    // A_max: maximum amplitude across all K measured peaks (paper: A_max = max(a_k))
    // Peaks are passed sorted by frequency ascending, so we must scan for A_max.
    let mut a_max = 0.0_f32;
    let mut max_obs_freq = 0.0_f32;
    let mut min_obs_freq = f32::MAX;
    for peak in peaks {
        if peak.magnitude > a_max {
            a_max = peak.magnitude;
        }
        if peak.frequency > max_obs_freq {
            max_obs_freq = peak.frequency;
        }
        if peak.frequency < min_obs_freq {
            min_obs_freq = peak.frequency;
        }
    }
    // Guard (ours, not in the paper): a degenerate all-zero-magnitude frame
    // would make a/A_max divide by zero. Inert whenever any peak has magnitude.
    a_max = a_max.max(1e-6);

    // ── Dynamic Bandwidth Cap (M&B Step 2, generalized) ──────────────────────
    // The paper predicts N = ⌈f_max/f_fund⌉ harmonics (Step 2): the series ends
    // at the first harmonic at/above the highest measured partial. For the
    // harmonic series {n·f0} that count-form is equivalent to the cutoff-form
    //   n·f0 < f_max + f0,
    // because consecutive harmonics are spaced exactly f0 apart (exact for
    // non-integer f_max/f0; the integer case is measure-zero). We keep the
    // cutoff-form for the inharmonic series: include predicted partials with
    // f_n·scale ≤ max_obs + f0·scale. At B = 0 this IS Step 2. For B > 0 the
    // stiff-string spacing is stretched (f_{n+1} − f_n > f0), so this is the
    // conservative generalization: versus the count-form reading ("through the
    // first partial ≥ max_obs") it differs by at most the single edge partial
    // m = min{n : f_n ≥ max_obs}, dropped iff f_m > max_obs + f0·scale — i.e.,
    // we never evaluate a partial more than one fundamental above the observed
    // band, where it carries no discriminative value and would inflate
    // Err_{p-m} with unconstrained forward error from harmonics the instrument
    // does not physically produce in this frame. See
    // docs/audits/faithfulness-audit-01-twm.md, finding 3.
    let cutoff_freq = max_obs_freq + profile.f0_et * scale;
    let mut active_predicted = 0_usize;
    for &p_freq in &profile.predicted_partials[..valid_count] {
        let f_n = p_freq * scale;
        if f_n <= cutoff_freq {
            active_predicted += 1;
        } else {
            break; // predicted_partials is sorted ascending
        }
    }
    if active_predicted == 0 {
        // Defensive (ours): reachable only if every observed peak sits below
        // f₁·scale − f0·scale ≈ (B/2)·f0·scale (~20 Hz worst-case at C8) — no
        // real capture does this. Keeps the /N normalization well-defined.
        active_predicted = 1;
    }
    let predicted = &profile.predicted_partials[..active_predicted]; // N terms

    // ── Eq. (1): Err_{p-m} (Predicted-to-Measured) ──────────────────────────
    // For each of N predicted harmonics f_n, find the nearest measured partial.
    // a_n is the amplitude of that nearest measured partial.
    // Δf_n = |f_n - f_nearest_measured|
    //
    // Per-term:  Δf_n·(f_n^-p) + (a_n/A_max)·[q·Δf_n·(f_n^-p) − r]
    // O(N + K) Two-Pointer Sweep: Find nearest peak for each predicted partial
    //
    // Note on Architectural Constraints (Duan et al. 2010, Eq 7)
    // While we use Duan's Eq 3 for the M-to-P bound below, we do not apply
    // Duan's Eq 7 (Non-Peak Region Likelihood) for this P-to-M calculation.
    // Duan is a polyphonic estimator, which uses a strict likelihood cliff to
    // harshly punish missing partials (preventing false polyphonic combinations).
    // Because TWM is a monophonic acoustic estimator, applying a cliff here
    // would negatively penalize low bass strings (like C1) that naturally exhibit
    // "missing fundamentals" due to soundboard impedance. We therefore rely on
    // M&B's f_n^-p diluted distance penalty to preserve missing-fundamental robustness.
    let mut err_pm = 0.0_f32;
    let mut nonpeak_count = 0_u32; // Duan: predicted partials hallucinated in-band
    // Emiya smoothness: incremental Σ(2nd-diff)² of matched-partial log-amplitudes.
    let mut smooth_accum = 0.0_f32;
    let mut matched_n = 0_u32;
    let mut prev_la = 0.0_f32;
    let mut prev2_la = 0.0_f32;
    let mut j = 0;
    let b = profile.beta;
    for (idx, &p_freq) in predicted.iter().enumerate() {
        let f_n = p_freq * scale;
        // Advance j while the next peak is closer or equally close to f_n
        while j + 1 < peaks.len()
            && (peaks[j + 1].frequency - f_n).abs() <= (peaks[j].frequency - f_n).abs()
        {
            j += 1;
        }
        let raw_delta = (peaks[j].frequency - f_n).abs();
        let mut delta_f_n = raw_delta;
        let a_n = peaks[j].magnitude;

        // Duan non-peak: count this predicted partial as "hallucinated" if it sits in
        // the observed active band with no peak within the match tolerance (2% ≈ 35¢).
        // Below min_obs_freq is the missing-fundamental zone → never counted.
        let matched = raw_delta <= 0.02 * f_n;
        if cfg.nonpeak_penalty > 0.0 && f_n >= min_obs_freq && f_n <= max_obs_freq && !matched {
            nonpeak_count += 1;
        }

        // Emiya smoothness: accumulate the second difference of consecutive MATCHED
        // partials' log-amplitudes (a true note decays smoothly → ~0; coincidental
        // impostor matches jump → large). Gated to matched partials only.
        if cfg.smoothness_penalty > 0.0 && matched {
            let la = (a_n.max(1e-6)).ln();
            if matched_n >= 2 {
                let d2 = la - 2.0 * prev_la + prev2_la;
                smooth_accum += d2 * d2;
            }
            prev2_la = prev_la;
            prev_la = la;
            matched_n += 1;
        }

        // n-kernel deadzone: forgive forward distance up to the predicted partial's
        // B-uncertainty (c=0 ⇒ no-op, byte-identical). n is 1-based here.
        if cfg.b_deadzone > 0.0 {
            let nf = (idx + 1) as f32;
            let tol_n = cfg.b_deadzone * b * nf * nf * f_n / (2.0 * (1.0 + b * nf * nf));
            delta_f_n = (delta_f_n - tol_n).max(0.0);
        }

        // Standard M&B diluted penalty (Maher & Beauchamp 1994).
        // `.max(1.0)` is a numerical guard (ours, not in the paper — Eq 1 uses
        // f^-p unguarded): caps the weight blow-up as f → 0. Inert in-band
        // (lowest predicted partial ≈ A0 at 27.5 Hz); upstream peak admission
        // only guarantees f > 0. The p == 0.5 branch is a fast path preserving
        // the original 1/sqrt bit pattern (powf(-0.5) may differ in ULPs).
        let f_weight = if cfg.p == 0.5 {
            1.0 / f_n.max(1.0).sqrt()
        } else {
            f_n.max(1.0).powf(-cfg.p)
        };
        let amp_ratio = a_n / a_max; // a_n / A_max
        let err_pm_n = delta_f_n * f_weight + amp_ratio * (cfg.q * delta_f_n * f_weight - cfg.r);

        err_pm += err_pm_n;
    }

    // ── Eq. (2): Err_{m-p} (Measured-to-Predicted) ──────────────────────────
    // For each of K measured peaks, find the nearest predicted harmonic.
    // f_k and a_k both refer to the measured peak itself.
    // Δf_k = |f_k - f_nearest_predicted|
    //
    // Per-term:  Δf_k·(f_k^-p) + (a_k/A_max)·[q·Δf_k·(f_k^-p) − r]
    // O(N + K) Two-Pointer Sweep: Find nearest predicted partial for each peak
    //
    // Bounded-error adaptation of Duan et al. (2010) Peak Mixture Model (Eq. 3):
    // We adapt the topological bound proved by Duan et al. Eq (3): as a peak
    // diverges from all predicted harmonics, the log-likelihood asymptotes to a
    // constant noise floor. We approximate this smooth asymptote with a piecewise
    // hard ceiling (.min(LAMBDA_PENALTY)), which preserves the bounded error
    // topology without computing Gaussians at runtime.
    //
    // Duan, Z., Pardo, B., & Zhang, C. (2010). "Multiple Fundamental Frequency
    // Estimation by Modeling Spectral Peaks and Non-Peak Regions."
    // IEEE TASLP 18(8). DOI: 10.1109/TASL.2010.2042119
    let mut err_mp = 0.0_f32;
    let mut i = 0;
    for peak in peaks {
        let f_k = peak.frequency;
        let a_k = peak.magnitude;

        // Advance i while the next predicted partial is closer or equally close to f_k
        while i + 1 < predicted.len()
            && (predicted[i + 1] * scale - f_k).abs() <= (predicted[i] * scale - f_k).abs()
        {
            i += 1;
        }
        let delta_f_k = (predicted[i] * scale - f_k).abs();

        // `.max(1.0)`: same sub-1-Hz guard as in Err_{p-m} (ours, not in Eq 2).
        let f_weight = if cfg.p == 0.5 {
            1.0 / f_k.max(1.0).sqrt()
        } else {
            f_k.max(1.0).powf(-cfg.p)
        };
        let amp_ratio = a_k / a_max; // a_k / A_max
        let mut err_mp_k =
            delta_f_k * f_weight + amp_ratio * (cfg.q * delta_f_k * f_weight - cfg.r);

        err_mp_k = err_mp_k.min(cfg.lambda_penalty);
        err_mp += err_mp_k;
    }

    // ── Eq. (3): Err_total ───────────────────────────────────────────────────
    // Err_total = Err_{p-m}/N + ρ·Err_{m-p}/K
    let n = active_predicted as f32;
    let k = peaks.len() as f32;

    let fwd_norm = if cfg.sum_forward { 1.0 } else { n };
    // Duan non-peak term: UN-normalized count of hallucinated in-band partials.
    // Deliberately not /N — it must scale with the number of bad predictions.
    let nonpeak_term = cfg.nonpeak_penalty * nonpeak_count as f32;
    // Emiya smoothness term: mean squared 2nd-diff over matched partials (≥3 needed).
    let smooth_term = if matched_n >= 3 {
        cfg.smoothness_penalty * smooth_accum / (matched_n - 2) as f32
    } else {
        0.0
    };
    (err_pm / fwd_norm) + cfg.rho * (err_mp / k) + nonpeak_term + smooth_term
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::get_expected_beta;

    /// Canonical Maher & Beauchamp (1994) constants. Pinned here (rather than
    /// using `TwmConfig::default()`) so this regression test guards the scoring
    /// *math* independently of whatever tuned constants the default carries —
    /// the golden bit patterns below were computed with these exact values.
    fn canonical_cfg() -> TwmConfig {
        TwmConfig {
            p: 0.5,
            q: 1.4,
            r: 0.5,
            rho: 0.33,
            lambda_penalty: 18.0,
            ..TwmConfig::default()
        }
    }

    /// Guards the *shipped* default constants (ADR 0006, provisional conservative
    /// config). `test_twm_regression` deliberately pins the canonical M&B values, so
    /// nothing else would catch an accidental edit to `Default`. This is a value
    /// assertion, not a score-bits golden — update it intentionally when the adopted
    /// config changes (which, per 0006, is still under review).
    #[test]
    fn test_shipped_default_constants() {
        let d = TwmConfig::default();
        assert_eq!(d.p, 0.5);
        assert_eq!(d.q, 3.88);
        assert_eq!(d.r, 1.426);
        assert_eq!(d.rho, 0.298);
        assert_eq!(d.lambda_penalty, 18.0);
        // Experimental terms must ship OFF.
        assert!(!d.sum_forward);
        assert_eq!(d.b_deadzone, 0.0);
        assert_eq!(d.nonpeak_penalty, 0.0);
        assert_eq!(d.smoothness_penalty, 0.0);
    }

    #[test]
    fn test_twm_regression() {
        let keys = [
            (0, 27.5, 1093269406),     // A0
            (17, 73.42, 1095578742),   // D2
            (42, 311.13, 1096597276),  // D#4
            (87, 4186.01, 1088106306), // C8
        ];

        let mut peaks = vec![
            SpectralPeak {
                frequency: 27.5,
                magnitude: 1.0,
            },
            SpectralPeak {
                frequency: 55.0,
                magnitude: 0.5,
            },
            SpectralPeak {
                frequency: 73.42,
                magnitude: 1.0,
            },
            SpectralPeak {
                frequency: 147.0,
                magnitude: 0.8,
            },
            SpectralPeak {
                frequency: 311.13,
                magnitude: 1.0,
            },
            SpectralPeak {
                frequency: 623.0,
                magnitude: 0.6,
            },
            SpectralPeak {
                frequency: 1000.0,
                magnitude: 0.2,
            },
            SpectralPeak {
                frequency: 2000.0,
                magnitude: 0.1,
            },
            SpectralPeak {
                frequency: 4186.01,
                magnitude: 1.0,
            },
        ];
        peaks.sort_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

        for (idx, f0, golden) in keys {
            let beta = get_expected_beta(idx);
            let profile = KeyProfile::new(f0, beta);
            let cfg = canonical_cfg();
            let score = score_candidate(&peaks, &profile, 1.0, &cfg);
            assert_eq!(score.to_bits(), golden, "Regression failed for key {}", idx);
        }

        // Negative score test (single peak A0 perfectly aligned)
        let beta_a0 = get_expected_beta(0);
        let profile_a0 = KeyProfile::new(27.5, beta_a0);
        let p1 = profile_a0.predicted_partials[0];
        let peaks_a0 = vec![SpectralPeak {
            frequency: p1,
            magnitude: 1.0,
        }];
        let cfg = canonical_cfg();
        let score = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg);
        assert_eq!(
            score.to_bits(),
            3207216497,
            "Regression failed for negative score case"
        );

        // Lambda-identity test
        let mut cfg_inf = canonical_cfg();
        cfg_inf.lambda_penalty = f32::INFINITY;

        let mut cfg_max = canonical_cfg();
        cfg_max.lambda_penalty = f32::MAX;

        let score_inf = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg_inf);
        let score_max = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg_max);
        assert_eq!(
            score_inf.to_bits(),
            score_max.to_bits(),
            "Lambda infinity identity failed"
        );
    }
}
