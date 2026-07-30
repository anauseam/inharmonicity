//! # Giordano — the sensory-dissonance octave-width recipe
//!
//! The *quantity* computed here is Plomp–Levelt sensory dissonance [2] in the
//! Sethares parametrization [3]; the *recipe* — scanning an interval for its
//! dissonance-minimal width from measured inharmonic partials — is Giordano's [1].
//!
//! Faithful implementation of Giordano's recipe [1] for locating the
//! dissonance-minimal width of an interval from *measured* partials
//! (frequencies **and** amplitudes): Plomp–Levelt pure-tone roughness [2] in
//! the Sethares parametrization [3] (Giordano's Eqs. 3–6),
//!
//! d₂(Δf, f_min) = e^(−b₁·s·Δf) − e^(−b₂·s·Δf),  with  s = x* / (s₁·f_min + s₂),
//!
//! summed over the cross partial pairs of the two notes weighted by the
//! amplitude product B_{ij} = a_i a_j (his bass-favored variant; design
//! note defaults #13.1), with each note normalized to equal total power.
//! The interval is scanned by rigidly shifting the upper note's measured
//! series — partial n moves by n df, Giordano's shift rule — over the
//! **coincidence bracket** (the hull of the pair's beatless 2j:j widths;
//! see [`octave_scan`]) and taking the dissonance-minimum offset.
//!
//! This is the perceptual layer of the tuning-curve design (§3.2): it runs
//! **offline on stable-capture partials only**, never in the live loop, and
//! its output feeds the octave-type calibration of engine (c) in
//! [`super::curves`]. Scan-edge minima and starved treble spectra are
//! excluded by the sufficiency gate (defaults #13.2) — with 3–6 partials the
//! dissonance well is shallow or absent, an information floor of the source,
//! not a capture flaw (§3.2, §8).
//!
//! # References
//! 1. N. Giordano, "Explaining the Railsback stretch in terms of the
//!    inharmonicity of piano tones and sensory dissonance", JASA
//!    138(4):2359–2366 (2015). DOI: 10.1121/1.4931439.
//! 2. R. Plomp, W. J. M. Levelt, "Tonal consonance and critical bandwidth",
//!    JASA 38:548–560 (1965).
//! 3. W. A. Sethares, "Local consonance and the relationship between timbre
//!    and scale", JASA 94(3):1218–1228 (1993).

/// Sethares/Giordano roughness constants (Giordano Eqs. 3–4 and the
/// accompanying text; originally Sethares 1993 Eqs. 1–4, where d* = 0.24
/// derives from the Eq-1 fit and s₁/s₂ from his least-squares interpolation).
/// Giordano's values, adopted verbatim (design note defaults #13.1); Gràcia &
/// Sanz-Perela's min-loudness/b_2=5.7 variant noted there and not used.
pub const B1: f64 = 3.5;
/// See [`B1`].
pub const B2: f64 = 5.75;
/// Peak-position normalizer x^* in s = x^*/(s_1 f_{min} + s_2).
pub const X_STAR: f64 = 0.24;
/// Critical-bandwidth slope s_1.
pub const S1: f64 = 0.021;
/// Critical-bandwidth intercept s_2.
pub const S2: f64 = 19.0;

/// Plomp–Levelt pure-tone dissonance of two sine components at `f_a`, `f_b`
/// Hz (Giordano Eqs. 3–4; unit peak, dimensionless). Zero at unison,
/// maximal near a quarter critical bandwidth, asymptotically zero for wide
/// separation.
pub fn pure_tone_dissonance(f_a: f64, f_b: f64) -> f64 {
    let f_min = f_a.min(f_b);
    let df = (f_a - f_b).abs();
    let s = X_STAR / (S1 * f_min + S2);
    (-B1 * s * df).exp() - (-B2 * s * df).exp()
}

/// Scales a partial list's amplitudes to unit total power
/// (∑ a_i² = 1) — Giordano's equal-total-power note normalization.
/// Entries are `(frequency_hz, amplitude)`. Returns `None` for an empty or
/// zero-power list.
fn normalize_power(partials: &[(f64, f64)]) -> Option<Vec<(f64, f64)>> {
    let power: f64 = partials.iter().map(|&(_, a)| a * a).sum();
    if power.is_nan() || power <= 0.0 {
        return None;
    }
    let scale = power.sqrt().recip();
    Some(partials.iter().map(|&(f, a)| (f, a * scale)).collect())
}

/// Total sensory dissonance between two notes' partial lists
/// (`(frequency_hz, amplitude)` pairs): amplitude-product-weighted
/// Plomp–Levelt roughness summed over all **cross** partial pairs
/// (Giordano Eqs. 5–6, the "amplitude product" model), each note
/// normalized to equal total power (his §VI.C).
///
/// Cross pairs only is **Giordano's own construction**: his Eq. 5 sums tone
/// 1's partials against tone 2's and explicitly omits self-dissonance as
/// immaterial to locating a two-tone minimum (the intra-note terms exist
/// only in Sethares' fuller 1993 Eq.-6 form). The design note (§3.2) makes
/// the same argument quantitatively: intra-note terms are (near-)invariant
/// under the interval scan. Giordano's ½ prefactor is dropped — a constant
/// scale, inert for the argmin and for downstream weight ratios.
pub fn dissonance(lower: &[(f64, f64)], upper: &[(f64, f64)]) -> f64 {
    let (Some(lo), Some(up)) = (normalize_power(lower), normalize_power(upper)) else {
        return 0.0;
    };
    let mut d = 0.0;
    for &(fl, al) in &lo {
        for &(fu, au) in &up {
            d += al * au * pure_tone_dissonance(fl, fu);
        }
    }
    d
}

/// Scan step. 0.5 ¢ resolves the dissonance well far below the per-key
/// measurement noise (§4: raw-B scatter ⇒ multi-cent octave-step noise).
pub const SCAN_STEP_CENTS: f64 = 0.5;

/// Margin added on both sides of the coincidence bracket, in cents.
/// **Ours**, documented — the scan window's one remaining soft constant:
/// the dissonance argmin is a compromise *inside* the hull of the
/// coincident pairs' beatless widths, but non-coincident cross terms and
/// per-key B measurement noise can perturb it slightly past the hull edge;
/// 10 ¢ is comfortable headroom for both (the raw-B scatter's multi-cent
/// octave-step noise, §4). A low-edge hit is compression evidence — the
/// optimum at or below the ρ = 1 floor (§2's theorem says a real optimum
/// cannot be there) — and a high-edge hit means the well is past every
/// admissible octave type; the caller excludes both from the ρ fit.
pub const SCAN_MARGIN_CENTS: f64 = 10.0;

/// Largest coincident pair 2j:j admitted to the scan bracket: j ≤ 7,
/// **derived** from the downstream consumer's own domain. The scan exists
/// to produce ρ points for the Eq.-9 fit, whose κ search domain
/// ([`super::rigaud::RHO_FIT_KAPPA_MAX`] = 6) can express octave
/// types up to ρ = κ + 1 = 7. Beatless widths of pairs beyond j = 7 imply
/// octave types the model cannot represent — and lie out where the
/// Plomp–Levelt terms decouple (d₂ → 0 for wide separation), a regime
/// whose shallow minima are interval-identity artifacts, not octave
/// optima. Deep-bass notes carry pairs to j ≈ 15 whose beatless widths
/// exceed +400 ¢; admitting them would invite exactly those artifacts.
fn max_pair_rank() -> u32 {
    super::rigaud::RHO_FIT_KAPPA_MAX as u32 + 1
}

/// Result of [`octave_scan`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OctaveScan {
    /// Dissonance-minimal offset of the upper note from its **measured**
    /// position, in cents (positive = retune upward). Relative to the
    /// note as captured — not to the pure ET octave; the caller applies it
    /// multiplicatively to the measured F₀ (both conventions scale
    /// together, design note §1).
    pub offset_cents: f64,
    /// `true` when the minimum is interior to the scan bracket. An edge hit
    /// means no dissonance well exists inside the admissible octave-type
    /// width range — compression evidence at the low edge, a starved or
    /// decoupled spectrum at the high edge (design note §2, defaults
    /// #13.2); the caller excludes the pair from the ρ fit either way.
    pub interior: bool,
    /// The dissonance value at the minimum (diagnostic only).
    pub dissonance_min: f64,
}

/// Giordano's per-octave 1-D scan: shifts the upper note's measured partial
/// series (partial of rank n moves by n df — his §VI.C shift rule, the
/// first-order retuning of a stiff string) across the **coincidence
/// bracket** and returns the dissonance-minimum offset.
///
/// **Scan window (coincidence bracket).** The interval-width axis is
/// scanned over [min, max] of the pair's beatless widths — the widths at
/// which the coincident pairs 2j:j present in both measured lists beat out
/// (pair 2j:j is beatless exactly at the Eq.-6 width with ρ = j;
/// `curves::interval_width_cents(b_l, b_u, 2j, j, 12)`), j capped by
/// [`max_pair_rank`] — extended by [`SCAN_MARGIN_CENTS`] on both sides.
/// Widths are computed from the pair's measured B, so the window is
/// **mistuning-independent** (the notes' current tuning never enters) and
/// register-adaptive by construction: tight around the 2:1 width in the
/// pair-starved treble, wide enough for high octave types in the bass.
///
/// `lower` / `upper` are `(n, frequency_hz, amplitude)` triples of measured
/// partials; `fb_l` / `fb_u` are the notes' `(F0_hz, B)` (flexible-string
/// convention, design note §1) — B fixes the bracket, F0 locates the
/// current width inside it. Returns `None` when either list is empty or
/// carries no power, when no coincident pair exists, or on non-physical
/// `(F0, B)`. Callers apply the sufficiency gate ([`coincident_pairs`]
/// ≥ 8 and `interior`) before trusting the result.
pub fn octave_scan(
    lower: &[(u32, f64, f64)],
    upper: &[(u32, f64, f64)],
    fb_l: (f64, f64),
    fb_u: (f64, f64),
) -> Option<OctaveScan> {
    let (f0_l, b_l) = fb_l;
    let (f0_u, b_u) = fb_u;
    if lower.is_empty() || upper.is_empty() {
        return None;
    }
    if !(f0_l.is_finite() && f0_l > 0.0 && f0_u.is_finite() && f0_u > 0.0)
        || !(b_l.is_finite() && b_l >= 0.0 && b_u.is_finite() && b_u >= 0.0)
    {
        return None;
    }
    let lo: Vec<(f64, f64)> = lower.iter().map(|&(_, f, a)| (f, a)).collect();

    // Coincidence bracket: beatless widths of the 2j:j pairs present in
    // both measured lists (deviation from the ET octave, audible-f₁ space).
    let has_rank = |note: &[(u32, f64, f64)], n: u32| note.iter().any(|&(r, _, _)| r == n);
    let mut w_min = f64::INFINITY;
    let mut w_max = f64::NEG_INFINITY;
    for j in 1..=max_pair_rank() {
        if has_rank(lower, 2 * j) && has_rank(upper, j) {
            let w = super::curves::interval_width_cents(b_l, b_u, 2 * j, j, 12);
            w_min = w_min.min(w);
            w_max = w_max.max(w);
        }
    }
    if !w_min.is_finite() {
        return None; // no coincident pair — nothing brackets an octave optimum
    }

    // Current width (audible-f₁ deviation from the ET octave). Retuning the
    // upper note by c cents moves the width to exactly w_now + c, so the
    // bracket maps to offsets [w_min − margin − w_now, w_max + margin − w_now].
    let f1_l = f0_l * (1.0 + b_l).sqrt();
    let f1_u = f0_u * (1.0 + b_u).sqrt();
    let w_now = 1200.0 * (f1_u / f1_l).log2() - 1200.0;
    let c_lo = w_min - SCAN_MARGIN_CENTS - w_now;
    let c_hi = w_max + SCAN_MARGIN_CENTS - w_now;

    let steps = (((c_hi - c_lo) / SCAN_STEP_CENTS).ceil() as usize).max(1);
    let mut best: Option<(usize, f64)> = None;
    let mut shifted: Vec<(f64, f64)> = Vec::with_capacity(upper.len());
    for s in 0..=steps {
        let c = c_lo + s as f64 * SCAN_STEP_CENTS;
        let df = f1_u * ((c / 1200.0).exp2() - 1.0);
        shifted.clear();
        shifted.extend(upper.iter().map(|&(n, f, a)| (f + n as f64 * df, a)));
        let d = dissonance(&lo, &shifted);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((s, d));
        }
    }
    let (s_min, d_min) = best?;
    Some(OctaveScan {
        offset_cents: c_lo + s_min as f64 * SCAN_STEP_CENTS,
        interior: s_min != 0 && s_min != steps,
        dissonance_min: d_min,
    })
}

/// Counts the coincident 2j:j pairs available to an octave scan — pairs
/// where the lower note's partial 2j and the upper note's partial j are
/// both present in the measured lists. The sufficiency gate's input
/// (design note defaults #13.2, ≥ 8 required), **derived from Giordano
/// §VI.C** (verified against the PDF): reaching the asymptotic stretch for
/// the A0–A1 / A1–A2 pairs "requires at least 16 partials of the lower
/// note and 8 of the higher member", i.e. min(⌊16/2⌋, 8) = 8 coincident
/// pairs; fewer "gives a significantly smaller predicted stretch" — a
/// biased ρ point. His A2–A3 case converges with 6/3 (= 3 pairs); the
/// bass-derived 8 is adopted compass-wide as the conservative floor
/// (mid-register captures carry ≥ ~14 partials, so the stricter bound
/// costs nothing there and correctly starves the 3–6-partial treble).
pub fn coincident_pairs(lower: &[(u32, f64, f64)], upper: &[(u32, f64, f64)]) -> usize {
    let has_rank = |note: &[(u32, f64, f64)], n: u32| note.iter().any(|&(r, _, _)| r == n);
    let j_max = upper.iter().map(|&(r, _, _)| r).max().unwrap_or(0);
    (1..=j_max)
        .filter(|&j| has_rank(lower, 2 * j) && has_rank(upper, j))
        .count()
}

/// First-order sensitivity of the Giordano dissonance functional to one
/// cent of width error at a coincident partial pair — the design note's
/// bridge Form 2, **derived** (§6(d)): near coincidence the pair's
/// roughness term is a_p·a_q·d₂(Δf) with
///
/// d₂(Δf) = e^(−b₁·s·Δf) − e^(−b₂·s·Δf) ≈ (b₂ − b₁)·s·Δf
///
/// (d₂ is linear at the origin), and a width error of ε cents separates
/// the pair by Δf ≈ f̄·(ln 2/1200)·ε, so
///
/// ∂D/∂ε = a_p·a_q·(b₂ − b₁)·s(f̄)·f̄·ln 2/1200,  s(f̄) = x*/(s₁·f̄ + s₂).
///
/// Every symbol is a published Giordano/Sethares constant
/// ([`B1`]…[`S2`]) — zero new free parameters. `a_p`/`a_q` are the two
/// partials' amplitudes under the equal-total-power note normalization
/// (Giordano's, [`dissonance`]); `f_bar` is the pair's coincidence
/// frequency in Hz. The linear regime comfortably covers the tempered
/// targets too (b₁·s·Δf ≈ 0.02 at 2 ¢ / 500 Hz). Consumed by
/// `curves::multi_interval` as the derived interval weight W_{m,k}.
pub fn pair_width_sensitivity(f_bar: f64, a_p: f64, a_q: f64) -> f64 {
    let s = X_STAR / (S1 * f_bar + S2);
    a_p * a_q * (B2 - B1) * s * f_bar * core::f64::consts::LN_2 / 1200.0
}

/// Cross partial pairs whose members are both strictly above their own
/// note's median partial amplitude.
///
/// **Diagnostic only — do not gate on this.** The product over-counts: it
/// passes 7×7-partial pairs whose coincident-pair count Giordano's §VI.C
/// convergence analysis rejects. The sufficiency gate uses
/// [`coincident_pairs`]; this count exists for harness reporting.
pub fn strong_cross_pairs(lower: &[(u32, f64, f64)], upper: &[(u32, f64, f64)]) -> usize {
    fn above_median(note: &[(u32, f64, f64)]) -> usize {
        if note.is_empty() {
            return 0;
        }
        let mut amps: Vec<f64> = note.iter().map(|&(_, _, a)| a).collect();
        amps.sort_by(|a, b| a.total_cmp(b));
        let mid = amps.len() / 2;
        let median = if amps.len() % 2 == 1 {
            amps[mid]
        } else {
            0.5 * (amps[mid - 1] + amps[mid])
        };
        note.iter().filter(|&&(_, _, a)| a > median).count()
    }
    above_median(lower) * above_median(upper)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic stiff-string note: `(n, f_n, a_n)` with geometric amplitude
    /// decay.
    fn note(f0: f64, b: f64, n_partials: u32) -> Vec<(u32, f64, f64)> {
        (1..=n_partials)
            .map(|n| {
                let nf = n as f64;
                (
                    n,
                    nf * f0 * (1.0 + b * nf * nf).sqrt(),
                    0.85f64.powi(n as i32 - 1),
                )
            })
            .collect()
    }

    #[test]
    fn test_d2_shape() {
        assert_eq!(pure_tone_dissonance(440.0, 440.0), 0.0);
        assert!(pure_tone_dissonance(440.0, 450.0) > 0.0);
        // Vanishes for wide separation.
        assert!(pure_tone_dissonance(440.0, 2000.0) < 1e-3);
        // Symmetric.
        assert!(
            (pure_tone_dissonance(440.0, 452.0) - pure_tone_dissonance(452.0, 440.0)).abs() < 1e-15
        );
    }

    /// Equal-power normalization makes the objective invariant to overall
    /// note level (strike strength).
    #[test]
    fn test_dissonance_level_invariant() {
        let a: Vec<(f64, f64)> = note(220.0, 1e-4, 10)
            .iter()
            .map(|&(_, f, m)| (f, m))
            .collect();
        let b: Vec<(f64, f64)> = note(440.0, 2e-4, 10)
            .iter()
            .map(|&(_, f, m)| (f, m))
            .collect();
        let b_loud: Vec<(f64, f64)> = b.iter().map(|&(f, m)| (f, 7.5 * m)).collect();
        let d1 = dissonance(&a, &b);
        let d2_ = dissonance(&a, &b_loud);
        assert!((d1 - d2_).abs() < 1e-12 * (1.0 + d1));
    }

    /// A harmonic (B = 0) octave is dissonance-minimal at the pure 2:1 —
    /// offset 0 within one scan step.
    #[test]
    fn test_harmonic_octave_optimum_at_zero() {
        let lower = note(220.0, 0.0, 12);
        let upper = note(440.0, 0.0, 12);
        let scan = octave_scan(&lower, &upper, (220.0, 0.0), (440.0, 0.0)).expect("scan");
        assert!(scan.interior, "harmonic optimum must be interior");
        assert!(
            scan.offset_cents.abs() <= SCAN_STEP_CENTS,
            "harmonic octave optimum at {} ¢",
            scan.offset_cents
        );
    }

    /// Stiff strings (B > 0) push the optimum strictly wide — the §2
    /// stretched-octave theorem seen through the perceptual objective.
    #[test]
    fn test_stiff_octave_optimum_is_stretched() {
        let b = 1.0e-3;
        let lower = note(110.0, b, 20);
        let upper = note(220.0, 1.5e-3, 16);
        let scan = octave_scan(&lower, &upper, (110.0, b), (220.0, 1.5e-3)).expect("scan");
        assert!(scan.interior, "expected interior minimum");
        assert!(
            scan.offset_cents > 0.0 && scan.offset_cents < 40.0,
            "stiff optimum at {} ¢",
            scan.offset_cents
        );
    }

    /// Coincidence-bracket property: the scan is mistuning-independent.
    /// Detuning the upper note by +30 ¢ (exact multiplicative retuning)
    /// must move the reported offset by −30 ¢ — the same absolute optimal
    /// width. Equality is approximate because the n·df shift rule is the
    /// first-order retuning (error O(detune · Bn²): the n·df ladder and
    /// the multiplicative retuning disagree by (γ−1)·(f_n − n·f₁) ≈ 1 Hz
    /// at the optimum-setting mid pairs here), so the tolerance is a
    /// couple of cents, not one grid step.
    #[test]
    fn test_scan_mistuning_independent() {
        let (b_l, b_u) = (1.0e-3, 1.5e-3);
        let lower = note(110.0, b_l, 20);
        let upper = note(220.0, b_u, 16);
        let base = octave_scan(&lower, &upper, (110.0, b_l), (220.0, b_u)).expect("scan");
        let gamma = (30.0f64 / 1200.0).exp2();
        let upper_det: Vec<(u32, f64, f64)> =
            upper.iter().map(|&(n, f, a)| (n, f * gamma, a)).collect();
        let det =
            octave_scan(&lower, &upper_det, (110.0, b_l), (220.0 * gamma, b_u)).expect("scan");
        assert!(det.interior, "detuned optimum must stay interior");
        assert!(
            ((base.offset_cents - det.offset_cents) - 30.0).abs() <= 2.0,
            "absolute width not preserved: in-tune offset {} ¢, detuned offset {} ¢",
            base.offset_cents,
            det.offset_cents
        );
    }

    /// No coincident pair (lower note lacks every even-rank partial the
    /// upper's ranks would need) ⇒ nothing brackets an octave optimum.
    #[test]
    fn test_scan_requires_coincident_pair() {
        let lower = note(1760.0, 5e-3, 1);
        let upper = note(3520.0, 8e-3, 3);
        assert_eq!(coincident_pairs(&lower, &upper), 0);
        assert_eq!(
            octave_scan(&lower, &upper, (1760.0, 5e-3), (3520.0, 8e-3)),
            None
        );
    }

    /// Form-2 derivation check: the closed-form ∂D/∂ε matches the finite
    /// difference of the actual pair term a_p·a_q·d₂ under a width error
    /// of ε cents, across registers.
    #[test]
    fn test_pair_width_sensitivity_matches_finite_difference() {
        for &(f_bar, a_p, a_q) in &[(55.0, 0.4, 0.3), (500.0, 0.3, 0.2), (3000.0, 0.15, 0.1)] {
            let sens = pair_width_sensitivity(f_bar, a_p, a_q);
            let eps = 0.01f64; // cents
            let df = f_bar * ((eps / 1200.0).exp2() - 1.0);
            let fd = a_p * a_q * pure_tone_dissonance(f_bar, f_bar + df) / eps;
            assert!(
                (sens - fd).abs() < 1e-3 * fd,
                "f̄={f_bar}: closed form {sens} vs finite difference {fd}"
            );
            // Linear regime still holds at the tempered targets' ~2 ¢ scale.
            let eps2 = 2.0f64;
            let df2 = f_bar * ((eps2 / 1200.0).exp2() - 1.0);
            let fd2 = a_p * a_q * pure_tone_dissonance(f_bar, f_bar + df2) / eps2;
            assert!(
                (sens - fd2).abs() < 0.05 * fd2,
                "f̄={f_bar}: nonlinearity at 2 ¢: {sens} vs {fd2}"
            );
        }
    }

    /// The §VI.C sufficiency gate excludes information-starved treble
    /// spectra: 3-partial notes carry a single coincident pair (2:1).
    #[test]
    fn test_sufficiency_gate_starves_treble() {
        let lower = note(1760.0, 5e-3, 3);
        let upper = note(3520.0, 8e-3, 3);
        assert_eq!(coincident_pairs(&lower, &upper), 1);
        // A bass pair passes with margin: min(⌊30/2⌋, 30) = 15 ≥ 8.
        let bl = note(27.5, 6.7e-4, 30);
        let bu = note(55.0, 5e-4, 30);
        assert_eq!(coincident_pairs(&bl, &bu), 15);
    }
}
