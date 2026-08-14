//! # Tuning-curve comparison harness (design note §11)
//!
//! Runs all four curve engines on a regenerated-partials JSON dump (the
//! output of `examples/regenerate_partials`) and reports the §11
//! no-ground-truth diagnostics side by side:
//!
//! 1. the per-instrument $B_\xi$ fit and curve-B fallback counts;
//! 2. stretch tables (curve values at the A keys, per-register median
//!    octave stretch);
//! 3. roughness (curvature $|\Delta^2 d|$ statistics);
//! 4. §2 detector / exclusion flag counts;
//! 5. implied beat-rate profiles per coincident-pair type (magnitude and
//!    jaggedness — aural practice expects slow, smoothly progressing
//!    beats: the Verituner patent's own criterion);
//! 6. Giordano cross-scoring of every engine's curve (descriptive ONLY —
//!    selecting on it would bias toward engine (c) by construction);
//! 7. leave-one-key-out prediction error in cents (the cleanest genuine
//!    error number available), which also exercises the DOF-growth claim;
//! 8. a DOF-growth sweep for engine (b): k = 4, 8, 16, 32, 88 measured keys.
//!
//! **These numbers are diagnostics, not selection evidence** (n = 1, no
//! aurally-tuned reference — §11's honesty clause). The captures are
//! auto-mode provenance, admitted via
//! `CurveInput::from_profile_unfiltered` — validation only; the user
//! path admits manual captures exclusively.
//!
//! Usage:
//!   cargo run --release --example regenerate_partials > /tmp/partials.json
//!   cargo run --release --example curve_compare -- /tmp/partials.json
//!   cargo run --release --example curve_compare -- /tmp/partials.json --json /tmp/curve_report.json
//!
//! `--json <path>` additionally writes the full report as machine-readable
//! JSON — the input of `scripts/plot_curves.py`, which renders the
//! one-image curve comparison (`curve_analysis.png`) for a capture set.

use tuner_core::algorithms::curves::{
    self, BALANCED_INTERVALS, CurveParams, IntervalSpec, PURE_TWELFTHS_INTERVALS,
};
use tuner_core::algorithms::{giordano, rigaud};
use tuner_core::models::{CurveInput, InharmonicityProfile, KeyMeasurement, Partial, TuningCurve};

/// Register split used throughout the report (stated per §11: bass =
/// A0–C#3 wound strings region, treble = C6 up where partial counts thin).
fn register(key: usize) -> &'static str {
    match key {
        0..=27 => "bass",
        28..=62 => "mid",
        _ => "treble",
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let mid = v.len() / 2;
    if v.len() % 2 == 1 {
        v[mid]
    } else {
        0.5 * (v[mid - 1] + v[mid])
    }
}

fn load_profile(path: &str) -> InharmonicityProfile {
    let text = std::fs::read_to_string(path).expect("read partials JSON");
    let entries: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parse JSON");
    let mut profile = InharmonicityProfile::default();
    for e in &entries {
        let key_index = e["key_index"].as_u64().expect("key_index") as u8;
        let partials: Vec<Partial> = e["partials"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| Partial {
                        number: p["number"].as_u64().unwrap_or(0) as u32,
                        frequency: p["frequency"].as_f64().unwrap_or(0.0) as f32,
                        amplitude: p["amplitude"].as_f64().unwrap_or(0.0) as f32,
                    })
                    .collect()
            })
            .unwrap_or_default();
        profile.record(KeyMeasurement {
            key_index,
            measured_f0: e["measured_f0"].as_f64().unwrap_or(0.0) as f32,
            partials,
            calculated_b: e["calculated_b"].as_f64().map(|b| b as f32),
            last_captured: String::new(),
            // Honest provenance: these are auto-mode captures.
            captured_in_auto: true,
            sounding_strings: None,
        });
    }
    profile
}

/// Engine runners, boxed so the report loops can treat them uniformly.
type Engine = Box<dyn Fn(&CurveInput) -> TuningCurve>;

fn engines(params: CurveParams) -> Vec<(&'static str, Engine)> {
    vec![
        (
            "a: rigaud-pure",
            Box::new(move |i: &CurveInput| curves::rigaud_pure(i, &params)),
        ),
        (
            "b: per-key+whittaker",
            Box::new(move |i: &CurveInput| curves::per_key_smoothed(i, &params)),
        ),
        (
            "c: giordano-calibrated",
            Box::new(move |i: &CurveInput| curves::giordano_calibrated(i, &params)),
        ),
        (
            "d: multi-interval",
            Box::new(move |i: &CurveInput| {
                curves::multi_interval(i, &params, BALANCED_INTERVALS, None)
            }),
        ),
        (
            "d: pure-12ths preset",
            Box::new(move |i: &CurveInput| {
                curves::multi_interval(i, &params, PURE_TWELFTHS_INTERVALS, None)
            }),
        ),
    ]
}

/// Raw measured B where available, else the instrument fit — the beat-rate
/// report's "physical string" B.
fn raw_b(input: &CurveInput, bxi: &rigaud::BXi, key: usize) -> f64 {
    input.keys[key]
        .as_ref()
        .map(|d| d.b)
        .unwrap_or_else(|| bxi.b_at_key(key))
}

fn partial_freq(f1: f64, b: f64, n: u32) -> f64 {
    let f0 = f1 / (1.0 + b).sqrt();
    let n = n as f64;
    n * f0 * (1.0 + b * n * n).sqrt()
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "partials_current.json".into());
    let json_out: Option<String> = args
        .iter()
        .position(|a| a == "--json")
        .and_then(|i| args.get(i + 1).cloned());
    let profile = load_profile(&path);
    // Validation data is auto-mode; see module doc.
    let input = CurveInput::from_profile_unfiltered(&profile);
    let bxi = curves::instrument_b_fit(&input);

    println!("curve_compare — §11 diagnostics on {path}");
    println!(
        "measured keys: {} of 88 (auto-mode provenance, validation only)\n",
        input.measured_count()
    );
    println!(
        "B_xi fit (Eq. 29, MIDI domain): s_B = {:+.4}, y_B = {:+.3}  \
         [default medium: -0.0660, -7.891]",
        bxi.s_b, bxi.y_b
    );
    println!(
        "B_xi at A0/A4/C8: {:.3e} / {:.3e} / {:.3e}\n",
        bxi.b_at_key(0),
        bxi.b_at_key(48),
        bxi.b_at_key(87)
    );

    let params = CurveParams::default();
    let runs: Vec<(&str, TuningCurve)> = engines(params)
        .iter()
        .map(|(n, f)| (*n, f(&input)))
        .collect();

    // ── 2. Stretch table ──
    println!("── curve values at the A keys (cents vs ET) ──");
    print!("{:<24}", "engine");
    for a in [0usize, 12, 24, 36, 48, 60, 72, 84, 87] {
        print!(
            "{:>8}",
            if a == 87 {
                "C8".into()
            } else {
                format!("A{}", a / 12)
            }
        );
    }
    println!();
    for (name, curve) in &runs {
        print!("{name:<24}");
        for a in [0usize, 12, 24, 36, 48, 60, 72, 84, 87] {
            print!("{:>8.1}", curve.cents[a]);
        }
        println!();
    }

    println!("\n── per-register median octave stretch (¢/oct) ──");
    println!("{:<24}{:>8}{:>8}{:>8}", "engine", "bass", "mid", "treble");
    for (name, curve) in &runs {
        print!("{name:<24}");
        for reg in ["bass", "mid", "treble"] {
            let vals: Vec<f64> = (0..76)
                .filter(|&m| register(m) == reg)
                .map(|m| (curve.cents[m + 12] - curve.cents[m]) as f64)
                .collect();
            print!("{:>8.2}", median(vals));
        }
        println!();
    }

    // ── 3. Roughness ──
    println!("\n── roughness: |Δ²d| curvature (¢) ──");
    println!("{:<24}{:>10}{:>10}", "engine", "median", "max");
    for (name, curve) in &runs {
        let d2: Vec<f64> = (0..86)
            .map(|m| {
                (curve.cents[m] as f64 - 2.0 * curve.cents[m + 1] as f64
                    + curve.cents[m + 2] as f64)
                    .abs()
            })
            .collect();
        let max = d2.iter().cloned().fold(0.0, f64::max);
        println!("{name:<24}{:>10.3}{max:>10.3}", median(d2));
    }

    // ── 4. Flags ──
    println!("\n── flag counts ──");
    println!(
        "{:<24}{:>10}{:>10}{:>12}{:>14}",
        "engine", "neg-str", "excluded", "b-fallback", "giordano-excl"
    );
    for (name, curve) in &runs {
        let count = |f: fn(&tuner_core::models::CurveKeyFlags) -> bool| {
            curve.flags.iter().filter(|k| f(k)).count()
        };
        println!(
            "{name:<24}{:>10}{:>10}{:>12}{:>14}",
            count(|f| f.negative_stretch),
            count(|f| f.excluded),
            count(|f| f.curve_b_fallback),
            count(|f| f.giordano_excluded),
        );
    }

    // ── 4b. Giordano calibration stage (engine (c) internals) ──
    let (rho_points, _excl) = curves::giordano_rho_points(&input);
    let reg = curves::select_rho_reg_weight(&rho_points, &params.rho);
    let phi = rigaud::fit_rho_phi(&rho_points, &params.rho, reg);
    println!(
        "\n── Giordano calibration stage ──\n\
         ρ points accepted: {} of 76 octave pairs; reg weight (LOO-CV): {reg}",
        rho_points.len()
    );
    if let Some(phi) = &phi {
        println!(
            "calibrated φ: κ = {:.3}, m0 = {:.1}, α = {:.1}  \
             [typical: 3.5, 60, 25]  ρ at A0/A4: {:.2}/{:.2}",
            phi.kappa,
            phi.m0,
            phi.alpha,
            phi.rho_at_midi(21.0),
            phi.rho_at_midi(69.0)
        );
    }
    // Demoted diagnostic: what the old above-median-amplitude gate would
    // have passed, vs the §VI.C coincident-pair gate.
    let (mut old_gate, mut new_gate, mut both_measured) = (0usize, 0usize, 0usize);
    for m in 0..76usize {
        let (Some(lo), Some(up)) = (&input.keys[m], &input.keys[m + 12]) else {
            continue;
        };
        both_measured += 1;
        if giordano::strong_cross_pairs(&lo.partials, &up.partials) >= 8 {
            old_gate += 1;
        }
        if giordano::coincident_pairs(&lo.partials, &up.partials)
            >= curves::GIORDANO_MIN_COINCIDENT_PAIRS
        {
            new_gate += 1;
        }
    }
    println!(
        "gate: {new_gate}/{both_measured} pairs pass §VI.C coincident-pair gate \
         (old strong-cross-pair diagnostic would pass {old_gate})"
    );

    // ── 5. Implied beat rates ──
    println!("\n── implied beat rates at coincident pairs (Hz; median / max / median jag) ──");
    let pair_types: [(&str, usize, u32, u32); 4] = [
        ("2:1 oct", 12, 2, 1),
        ("4:2 oct", 12, 4, 2),
        ("6:3 oct", 12, 6, 3),
        ("3:1 12th", 19, 3, 1),
    ];
    for (name, curve) in &runs {
        println!("{name}:");
        for &(label, k, p, q) in &pair_types {
            let mut rates: Vec<(usize, f64)> = Vec::new();
            for m in 0..(88 - k) {
                let (bl, bu) = (raw_b(&input, &bxi, m), raw_b(&input, &bxi, m + k));
                let fp = partial_freq(curve.target_f1(m as u8) as f64, bl, p);
                let fq = partial_freq(curve.target_f1((m + k) as u8) as f64, bu, q);
                if fp < 22050.0 && fq < 22050.0 {
                    rates.push((m, (fp - fq).abs()));
                }
            }
            let vals: Vec<f64> = rates.iter().map(|&(_, r)| r).collect();
            let jag: Vec<f64> = rates.windows(2).map(|w| (w[1].1 - w[0].1).abs()).collect();
            let max = vals.iter().cloned().fold(0.0, f64::max);
            println!(
                "  {label:<9} median {:>7.3}  max {:>8.3}  jag {:>7.3}",
                median(vals),
                max,
                median(jag)
            );
        }
    }

    // ── 6. Giordano cross-scoring ──
    println!("\n── Giordano cross-score (Σ octave-pair dissonance at prescribed widths) ──");
    println!("   (descriptive only — this objective favors engine (c) by construction)");
    let mut cross_scores: Vec<(String, f64)> = Vec::new();
    for (name, curve) in &runs {
        let mut total = 0.0;
        let mut pairs = 0;
        for m in 0..76usize {
            let (Some(lo), Some(up)) = (&input.keys[m], &input.keys[m + 12]) else {
                continue;
            };
            let f1 = |d: &tuner_core::models::CurveKeyData| d.f0 * (1.0 + d.b).sqrt();
            // Rigid Giordano-style shift of each note onto the engine's target.
            let shift = |d: &tuner_core::models::CurveKeyData, target: f64| -> Vec<(f64, f64)> {
                let df = target - f1(d); // partial n moves by n·df (Giordano's shift rule)
                d.partials
                    .iter()
                    .map(|&(n, f, a)| (f + n as f64 * df, a))
                    .collect()
            };
            let lo_t = shift(lo, curve.target_f1(m as u8) as f64);
            let up_t = shift(up, curve.target_f1((m + 12) as u8) as f64);
            total += giordano::dissonance(&lo_t, &up_t);
            pairs += 1;
        }
        println!("{name:<24}{total:>10.4}  ({pairs} pairs)");
        cross_scores.push((name.to_string(), total));
    }

    // ── 7. Leave-one-key-out prediction error ──
    println!("\n── leave-one-key-out prediction error (¢): engine-without-key vs raw data ──");
    println!(
        "   (reference = the Eq-6 octave-chain raw value at the held-out key, which\n\
         \x20   structurally favors octave-chain engines; treble keys carry no measured\n\
         \x20   curve-B, hence no reference — reported as NaN)"
    );
    let (raw_full, raw_measured) = curves::raw_octave_chain(&input, &params);
    println!("{:<24}{:>8}{:>8}{:>8}", "engine", "bass", "mid", "treble");
    let mut lko_medians: Vec<(String, [f64; 3])> = Vec::new();
    for (name, f) in &engines(params) {
        let mut errs: Vec<(usize, f64)> = Vec::new();
        for k in 0..88 {
            if !raw_measured[k] {
                continue; // no data-implied value to predict
            }
            let mut lko = input.clone();
            lko.keys[k] = None;
            let curve = f(&lko);
            errs.push((k, (curve.cents[k] as f64 - raw_full[k]).abs()));
        }
        print!("{name:<24}");
        let mut meds = [f64::NAN; 3];
        for (i, reg) in ["bass", "mid", "treble"].iter().enumerate() {
            let vals: Vec<f64> = errs
                .iter()
                .filter(|&&(k, _)| register(k) == *reg)
                .map(|&(_, e)| e)
                .collect();
            meds[i] = median(vals);
            print!("{:>8.2}", meds[i]);
        }
        println!();
        lko_medians.push((name.to_string(), meds));
    }

    // ── 8. DOF growth for engine (b) ──
    println!("\n── DOF growth, engine (b): median |d_k − d_(a)| and |d_k − d_88| (¢) ──");
    let full_b = curves::per_key_smoothed(&input, &params);
    let curve_a = curves::rigaud_pure(&input, &params);
    let measured: Vec<usize> = (0..88).filter(|&k| input.keys[k].is_some()).collect();
    for n in [4usize, 8, 16, 32, measured.len()] {
        let mut subset = CurveInput {
            keys: (0..88).map(|_| None).collect(),
        };
        for i in 0..n.min(measured.len()) {
            let k = measured[i * (measured.len() - 1) / (n.max(2) - 1).max(1)];
            subset.keys[k] = input.keys[k].clone();
        }
        let got = subset.measured_count();
        let curve = curves::per_key_smoothed(&subset, &params);
        let to_a = median(
            (0..88)
                .map(|k| (curve.cents[k] - curve_a.cents[k]).abs() as f64)
                .collect(),
        );
        let to_full = median(
            (0..88)
                .map(|k| (curve.cents[k] - full_b.cents[k]).abs() as f64)
                .collect(),
        );
        println!("  k={got:<3} |d−a| {to_a:>6.2}   |d−full| {to_full:>6.2}");
    }

    // Interval preset sanity line: τ values as shipped.
    let fifth = IntervalSpec {
        k: 7,
        p: 3,
        q: 2,
        tempered: true,
        weight: 1.0,
    };
    println!(
        "\n(interval table: τ_fifth = {:+.3} ¢, τ_fourth = {:+.3} ¢; octave family pure)",
        fifth.tau(),
        IntervalSpec {
            k: 5,
            p: 4,
            q: 3,
            tempered: true,
            weight: 1.0
        }
        .tau()
    );
    println!("\nAll numbers are diagnostics, not selection evidence (n = 1; §11).");

    // ── Machine-readable report (`--json <path>`) — plot_curves.py input ──
    if let Some(out) = json_out {
        let engines_json: Vec<serde_json::Value> = runs
            .iter()
            .map(|(name, curve)| {
                let cents: Vec<f64> = curve.cents.iter().map(|&c| c as f64).collect();
                let flag_keys = |f: fn(&tuner_core::models::CurveKeyFlags) -> bool| {
                    curve
                        .flags
                        .iter()
                        .enumerate()
                        .filter(|(_, k)| f(k))
                        .map(|(i, _)| i)
                        .collect::<Vec<_>>()
                };
                let stretch = |reg: &str| {
                    median(
                        (0..76)
                            .filter(|&m| register(m) == reg)
                            .map(|m| (curve.cents[m + 12] - curve.cents[m]) as f64)
                            .collect(),
                    )
                };
                let d2: Vec<f64> = (0..86)
                    .map(|m| {
                        (curve.cents[m] as f64 - 2.0 * curve.cents[m + 1] as f64
                            + curve.cents[m + 2] as f64)
                            .abs()
                    })
                    .collect();
                let rough_max = d2.iter().cloned().fold(0.0, f64::max);
                let cross = cross_scores
                    .iter()
                    .find(|(n, _)| n == name)
                    .map(|&(_, v)| v);
                let lko = lko_medians.iter().find(|(n, _)| n == name).map(|(_, m)| m);
                serde_json::json!({
                    "name": name,
                    "cents": cents,
                    "flags": {
                        "negative_stretch": flag_keys(|f| f.negative_stretch),
                        "excluded": flag_keys(|f| f.excluded),
                        "curve_b_fallback": flag_keys(|f| f.curve_b_fallback),
                        "giordano_excluded": flag_keys(|f| f.giordano_excluded),
                        "measured": flag_keys(|f| f.measured),
                    },
                    "stretch_median": {
                        "bass": stretch("bass"), "mid": stretch("mid"), "treble": stretch("treble")
                    },
                    "roughness": { "median": median(d2.clone()), "max": rough_max },
                    "cross_score": cross,
                    "lko_median": lko.map(|m| serde_json::json!({
                        "bass": m[0], "mid": m[1], "treble": m[2]
                    })),
                })
            })
            .collect();
        let report = serde_json::json!({
            "source": path,
            "measured_keys": input.measured_count(),
            "b_xi": { "s_b": bxi.s_b, "y_b": bxi.y_b },
            "calibration": {
                "rho_points": rho_points,
                "reg_weight": reg,
                "phi": phi.as_ref().map(|p| serde_json::json!({
                    "kappa": p.kappa, "m0": p.m0, "alpha": p.alpha
                })),
                "gate_pass": new_gate,
                "gate_pass_old_diagnostic": old_gate,
                "pairs_both_measured": both_measured,
            },
            "engines": engines_json,
        });
        std::fs::write(
            &out,
            serde_json::to_string_pretty(&report).expect("serialize"),
        )
        .expect("write JSON report");
        println!("JSON report written to {out}");
    }
}
