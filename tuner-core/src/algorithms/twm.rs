//! # Two-Way Mismatch (TWM)
//!
//! Replaces legacy discrete template matching with continuous, sub-bin precision
//! scoring using the Maher & Beauchamp (1994) distance-sum formulation.
//! This stateless module provides robust fundamental frequency discovery.

use crate::algorithms::peaks::SpectralPeak;
use crate::engine::KeyProfile;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TwmConfig {
    pub p: f32,              // frequency weighting exponent
    pub q: f32,              // amplitude penalty scaling
    pub r: f32,              // reward constant
    pub rho: f32,            // reverse error weight
    pub lambda_penalty: f32, // Duan M→P error ceiling
}

impl Default for TwmConfig {
    fn default() -> Self {
        Self { p: 0.5, q: 1.4, r: 0.5, rho: 0.33, lambda_penalty: 18.0 }
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
/// Parameters (empirically calibrated by Maher & Beauchamp):
///   p = 0.5  (frequency weighting exponent)
///   q = 1.4  (amplitude penalty scaling)
///   r = 0.5  (reward constant for aligned strong peaks)
///   ρ = 0.33 (reverse error combination weight)
///
/// Returns Err_total in units of Hz — dimensionally consistent regardless
/// of the number of active partials or observed peaks.
///
/// # Noise Floor Boundary
/// To prevent unbounded error accumulation from low-frequency noise (which causes
/// high-treble ghost locks), we enforce a topological boundary on the
/// Measured-to-Predicted error. As derived by:
///
/// Duan, Z., Pardo, B., & Zhang, C. (2010). "Multiple Fundamental Frequency
/// Estimation by Modeling Spectral Peaks and Non-Peak Regions."
/// IEEE TASLP 18(8). DOI: 10.1109/TASL.2010.2042119
///
/// We apply a ceiling to the distance penalty to model the asymptote to the
/// uniform noise distribution for distant peaks.
///
/// Note: Parameters come from TwmConfig (defaults = Maher & Beauchamp 1994 values,
/// pending MOBO calibration per ADR 0001).
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
    for peak in peaks {
        if peak.magnitude > a_max {
            a_max = peak.magnitude;
        }
        if peak.frequency > max_obs_freq {
            max_obs_freq = peak.frequency;
        }
    }
    a_max = a_max.max(1e-6);

    // ── Dynamic Bandwidth Cap ────────────────────────────────────────────────
    // Only evaluate predicted partials within the observable spectral range
    // plus one fundamental of margin. Partials beyond this carry no
    // discriminative value and would inflate Err_{p-m} with unconstrained
    // forward error from harmonics the instrument does not physically produce
    // in this frame.
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
    let mut j = 0;
    for &p_freq in predicted {
        let f_n = p_freq * scale;
        // Advance j while the next peak is closer or equally close to f_n
        while j + 1 < peaks.len()
            && (peaks[j + 1].frequency - f_n).abs() <= (peaks[j].frequency - f_n).abs()
        {
            j += 1;
        }
        let delta_f_n = (peaks[j].frequency - f_n).abs();
        let a_n = peaks[j].magnitude;

        // Standard M&B diluted penalty (Maher & Beauchamp 1994)
        let f_weight = if cfg.p == 0.5 {
            1.0 / f_n.max(1.0).sqrt()          // fast path, bit-identical to today
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
    // Mathematical Equivalency to Duan et al. (2010) Peak Mixture Model (Eq. 3):
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

        let f_weight = if cfg.p == 0.5 {
            1.0 / f_k.max(1.0).sqrt()          // fast path, bit-identical to today
        } else {
            f_k.max(1.0).powf(-cfg.p)
        };
        let amp_ratio = a_k / a_max; // a_k / A_max
        let mut err_mp_k = delta_f_k * f_weight + amp_ratio * (cfg.q * delta_f_k * f_weight - cfg.r);

        err_mp_k = err_mp_k.min(cfg.lambda_penalty);
        err_mp += err_mp_k;
    }

    // ── Eq. (3): Err_total ───────────────────────────────────────────────────
    // Err_total = Err_{p-m}/N + ρ·Err_{m-p}/K
    let n = active_predicted as f32;
    let k = peaks.len() as f32;

    (err_pm / n) + cfg.rho * (err_mp / k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::get_expected_beta;

    #[test]
    fn test_twm_regression() {
        let keys = [
            (0, 27.5, 1093269406),        // A0
            (17, 73.42, 1095578742),      // D2
            (42, 311.13, 1096597276),     // D#4
            (87, 4186.01, 1088106306),    // C8
        ];

        let mut peaks = vec![
            SpectralPeak { frequency: 27.5, magnitude: 1.0 },
            SpectralPeak { frequency: 55.0, magnitude: 0.5 },
            SpectralPeak { frequency: 73.42, magnitude: 1.0 },
            SpectralPeak { frequency: 147.0, magnitude: 0.8 },
            SpectralPeak { frequency: 311.13, magnitude: 1.0 },
            SpectralPeak { frequency: 623.0, magnitude: 0.6 },
            SpectralPeak { frequency: 1000.0, magnitude: 0.2 },
            SpectralPeak { frequency: 2000.0, magnitude: 0.1 },
            SpectralPeak { frequency: 4186.01, magnitude: 1.0 },
        ];
        peaks.sort_by(|a, b| a.frequency.partial_cmp(&b.frequency).unwrap());

        for (idx, f0, golden) in keys {
            let beta = get_expected_beta(idx);
            let profile = KeyProfile::new(f0, beta);
            let cfg = TwmConfig::default();
            let score = score_candidate(&peaks, &profile, 1.0, &cfg);
            assert_eq!(score.to_bits(), golden, "Regression failed for key {}", idx);
        }

        // Negative score test (single peak A0 perfectly aligned)
        let beta_a0 = get_expected_beta(0);
        let profile_a0 = KeyProfile::new(27.5, beta_a0);
        let p1 = profile_a0.predicted_partials[0];
        let peaks_a0 = vec![SpectralPeak { frequency: p1, magnitude: 1.0 }];
        let cfg = TwmConfig::default();
        let score = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg);
        assert_eq!(score.to_bits(), 3207216497, "Regression failed for negative score case");

        // Lambda-identity test
        let mut cfg_inf = TwmConfig::default();
        cfg_inf.lambda_penalty = f32::INFINITY;
        
        let mut cfg_max = TwmConfig::default();
        cfg_max.lambda_penalty = f32::MAX;

        let score_inf = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg_inf);
        let score_max = score_candidate(&peaks_a0, &profile_a0, 1.0, &cfg_max);
        assert_eq!(score_inf.to_bits(), score_max.to_bits(), "Lambda infinity identity failed");
    }
}
