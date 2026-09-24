//! # Tuning-curve engines
//!
//! Four engines turn a profile's measurements into a tuning curve by composing the
//! cited leaves [`super::rigaud`], [`super::giordano`] and [`super::whittaker`]:
//! [`rigaud_pure`] (a), [`per_key_smoothed`] (b), [`giordano_calibrated`] (c) and
//! [`multi_interval`] (d). Engines (b) and (c) each add one stage to the one before.
//!
//! Conventions: d(m) is defined on the audible first partial f₁ = F₀√(1+B), while
//! MAT and Eq. 6 work on the flexible-string F₀, so every width here converts
//! explicitly ([`octave_stretch_cents`], [`interval_width_cents`]). Strobe targets
//! use the key's raw measured B instead ([`TuningCurve::strobe_partials`]). A
//! negative octave stretch is flagged by a detector, never clamped.
//!
//! # References
//! * Rigaud, David & Daudet 2013, JASA 133(5), DOI: 10.1121/1.4799806 —
//!   Eqs. 4, 6, 9, 29–32, §II.B procedure.
//! * Giordano 2015, JASA 138(4), DOI: 10.1121/1.4931439 — via
//!   [`super::giordano`].
//! * Eilers 2003, Analytical Chemistry 75(14) — via [`super::whittaker`].
//! * D. J. Carpenter, US Patent 6,529,843 B1 (Verituner) — engine (d)'s
//!   industry precedent (weighted interval prioritization).

use crate::algorithms::giordano;
use crate::algorithms::rigaud::{self, BXi, RhoPhi, midi_from_key};
use crate::algorithms::whittaker::{self, BandedSystem};
use crate::models::{
    CurveInput, CurveKeyData, CurveKeyFlags, InharmonicityProfile, NOTES, TuningCurve,
};

// ─── CurveInput construction ─────────────────────────────────────────────────

impl CurveInput {
    /// Builds the engine input from a profile's trusted captures
    /// ([`is_trusted`](crate::models::KeyMeasurement::is_trusted)).
    pub fn from_profile(profile: &InharmonicityProfile) -> Self {
        Self::build(profile, false)
    }

    /// Builds the engine input with only the auto-mode disqualification waived; a
    /// string-isolated capture is still refused, since a curve is per note. For
    /// diagnostics on auto-mode captures, never for a curve a tuner follows.
    pub fn from_profile_including_auto(profile: &InharmonicityProfile) -> Self {
        Self::build(profile, true)
    }

    /// Trust filter + Eq.-20 F₀ derivation: admits a key only when its
    /// provenance passes, B is finite and positive, it carries ≥ 2 partials,
    /// and the Rigaud Eq.-20 F₀ is solvable.
    fn build(profile: &InharmonicityProfile, include_auto: bool) -> Self {
        let mut keys: Vec<Option<CurveKeyData>> = (0..88).map(|_| None).collect();
        // One entry per key. Both resolvers refuse a partial unison; they
        // differ only in whether an auto-mode capture may stand for the key.
        let resolved = (0..88u8).filter_map(|idx| {
            let m = if include_auto {
                profile.newest_whole_note(idx)?
            } else {
                profile.active(idx)?
            };
            Some((idx, m))
        });
        for (idx, m) in resolved {
            let Some(b) = m
                .calculated_b
                .map(f64::from)
                .filter(|b| b.is_finite() && *b > 0.0)
            else {
                continue;
            };
            let partials: Vec<(u32, f64, f64)> = m
                .partials
                .iter()
                .filter(|p| p.number > 0 && p.frequency > 0.0)
                .map(|p| (p.number, f64::from(p.frequency), f64::from(p.amplitude)))
                .collect();
            if partials.len() < 2 {
                continue;
            }
            let pairs: Vec<(u32, f64)> = partials.iter().map(|&(n, f, _)| (n, f)).collect();
            let Some(f0) = rigaud::f0_from_partials(&pairs, b) else {
                continue;
            };
            keys[idx as usize] = Some(CurveKeyData { b, f0, partials });
        }
        Self { keys }
    }
}

// ─── Shared curve primitives ─────────────────────────────────────────────────

/// Beat-minimized octave stretch in cents on the audible first partial,
/// from Rigaud Eq. 6 with explicit F_0 → f_1 conversion:
///
/// s = 1200·log₂[ 2·√((1+4ρ²·B_L)/(1+ρ²·B_U)) · √((1+B_U)/(1+B_L)) ] − 1200
///
/// (the first factor is the Eq.-6 F₀ ratio; the second converts to
/// f₁ = F₀·√(1+B), the coordinate d(m) is defined on).
pub fn octave_stretch_cents(b_l: f64, b_u: f64, rho: f64) -> f64 {
    let r0 = 2.0 * ((1.0 + 4.0 * rho * rho * b_l) / (1.0 + rho * rho * b_u)).sqrt();
    let r1 = r0 * ((1.0 + b_u) / (1.0 + b_l)).sqrt();
    1200.0 * r1.log2() - 1200.0
}

/// Beatless width of the coincident pair p:q over `k` semitones, as a
/// deviation from the ET width in cents, on the audible first partial
/// (c_{m,k}, converted from its F_0-space form):
///
/// c = 1200·log₂[ (p/q)·√((1+B_L·p²)/(1+B_U·q²)) · √((1+B_U)/(1+B_L)) ] − 100k
///
/// The p:q = 2:1, k = 12 case coincides with [`octave_stretch_cents`] at ρ = 1.
// asserted: test_interval_width_octave_consistency
pub fn interval_width_cents(b_l: f64, b_u: f64, p: u32, q: u32, k: usize) -> f64 {
    let (p, q) = (p as f64, q as f64);
    let r0 = (p / q) * ((1.0 + b_l * p * p) / (1.0 + b_u * q * q)).sqrt();
    let r1 = r0 * ((1.0 + b_u) / (1.0 + b_l)).sqrt();
    1200.0 * r1.log2() - 100.0 * k as f64
}

/// Stretch preset: the mean octave-type model and its ±1 high and low variants
/// (Rigaud §IV.C.2). The low variant floors at the 2:1 asymptote ρ = 1, where the
/// paper prints `min`, a typo for the physical floor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StretchPreset {
    /// ρ̄_φ(m) - 1, floored at 1 (gentler stretch).
    Low,
    /// ρ̄_φ(m) as fitted/configured.
    #[default]
    Mean,
    /// ρ̄_φ(m) + 1 (wider stretch).
    High,
}

impl StretchPreset {
    fn apply(self, rho: f64) -> f64 {
        match self {
            StretchPreset::Low => (rho - 1.0).max(1.0),
            StretchPreset::Mean => rho,
            StretchPreset::High => rho + 1.0,
        }
    }
}

/// Which per-key B engine (d)'s interval widths are computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CurveBSource {
    /// The precision-weighted blend of the key's measured B and the B_ξ
    /// fit, weighted by each one's variance.
    #[default]
    Blend,
    /// The Eq.-29 fit B_ξ alone (Rigaud's model; per-key deviations from
    /// the fit do not enter the widths).
    Model,
}

/// Parameters shared by all engines.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CurveParams {
    /// Octave-type curve ρ_φ (Eq. 9). [`RhoPhi::TYPICAL`] unless
    /// engine (c) has calibrated an instrument-specific one.
    pub rho: RhoPhi,
    /// Stretch preset applied on top of `rho`.
    pub preset: StretchPreset,
    /// Global deviation d_g in cents (reference-pitch offset; carried on
    /// the output curve, not baked into its shape).
    pub d_g: f64,
    /// B the multi-interval widths read (engine (d) only; the chain
    /// engines always use the blend).
    pub b_source: CurveBSource,
}

impl Default for CurveParams {
    fn default() -> Self {
        Self {
            rho: RhoPhi::TYPICAL,
            preset: StretchPreset::Mean,
            d_g: 0.0,
            b_source: CurveBSource::Blend,
        }
    }
}

impl CurveParams {
    fn rho_at(&self, m_midi: f64) -> f64 {
        self.preset.apply(self.rho.rho_at_midi(m_midi))
    }
}

// ─── Curve-B precision-weighted shrinkage ────────────────────────────────────
//
// Curve-side B is the precision-weighted (inverse-variance) combination of the
// key's measurement and the B_ξ fit, the posterior mean of a normal mean under a
// normal prior:
//
//   ln B_curve = ( ln B_meas/σ_m² + ln B_ξ/σ_p² ) / ( 1/σ_m² + 1/σ_p² )
//              = w·ln B_meas + (1−w)·ln B_ξ,   w = σ_p²/(σ_p² + σ_m²).
//
// Both terms are in ln B because the repeat noise is multiplicative. w → 1 where
// the measurement is precise (bass/mid, σ_m ≪ 1 %) and w → 0 in the treble (σ_m up
// to ≈ 100 %), continuously in the partial count.

/// Coefficient of the capture-to-capture SD of ln B as a function of the persisted
/// partial count n:
///
///   σ_m(n) = max( SIGMA_LNB_COEFF · n⁻³ , SIGMA_LNB_FLOOR ).
// Ours, measured: a least-squares fit of ln σ on ln n over repeat captures, at
// least 5 per key, with exponent −3.00. The blend weight is insensitive to 2× errors
// in σ_m except within ≈ 1 partial of the σ_m = σ_p crossover, so the third digit
// does not matter.
// report 0009
pub const SIGMA_LNB_COEFF: f64 = 19.3;
/// Bass/mid plateau of σ_m (≈ 0.35 %), where 20–32 partials pin B.
pub const SIGMA_LNB_FLOOR: f64 = 3.5e-3;

/// Repeat-capture SD of ln B for a key persisting `partial_count` partials
/// (the shrinkage's σ_m; see [`SIGMA_LNB_COEFF`]).
pub fn sigma_ln_b(partial_count: usize) -> f64 {
    let n = (partial_count.max(1)) as f64;
    (SIGMA_LNB_COEFF / (n * n * n)).max(SIGMA_LNB_FLOOR)
}

/// Keys whose σ_m is at most this calibrate σ_p (n ≥ 10 partials under
/// [`sigma_ln_b`]): their scatter about the B_ξ fit is per-key structure, not
/// capture noise.
// Measured: bass/mid repeat σ ≈ 0.4 % against fit-residual scatter of 6–19 %.
// report 0009
pub const SIGMA_PRIOR_NOISE_CAP: f64 = 0.02;
/// Fewer calibrating keys than this and [`sigma_prior`] returns
/// [`SIGMA_PRIOR_DEFAULT`] — a 2-parameter L1 fit near-interpolates very
/// small key sets, deflating the residual scatter.
pub const SIGMA_PRIOR_MIN_KEYS: usize = 4;
/// Fallback σ_p.
// Between the two measured instruments' 0.062 and 0.186.
// report 0009
pub const SIGMA_PRIOR_DEFAULT: f64 = 0.12;
/// Floor on σ_p — guards the degenerate near-interpolating fit; at the
/// floor a 32-partial measurement still carries w ≈ 0.89.
pub const SIGMA_PRIOR_FLOOR: f64 = 0.01;

/// Per-instrument prior scatter σ_p: the robust SD (1.4826 × MAD, the
/// normal-consistent scale) of ln(B_meas/B_ξ) over the keys whose measurement noise
/// is negligible ([`SIGMA_PRIOR_NOISE_CAP`]), the per-key B structure the
/// 2-parameter B_ξ family cannot express.
// Calibrated per instrument, not a constant: it measures 0.062 on one piano and
// 0.186 on the other.
// report 0009
pub fn sigma_prior(input: &CurveInput, bxi: &BXi) -> f64 {
    let mut residuals: Vec<f64> = input
        .keys
        .iter()
        .enumerate()
        .filter_map(|(k, d)| {
            let d = d.as_ref()?;
            (sigma_ln_b(d.partials.len()) <= SIGMA_PRIOR_NOISE_CAP)
                .then(|| d.b.ln() - bxi.b_at_key(k).ln())
        })
        .collect();
    if residuals.len() < SIGMA_PRIOR_MIN_KEYS {
        return SIGMA_PRIOR_DEFAULT;
    }
    let med = |v: &mut Vec<f64>| -> f64 {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    let m = med(&mut residuals);
    let mut devs: Vec<f64> = residuals.iter().map(|r| (r - m).abs()).collect();
    (1.4826 * med(&mut devs)).max(SIGMA_PRIOR_FLOOR)
}

/// Tolerance below which a computed octave stretch counts as negative
/// (cents). Guards the negative-stretch detector against floating-point
/// noise, nothing more — real violations are on the order of whole cents.
pub const NEGATIVE_STRETCH_TOL_CENTS: f64 = 0.01;

/// Reversion length ℓ of the prior-mean reversion term, in keys.
///
/// The second-difference penalty's null space is the affine functions, so in a
/// data-free region, such as the treble past the last trusted key, the penalized
/// least-squares minimizer continues the residual's end slope as a straight line
/// (Eilers 2003: extrapolation is a polynomial of degree d − 1). The term observes
/// the prior mean, residual 0, at every key carrying no data, with weight w₀: a
/// Gaussian prior in the objective, not an output clamp. The tail Euler–Lagrange
/// equation w₀z + λz'''' = 0 decays at rate (w₀/λ)^(1/4)/√2, so
/// ℓ = √2·(λ/w₀)^(1/4), i.e. w₀ = 4λ/ℓ⁴. Tying w₀ to λ keeps ℓ fixed across the
/// CV grid, and λ → ∞ reproduces the prior.
// Ours: one octave, inside the B_ξ model's own correlation lengths, its asymptote
// e-foldings 1/s_T ≈ 10.8 and 1/|s_B| ≈ 12–15 keys, since a trend is not carried
// past the model generating it.
// report 0007
pub const REVERSION_LENGTH_KEYS: f64 = 12.0;

/// Prior-reversion pseudo-observation weight w_0 = 4λ/ℓ⁴
/// (see [`REVERSION_LENGTH_KEYS`] for the derivation).
fn reversion_weight(lambda: f64) -> f64 {
    4.0 * lambda / REVERSION_LENGTH_KEYS.powi(4)
}

/// Fewest coincident 2j:j pairs for a Giordano scan to enter the ρ fit. Giordano
/// §VI.C's A0–A1 and A1–A2 stretch converges only with 16 lower and 8 upper
/// partials, min(⌊16/2⌋, 8) = 8 pairs, and fewer under-predict the stretch
/// ([`giordano::coincident_pairs`]).
// Measured: 42 of 74 octave pairs pass it.
// report 0008
pub const GIORDANO_MIN_COINCIDENT_PAIRS: usize = 8;

/// Fewest accepted ρ points for engine (c)'s Eq.-9 refit; below it (c) degrades
/// to (b).
// Ours: twice φ's three parameters.
pub const RHO_FIT_MIN_POINTS: usize = 6;

/// Decade grid for the Eq.-9 regularization weight, (lo, hi, steps) in log₁₀: from
/// 10⁻², where the fit follows the ρ points, to 10², where it is pinned to the
/// prior. Both ends lie past the useful range, so the CV minimum is interior.
pub const RHO_REG_GRID_DECADES: (f64, f64, usize) = (-2.0, 2.0, 5);

/// Selects the Eq.-9 regularization weight by leave-one-out cross-validation over
/// the ρ points: for each weight on [`RHO_REG_GRID_DECADES`], drop one octave pair,
/// refit, and score the absolute error at the held-out pair. The one-standard-error
/// rule then takes the largest weight whose mean error is within one SE of the
/// minimum (SE = std of the per-point errors / √n): the most parsimonious model the
/// data cannot tell from the best (Hastie, Tibshirani & Friedman, *The Elements of
/// Statistical Learning*, 2nd ed., §7.10). Falls back to the grid's strongest
/// weight when no candidate scores.
// Do not replace the 1-SE rule with the bare CV argmin: the LOO curve is flat at
// noise level across four decades, and the ρ points end near key 44, so the argmin
// lands on a grid edge and drives engine (c) to A7 +47.2 ¢ / C8 +60.5 ¢.
// report 0008
pub fn select_rho_reg_weight(points: &[(f64, f64)], prior: &RhoPhi) -> f64 {
    let (lo, hi, steps) = RHO_REG_GRID_DECADES;
    let strongest = 10f64.powf(hi);
    if points.len() < 2 {
        return strongest;
    }
    // Per-weight LOO error statistics: (weight, mean, se).
    let mut stats: Vec<(f64, f64, f64)> = Vec::with_capacity(steps);
    for s in 0..steps {
        let w = 10f64.powf(lo + (hi - lo) * s as f64 / (steps - 1) as f64);
        let mut errs: Vec<f64> = Vec::with_capacity(points.len());
        for i in 0..points.len() {
            let held_out = points[i];
            let rest: Vec<(f64, f64)> = points
                .iter()
                .enumerate()
                .filter(|&(j, _)| j != i)
                .map(|(_, &p)| p)
                .collect();
            let Some(phi) = rigaud::fit_rho_phi(&rest, prior, w) else {
                errs.clear();
                break;
            };
            errs.push((held_out.1 - phi.rho_at_midi(held_out.0)).abs());
        }
        if errs.len() == points.len() {
            let n = errs.len() as f64;
            let mean = errs.iter().sum::<f64>() / n;
            let var = errs.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>() / (n - 1.0);
            let se = (var / n).sqrt();
            if mean.is_finite() && se.is_finite() {
                stats.push((w, mean, se));
            }
        }
    }
    let Some(&(_, min_mean, min_se)) = stats.iter().min_by(|a, b| a.1.total_cmp(&b.1)) else {
        return strongest;
    };
    // The minimizer itself always satisfies the filter, so the fold is
    // over a non-empty set.
    stats
        .iter()
        .filter(|&&(_, mean, _)| mean <= min_mean + min_se)
        .map(|&(w, _, _)| w)
        .fold(f64::MIN, f64::max)
}

/// The 88-key indices of the A notes (A0…A7) — the Eq.-6 chain skeleton.
const A_KEYS: [usize; 8] = [0, 12, 24, 36, 48, 60, 72, 84];

/// 8-point Lagrange polynomial evaluation (Rigaud §II.B.4 interpolates the
/// deviation-from-ET of the tuned A notes over the whole compass; the
/// interpolant is constrained to coincide with the data).
fn lagrange_eval(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    let mut sum = 0.0;
    for i in 0..xs.len() {
        let mut term = ys[i];
        for j in 0..xs.len() {
            if i != j {
                term *= (x - xs[j]) / (xs[i] - xs[j]);
            }
        }
        sum += term;
    }
    sum
}

/// The per-instrument B_ξ fit (Eq. 29) over the input's keys, degrading to the
/// medium-piano default when fewer than 2 keys are measured.
pub fn instrument_b_fit(input: &CurveInput) -> BXi {
    let points: Vec<(f64, f64)> = input
        .keys
        .iter()
        .enumerate()
        .filter_map(|(k, d)| d.as_ref().map(|d| (midi_from_key(k), d.b)))
        .collect();
    rigaud::fit_b_xi(&points).unwrap_or(BXi::DEFAULT_MEDIUM)
}

/// What every engine starts from: the instrument's curve-side B, after the
/// negative-stretch pre-exclusion, and engine (a)'s curve as the prior.
struct CurveBasis {
    /// Curve-side B per key: the precision-weighted blend of the key's
    /// measured B and the B_ξ fit, the fit alone where nothing is measured.
    /// Never used for strobe targets, which take raw measured B.
    curve_b: [f64; 88],
    /// True where `curve_b` is measurement-dominated (blend weight ≥ 1/2).
    b_is_measured: [bool; 88],
    /// Engine (a)'s curve — the prior every other engine corrects.
    prior: [f64; 88],
    flags: [CurveKeyFlags; 88],
    /// The Eq.-29 fit the blend shrinks toward.
    bxi: BXi,
}

fn curve_basis(input: &CurveInput, params: &CurveParams) -> CurveBasis {
    let mut flags = [CurveKeyFlags::default(); 88];

    // Eq. 29 fit over all trusted measured keys (L1 is the outlier guard).
    let bxi = instrument_b_fit(input);

    // `b_is_measured` grades a key as data at the point of equal information,
    // w ≥ 1/2 ⇔ σ_m ≤ σ_p; the blended B itself is continuous across it.
    let sigma_p = sigma_prior(input, &bxi);
    let mut curve_b = [0.0f64; 88];
    let mut b_is_measured = [false; 88];
    for k in 0..88 {
        match &input.keys[k] {
            Some(d) => {
                let sm = sigma_ln_b(d.partials.len());
                let w = sigma_p * sigma_p / (sigma_p * sigma_p + sm * sm);
                curve_b[k] = (w * d.b.ln() + (1.0 - w) * bxi.b_at_key(k).ln()).exp();
                flags[k].measured = true;
                if w >= 0.5 {
                    b_is_measured[k] = true;
                } else {
                    flags[k].curve_b_fallback = true;
                }
            }
            None => {
                curve_b[k] = bxi.b_at_key(k);
                flags[k].curve_b_fallback = true;
            }
        }
    }

    // Negative-stretch pre-exclusion: a negative Eq.-6 octave stretch is an
    // estimator artifact. Exclude the pair's measured key deviating most from B_ξ
    // (by |ln(B/B_ξ)|), fall back to B_ξ, and re-check: flag and exclude, never
    // clamp.
    loop {
        let mut worst: Option<(usize, f64)> = None; // (key to exclude, deviation)
        for m in 0..76 {
            let s =
                octave_stretch_cents(curve_b[m], curve_b[m + 12], params.rho_at(midi_from_key(m)));
            if s >= -NEGATIVE_STRETCH_TOL_CENTS {
                continue;
            }
            for k in [m, m + 12] {
                if b_is_measured[k] {
                    let dev = (curve_b[k].ln() - bxi.b_at_key(k).ln()).abs();
                    if worst.is_none_or(|(_, w)| dev > w) {
                        worst = Some((k, dev));
                    }
                }
            }
        }
        let Some((k, _)) = worst else { break };
        curve_b[k] = bxi.b_at_key(k);
        b_is_measured[k] = false;
        flags[k].excluded = true;
        flags[k].curve_b_fallback = true;
    }

    // Engine (a)'s curve doubles as every engine's prior.
    let prior = a_chain_prior(&bxi, params);

    CurveBasis {
        curve_b,
        b_is_measured,
        prior,
        flags,
        bxi,
    }
}

/// Rigaud §II.B tuning procedure on the B_ξ model: A4 anchored at the
/// reference (f₁ = 440 Hz ⇒ d(A4) = 0), Eq.-6 A-chain up and down, Lagrange
/// interpolation of d over the A notes across the compass.
fn a_chain_prior(bxi: &BXi, params: &CurveParams) -> [f64; 88] {
    let b = |k: usize| bxi.b_at_key(k);
    let mut f0 = [0.0f64; 88];
    // f1(A4) = 440 ⇒ F0 = 440/√(1+B) (flexible-string convention).
    f0[48] = 440.0 / (1.0 + b(48)).sqrt();
    for a in [60usize, 72, 84] {
        // ρ is indexed by the reference (lower) note m (Eq. 9 note).
        let rho = params.rho_at(midi_from_key(a - 12));
        f0[a] = 2.0
            * f0[a - 12]
            * ((1.0 + 4.0 * rho * rho * b(a - 12)) / (1.0 + rho * rho * b(a))).sqrt();
    }
    for a in [36usize, 24, 12, 0] {
        let rho = params.rho_at(midi_from_key(a));
        f0[a] = f0[a + 12]
            / (2.0 * ((1.0 + 4.0 * rho * rho * b(a)) / (1.0 + rho * rho * b(a + 12))).sqrt());
    }

    let xs: Vec<f64> = A_KEYS.iter().map(|&a| midi_from_key(a)).collect();
    let ys: Vec<f64> = A_KEYS
        .iter()
        .map(|&a| {
            let f1 = f0[a] * (1.0 + b(a)).sqrt();
            1200.0 * (f1 / NOTES[a].frequency as f64).log2()
        })
        .collect();
    core::array::from_fn(|k| lagrange_eval(&xs, &ys, midi_from_key(k)))
}

/// Finalizes an engine's cents vector into a [`TuningCurve`]: re-anchors
/// A4 to exactly 0 (a vertical gauge shift — the reference pitch lives in
/// `d_g`), then runs the negative-stretch detector over the final curve, flagging
/// (never fixing) any remaining d(m+12) < d(m) pair.
fn finish(mut cents: [f64; 88], mut flags: [CurveKeyFlags; 88], d_g: f64) -> TuningCurve {
    let a4 = cents[48];
    for c in cents.iter_mut() {
        *c -= a4;
    }
    for m in 0..76 {
        if cents[m + 12] < cents[m] - NEGATIVE_STRETCH_TOL_CENTS {
            flags[m].negative_stretch = true;
            flags[m + 12].negative_stretch = true;
        }
    }
    TuningCurve {
        cents: core::array::from_fn(|k| cents[k] as f32),
        d_g: d_g as f32,
        flags,
    }
}

// ─── Engine (a): Rigaud-pure ─────────────────────────────────────────────────

/// Engine (a): Rigaud, David & Daudet's §II.B procedure end to end. The
/// per-instrument B_ξ fit (Eq. 29) and the octave-type curve ρ̄_φ(m) (Eq. 9, with
/// the §IV.C.2 presets) drive an Eq.-6 chain of A notes from A4, Lagrange
/// interpolation (§II.B.4) spreads it over the compass, and d_g carries the global
/// deviation (Eq. 32's role). About 3 effective degrees of freedom: heavy on bias,
/// light on variance. With no trusted measurements the fit degrades to the
/// medium-piano default, a generic starting curve.
pub fn rigaud_pure(input: &CurveInput, params: &CurveParams) -> TuningCurve {
    let basis = curve_basis(input, params);
    finish(basis.prior, basis.flags, params.d_g)
}

// ─── Engine (b): per-key coincidence + Whittaker ─────────────────────────────

/// Engine (b)'s raw per-key octave-chain deviations, before smoothing. Returns the
/// raw d(m) and, per key, whether its curve-B is measurement-dominated.
///
/// **Gauge.** Eq. 6 fixes only octave differences, so each of the 12 semitone
/// chains carries an offset the data cannot identify. It takes the minimum-norm
/// (Moore–Penrose) choice: the offset that zeroes the mean of (raw − prior) over
/// the chain's measured keys. No key is pinned, so no fabricated zero-residual
/// point reaches the smoother; A4 = 0 is the one physical anchor, applied to the
/// finished curve.
pub fn raw_octave_chain(input: &CurveInput, params: &CurveParams) -> ([f64; 88], [bool; 88]) {
    let basis = curve_basis(input, params);
    (raw_chain(&basis, params), basis.b_is_measured)
}

fn raw_chain(basis: &CurveBasis, params: &CurveParams) -> [f64; 88] {
    let mut raw = basis.prior;
    for c in 0..12usize {
        // Chain bottom-up from an arbitrary start; the start value is pure
        // gauge and is removed by the mean-centering below.
        raw[c] = basis.prior[c];
        let mut m = c;
        while m + 12 < 88 {
            let s = octave_stretch_cents(
                basis.curve_b[m],
                basis.curve_b[m + 12],
                params.rho_at(midi_from_key(m)),
            );
            raw[m + 12] = raw[m] + s;
            m += 12;
        }
        // Minimum-norm gauge: zero the mean residual over the chain's
        // measured keys (over all its keys when none are measured — the
        // residual then carries no weight downstream anyway).
        let chain_keys = || (c..88).step_by(12);
        let measured: Vec<usize> = chain_keys().filter(|&k| basis.b_is_measured[k]).collect();
        let gauge_keys: Vec<usize> = if measured.is_empty() {
            chain_keys().collect()
        } else {
            measured
        };
        let offset: f64 = gauge_keys
            .iter()
            .map(|&k| raw[k] - basis.prior[k])
            .sum::<f64>()
            / gauge_keys.len() as f64;
        for k in chain_keys() {
            raw[k] -= offset;
        }
    }
    raw
}

/// Engine (b): engine (a) with measured per-key B in the Eq.-6 stretches, and a
/// Whittaker smoother (Eilers 2003) on the cents residual from (a)'s curve, λ by
/// leave-one-out CV. Effective degrees of freedom grow with trusted captures; with
/// fewer than 3 measured keys the smoother is skipped and the curve is the prior.
/// ρ stays the configured model, so measured B improves local fidelity, not the
/// global stretch.
///
/// Keys without measured curve-B enter the smoother as observations of the prior
/// mean (residual 0) at weight w₀ = 4λ/ℓ⁴ ([`REVERSION_LENGTH_KEYS`]), so the curve
/// decays to the prior within ≈ ℓ keys of the last data. The CV scores measured
/// keys only.
// report 0007
pub fn per_key_smoothed(input: &CurveInput, params: &CurveParams) -> TuningCurve {
    let basis = curve_basis(input, params);
    let raw = raw_chain(&basis, params);

    // Residual from the prior mean at measured keys; the prior mean itself
    // (0) everywhere else — the reversion pseudo-observations' targets.
    let residual: Vec<f64> = (0..88)
        .map(|k| {
            if basis.b_is_measured[k] {
                raw[k] - basis.prior[k]
            } else {
                0.0
            }
        })
        .collect();
    let cv_mask: Vec<bool> = basis.b_is_measured.to_vec();
    let measured_count = cv_mask.iter().filter(|&&m| m).count();

    let mut cents = basis.prior;
    if measured_count >= 3 {
        // w₀ is tied to λ (fixed reversion length ℓ), so each grid point
        // gets its own weight vector; CV scores measured keys only.
        let weights_for = |lambda: f64| -> Vec<f64> {
            let w0 = reversion_weight(lambda);
            (0..88)
                .map(|k| if basis.b_is_measured[k] { 1.0 } else { w0 })
                .collect()
        };
        let (lo, hi, steps) = whittaker::LAMBDA_GRID_DECADES;
        let mut best: Option<(f64, f64)> = None; // (cv, lambda)
        for s in 0..steps {
            let lambda = 10f64.powf(lo + (hi - lo) * s as f64 / (steps - 1) as f64);
            if let Some(cv) =
                whittaker::cv_masked(&residual, &weights_for(lambda), lambda, &cv_mask)
                && best.is_none_or(|(bcv, _)| cv < bcv)
            {
                best = Some((cv, lambda));
            }
        }
        if let Some((_, lambda)) = best
            && let Some(smoothed) = whittaker::smooth(&residual, &weights_for(lambda), lambda)
        {
            for k in 0..88 {
                cents[k] += smoothed[k];
            }
        }
    }
    finish(cents, basis.flags, params.d_g)
}

// ─── Engine (c): (b) + Giordano-calibrated octave type ───────────────────────

/// Engine (c)'s calibration stage: per-octave coincidence-bracket scans, the §VI.C
/// sufficiency gate, and the Eq.-30 inversion. Returns the accepted
/// `(m_midi, rho)` points and the per-key exclusion flags.
pub fn giordano_rho_points(input: &CurveInput) -> (Vec<(f64, f64)>, [bool; 88]) {
    let mut excluded = [false; 88];
    let mut points: Vec<(f64, f64)> = Vec::new();

    for (m, excl) in excluded.iter_mut().enumerate().take(76) {
        let (Some(lo), Some(up)) = (&input.keys[m], &input.keys[m + 12]) else {
            continue;
        };
        // Sufficiency gate (Giordano §VI.C): ≥ 8 coincident 2j:j pairs and an
        // interior dissonance minimum; else the pair is excluded from the ρ fit
        // (an edge hit is a detector artifact).
        if giordano::coincident_pairs(&lo.partials, &up.partials) < GIORDANO_MIN_COINCIDENT_PAIRS {
            *excl = true;
            continue;
        }
        let Some(scan) =
            giordano::octave_scan(&lo.partials, &up.partials, (lo.f0, lo.b), (up.f0, up.b))
        else {
            *excl = true;
            continue;
        };
        if !scan.interior {
            *excl = true;
            continue;
        }
        // The scan's optimal offset retunes the upper note multiplicatively,
        // so its flexible-string F0 scales by the same factor (both
        // conventions scale together).
        let f0_u_star = up.f0 * (scan.offset_cents / 1200.0).exp2();
        match rigaud::invert_rho(lo.f0, lo.b, f0_u_star, up.b) {
            Some(rho) => points.push((midi_from_key(m), rho)),
            // No real ρ: the dissonance optimum implies a compressed octave
            // — the paper's own missing-data case (§IV.C.1).
            None => *excl = true,
        }
    }
    (points, excluded)
}

/// Engine (c): engine (b) under an instrument-measured octave type. Where partials
/// suffice, per-octave Giordano dissonance scans give optimal widths, Eq. 30 inverts
/// them to ρ points, and an Eq.-9 refit ([`select_rho_reg_weight`]) gives the
/// instrument's octave-type curve; the treble rides its ρ → 1 asymptote. Perceptual
/// taste enters once, as the octave-type selection. When fewer than
/// [`RHO_FIT_MIN_POINTS`] octave pairs pass the gate it is engine (b), with the
/// excluded pairs flagged.
// Ours: the composition. Each method in it is used as published.
pub fn giordano_calibrated(input: &CurveInput, params: &CurveParams) -> TuningCurve {
    let (points, giordano_excluded) = giordano_rho_points(input);

    let calibrated = if points.len() >= RHO_FIT_MIN_POINTS {
        let reg = select_rho_reg_weight(&points, &params.rho);
        rigaud::fit_rho_phi(&points, &params.rho, reg)
    } else {
        None
    };

    let cal_params = CurveParams {
        rho: calibrated.unwrap_or(params.rho),
        ..*params
    };
    let mut curve = per_key_smoothed(input, &cal_params);
    for (f, &g) in curve.flags.iter_mut().zip(&giordano_excluded) {
        f.giordano_excluded = g;
    }
    curve
}

// ─── Engine (d): weighted multi-interval least squares ───────────────────────

/// One interval family in engine (d)'s objective: the coincident pair
/// p:q spanning `k` semitones, tuned pure (`tempered = false`) or to its
/// ET tempering (`tempered = true`, e.g. fifths −1.955 ¢ from pure).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IntervalSpec {
    /// Interval size in semitones.
    pub k: usize,
    /// Lower note's coincident partial rank.
    pub p: u32,
    /// Upper note's coincident partial rank.
    pub q: u32,
    /// `false`: beatless target (τ = 0). `true`: ET-tempered target
    /// (τ = 100k − 1200·log₂(p/q), computed exactly).
    pub tempered: bool,
    /// Relative weight W of this family's residuals.
    pub weight: f64,
}

impl IntervalSpec {
    /// The tempering offset τ_k in cents.
    pub fn tau(&self) -> f64 {
        if self.tempered {
            100.0 * self.k as f64 - 1200.0 * (self.p as f64 / self.q as f64).log2()
        } else {
            0.0
        }
    }
}

/// Default interval set and weights for engine (d): octaves as the 2:1/4:2/6:3
/// compromise, the 3:1 twelfth and 4:1 double octave for long-range coherence, and
/// tempered fifths and fourths. The `weight`s are taste multipliers on the derived
/// sensitivities (see [`multi_interval`]): the measured amplitudes set the register
/// balance, and the preset only prefers interval families, octaves first, after the
/// Verituner patent's interval prioritization.
pub const BALANCED_INTERVALS: &[IntervalSpec] = &[
    IntervalSpec {
        k: 12,
        p: 2,
        q: 1,
        tempered: false,
        weight: 1.0,
    },
    IntervalSpec {
        k: 12,
        p: 4,
        q: 2,
        tempered: false,
        weight: 1.0,
    },
    IntervalSpec {
        k: 12,
        p: 6,
        q: 3,
        tempered: false,
        weight: 1.0,
    },
    IntervalSpec {
        k: 19,
        p: 3,
        q: 1,
        tempered: false,
        weight: 0.5,
    },
    IntervalSpec {
        k: 24,
        p: 4,
        q: 1,
        tempered: false,
        weight: 0.5,
    },
    IntervalSpec {
        k: 7,
        p: 3,
        q: 2,
        tempered: true,
        weight: 0.25,
    },
    IntervalSpec {
        k: 5,
        p: 4,
        q: 3,
        tempered: true,
        weight: 0.25,
    },
];

/// Pure-twelfths style preset (OnlyPure/Stopper precedent): the 3:1
/// twelfth dominates; octaves ride along lightly.
pub const PURE_TWELFTHS_INTERVALS: &[IntervalSpec] = &[
    IntervalSpec {
        k: 19,
        p: 3,
        q: 1,
        tempered: false,
        weight: 2.0,
    },
    IntervalSpec {
        k: 12,
        p: 2,
        q: 1,
        tempered: false,
        weight: 0.2,
    },
    IntervalSpec {
        k: 12,
        p: 4,
        q: 2,
        tempered: false,
        weight: 0.2,
    },
    IntervalSpec {
        k: 24,
        p: 4,
        q: 1,
        tempered: false,
        weight: 0.2,
    },
    IntervalSpec {
        k: 7,
        p: 3,
        q: 2,
        tempered: true,
        weight: 0.1,
    },
];

/// One λ's solution in [`multi_interval`]: the coefficient vector x and
/// the per-data-row weighted fast-LOO scores (empty when not requested).
type IntervalSolve = (Vec<f64>, Vec<f64>);

/// Engine (d): weighted multi-interval least squares in cents space, its components
/// published and its assembly industry practice (Verituner, US 6,529,843). Solving
/// for x = d − d_prior with A4 eliminated (the hard anchor d(A4) = 0):
///
/// J(x) = ∑_{(m,k)} W_{m,k} (x_{m+k} - x_m - t_{m,k})²
///   + λ ∑_m (Δ² x_m)²,
///
/// a banded SPD system, its half-bandwidth the widest interval, solved by Cholesky.
/// The beatless widths t_{m,k} read the B named by `params.b_source`. A data row
/// exists only where both endpoints carry measured curve-B and both coincident
/// partials are measured, and its weight W_{m,k} is derived: the preset's taste
/// multiplier times Giordano's width sensitivity ∂D/∂ε at the pair
/// ([`giordano::pair_width_sensitivity`]), normalized to unit mean. Register
/// follows from the data: in the bass the fundamental is weak and the 6:3 and 4:2
/// pairs carry the amplitude product, so the wide-stretch rows dominate; in the
/// treble only the 2:1 pair exists.
///
/// A key touched by no data row gets a pseudo-row x_m = 0 at weight w₀ = 4λ/ℓ⁴
/// ([`REVERSION_LENGTH_KEYS`]), so the curve decays to the prior there.
/// Pseudo-rows shape the system but are not scored.
///
/// `Some(lambda)` fixes the smoothness weight. `None` selects it by
/// leave-one-row-out CV (the penalized-WLS fast-LOO identity, Eilers 2003
/// Eqs. 10–11) with the one-standard-error rule (ESL §7.10) over
/// [`whittaker::LAMBDA_GRID_DECADES`].
pub fn multi_interval(
    input: &CurveInput,
    params: &CurveParams,
    intervals: &[IntervalSpec],
    lambda: Option<f64>,
) -> TuningCurve {
    let basis = curve_basis(input, params);

    // Column mapping with x[48] eliminated.
    let ncol = 87usize;
    let col = |k: usize| -> Option<usize> {
        match k.cmp(&48) {
            core::cmp::Ordering::Less => Some(k),
            core::cmp::Ordering::Equal => None,
            core::cmp::Ordering::Greater => Some(k - 1),
        }
    };

    // Equal-total-power amplitude normalization per note (Giordano's), the scale of
    // the sensitivity's a_p and a_q.
    let power_norm: [f64; 88] = core::array::from_fn(|k| {
        input.keys[k]
            .as_ref()
            .and_then(|d| {
                let p: f64 = d.partials.iter().map(|&(_, _, a)| a * a).sum();
                (p.is_finite() && p > 0.0).then(|| p.sqrt().recip())
            })
            .unwrap_or(0.0)
    });

    // Data rows: (cols, coefs, target, weight), one or two columns each. An interval
    // missing a coincident partial carries no Giordano evidence, so its row is
    // absent, not down-weighted.
    let mut rows: Vec<(Vec<usize>, Vec<f64>, f64, f64)> = Vec::new();
    for spec in intervals {
        for m in 0..(88 - spec.k) {
            let u = m + spec.k;
            if !(basis.b_is_measured[m] && basis.b_is_measured[u]) {
                continue;
            }
            let find = |k: usize, n: u32| {
                input.keys[k]
                    .as_ref()
                    .and_then(|d| d.partials.iter().find(|&&(r, _, _)| r == n).copied())
            };
            let (Some((_, f_p, a_p)), Some((_, f_q, a_q))) = (find(m, spec.p), find(u, spec.q))
            else {
                continue;
            };
            let f_bar = 0.5 * (f_p + f_q);
            let sens =
                giordano::pair_width_sensitivity(f_bar, a_p * power_norm[m], a_q * power_norm[u]);
            if !(sens.is_finite() && sens > 0.0) {
                continue;
            }
            let (b_m, b_u) = match params.b_source {
                CurveBSource::Blend => (basis.curve_b[m], basis.curve_b[u]),
                CurveBSource::Model => (basis.bxi.b_at_key(m), basis.bxi.b_at_key(u)),
            };
            let c = interval_width_cents(b_m, b_u, spec.p, spec.q, spec.k);
            let t = c + spec.tau() - (basis.prior[u] - basis.prior[m]);
            let (mut cols, mut coefs) = (Vec::with_capacity(2), Vec::with_capacity(2));
            if let Some(cm) = col(m) {
                cols.push(cm);
                coefs.push(-1.0);
            }
            if let Some(cu) = col(u) {
                cols.push(cu);
                coefs.push(1.0);
            }
            rows.push((cols, coefs, t, spec.weight * sens));
        }
    }
    // Unit-mean weights: the derived weights are defined up to a common scale, which
    // λ absorbs. Mean 1 keeps the λ grid's minimum interior and w₀ = 4λ/ℓ⁴
    // commensurate with the data rows.
    if !rows.is_empty() {
        let mean_w: f64 = rows.iter().map(|&(_, _, _, w)| w).sum::<f64>() / rows.len() as f64;
        if mean_w.is_finite() && mean_w > 0.0 {
            for (_, _, _, w) in &mut rows {
                *w /= mean_w;
            }
        }
    }

    // Keys carrying no data row receive the reversion pseudo-rows; A4, eliminated,
    // never does.
    let mut has_data = [false; 88];
    for (cols, _, _, _) in &rows {
        for &c in cols {
            let k = if c < 48 { c } else { c + 1 };
            has_data[k] = true;
        }
    }

    // Assembles A = M + λ·P + w₀·R and solves; optionally computes the
    // per-data-row weighted fast-LOO scores.
    let hbw = intervals.iter().map(|s| s.k).max().unwrap_or(2).max(2);
    let build_data = |sys: &mut BandedSystem| {
        for (cols, coefs, t, w) in &rows {
            sys.add_row(cols, coefs, *t, *w);
        }
    };
    let add_penalty = |sys: &mut BandedSystem, lambda: f64| {
        for r in 0..86 {
            let (mut cols, mut coefs) = (Vec::with_capacity(3), Vec::with_capacity(3));
            for (i, cf) in [(r, 1.0), (r + 1, -2.0), (r + 2, 1.0)] {
                if let Some(ci) = col(i) {
                    cols.push(ci);
                    coefs.push(cf);
                }
            }
            sys.add_row(&cols, &coefs, 0.0, lambda);
        }
        // Reversion pseudo-rows, x_k = 0 at w₀ = 4λ/ℓ⁴, never scored by the CV.
        let w0 = reversion_weight(lambda);
        for (k, &spoken_for) in has_data.iter().enumerate() {
            if !spoken_for && let Some(ck) = col(k) {
                sys.add_row(&[ck], &[1.0], 0.0, w0);
            }
        }
    };
    // Fast leave-one-row-out CV per data row (the penalized-WLS identity,
    // same form as `whittaker::cv` / Eilers 2003 Eqs. 10–11):
    // score_i = w_i·(r_i/(1 − h_ii))² with leverage
    // h_ii = w_i·aᵢᵀA⁻¹aᵢ. Returns `None` when some row is reproduced
    // exactly (h_ii → 1 — cannot be cross-validated).
    let solve_for = |lambda: f64, want_cv: bool| -> Option<IntervalSolve> {
        let mut sys = BandedSystem::new(ncol, hbw);
        build_data(&mut sys);
        add_penalty(&mut sys, lambda);
        let chol = sys.cholesky()?;
        let mut x = sys.rhs.clone();
        chol.solve_in_place(&mut x);
        if !want_cv {
            return Some((x, Vec::new()));
        }
        let mut scores = Vec::with_capacity(rows.len());
        let mut z = vec![0.0; ncol];
        for (cols, coefs, t, w) in &rows {
            z.fill(0.0);
            for (&c, &a) in cols.iter().zip(coefs) {
                z[c] = a;
            }
            chol.solve_in_place(&mut z);
            let h: f64 = w * cols.iter().zip(coefs).map(|(&c, &a)| a * z[c]).sum::<f64>();
            let denom = 1.0 - h;
            if denom <= 1e-12 {
                return None;
            }
            let pred: f64 = cols.iter().zip(coefs).map(|(&c, &a)| a * x[c]).sum();
            let r = (pred - t) / denom;
            scores.push(w * r * r);
        }
        Some((x, scores))
    };

    let solution = if rows.is_empty() {
        None // No measured intervals: the curve is the prior.
    } else if let Some(l) = lambda {
        solve_for(l, false)
    } else {
        // λ by leave-one-row-out CV with the one-standard-error rule (ESL §7.10):
        // the largest λ whose mean score is within one SE of the minimum. Not GCV:
        // the derived weights span ≈ 2 orders of magnitude, breaking its
        // equal-variance premise. Per-row LOO scores each row on its own weighted
        // scale, and the 1-SE rule breaks the shallow valley toward the prior.
        // report 0008
        let (lo, hi, steps) = whittaker::LAMBDA_GRID_DECADES;
        let mut stats: Vec<(f64, f64, f64, Vec<f64>)> = Vec::new(); // (λ, mean, se, x)
        for s in 0..steps {
            let l = 10f64.powf(lo + (hi - lo) * s as f64 / (steps - 1) as f64);
            if let Some((x, scores)) = solve_for(l, true) {
                let n = scores.len() as f64;
                let mean = scores.iter().sum::<f64>() / n;
                let var = scores.iter().map(|e| (e - mean) * (e - mean)).sum::<f64>() / (n - 1.0);
                let se = (var / n).sqrt();
                if mean.is_finite() && se.is_finite() {
                    stats.push((l, mean, se, x));
                }
            }
        }
        stats
            .iter()
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|&(_, min_mean, min_se, _)| min_mean + min_se)
            .and_then(|threshold| {
                stats
                    .into_iter()
                    .filter(|&(_, mean, _, _)| mean <= threshold)
                    .max_by(|a, b| a.0.total_cmp(&b.0))
            })
            .map(|(_, _, _, x)| (x, Vec::new()))
    };

    let mut cents = basis.prior;
    if let Some((x, _)) = solution {
        for (k, c) in cents.iter_mut().enumerate() {
            if let Some(ck) = col(k) {
                *c += x[ck];
            }
        }
    }
    finish(cents, basis.flags, params.d_g)
}

/// The fallback displayed strobe partial `n*` per key, where amplitude-informed
/// selection ([`select_display_partials`]) has nothing to go on: the default of
/// TuneLab's editable "Table of Partials", the 6th partial in the low bass, then
/// the 4th, then the 2nd, then the fundamental from A4 up. The boundaries follow
/// the aural octave-type registers (6:3 bass, 4:2 tenor, 2:1 toward the treble).
///
/// The treble entry is derived as well as precedented: the n = 1 target is immune
/// to B (f₁ anchors the target chain), so the treble's ×8 repeat σ_lnB cannot move
/// it.
// No choice of partial fixes deep-bass window leakage; the strobe's long window
// does.
// report 0009
pub fn default_display_partials() -> [u8; 88] {
    let mut table = [1u8; 88];
    for (key, n) in table.iter_mut().enumerate() {
        *n = match key {
            0..=26 => 6,  // A0–B2: bass — weak fundamental, 6:3 register
            27..=38 => 4, // C3–B3: tenor — 4:2 register
            39..=47 => 2, // C4–G#4: low treble — 2:1 register
            _ => 1,       // A4–C8: fundamental — immune to B
        };
    }
    table
}

/// The partials the amplitude search may pick from per key: the registers of
/// [`default_display_partials`], widened to bands. δcents(n) ≈ 866·B·(n²−1)·σ_lnB
/// shapes them: any n > 1 target in the treble moves with its ×8 σ_lnB, so the
/// treble is pinned to the fundamental, while bass B is small enough
/// (δcents < 0.1 ¢ even at n = 8) for amplitude to choose.
fn register_window(key: usize) -> (u32, u32) {
    match key {
        0..=26 => (4, 8),  // bass — 6:3 register
        27..=38 => (2, 4), // tenor — 4:2 register
        39..=47 => (1, 2), // low treble — 2:1 register
        _ => (1, 1),       // treble — fundamental only (B-immune)
    }
}

/// Per-key displayed strobe partial `n*`, after CyberTuner's "Smart Partials": the
/// loudest measured partial within the key's register window. Captures are taken
/// after the attack, so these are the partials strong while a note sustains. A key
/// with no measurement keeps its [`default_display_partials`] value.
pub fn select_display_partials(input: &CurveInput) -> [u8; 88] {
    let mut out = default_display_partials();
    for (key, slot) in out.iter_mut().enumerate() {
        let Some(data) = input.keys[key].as_ref() else {
            continue;
        };
        let (lo, hi) = register_window(key);
        // A window with no measured partial keeps the register default.
        if let Some((n, _, _)) = data
            .partials
            .iter()
            .filter(|(n, _, _)| *n >= lo && *n <= hi)
            .max_by(|a, b| a.2.total_cmp(&b.2))
        {
            *slot = *n as u8;
        }
    }
    out
}

/// First key whose coarse read centres on the fundamental; below it the
/// fundamental is too weak to admit.
// Measured: n = 1 availability is 0 % at G1 and 32 % at F1, B1 reads 10.7 ¢ off at
// 81 %, and it is clean from C#2 up. n = 4 is clean across keys 8–20, so the
// handover sits inside an overlap.
// report 0011
const COARSE_READ_FUNDAMENTAL_KEY: usize = 16;

/// Per-key coarse-read partial `n*`, the partial the wide-range magnitude read
/// centres on: the 4th below `COARSE_READ_FUNDAMENTAL_KEY`, the fundamental from
/// there up. Not [`default_display_partials`]: the strobe band and the coarse
/// number answer different questions.
// Ours, measured: a fixed n* = 4 is the cross-instrument answer (n = 5 is the
// piano's only strict all-pass, but a guitar's E2 fails on it, jitter 13.8 ¢), and
// both tiebreaks agree: cold-start bias grows as n² − 1 (5 ¢ at n = 4, 8 ¢ at
// n = 5), and a per-key margin argmax rides high (B0 +7.5 ¢ against +0.7 ¢). Do
// not select per hop: switching partials steps the number by 866·ΔB·n², 20 ¢+.
// report 0011
pub fn coarse_read_partial(key_index: u8) -> u8 {
    if (key_index as usize) < COARSE_READ_FUNDAMENTAL_KEY {
        4
    } else {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{InharmonicityProfile, KeyMeasurement, Partial};

    /// Fallback display-partial table: values from the TuneLab default
    /// set, monotone non-increasing across the compass, fundamental from A4
    /// (key 48) up.
    #[test]
    fn test_default_display_partials() {
        let t = default_display_partials();
        assert!(t.iter().all(|n| [1, 2, 4, 6].contains(n)));
        assert!(
            t.windows(2).all(|w| w[0] >= w[1]),
            "register rule must be monotone toward the treble"
        );
        assert_eq!(t[0], 6, "deep bass tunes on the 6th partial");
        assert!(
            t[48..].iter().all(|&n| n == 1),
            "A4 and above tune on the fundamental"
        );
    }

    /// Smart Partials: a measured key picks the loudest partial
    /// inside its register window; partials outside the window are ignored
    /// however loud; an unmeasured key keeps the default; treble is pinned to
    /// the fundamental (window [1, 1]).
    #[test]
    fn test_select_display_partials() {
        let mut input = CurveInput::default();
        // A0 (key 0), window [4, 8]: n = 7 is the loudest in-window partial.
        // n = 1 (amp 100) is below the window and n = 9 (amp 50) above it, so
        // both are ignored despite being louder — the window is the stability
        // guard.
        input.keys[0] = Some(CurveKeyData {
            b: 4e-4,
            f0: 27.5,
            partials: vec![
                (1, 27.5, 100.0),
                (4, 110.0, 3.0),
                (6, 165.0, 8.0),
                (7, 192.0, 20.0),
                (9, 247.0, 50.0),
            ],
        });
        // A5 (key 60), window [1, 1]: a loud 2nd partial must not pull the
        // treble band off the fundamental.
        input.keys[60] = Some(CurveKeyData {
            b: 6e-3,
            f0: 880.0,
            partials: vec![(1, 880.0, 5.0), (2, 1760.0, 50.0)],
        });
        let t = select_display_partials(&input);
        let fallback = default_display_partials();
        assert_eq!(t[0], 7, "bass picks the loudest partial within [4, 8]");
        assert_eq!(t[60], 1, "treble is pinned to the fundamental");
        assert_eq!(t[10], fallback[10], "unmeasured bass key keeps the default");
        assert_eq!(
            t[48], fallback[48],
            "unmeasured treble key keeps the default"
        );
    }

    /// Fixed n* = 4 below the handover key, the fundamental from it up, and
    /// independent of the display table, whose bass entry is 6.
    // report 0011
    #[test]
    fn test_coarse_read_partial() {
        assert_eq!(coarse_read_partial(0), 4, "A0 reads on the 4th partial");
        assert_eq!(coarse_read_partial(15), 4, "C2 is the last n* = 4 key");
        assert_eq!(coarse_read_partial(16), 1, "C#2 hands over to n = 1");
        assert_eq!(coarse_read_partial(87), 1);
        // Every guitar string sits above the handover.
        assert!((19..=88).all(|k| coarse_read_partial(k as u8) == 1));
        // The two tables are answering different questions.
        assert_ne!(coarse_read_partial(0), default_display_partials()[0]);
    }

    /// Synthetic trusted profile: exact stiff-string partials from a smooth
    /// physical B(m) (the medium default), manual provenance.
    fn synth_profile(keys: impl Iterator<Item = usize>) -> InharmonicityProfile {
        let mut profile = InharmonicityProfile::default();
        for k in keys {
            let b = BXi::DEFAULT_MEDIUM.b_at_key(k) as f32;
            let f0 = NOTES[k].frequency; // flexible-string fundamental
            let n_partials = match k {
                0..=27 => 24u32,
                28..=59 => 14,
                _ => 10,
            };
            let partials: Vec<Partial> = (1..=n_partials)
                .map(|n| {
                    let nf = n as f32;
                    Partial {
                        number: n,
                        frequency: nf * f0 * (1.0 + b * nf * nf).sqrt(),
                        amplitude: 0.85f32.powi(n as i32 - 1),
                    }
                })
                .filter(|p| p.frequency < crate::audio::SAMPLE_RATE as f32 / 2.0)
                .collect();
            profile.record(KeyMeasurement {
                key_index: k as u8,
                measured_f0: f0,
                partials,
                calculated_b: Some(b),
                last_captured: String::new(),
                captured_in_auto: false,
                sounding_strings: None,
            });
        }
        profile
    }

    /// Eq.-6 closed-form sanity — zero at B = 0, strictly positive stretch for
    /// B > 0 (the stretched-octave theorem at moderate ρ), monotone in B_L.
    #[test]
    fn test_octave_stretch_closed_form() {
        assert!(octave_stretch_cents(0.0, 0.0, 2.0).abs() < 1e-12);
        let s = octave_stretch_cents(1e-3, 1.5e-3, 2.0);
        assert!(s > 0.0, "stiff octave must stretch, got {s}");
        assert!(octave_stretch_cents(2e-3, 1.5e-3, 2.0) > s);
        // Hand value: b_l=1e-3, b_u=0, ρ=1 ⇒ ratio 2√(1.004/1.001),
        // s = 600·log₂(1.004/1.001) ≈ 2.5924 ¢.
        let hand = 600.0 * (1.004f64 / 1.001).log2();
        assert!((octave_stretch_cents(1e-3, 0.0, 1.0) - hand).abs() < 1e-9);
    }

    /// At ρ = 1 the upper note's B cancels exactly — the treble fallback's
    /// insensitivity result.
    #[test]
    fn test_rho1_upper_b_cancellation() {
        let s1 = octave_stretch_cents(8e-4, 1e-5, 1.0);
        let s2 = octave_stretch_cents(8e-4, 5e-3, 1.0);
        assert!(
            (s1 - s2).abs() < 1e-9,
            "B_U leaked through at ρ=1: {s1} vs {s2}"
        );
    }

    /// Negative stretch requires B_U/B_L beyond (4ρ²−1)/(ρ²−1) — the
    /// detector's trigger condition, checked on both sides.
    #[test]
    fn test_negative_stretch_threshold() {
        // ρ=2: threshold 15/3 = 5.
        assert!(octave_stretch_cents(1e-3, 4.0e-3, 2.0) > 0.0);
        assert!(octave_stretch_cents(1e-3, 6.0e-3, 2.0) < 0.0);
    }

    /// The (2:1, k=12) interval width coincides with Eq. 6 at ρ = 1 — the
    /// f₀→f₁ conversion is consistent across both width functions.
    #[test]
    fn test_interval_width_octave_consistency() {
        let (b_l, b_u) = (9e-4, 1.3e-3);
        let a = interval_width_cents(b_l, b_u, 2, 1, 12);
        let b = octave_stretch_cents(b_l, b_u, 1.0);
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    /// Tempering offsets: pure intervals get τ = 0; the ET fifth is
    /// −1.955 ¢ from pure, the fourth +1.955 ¢.
    #[test]
    fn test_tau() {
        let fifth = IntervalSpec {
            k: 7,
            p: 3,
            q: 2,
            tempered: true,
            weight: 1.0,
        };
        let fourth = IntervalSpec {
            k: 5,
            p: 4,
            q: 3,
            tempered: true,
            weight: 1.0,
        };
        let twelfth = IntervalSpec {
            k: 19,
            p: 3,
            q: 1,
            tempered: false,
            weight: 1.0,
        };
        assert!((fifth.tau() + 1.955).abs() < 1e-3);
        assert!((fourth.tau() - 1.955).abs() < 1e-3);
        assert_eq!(twelfth.tau(), 0.0);
    }

    #[test]
    fn test_lagrange_through_points() {
        let xs = [1.0, 2.0, 4.0, 7.0];
        let ys = [3.0, -1.0, 0.5, 10.0];
        for i in 0..4 {
            assert!((lagrange_eval(&xs, &ys, xs[i]) - ys[i]).abs() < 1e-9);
        }
    }

    /// Property test: on a valid synthetic profile every engine yields a
    /// finite curve, anchored at A4 = 0, with monotone octaves (no detector
    /// flags).
    #[test]
    fn test_engines_on_valid_synthetic_profile() {
        let profile = synth_profile(0..88);
        let input = CurveInput::from_profile(&profile);
        assert_eq!(input.measured_count(), 88);
        let params = CurveParams::default();

        let curves = [
            ("a", rigaud_pure(&input, &params)),
            ("b", per_key_smoothed(&input, &params)),
            ("c", giordano_calibrated(&input, &params)),
            (
                "d",
                multi_interval(&input, &params, BALANCED_INTERVALS, None),
            ),
        ];
        for (name, curve) in &curves {
            assert!(
                curve.cents.iter().all(|c| c.is_finite()),
                "engine {name} non-finite"
            );
            assert_eq!(curve.cents[48], 0.0, "engine {name} A4 not anchored");
            for m in 0..76 {
                assert!(
                    !curve.flags[m].negative_stretch,
                    "engine {name} flags negative stretch at key {m} on clean input \
                     (d(m)={}, d(m+12)={})",
                    curve.cents[m],
                    curve.cents[m + 12]
                );
            }
            // Railsback shape: bass below ET, treble above.
            assert!(curve.cents[0] < 0.0, "engine {name}: A0 not below ET");
            assert!(curve.cents[87] > 0.0, "engine {name}: C8 not above ET");
        }
    }

    /// The provenance rule holds: auto-mode captures never feed the curve.
    #[test]
    fn test_auto_captures_are_untrusted() {
        let mut profile = synth_profile(0..88);
        for entries in profile.measurements.values_mut() {
            for m in entries {
                m.captured_in_auto = true;
            }
        }
        let input = CurveInput::from_profile(&profile);
        assert_eq!(input.measured_count(), 0);
        // Engines still produce the generic prior curve.
        let curve = rigaud_pure(&input, &CurveParams::default());
        assert!(curve.cents.iter().all(|c| c.is_finite()));
        assert!(curve.flags.iter().all(|f| !f.measured));

        // The diagnostics builder waives this disqualification, and only this one.
        let diagnostic = CurveInput::from_profile_including_auto(&profile);
        assert_eq!(diagnostic.measured_count(), 88);
    }

    /// A string-isolated capture measured one string, not the note, so it feeds no
    /// curve on either path: the diagnostics builder waives only auto-mode
    /// provenance.
    // report 0014
    #[test]
    fn test_solo_captures_never_feed_a_curve() {
        let mut profile = synth_profile(0..88);
        for entries in profile.measurements.values_mut() {
            for m in entries {
                m.sounding_strings = Some(crate::models::SoundingStrings::UNDECLARED.toggled(1));
            }
        }
        assert_eq!(CurveInput::from_profile(&profile).measured_count(), 0);
        assert_eq!(
            CurveInput::from_profile_including_auto(&profile).measured_count(),
            0,
            "a solo is not a stand-in for the note on any path"
        );
    }

    /// Negative-stretch detector: a wildly broken measured B (upper of a bass octave far
    /// above its lower) is excluded — flagged, replaced by the fit, never
    /// clamped into the output.
    #[test]
    fn test_detector_excludes_broken_measurement() {
        let mut profile = synth_profile(0..88);
        // Poison key 14's B to 40× its physical value: pair (2, 14) then
        // implies a compressed octave under ρ(bass) ≈ 4.4.
        let poisoned = 14u8;
        let m = profile.active_mut(poisoned).unwrap();
        let b_bad = m.calculated_b.unwrap() * 40.0;
        m.calculated_b = Some(b_bad);
        let input = CurveInput::from_profile(&profile);
        let params = CurveParams::default();
        let curve = per_key_smoothed(&input, &params);
        assert!(
            curve.flags[poisoned as usize].excluded,
            "poisoned key not excluded"
        );
        assert!(curve.flags[poisoned as usize].curve_b_fallback);
        // With the offender excluded the final curve stays monotone.
        for m in 0..76 {
            assert!(
                !curve.flags[m].negative_stretch,
                "negative stretch survived at {m}"
            );
        }
    }

    /// σ_m(n) falls to its floor; σ_p self-calibrates, small on an on-model profile
    /// and near the deviation scale on a deviating one; the blend keeps a precise
    /// measurement and marks a starved one prior-dominated, which is not an
    /// exclusion.
    // report 0009
    #[test]
    fn test_curve_b_shrinkage() {
        // σ model shape.
        assert!(sigma_ln_b(4) > sigma_ln_b(6));
        assert!(sigma_ln_b(6) > sigma_ln_b(10));
        assert_eq!(sigma_ln_b(32), SIGMA_LNB_FLOOR);
        assert!((sigma_ln_b(10) - 19.3e-3).abs() < 1e-4);

        // Self-calibrated σ_p: an exactly on-model profile deflates to the
        // floor; the ±30 % deviating profile measures its own scatter.
        let on_model = CurveInput::from_profile(&synth_profile(0..88));
        let bxi = instrument_b_fit(&on_model);
        assert!(sigma_prior(&on_model, &bxi) <= 0.02);
        let deviating = CurveInput::from_profile(&synth_profile_deviating(0..88));
        let bxi_dev = instrument_b_fit(&deviating);
        let sp = sigma_prior(&deviating, &bxi_dev);
        assert!((0.1..0.5).contains(&sp), "sigma_prior {sp} not near 0.3/√2");

        // On the deviating profile a 24-partial bass key is measurement-dominated,
        // and the same key starved to 4 partials is prior-dominated: flagged, not
        // excluded.
        let params = CurveParams::default();
        let full = per_key_smoothed(&deviating, &params);
        assert!(!full.flags[14].curve_b_fallback);
        let mut starved_profile = synth_profile_deviating(0..88);
        starved_profile.active_mut(14).unwrap().partials.truncate(4);
        let starved = CurveInput::from_profile(&starved_profile);
        let curve = per_key_smoothed(&starved, &params);
        assert!(
            curve.flags[14].curve_b_fallback,
            "4 partials not prior-dominated"
        );
        assert!(!curve.flags[14].excluded, "shrinkage is not an exclusion");
        assert!(curve.flags[14].measured, "a measurement did feed the key");
    }

    /// Synthetic profile whose true B deviates from the B_ξ model family —
    /// a ±30 % multiplicative sine on the model B — so the residual from
    /// the fitted prior is genuine and cannot be absorbed by the Eq.-29 fit
    /// (the deviation is orthogonal to the family; L1 keeps the majority
    /// level). Amplitude 0.3 stays below the negative-stretch detector's threshold
    /// (max pair ratio ≈ 2.8 < 4.16 at deep-bass ρ). Only `calculated_b`
    /// is perturbed: engines (a)/(b)/(d) consume B and the partial count,
    /// not the partial frequencies.
    fn synth_profile_deviating(keys: impl Iterator<Item = usize>) -> InharmonicityProfile {
        let mut profile = synth_profile(keys);
        let measured: Vec<u8> = profile.measurements.keys().copied().collect();
        for k in measured {
            let factor = 1.0 + 0.3 * (k as f32 / 6.0).sin();
            let m = profile.active_mut(k).unwrap();
            m.calculated_b = Some(m.calculated_b.unwrap() * factor);
        }
        profile
    }

    /// With data ending at key 40, the curve's deviation from the prior decays
    /// toward the treble instead of extrapolating linearly.
    // report 0007
    #[test]
    fn test_reversion_decays_extrapolation() {
        let profile = synth_profile_deviating(0..=40);
        let input = CurveInput::from_profile(&profile);
        let params = CurveParams::default();
        let a = rigaud_pure(&input, &params);

        for (name, curve) in [
            ("b", per_key_smoothed(&input, &params)),
            (
                "d",
                multi_interval(&input, &params, BALANCED_INTERVALS, None),
            ),
        ] {
            let resid = |k: usize| (curve.cents[k] - a.cents[k]) as f64;
            // The test must have power: a genuine deviation near the data
            // boundary…
            let peak: f64 = (28..=40).map(|k| resid(k).abs()).fold(0.0, f64::max);
            assert!(
                peak > 0.3,
                "engine {name}: no boundary residual to test decay against (peak {peak})"
            );
            // …whose shape has decayed by 3ℓ past it. A constant tail offset is
            // allowed: with A4 unmeasured, re-anchoring A4 shifts the whole curve. A
            // sloped tail is the failure: linear extrapolation gives ≈ −4.3 ¢/octave.
            let tail_ref = resid(87);
            for k in 80..88 {
                assert!(
                    (resid(k) - tail_ref).abs() < 0.15 * peak + 0.03,
                    "engine {name}: tail not flat at key {k}: resid {} vs C8 {tail_ref} \
                     (boundary peak {peak})",
                    resid(k)
                );
            }
            // And the tail offset itself stays bounded by the boundary
            // deviation (no amplification past the data).
            assert!(
                tail_ref.abs() <= peak + 0.05,
                "engine {name}: tail offset {tail_ref} exceeds boundary peak {peak}"
            );
        }
    }

    /// The chain offset is the minimum-norm choice: the residual from the prior
    /// averages to the same value over each chain's measured keys.
    // report 0007
    #[test]
    fn test_chain_gauge_is_mean_centered() {
        let profile = synth_profile_deviating(0..88);
        let input = CurveInput::from_profile(&profile);
        let params = CurveParams::default();
        let (raw, measured) = raw_octave_chain(&input, &params);
        let prior = rigaud_pure(&input, &params);
        // Engine (a)'s finished curve is re-anchored at A4, so raw − (a) carries one
        // global shift, and every chain's mean residual must equal it.
        for c in 0..12usize {
            let keys: Vec<usize> = (c..88).step_by(12).filter(|&k| measured[k]).collect();
            assert!(!keys.is_empty());
            let mean: f64 = keys
                .iter()
                .map(|&k| raw[k] - prior.cents[k] as f64)
                .sum::<f64>()
                / keys.len() as f64;
            let ref_mean: f64 = (0..88)
                .step_by(12)
                .filter(|&k| measured[k])
                .map(|k| raw[k] - prior.cents[k] as f64)
                .sum::<f64>()
                / (0..88).step_by(12).filter(|&k| measured[k]).count() as f64;
            assert!(
                (mean - ref_mean).abs() < 1e-4,
                "chain {c} gauge mean {mean} differs from reference {ref_mean}"
            );
        }
    }

    /// With few keys, engine (b) stays near the prior.
    #[test]
    fn test_few_keys_stay_near_prior() {
        let profile = synth_profile([0usize, 24, 48, 72].into_iter());
        let input = CurveInput::from_profile(&profile);
        let params = CurveParams::default();
        let a = rigaud_pure(&input, &params);
        let b = per_key_smoothed(&input, &params);
        // Synthetic B equals the fitted model, so raw ≈ prior and (b) must
        // track (a) closely everywhere.
        for k in 0..88 {
            assert!(
                (a.cents[k] - b.cents[k]).abs() < 3.0,
                "key {k}: a={} b={}",
                a.cents[k],
                b.cents[k]
            );
        }
    }
}
