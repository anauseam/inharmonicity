//! # Discovery — the split search
//!
//! Coarse-to-fine fundamental discovery: a discrete 88-key TWM scan (Stage A),
//! then basin-clamped continuous scale refinement of the best candidates
//! (Stage B), restoring the continuous minimisation canonical TWM defines (Maher &
//! Beauchamp 1994), which the ET lattice alone never reaches for a mistuned note.
//! Allocation-free.
//!
//! Stage B scales every partial uniformly, while a detuned string also shifts its
//! B (ΔB/B = −2·Δf₀/f₀, Rigaud et al. 2011): about 13 ¢ of residual at partial 60
//! of a bass string 50 ¢ off.

// The one implementation of the split search: every caller, the lab's NSGA-II
// evaluator included, uses it, or the evaluator loses parity with the pipeline.
// If refined residuals show systematic B structure, make refinement a joint
// (f₀, B) search rather than patching the scale model.
// report 0005

use crate::algorithms::twm::{self, TwmConfig};
use crate::models::{KeyProfile, SpectralPeak};

/// Candidates carried from Stage A into refinement. The scan is also a filter:
/// refining every key exposes the true one to the bass attractors it excludes.
/// The price is pitch-raise reach, since a note far off ET misses the top K.
// Measured on real captures: K = 88 scored 61/87 (bass 19 → 12) against 77/87
// at K = 3, and K = 3 holds the true key to ~69 ¢ of pitch raise. Do not raise
// K on synthetic evidence — the synthetic set under-represents the attractor
// field that makes small K correct. report 0006
pub const TOP_K: usize = 3;

/// Half-width of the Stage B refinement window, in cents. Adjacent-key basins
/// (100 cents apart) barely overlap, and sub-harmonics sit 1200 cents away, so
/// refinement can only re-rank Stage A's candidates — never escape toward a new
/// false lock.
// report 0005
pub const REFINE_WINDOW_CENTS: f32 = 80.0;

/// Pre-grid spacing in cents: 9 points over ±80 ¢. Error against scale is
/// piecewise, as peak-to-partial associations switch, so a line search needs this
/// bracket first.
const PRE_GRID_STEP_CENTS: f32 = 20.0;
const PRE_GRID_POINTS: usize = 9;

/// Golden-section iterations inside the 40-cent bracket:
/// 0.618^7 × 40 ≈ 1.4 cents ≤ the ~2-cent precision target.
const GOLDEN_ITERS: usize = 7;
const INV_PHI: f32 = 0.618_034; // 1/φ

/// Outcome of one discovery pass over a frame's masked peaks.
#[derive(Debug, Clone, Copy)]
pub struct DiscoveryResult {
    /// 0–87 index of the winning key.
    pub key_index: u8,
    /// Winning continuous scale factor `s_win` on the key's predicted partials;
    /// 1.0 in discrete mode.
    pub scale: f32,
    /// The winning (refined, if Stage B ran) TWM error.
    pub error: f32,
}

#[inline]
fn cents_to_scale(cents: f32) -> f32 {
    (cents / 1200.0).exp2()
}

/// Stage B: basin-clamped continuous scale refinement of a single candidate.
///
/// A 9-point pre-grid over ±80 ¢ finds the best 40 ¢ bracket, and golden-section
/// search polishes inside it, about 18 `score_candidate` calls in all. Returns
/// `(scale, refined_error)`, or `(1.0, f32::MAX)` when there is nothing to score.
pub fn refine_scale(peaks: &[SpectralPeak], profile: &KeyProfile, cfg: &TwmConfig) -> (f32, f32) {
    if peaks.is_empty() || profile.valid_partial_count == 0 {
        return (1.0, f32::MAX);
    }

    // ── Pre-grid bracketing ──
    let mut best_i = 0_usize;
    let mut best_err = f32::MAX;
    for i in 0..PRE_GRID_POINTS {
        let cents = -REFINE_WINDOW_CENTS + (i as f32) * PRE_GRID_STEP_CENTS;
        let err = twm::score_candidate(peaks, profile, cents_to_scale(cents), cfg);
        if err < best_err {
            best_err = err;
            best_i = i;
        }
    }
    let mut best_cents = -REFINE_WINDOW_CENTS + (best_i as f32) * PRE_GRID_STEP_CENTS;

    // Bracket = the two grid intervals flanking the best point (clamped at the
    // window edges, where the bracket degenerates to a single interval).
    let lo_i = best_i.saturating_sub(1);
    let hi_i = (best_i + 1).min(PRE_GRID_POINTS - 1);
    let mut a = -REFINE_WINDOW_CENTS + (lo_i as f32) * PRE_GRID_STEP_CENTS;
    let mut b = -REFINE_WINDOW_CENTS + (hi_i as f32) * PRE_GRID_STEP_CENTS;

    // ── Golden-section minimization in the bracket ──
    let mut c = b - (b - a) * INV_PHI;
    let mut d = a + (b - a) * INV_PHI;
    let mut fc = twm::score_candidate(peaks, profile, cents_to_scale(c), cfg);
    let mut fd = twm::score_candidate(peaks, profile, cents_to_scale(d), cfg);
    for _ in 0..GOLDEN_ITERS {
        if fc < fd {
            b = d;
            d = c;
            fd = fc;
            c = b - (b - a) * INV_PHI;
            fc = twm::score_candidate(peaks, profile, cents_to_scale(c), cfg);
        } else {
            a = c;
            c = d;
            fc = fd;
            d = a + (b - a) * INV_PHI;
            fd = twm::score_candidate(peaks, profile, cents_to_scale(d), cfg);
        }
    }

    // Error-vs-scale is piecewise: keep the best of (pre-grid, golden) rather
    // than trusting the line search unconditionally.
    let (g_cents, g_err) = if fc < fd { (c, fc) } else { (d, fd) };
    if g_err < best_err {
        best_cents = g_cents;
        best_err = g_err;
    }

    (cents_to_scale(best_cents), best_err)
}

/// Stage A: the 88-key scan at `scale = 1.0`, returning the top-`TOP_K`
/// `(key, error)` candidates by ascending error (ties keep the lower key).
pub fn stage_a(
    peaks: &[SpectralPeak],
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
) -> [(usize, f32); TOP_K] {
    let mut top: [(usize, f32); TOP_K] = [(0, f32::MAX); TOP_K];
    for (k, profile) in profiles.iter().enumerate() {
        let err = twm::score_candidate(peaks, profile, 1.0, cfg);
        if err < top[TOP_K - 1].1 {
            top[TOP_K - 1] = (k, err);
            let mut i = TOP_K - 1;
            while i > 0 && top[i].1 < top[i - 1].1 {
                top.swap(i, i - 1);
                i -= 1;
            }
        }
    }
    top
}

/// Full split discovery over one frame. With `refine = false`, the discrete 88-key
/// argmin; with `refine = true`, Stage B refines each candidate and the least
/// refined error wins, at about 88 + 18·`TOP_K` `score_candidate` calls.
pub fn discover(
    peaks: &[SpectralPeak],
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    refine: bool,
) -> DiscoveryResult {
    // ── Stage A: coarse 88-key scan, top-K collection ──
    let top = stage_a(peaks, profiles, cfg);

    // Discrete mode, or nothing scoreable at all: the plain argmin.
    if !refine || top[0].1 == f32::MAX {
        return DiscoveryResult {
            key_index: top[0].0 as u8,
            scale: 1.0,
            error: top[0].1,
        };
    }

    // ── Stage B: refine each candidate; minimum refined error wins ──
    let mut best = DiscoveryResult {
        key_index: top[0].0 as u8,
        scale: 1.0,
        error: f32::MAX,
    };
    for &(k, stage_a_err) in &top {
        if stage_a_err == f32::MAX {
            continue; // unfilled slot (fewer than TOP_K scoreable candidates)
        }
        let (scale, error) = refine_scale(peaks, &profiles[k], cfg);
        if error < best.error {
            best = DiscoveryResult {
                key_index: k as u8,
                scale,
                error,
            };
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{NOTES, get_expected_beta};

    fn build_profiles() -> Box<[KeyProfile; 88]> {
        let mut v = Vec::with_capacity(88);
        for i in 0..88 {
            v.push(KeyProfile::new(
                NOTES[i].frequency,
                get_expected_beta(i as u8),
            ));
        }
        let arr: [KeyProfile; 88] = v.try_into().unwrap();
        Box::new(arr)
    }

    /// Peaks exactly on the profile's stretched partials × s_true, 1/n magnitudes,
    /// ascending frequency (the mask_peaks output contract).
    fn synth_peaks(profile: &KeyProfile, s_true: f32, n_partials: usize) -> Vec<SpectralPeak> {
        (0..profile.valid_partial_count.min(n_partials))
            .map(|i| SpectralPeak {
                frequency: profile.predicted_partials[i] * s_true,
                magnitude: 1.0 / (i as f32 + 1.0),
            })
            .collect()
    }

    #[test]
    fn discrete_mode_matches_argmin() {
        let profiles = build_profiles();
        let cfg = TwmConfig::default();
        for &(key, s) in &[(0_usize, 1.0_f32), (17, 0.977), (42, 1.02), (87, 1.0)] {
            let peaks = synth_peaks(&profiles[key], s, 20);
            let res = discover(&peaks, &profiles, &cfg, false);

            let mut min_e = f32::MAX;
            let mut win = 0_u8;
            for k in 0..88 {
                let e = twm::score_candidate(&peaks, &profiles[k], 1.0, &cfg);
                if e < min_e {
                    min_e = e;
                    win = k as u8;
                }
            }
            assert_eq!(res.key_index, win, "argmin parity (key {key}, s {s})");
            assert_eq!(res.error.to_bits(), min_e.to_bits());
            assert_eq!(res.scale, 1.0);
        }
    }

    #[test]
    fn refined_recovers_detuned_notes() {
        let profiles = build_profiles();
        let cfg = TwmConfig::default();
        // (key, detune ¢) over bass, mid and treble, both directions, and in tune;
        // ±3 ¢ against the ≈ 2 ¢ design target. +60 ¢ is a strong pitch raise, inside
        // the ≈ 69 ¢ this key's Stage-A top-3 gate allows (report 0006).
        for &(key, cents) in &[(17_usize, -40.0_f32), (40, 0.0), (40, 60.0), (64, -25.0)] {
            let s_true = cents_to_scale(cents);
            let peaks = synth_peaks(&profiles[key], s_true, 20);
            let res = discover(&peaks, &profiles, &cfg, true);

            assert_eq!(res.key_index, key as u8, "wrong key at {cents} cents");
            let got_cents = 1200.0 * res.scale.log2();
            assert!(
                (got_cents - cents).abs() <= 3.0,
                "refined {got_cents:.2} cents vs true {cents} cents (key {key})"
            );
            // Basin clamp: the refined scale can never leave the window.
            assert!(got_cents.abs() <= REFINE_WINDOW_CENTS + 0.1);
        }
    }

    #[test]
    fn refine_scale_handles_empty_input() {
        let profiles = build_profiles();
        let cfg = TwmConfig::default();
        let (s, e) = refine_scale(&[], &profiles[10], &cfg);
        assert_eq!(s, 1.0);
        assert_eq!(e, f32::MAX);
    }
}
