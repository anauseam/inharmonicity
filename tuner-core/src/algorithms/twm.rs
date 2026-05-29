//! # Two-Way Mismatch (TWM)
//!
//! Replaces legacy discrete template matching with continuous, sub-bin precision
//! scoring using the Maher & Beauchamp (1994) distance-sum formulation.
//! This stateless module provides robust fundamental frequency discovery.

use crate::algorithms::peaks::SpectralPeak;
use crate::engine::KeyProfile;

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
pub fn score_candidate(peaks: &[SpectralPeak], profile: &KeyProfile) -> f32 {
    #[allow(unused)]
    const P: f32 = 0.5; // frequency weighting exponent (paper: p)
    const Q: f32 = 1.4; // amplitude penalty scaling    (paper: q)
    const R: f32 = 0.5; // reward for correct matches   (paper: r)
    const RHO: f32 = 0.33; // reverse error weight         (paper: ρ)

    // These constants will be empirically tuned using the diagnostic tool.
    const LAMBDA_PENALTY: f32 = 18.0;

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
    let cutoff_freq = max_obs_freq + profile.f0_et;
    let mut active_predicted = 0_usize;
    for &p_freq in &profile.predicted_partials[..valid_count] {
        if p_freq <= cutoff_freq {
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
    for &f_n in predicted {
        // Advance j while the next peak is closer or equally close to f_n
        while j + 1 < peaks.len()
            && (peaks[j + 1].frequency - f_n).abs() <= (peaks[j].frequency - f_n).abs()
        {
            j += 1;
        }
        let delta_f_n = (peaks[j].frequency - f_n).abs();
        let a_n = peaks[j].magnitude;

        // Standard M&B diluted penalty (Maher & Beauchamp 1994)
        // Mathematically identical to f_n.powf(-P) since P=0.5, optimized via hardware sqrt
        let f_weight = 1.0 / f_n.max(1.0).sqrt();
        let amp_ratio = a_n / a_max; // a_n / A_max
        let err_pm_n = delta_f_n * f_weight + amp_ratio * (Q * delta_f_n * f_weight - R);

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
            && (predicted[i + 1] - f_k).abs() <= (predicted[i] - f_k).abs()
        {
            i += 1;
        }
        let delta_f_k = (predicted[i] - f_k).abs();

        // Mathematically identical to f_k.powf(-P) since P=0.5, optimized via hardware sqrt
        let f_weight = 1.0 / f_k.max(1.0).sqrt();
        let amp_ratio = a_k / a_max; // a_k / A_max
        let mut err_mp_k = delta_f_k * f_weight + amp_ratio * (Q * delta_f_k * f_weight - R);

        err_mp_k = err_mp_k.min(LAMBDA_PENALTY);
        err_mp += err_mp_k;
    }

    // ── Eq. (3): Err_total ───────────────────────────────────────────────────
    // Err_total = Err_{p-m}/N + ρ·Err_{m-p}/K
    let n = active_predicted as f32;
    let k = peaks.len() as f32;

    (err_pm / n) + RHO * (err_mp / k)
}

/// ── Dynamic Programming Temporal Tracking (Viterbi) ──────────────
/// Resolves sub-harmonic false locks by applying a temporal trajectory
/// constraint. A jump from the fundamental to a sub-harmonic incurs a
/// mathematical penalty.
///
/// # Reference
/// Rao, V. & Rao, P. (2010). "Vocal Melody Extraction in the Presence
/// of Pitched Accompaniment in Polyphonic Audio." IEEE TASLP, 18(8).
/// DOI: 10.1109/TASL.2010.2042124
///
/// # Note on Empirical Constants
/// Rao & Rao used a Gaussian transition matrix tuned for vocal sliding.
/// Because a piano string cannot slide, we adapt their architecture to a
/// binary transition matrix with an empirically tuned flat penalty.
///
/// # Algorithm
/// Online Viterbi decoding. C_t(k) = E_t(k) + min(C_{t-1}(j) + T(j,k))
/// Since T(j,k) is a constant P_jump when j != k, we optimize the O(|V|^2)
/// transition matrix into an O(|V|) sweep by precomputing the global minimum.
pub fn viterbi_update(path_costs: &mut [f32; 88], current_errors: &[f32; 88]) -> u8 {
    const JUMP_PENALTY: f32 = 12.0; // Temporal rigidity (empirical Hz error equivalent)
    let min_prev_cost = path_costs.iter().cloned().fold(f32::MAX, f32::min);

    for k in 0..88 {
        let cost_stay = path_costs[k];
        let cost_jump = min_prev_cost + JUMP_PENALTY;
        path_costs[k] = current_errors[k] + cost_stay.min(cost_jump);
    }

    // Normalize to prevent float explosion over infinite frames
    let new_min_cost = path_costs.iter().cloned().fold(f32::MAX, f32::min);
    for cost in path_costs.iter_mut() {
        *cost -= new_min_cost;
    }

    // Select the candidate with the lowest cumulative temporal cost

    path_costs
        .iter()
        .enumerate()
        .min_by(|a, b| a.1.partial_cmp(b.1).unwrap())
        .unwrap()
        .0 as u8
}
