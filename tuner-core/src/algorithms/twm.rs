//! # Two-Way Mismatch (TWM)
//!
//! Continuous, sub-bin-precision fundamental-frequency scoring via the
//! Maher & Beauchamp (1994) distance-sum formulation. Stateless.

use crate::models::{KeyProfile, SpectralPeak};

/// The TWM error's weights, and its experimental terms, which ship off.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwmConfig {
    /// Frequency weighting exponent.
    pub p: f32,
    /// Amplitude penalty scaling.
    pub q: f32,
    /// Reward for an aligned strong peak.
    pub r: f32,
    /// Weight of the measured-to-predicted error.
    pub rho: f32,
    /// Ceiling on each measured-to-predicted term.
    pub lambda_penalty: f32,
    /// Experimental: sum the forward error instead of averaging it over N, testing
    /// whether the average hides a dense candidate's absent partials. `false` is
    /// canonical.
    pub sum_forward: bool,
    /// Experimental: forgive forward distance up to partial n's B-uncertainty,
    /// `c·B·n²·f_n / (2(1 + Bn²))` with c this value (≈ 0.14 is Rigaud's σ_B). Wider
    /// absorbs per-note B scatter but forgives an octave candidate's divergence.
    /// 0 is off.
    pub b_deadzone: f32,
    /// Experimental: a penalty per predicted partial inside the observed band with
    /// no peak within 2 %, un-normalized so it charges dense bass impostors (Duan,
    /// Pardo & Zhang 2010). Below the lowest peak, where a fundamental may be
    /// missing, it does not apply. 0 is off.
    // Scoped in the Duan likelihood design note.
    pub nonpeak_penalty: f32,
    /// Experimental: a penalty on the jaggedness of the matched partials'
    /// log-amplitudes (Emiya's spectral smoothness), over at least three. A true
    /// note decays smoothly; an impostor's coincidental matches do not. 0 is off.
    pub smoothness_penalty: f32,
}

impl Default for TwmConfig {
    fn default() -> Self {
        // NSGA-II-tuned amplitude terms (q, r), with p and the λ ceiling at their
        // canonical values: 76/87 discrete, 77/87 refined on real captures.
        // `test_twm_regression` pins the canonical M&B constants.
        // report 0006
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

/// Scores a key profile, scaled by `scale`, against observed peaks: the Two-Way
/// Mismatch error of Maher & Beauchamp (1994), JASA 95(4), 2254–2263, DOI
/// 10.1121/1.408685, Eqs. 1–3.
///
/// Each term is `E_w = Δf·f^−p + (a/A_max)·[q·Δf·f^−p − r]`, and the total is
/// `Err_{p→m}/N + ρ·Err_{m→p}/K`. The paper's values are p = 0.5, q = 1.4, r = 0.5,
/// ρ = 0.33. The score is a figure of merit, not Hz: `Δf·f^−p` has units Hz^(1−p)
/// while `r` is a pure number.
///
/// Each measured-to-predicted term is capped at `lambda_penalty`, so distant
/// low-frequency noise cannot pile up error and lock the treble onto a ghost. The
/// cap approximates the bounded noise asymptote of Duan, Pardo & Zhang (2010),
/// IEEE TASLP 18(8), DOI 10.1109/TASL.2010.2042119; a hard ceiling is our
/// adaptation, not a port of their likelihood.
// audit 01
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

    // A_max = max(a_k), scanned: the peaks arrive sorted by frequency.
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

    // ── Bandwidth cap (M&B Step 2, generalized) ──
    // Step 2 predicts harmonics up to the first at or above the highest measured
    // peak. Its cutoff form, f_n ≤ max_obs + f0·scale, is Step 2 exactly at B = 0;
    // for B > 0 it differs by at most one edge partial more than a fundamental above
    // the observed band, which would only add forward error. audit 01
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

    // ── Eq. (1): Err_{p→m} ──
    // For each predicted partial, the nearest measured peak, by a two-pointer
    // sweep. Duan's non-peak likelihood (Eq. 7) is not applied by default: its
    // cliff would punish the missing fundamentals of bass strings.
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

        // In the observed band with no peak within 2 % (≈ 35 ¢); below the lowest
        // peak a fundamental may be missing, so it is never counted.
        let matched = raw_delta <= 0.02 * f_n;
        if cfg.nonpeak_penalty > 0.0 && f_n >= min_obs_freq && f_n <= max_obs_freq && !matched {
            nonpeak_count += 1;
        }

        // Second differences of consecutive matched partials' log-amplitudes.
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

        // `.max(1.0)` is ours, not Eq. 1's: it caps the weight as f → 0, and is inert
        // in band. The p == 0.5 branch keeps the 1/sqrt bit pattern, which
        // `powf(-0.5)` may not reproduce.
        let f_weight = if cfg.p == 0.5 {
            1.0 / f_n.max(1.0).sqrt()
        } else {
            f_n.max(1.0).powf(-cfg.p)
        };
        let amp_ratio = a_n / a_max; // a_n / A_max
        let err_pm_n = delta_f_n * f_weight + amp_ratio * (cfg.q * delta_f_n * f_weight - cfg.r);

        err_pm += err_pm_n;
    }

    // ── Eq. (2): Err_{m→p} ──
    // For each measured peak, the nearest predicted partial, by a two-pointer sweep;
    // each term capped at λ, Duan's Eq. 3 asymptote as a hard ceiling.
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
    // Not divided by N: it must grow with the number of bad predictions.
    let nonpeak_term = cfg.nonpeak_penalty * nonpeak_count as f32;
    // The mean squared second difference, which needs three matched partials.
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
    /// math independently of whatever tuned constants the default carries —
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

    // Guards the shipped default constants (report 0006). `test_twm_regression`
    // deliberately pins the canonical M&B values, so nothing else would catch an
    // accidental edit to `Default`. This is a value assertion, not a score-bits
    // golden — update it intentionally when the adopted config changes.
    #[test]
    fn test_shipped_default_constants() {
        let d = TwmConfig::default();
        assert_eq!(d.p, 0.5);
        assert_eq!(d.q, 3.88);
        assert_eq!(d.r, 1.426);
        assert_eq!(d.rho, 0.298);
        assert_eq!(d.lambda_penalty, 18.0);
        // Experimental terms must ship off.
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
