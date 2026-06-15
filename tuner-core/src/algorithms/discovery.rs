//! # Split Discovery Search (Stage A → Stage B)
//!
//! Coarse-to-fine fundamental discovery per ADR 0005: a discrete 88-key TWM scan
//! (Stage A) followed by basin-clamped continuous scale refinement of the top
//! candidates (Stage B). Restores the continuous minimization that canonical TWM
//! (Maher & Beauchamp 1994) defines — the 88-key ET lattice alone never evaluates
//! a mistuned note at its true error minimum, which both inflates the true key's
//! error and starves the Goertzel trackers of in-range seeds. Quantitative
//! rationale: `docs/design/discovery-search-analysis.md`.
//!
//! This module is the SINGLE implementation of the split search. The Engine, the
//! MOBO evaluator, and the offline diagnostics must all call into it; duplicating
//! the search logic elsewhere invalidates the evaluator's pipeline parity.
//!
//! Zero-allocation: fixed-size arrays only, suitable for the audio hot path.
//!
//! Known approximation (deliberate, see ADR 0005 revisit condition 3): Stage B
//! scales all predicted partials uniformly, while a physically detuned string
//! also shifts its inharmonicity (ΔB/B = −2·Δf0/f0, Rigaud et al. 2011) —
//! ~13 cents of residual at partial 60 of a 50-cent-detuned bass string. If
//! refined residuals show systematic B-structure, promote refinement to a joint
//! (f0, B) search; do not patch the scale model piecemeal.

use crate::algorithms::peaks::SpectralPeak;
use crate::algorithms::twm::{self, TwmConfig};
use crate::engine::KeyProfile;

/// Candidates carried from Stage A into Stage B refinement. 88 = **exhaustive**:
/// every key is refined, so there is no shortlist cutoff to justify (no magic
/// number under the Topological Scrutiny Test) and the true key can never be
/// dropped before refinement — recall is total, and production false-lock then
/// equals pure separability. This is canonical Maher & Beauchamp continuous f0
/// search tiled into the 88 physical key-basins (ADR 0005 §3); the per-candidate
/// ±80-cent basin clamp still prevents any escape toward a sub-harmonic.
///
/// Cost is ~1 ms per discovery frame (onset-only, until the 3-frame lock) — set
/// below 88 ONLY if a micro-bench on target hardware shows the hop budget can't
/// afford it, at which point it becomes a latency-vs-recall knob.
pub const TOP_K: usize = 88;

/// Half-width of the Stage B refinement window, in cents. Adjacent-key basins
/// (100 cents apart) barely overlap, and sub-harmonics sit 1200 cents away, so
/// refinement can only re-rank Stage A's candidates — never escape toward a new
/// false lock (ADR 0005).
pub const REFINE_WINDOW_CENTS: f32 = 80.0;

/// Pre-grid spacing in cents. 9 points over ±80 cents. The pre-grid is mandatory:
/// error-vs-scale is piecewise (peak-to-partial nearest-neighbor associations
/// switch discretely as the scale sweeps), so a pure unimodal line search is
/// unsafe without bracketing first.
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
    /// Winning continuous scale factor `s_win` (1.0 in discrete mode). Multiplies
    /// the key's predicted partials; used to seed the Goertzel trackers inside
    /// their ±21.5 Hz phase-unwrap range.
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
/// 9-point pre-grid over ±80 cents locates the best 40-cent bracket, then
/// golden-section minimization polishes inside it (~18 `score_candidate` calls
/// total). Returns `(scale, refined_error)`; `(1.0, f32::MAX)` when there is
/// nothing to score.
///
/// Also the manual-mode (`target_note`) path: refining the single target profile
/// seeds tracking correctly for mistuned strings — critical for Pitch Raise.
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

/// Full split discovery over one frame.
///
/// Stage A scans all 88 keys at `scale = 1.0`, keeping the top-`TOP_K` candidates
/// (fixed-size insertion; ties keep the lower key, matching the legacy argmin).
/// With `refine = false` the result is exactly the legacy discrete behavior.
/// With `refine = true`, Stage B refines each candidate and the minimum refined
/// error wins. Cost: 88 + ~18·TOP_K ≈ 142 `score_candidate` calls per frame,
/// discovery frames only.
/// Stage A only: the coarse 88-key scan at `scale = 1.0`, returning the top-K
/// `(key, error)` candidates ascending by error (ties keep the lower key,
/// matching the legacy argmin). Exposed for the MOBO evaluator's Stage A recall
/// metric; `discover` is built on this same function.
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

pub fn discover(
    peaks: &[SpectralPeak],
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    refine: bool,
) -> DiscoveryResult {
    // ── Stage A: coarse 88-key scan, top-K collection ──
    let top = stage_a(peaks, profiles, cfg);

    // Discrete mode, or nothing scoreable at all: legacy behavior.
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
            v.push(KeyProfile::new(NOTES[i].frequency, get_expected_beta(i as u8)));
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
    fn discrete_mode_matches_legacy_argmin() {
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
        // (key, detune in cents) — spans bass/mid/treble, both directions, and
        // the in-tune case. ±3-cent tolerance vs the ~2-cent design target.
        for &(key, cents) in &[
            (17_usize, -40.0_f32),
            (40, 0.0),
            (40, 70.0),
            (64, -25.0),
        ] {
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

    #[test]
    fn split_discovery_cost_within_budget() {
        let profiles = build_profiles();
        let cfg = TwmConfig::default();
        let peaks = synth_peaks(&profiles[17], 0.98, 24);

        let t0 = std::time::Instant::now();
        for _ in 0..200 {
            core::hint::black_box(discover(&peaks, &profiles, &cfg, false));
        }
        let discrete = t0.elapsed();

        let t1 = std::time::Instant::now();
        for _ in 0..200 {
            core::hint::black_box(discover(&peaks, &profiles, &cfg, true));
        }
        let refined = t1.elapsed();

        // At TOP_K=88 (exhaustive): discrete = 88 score calls; refined adds
        // ~18 calls per key (pre-grid + golden), so refined/discrete ≈ 1+18 ≈ 19×.
        // The ratio is hardware-independent; a generous 40× bound catches a
        // structural regression (an alloc or O(n²) creeping into the hot path)
        // while tolerating CI jitter. Absolute cost is ~1 ms/frame, onset-only.
        assert!(
            refined < discrete * 40,
            "refined {refined:?} vs discrete {discrete:?} exceeds budget"
        );
    }
}
