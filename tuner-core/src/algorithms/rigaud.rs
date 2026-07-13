//! # Rigaud — the parametric inharmonicity **and tuning** model
//!
//! Rigaud, David & Daudet 2013 [1] jointly model the inharmonicity **and the
//! tuning** of the whole compass: B_ξ(m) below is the inharmonicity half, the
//! octave-type curve ρ_φ(m) the tuning half.
//!
//! Per-instrument modelling primitives from the paper, composed by the
//! tuning-curve engines in [`super::curves`]:
//!
//! * [`erf`] — Abramowitz & Stegun 7.1.26 error function (hand-rolled, no new
//!   dependency; |error| ≤ 1.5e-7).
//! * [`BXi`] / [`fit_b_xi`] — the whole-compass inharmonicity model
//!   B_ξ(m) (paper Eqs. 7–8) and its L1 fit from measured keys (Eq. 29).
//!   The treble-bridge asymptote is fixed universal (paper §III.B.1, after
//!   Young 1952); only the bass pair (s_B, y_B) is per-instrument.
//! * [`RhoPhi`] / [`fit_rho_phi`] — the octave-type curve ρ_φ(m)
//!   (Eq. 9) and its fit from estimated ρ(m) points (Eq. 31).
//! * [`invert_rho`] — octave-type estimation from a tuned octave (Eq. 30).
//! * [`f0_from_partials`] — closed-form F_0 from partial frequencies given
//!   B (Eq. 20, the paper's exact least-squares reduction).
//!
//! **Index convention:** the paper indexes keys by MIDI note number
//! m ∈ [21, 108] (A0 = 21 … C8 = 108). This crate's 88-key index is
//! `key ∈ [0, 87]`, so m = key + 21; use [`midi_from_key`] at call
//! sites. `models::get_expected_beta` carries the same model re-indexed to
//! 1-indexed keys — [`BXi::DEFAULT_MEDIUM`] is that default expressed in the
//! paper's MIDI domain.
//!
//! **Fundamental convention (design note §1):** the stiff-string law
//! f_n = n F_0 √(1 + B n²) uses the *flexible-string* fundamental
//! F_0; the audible first partial is f_1 = F_0√(1+B). Everything in
//! this module is in the F_0 convention.
//!
//! # Reference
//! 1. Rigaud, F., David, B., & Daudet, L. (2013). "A parametric model and
//!    estimation techniques for the inharmonicity and tuning of the piano".
//!    JASA 133(5), pp. 3107–3118. DOI: 10.1121/1.4802644.

/// Converts this crate's 88-key index (0 = A0) to the paper's MIDI note
/// number (A0 = 21, C8 = 108).
pub fn midi_from_key(key: usize) -> f64 {
    key as f64 + 21.0
}

/// Error function, Abramowitz & Stegun *Handbook of Mathematical Functions*
/// formula 7.1.26 (rational polynomial approximation, |error| ≤ 1.5e-7).
///
/// Hand-rolled to keep the crate dependency-free (design note §13 #5).
/// `models.rs` keeps a private `f32` twin so the discovery-side Railsback
/// curve stays bit-identical; the curve layer needs the `f64` precision.
pub fn erf(x: f64) -> f64 {
    // A&S 7.1.26 coefficients.
    const P: f64 = 0.327_591_1;
    const A1: f64 = 0.254_829_592;
    const A2: f64 = -0.284_496_736;
    const A3: f64 = 1.421_413_741;
    const A4: f64 = -1.453_152_027;
    const A5: f64 = 1.061_405_429;

    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + P * x);
    let poly = ((((A5 * t + A4) * t + A3) * t + A2) * t + A1) * t;
    sign * (1.0 - poly * (-x * x).exp())
}

/// Treble-bridge asymptote slope s_T — fixed universal across pianos
/// (Rigaud §III.B.1, L1-fit over 6 pianos; consistent with Young 1952).
pub const S_T: f64 = 9.26e-2;
/// Treble-bridge asymptote intercept y_T — fixed universal (see [`S_T`]).
pub const Y_T: f64 = -13.64;

/// The whole-compass inharmonicity model B_ξ(m) (Rigaud Eqs. 7–8):
/// the sum of the bass- and treble-bridge log-linear asymptotes,
///
/// B_ξ(m) = e^(s_B m + y_B) + e^(s_T m + y_T),
///
/// with the treble pair fixed universal ([`S_T`], [`Y_T`]) and the bass pair
/// (s_B, y_B) the per-instrument free parameters ξ (fit by
/// [`fit_b_xi`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BXi {
    /// Bass-bridge asymptote slope s_B (per MIDI step; negative — bass B
    /// grows toward A0).
    pub s_b: f64,
    /// Bass-bridge asymptote intercept y_B (log scale).
    pub y_b: f64,
}

impl BXi {
    /// The medium-piano default bass pair, i.e. `models::get_expected_beta`
    /// expressed in the MIDI domain (that function uses 1-indexed keys:
    /// -0.066n - 9.211 with n = m - 20 ⇒ s_B = -0.066,
    /// y_B = -7.891). Used only when a profile has no measured keys to fit.
    pub const DEFAULT_MEDIUM: BXi = BXi {
        s_b: -0.066,
        y_b: -7.891,
    };

    /// Evaluates B_ξ(m) at MIDI note number `m` (Eq. 8).
    pub fn b_at_midi(&self, m: f64) -> f64 {
        (self.s_b * m + self.y_b).exp() + (S_T * m + Y_T).exp()
    }

    /// Evaluates B_ξ at an 88-key index (0 = A0).
    pub fn b_at_key(&self, key: usize) -> f64 {
        self.b_at_midi(midi_from_key(key))
    }
}

/// Fits the per-instrument bass pair ξ = (s_B, y_B) by least absolute
/// deviation in log-B (Rigaud Eq. 29):
///
/// ξ̂ = argmin_ξ ∑_{m ∈ M} |log B(m) - log B_ξ(m)|.
///
/// `points` are `(m_midi, b_measured)` pairs from the profile's trusted keys
/// (all of them — the L1 norm is the paper's outlier guard, and treble points
/// carry almost no gradient for the bass pair since the treble asymptote is
/// fixed). Returns `None` with fewer than 2 points or any non-positive `b`.
///
/// The *objective* is the paper's; the *optimizer* is ours: a deterministic
/// coarse-to-fine grid search (the objective is 2-D, piecewise-smooth, and
/// cold-path — no solver dependency is warranted). Search domain
/// s_B ∈ [-0.20, 0], y_B ∈ [-14, -2] brackets every physical piano
/// (typical values: paper init (-0.089, -7); this upright (-0.050, -6.3)).
pub fn fit_b_xi(points: &[(f64, f64)]) -> Option<BXi> {
    if points.len() < 2 || points.iter().any(|&(_, b)| b <= 0.0 || !b.is_finite()) {
        return None;
    }
    let objective = |s_b: f64, y_b: f64| -> f64 {
        let xi = BXi { s_b, y_b };
        points
            .iter()
            .map(|&(m, b)| (b.ln() - xi.b_at_midi(m).ln()).abs())
            .sum()
    };

    // Coarse pass over the full domain, then four zoom rounds.
    let (mut s0, mut s1, mut y0, mut y1) = (-0.20, 0.0, -14.0, -2.0);
    let mut best = (BXi::DEFAULT_MEDIUM, f64::INFINITY);
    for round in 0..5 {
        let steps = if round == 0 { 60 } else { 20 };
        let (ds, dy) = ((s1 - s0) / steps as f64, (y1 - y0) / steps as f64);
        for i in 0..=steps {
            for j in 0..=steps {
                let (s, y) = (s0 + ds * i as f64, y0 + dy * j as f64);
                let e = objective(s, y);
                if e < best.1 {
                    best = (BXi { s_b: s, y_b: y }, e);
                }
            }
        }
        // Zoom to ±1.5 grid steps around the incumbent.
        let (s, y) = (best.0.s_b, best.0.y_b);
        (s0, s1, y0, y1) = (s - 1.5 * ds, s + 1.5 * ds, y - 1.5 * dy, y + 1.5 * dy);
    }
    Some(best.0)
}

/// The octave-type curve ρ_φ(m) (Rigaud Eq. 9):
///
/// ρ_φ(m) = κ/2 · (1 − erf((m − m₀)/α)) + 1,
///
/// an erf-shaped descent from κ + 1 in the low bass to the 2:1
/// asymptote ρ → 1 in the treble (pitch perception rides the first
/// partial above ~F6, paper §II.B.3). φ = {κ, m₀, α}.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RhoPhi {
    /// Bass asymptote height: ρ → κ + 1 toward A0.
    pub kappa: f64,
    /// Translation along the compass (MIDI note number of the descent center).
    pub m0: f64,
    /// Descent slope scale (larger = gentler).
    pub alpha: f64,
}

impl RhoPhi {
    /// The paper's "typical" octave-type parameters (§III.B.3 / Algorithm 1
    /// initialization: κ = 3.5, m_0 = 60, α = 25).
    pub const TYPICAL: RhoPhi = RhoPhi {
        kappa: 3.5,
        m0: 60.0,
        alpha: 25.0,
    };

    /// Evaluates ρ_φ at MIDI note number `m` (Eq. 9). The paper
    /// defines the model for m ∈ [21, 96]; beyond, the erf continues
    /// smoothly onto its asymptotes.
    pub fn rho_at_midi(&self, m: f64) -> f64 {
        self.kappa / 2.0 * (1.0 - erf((m - self.m0) / self.alpha)) + 1.0
    }
}

/// Estimates the octave type ρ(m) realized by a tuned octave pair, by
/// inverting the octave relation — Rigaud Eq. 30:
///
/// ρ(m) = √((4F_0(m)² - F_0(m+12)²)/(F_0(m+12)² B(m+12) - 16 F_0(m)² B(m))).
///
/// Inputs are flexible-string fundamentals (Hz) and inharmonicity
/// coefficients of the lower (`f0_l`, `b_l`) and upper (`f0_u`, `b_u`) notes.
/// Returns `None` when the quantity under the square root is non-positive —
/// the paper's own missing-data case, which happens when the octave is
/// compressed instead of stretched (§IV.C.1).
pub fn invert_rho(f0_l: f64, b_l: f64, f0_u: f64, b_u: f64) -> Option<f64> {
    let (l2, u2) = (f0_l * f0_l, f0_u * f0_u);
    let num = 4.0 * l2 - u2;
    let den = u2 * b_u - 16.0 * l2 * b_l;
    if den == 0.0 {
        return None;
    }
    let rho2 = num / den;
    if rho2 > 0.0 && rho2.is_finite() {
        Some(rho2.sqrt())
    } else {
        None
    }
}

/// Upper edge of [`fit_rho_phi`]'s κ search domain. **Ours**, documented:
/// the grid brackets the paper's Fig. 9 range and both calibrations observed
/// on the real captures. Exported because it bounds the octave types the
/// fit can express (ρ ≤ κ + 1) — [`super::giordano`]'s scan bracket
/// derives its largest admitted coincident pair from it.
pub const RHO_FIT_KAPPA_MAX: f64 = 6.0;

/// Fits φ = {κ, m₀, α} to estimated ρ(m) points by
/// least absolute deviation (Rigaud Eq. 31), plus a quadratic penalty toward
/// a prior parameter set:
///
/// φ̂ = argmin_φ ∑ |ρ_i − ρ_φ(m_i)|
///     + w · [ (κ − κ_p)² + ((m₀ − m₀_p)/12)² + ((α − α_p)/12)² ]
///
/// The L1 term is the paper's (Eq. 31); the penalty is **ours** — the
/// Giordano-calibration composition (design note §6(c)) demands "strong
/// regularization" because the ρ points come from per-octave dissonance
/// scans that are sparse and noisy on a single capture set. `reg_weight` is
/// in ρ-units per normalized-parameter-unit² (m₀ and α normalized
/// by 12 MIDI steps = one octave); the caller documents its value. Pass
/// `reg_weight = 0.0` for the paper's pure Eq. 31.
///
/// `points` are `(m_midi, rho)` pairs. Returns `None` when `points` is empty.
/// Deterministic coarse-to-fine grid search over κ ∈ [0, [`RHO_FIT_KAPPA_MAX`]],
/// m₀ ∈ [21, 108], α ∈ [5, 80] (brackets the paper's Fig. 9
/// range and both calibrations observed on the real captures).
pub fn fit_rho_phi(points: &[(f64, f64)], prior: &RhoPhi, reg_weight: f64) -> Option<RhoPhi> {
    if points.is_empty() {
        return None;
    }
    let objective = |kappa: f64, m0: f64, alpha: f64| -> f64 {
        let phi = RhoPhi { kappa, m0, alpha };
        let l1: f64 = points
            .iter()
            .map(|&(m, r)| (r - phi.rho_at_midi(m)).abs())
            .sum();
        let dk = kappa - prior.kappa;
        let dm = (m0 - prior.m0) / 12.0;
        let da = (alpha - prior.alpha) / 12.0;
        l1 + reg_weight * (dk * dk + dm * dm + da * da)
    };

    let (mut k0, mut k1) = (0.0, RHO_FIT_KAPPA_MAX);
    let (mut m0lo, mut m0hi) = (21.0, 108.0);
    let (mut a0, mut a1) = (5.0, 80.0);
    let mut best = (*prior, f64::INFINITY);
    for round in 0..4 {
        let steps = if round == 0 { 24 } else { 10 };
        let dk = (k1 - k0) / steps as f64;
        let dm = (m0hi - m0lo) / steps as f64;
        let da = (a1 - a0) / steps as f64;
        for i in 0..=steps {
            for j in 0..=steps {
                for l in 0..=steps {
                    let (k, m, a) = (k0 + dk * i as f64, m0lo + dm * j as f64, a0 + da * l as f64);
                    let e = objective(k, m, a);
                    if e < best.1 {
                        best = (
                            RhoPhi {
                                kappa: k,
                                m0: m,
                                alpha: a,
                            },
                            e,
                        );
                    }
                }
            }
        }
        let RhoPhi { kappa, m0, alpha } = best.0;
        (k0, k1) = ((kappa - 1.5 * dk).max(0.0), kappa + 1.5 * dk);
        (m0lo, m0hi) = (m0 - 1.5 * dm, m0 + 1.5 * dm);
        (a0, a1) = ((alpha - 1.5 * da).max(1e-3), alpha + 1.5 * da);
    }
    Some(best.0)
}

/// Closed-form flexible-string fundamental from a partial set given B —
/// Rigaud Eq. 20 (the exact solution of the inharmonicity-constraint
/// least squares, ∂C₁/∂F₀ = 0):
///
/// F₀ = [ ∑ f_n·n·√(1 + B·n²) ] / [ ∑ n²·(1 + B·n²) ]
///
/// `partials` are `(n, f_n_hz)` pairs. Returns `None` on an empty set or a
/// degenerate denominator. This is how curve code derives F_0 from the
/// persisted partial list — `KeyMeasurement::measured_f0` is the Goertzel
/// *seed*, not the refined fundamental (design note §1's convention rule).
pub fn f0_from_partials(partials: &[(u32, f64)], b: f64) -> Option<f64> {
    let mut num = 0.0;
    let mut den = 0.0;
    for &(n, f) in partials {
        if n == 0 || f.is_nan() || f <= 0.0 {
            continue;
        }
        let n = n as f64;
        let stiff = 1.0 + b * n * n;
        num += f * n * stiff.sqrt();
        den += n * n * stiff;
    }
    if den > 0.0 && num > 0.0 {
        Some(num / den)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §11 test: erf against A&S table values, to the approximation's own
    /// 1.5e-7 bound.
    #[test]
    fn test_erf_vs_table() {
        // Reference values (A&S Table 7.1 / standard erf).
        let table = [
            (0.0, 0.0),
            (0.1, 0.112_462_916),
            (0.5, 0.520_499_878),
            (1.0, 0.842_700_793),
            (1.5, 0.966_105_146),
            (2.0, 0.995_322_265),
            (3.0, 0.999_977_910),
        ];
        for &(x, want) in &table {
            assert!(
                (erf(x) - want).abs() < 1.5e-7,
                "erf({x}) = {} want {want}",
                erf(x)
            );
            // Odd symmetry.
            assert!((erf(-x) + want).abs() < 1.5e-7);
        }
    }

    #[test]
    fn test_rho_phi_asymptotes() {
        let phi = RhoPhi::TYPICAL;
        // Deep bass → κ + 1; treble → 1; center → κ/2 + 1. Tolerance is the
        // A&S 7.1.26 approximation bound (1.5e-7), not machine epsilon.
        assert!((phi.rho_at_midi(-400.0) - (phi.kappa + 1.0)).abs() < 1.5e-7);
        assert!((phi.rho_at_midi(500.0) - 1.0).abs() < 1.5e-7);
        assert!((phi.rho_at_midi(phi.m0) - (phi.kappa / 2.0 + 1.0)).abs() < 1.5e-7);
    }

    /// Eq. 30 must exactly invert Eq. 6: build the octave with a known ρ,
    /// then recover it.
    #[test]
    fn test_invert_rho_round_trip() {
        let cases: [(f64, f64, f64); 4] = [
            (1e-4, 2e-4, 1.0),
            (5e-4, 8e-4, 2.0),
            (1.4e-3, 2.0e-3, 4.4),
            (6.7e-4, 9.0e-4, 3.0),
        ];
        for &(b_l, b_u, rho) in &cases {
            let f0_l = 110.0;
            // Eq. 6.
            let f0_u =
                2.0 * f0_l * ((1.0 + 4.0 * rho * rho * b_l) / (1.0 + rho * rho * b_u)).sqrt();
            let got = invert_rho(f0_l, b_l, f0_u, b_u).expect("stretched octave inverts");
            assert!(
                (got - rho).abs() < 1e-9,
                "recovered {got} want {rho} (b_l={b_l}, b_u={b_u})"
            );
        }
        // A compressed octave (ratio < 2 with these B's) has no real ρ.
        assert_eq!(invert_rho(110.0, 1e-4, 219.5, 2e-4), None);
    }

    /// Eq. 29 fit recovers a known bass pair from synthetic B(m) samples.
    #[test]
    fn test_fit_b_xi_recovery() {
        let truth = BXi {
            s_b: -0.0503,
            y_b: -6.293,
        };
        let points: Vec<(f64, f64)> = (21..=90)
            .step_by(3)
            .map(|m| (m as f64, truth.b_at_midi(m as f64)))
            .collect();
        let fit = fit_b_xi(&points).expect("fit succeeds");
        assert!(
            (fit.s_b - truth.s_b).abs() < 1e-3,
            "s_b {} want {}",
            fit.s_b,
            truth.s_b
        );
        assert!(
            (fit.y_b - truth.y_b).abs() < 5e-2,
            "y_b {} want {}",
            fit.y_b,
            truth.y_b
        );
        // The fitted curve itself must match to well under measurement noise.
        for m in 21..=108 {
            let (a, b) = (fit.b_at_midi(m as f64), truth.b_at_midi(m as f64));
            assert!((a.ln() - b.ln()).abs() < 0.01, "B mismatch at m={m}");
        }
    }

    /// Eq. 31 fit recovers known φ from clean ρ points (no regularization).
    #[test]
    fn test_fit_rho_phi_recovery() {
        let truth = RhoPhi {
            kappa: 4.1,
            m0: 42.0,
            alpha: 30.0,
        };
        let points: Vec<(f64, f64)> = (21..=96)
            .step_by(2)
            .map(|m| (m as f64, truth.rho_at_midi(m as f64)))
            .collect();
        let fit = fit_rho_phi(&points, &RhoPhi::TYPICAL, 0.0).expect("fit succeeds");
        for m in (21..=96).step_by(5) {
            let (a, b) = (fit.rho_at_midi(m as f64), truth.rho_at_midi(m as f64));
            assert!((a - b).abs() < 0.05, "rho mismatch at m={m}: {a} vs {b}");
        }
    }

    /// Eq. 20 recovers F0 exactly from an exact stiff-string partial series.
    #[test]
    fn test_f0_from_partials_round_trip() {
        let (f0, b) = (55.3, 6.7e-4);
        let partials: Vec<(u32, f64)> = (1..=30)
            .map(|n| {
                let nf = n as f64;
                (n, nf * f0 * (1.0 + b * nf * nf).sqrt())
            })
            .collect();
        let got = f0_from_partials(&partials, b).expect("f0 recovers");
        assert!((got - f0).abs() < 1e-9, "got {got} want {f0}");
        assert_eq!(f0_from_partials(&[], b), None);
    }
}
