//! # Repeat-capture noise decomposition harness (handoff Prompt H)
//!
//! Consumes a **per-capture** regenerated-partials JSON (the output of
//! `examples/regenerate_partials` on a diagnostics tree holding repeat
//! captures — multiple timestamped dumps per key) and measures the
//! capture-to-capture variance of every quantity the curve layer consumes:
//!
//! 1. **ρ-point reproducibility** (Prompt H analysis 2): for each octave
//!    pair, every (lower-capture × upper-capture) combination is pushed
//!    through the exact engine-(c) calibration path — §VI.C gate →
//!    coincidence-bracket scan → Eq.-30 inversion — giving per-pair σ of
//!    the optimal width and of ρ, plus the inversion's local conditioning
//!    |∂ρ/∂w| (central difference of `invert_rho` under a ±1 ¢ width
//!    perturbation of the upper note) so σ_ρ ≈ |∂ρ/∂w|·σ_w can be checked
//!    against the observed σ_ρ.
//! 2. **Strike-strength sensitivity** (analysis 3): per pair, the optimal
//!    width is regressed on the combo's summed partial power (dB) — the
//!    slope is the Giordano optimum's amplitude-condition dependence the
//!    design note §3.2 left unquantified.
//! 3. **Resampled curve draws** (analysis 4): R deterministic pseudo-random
//!    draws of one capture per key; per draw the raw octave chain, the
//!    per-draw ρ-fit φ, and engines (b), (c), (d)-BALANCED and
//!    (d)-octaves-only are computed. The per-draw outputs let the offline
//!    post-processor measure chain-noise correlation across keys (the
//!    LOO-CV independence question deferred from Prompt G) and per-engine
//!    curve variance.
//!
//! Output: one machine-readable JSON on stdout (post-processed by the
//! Prompt-H analysis scripts). Diagnostics, not selection — the repeat set
//! is validation data (n = 1 instrument at a time).
//!
//! Usage:
//!   cargo run --release --example regenerate_partials -- diagnostics > p2.json
//!   cargo run --release --example repeat_noise -- p2.json > repeat_report.json

use std::collections::BTreeMap;

use tuner_core::algorithms::curves::{self, BALANCED_INTERVALS, CurveParams, IntervalSpec};
use tuner_core::algorithms::{giordano, rigaud};
use tuner_core::models::{CurveInput, CurveKeyData};

/// Engine (d) restricted to the octave family — the Prompt-G deferred
/// "chain-noise vs LOO independence" comparison line: same interval
/// evidence as engine (b)'s chains, different estimator (joint LS vs
/// chain + smoother).
const OCTAVES_ONLY: &[IntervalSpec] = &[
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
];

/// One capture of one key, in engine terms plus the strength proxy.
struct Capture {
    data: CurveKeyData,
    /// Summed squared partial amplitude, dB (arbitrary reference) — the
    /// strike-strength proxy. Equal-power normalization inside the
    /// dissonance engine removes the absolute level; what varies with
    /// strike strength is the spectral *balance*, and this scalar tags the
    /// combos so the regression can see it.
    power_db: f64,
    source_dir: String,
}

fn load_captures(path: &str) -> BTreeMap<usize, Vec<Capture>> {
    let text = std::fs::read_to_string(path).expect("read partials JSON");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parse JSON");
    let mut keys: BTreeMap<usize, Vec<Capture>> = BTreeMap::new();
    for e in &entries {
        let key = e["key_index"].as_u64().expect("key_index") as usize;
        if key >= 88 {
            continue;
        }
        // Mirror CurveInput::build's trust rules: B finite and positive,
        // ≥ 2 usable partials, Eq.-20 F₀ solvable.
        let Some(b) = e["calculated_b"]
            .as_f64()
            .filter(|b| b.is_finite() && *b > 0.0)
        else {
            continue;
        };
        let partials: Vec<(u32, f64, f64)> = e["partials"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|p| {
                        let n = p["number"].as_u64()? as u32;
                        let f = p["frequency"].as_f64()?;
                        let a = p["amplitude"].as_f64()?;
                        (n > 0 && f > 0.0).then_some((n, f, a))
                    })
                    .collect()
            })
            .unwrap_or_default();
        if partials.len() < 2 {
            continue;
        }
        let pairs: Vec<(u32, f64)> = partials.iter().map(|&(n, f, _)| (n, f)).collect();
        let Some(f0) = rigaud::f0_from_partials(&pairs, b) else {
            continue;
        };
        let power: f64 = partials.iter().map(|&(_, _, a)| a * a).sum();
        keys.entry(key).or_default().push(Capture {
            data: CurveKeyData { b, f0, partials },
            power_db: 10.0 * power.log10(),
            source_dir: e["source_dir"].as_str().unwrap_or("").to_string(),
        });
    }
    keys
}

/// Optimal octave *width* (audible-f₁ deviation from the pure ET octave,
/// cents) for one capture combo, via the engine-(c) path. Returns the
/// width, the implied ρ (`None` when Eq. 30 has no real root), and the
/// scan's interior flag.
fn scanned_width(lo: &CurveKeyData, up: &CurveKeyData) -> Option<(f64, Option<f64>, bool)> {
    let scan = giordano::octave_scan(&lo.partials, &up.partials, (lo.f0, lo.b), (up.f0, up.b))?;
    let f1_l = lo.f0 * (1.0 + lo.b).sqrt();
    let f1_u = up.f0 * (1.0 + up.b).sqrt();
    let w_now = 1200.0 * (f1_u / f1_l).log2() - 1200.0;
    let width = w_now + scan.offset_cents;
    let f0_u_star = up.f0 * (scan.offset_cents / 1200.0).exp2();
    let rho = rigaud::invert_rho(lo.f0, lo.b, f0_u_star, up.b);
    Some((width, rho, scan.interior))
}

/// |∂ρ/∂w| at the accepted width (ρ per cent of width), by central
/// difference of the Eq.-30 inversion under a ±1 ¢ multiplicative
/// perturbation of the upper note — the inversion's local conditioning.
fn rho_conditioning(lo: &CurveKeyData, up_f0_star: f64, up_b: f64) -> Option<f64> {
    let eps: f64 = 1.0; // cents
    let hi = rigaud::invert_rho(lo.f0, lo.b, up_f0_star * (eps / 1200.0).exp2(), up_b)?;
    let lo_r = rigaud::invert_rho(lo.f0, lo.b, up_f0_star * (-eps / 1200.0f64).exp2(), up_b)?;
    Some(((hi - lo_r) / (2.0 * eps)).abs())
}

fn mean_sd(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    let mu = v.iter().sum::<f64>() / n;
    if v.len() < 2 {
        return (mu, f64::NAN);
    }
    let var = v.iter().map(|x| (x - mu) * (x - mu)).sum::<f64>() / (n - 1.0);
    (mu, var.sqrt())
}

/// OLS slope of y on x (per-pair strike-strength regression).
fn slope(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len() as f64;
    let mx = x.iter().sum::<f64>() / n;
    let my = y.iter().sum::<f64>() / n;
    let sxy: f64 = x.iter().zip(y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sxx: f64 = x.iter().map(|a| (a - mx) * (a - mx)).sum();
    if sxx > 0.0 { sxy / sxx } else { f64::NAN }
}

/// xorshift64* — deterministic draw sequence, no rand dependency.
struct Rng(u64);
impl Rng {
    fn next(&mut self, bound: usize) -> usize {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        (x.wrapping_mul(0x2545F4914F6CDD1D) >> 33) as usize % bound
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .expect("usage: repeat_noise <per-capture partials.json>");
    let keys = load_captures(path);
    let params = CurveParams::default();

    // ── 1+2: per-octave-pair combo sweep (ρ reproducibility + strength) ──
    let mut pair_reports: Vec<serde_json::Value> = Vec::new();
    for m in 0..76usize {
        let (Some(lo_caps), Some(up_caps)) = (keys.get(&m), keys.get(&(m + 12))) else {
            continue;
        };
        let mut widths = Vec::new();
        let mut rhos = Vec::new();
        let mut strengths = Vec::new(); // combo power sum, dB
        let mut conds = Vec::new();
        let (mut gated, mut edge, mut no_root) = (0usize, 0usize, 0usize);
        for lo in lo_caps {
            for up in up_caps {
                if giordano::coincident_pairs(&lo.data.partials, &up.data.partials)
                    < curves::GIORDANO_MIN_COINCIDENT_PAIRS
                {
                    gated += 1;
                    continue;
                }
                let Some((width, rho, interior)) = scanned_width(&lo.data, &up.data) else {
                    gated += 1;
                    continue;
                };
                if !interior {
                    edge += 1;
                    continue;
                }
                let Some(rho) = rho else {
                    no_root += 1;
                    continue;
                };
                widths.push(width);
                rhos.push(rho);
                strengths.push(lo.power_db + up.power_db);
                let f1_l = lo.data.f0 * (1.0 + lo.data.b).sqrt();
                let f0_u_star =
                    f1_l * ((1200.0 + width) / 1200.0).exp2() / (1.0 + up.data.b).sqrt();
                if let Some(c) = rho_conditioning(&lo.data, f0_u_star, up.data.b) {
                    conds.push(c);
                }
            }
        }
        let combos = lo_caps.len() * up_caps.len();
        if widths.len() < 2 {
            pair_reports.push(serde_json::json!({
                "m": m, "combos": combos, "accepted": widths.len(),
                "gated": gated, "edge": edge, "no_root": no_root,
            }));
            continue;
        }
        let (w_mu, w_sd) = mean_sd(&widths);
        let (r_mu, r_sd) = mean_sd(&rhos);
        let (c_mu, _) = mean_sd(&conds);
        pair_reports.push(serde_json::json!({
            "m": m, "combos": combos, "accepted": widths.len(),
            "gated": gated, "edge": edge, "no_root": no_root,
            "width_mean": w_mu, "width_sd": w_sd,
            "rho_mean": r_mu, "rho_sd": r_sd,
            "cond_mean": c_mu,               // |∂ρ/∂w|, ρ per cent
            "strength_slope": slope(&strengths, &widths), // ¢ per dB
            "strength_span_db": strengths.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
                - strengths.iter().cloned().fold(f64::INFINITY, f64::min),
        }));
    }

    // ── 3: resampled curve draws ──
    const DRAWS: usize = 24;
    let mut rng = Rng(0x9E3779B97F4A7C15);
    let mut draw_reports: Vec<serde_json::Value> = Vec::new();
    for _ in 0..DRAWS {
        let mut input = CurveInput::default();
        let mut picks: Vec<String> = Vec::new();
        for (&k, caps) in &keys {
            let pick = rng.next(caps.len());
            input.keys[k] = Some(caps[pick].data.clone());
            picks.push(caps[pick].source_dir.clone());
        }
        let (raw, _) = curves::raw_octave_chain(&input, &params);

        // Per-draw engine-(c) calibration internals.
        let (points, _) = curves::giordano_rho_points(&input);
        let phi = (points.len() >= curves::RHO_FIT_MIN_POINTS)
            .then(|| {
                let reg = curves::select_rho_reg_weight(&points, &params.rho);
                rigaud::fit_rho_phi(&points, &params.rho, reg)
            })
            .flatten();

        let curve_b = curves::per_key_smoothed(&input, &params);
        let curve_c = curves::giordano_calibrated(&input, &params);
        let curve_d = curves::multi_interval(&input, &params, BALANCED_INTERVALS, None);
        let curve_do = curves::multi_interval(&input, &params, OCTAVES_ONLY, None);
        let cents = |c: &tuner_core::models::TuningCurve| -> Vec<f64> {
            c.cents.iter().map(|&x| x as f64).collect()
        };
        draw_reports.push(serde_json::json!({
            "raw_chain": raw.to_vec(),
            "rho_points": points,
            "phi": phi.map(|p| serde_json::json!({
                "kappa": p.kappa, "m0": p.m0, "alpha": p.alpha,
                "rho_a0": p.rho_at_midi(21.0),
            })),
            "b": cents(&curve_b),
            "c": cents(&curve_c),
            "d_balanced": cents(&curve_d),
            "d_octaves_only": cents(&curve_do),
            "picks": picks,
        }));
    }

    let report = serde_json::json!({
        "source": path,
        "keys": keys.iter().map(|(k, v)| serde_json::json!({
            "key": k, "captures": v.len(),
        })).collect::<Vec<_>>(),
        "pairs": pair_reports,
        "draws": draw_reports,
    });
    println!("{}", serde_json::to_string(&report).expect("serialize"));
}
