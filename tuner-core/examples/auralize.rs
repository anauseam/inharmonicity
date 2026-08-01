//! # Auralize the tuning curves — offline additive-resynthesis A/B/C/D (Prompt J)
//!
//! There is **no ground-truth-free "optimal" curve**. With inharmonicity the
//! octave, fifth, and twelfth beats are mutually incompatible objectives, so
//! engines (a)–(d) each minimize a *different* functional and "best" is a
//! listening judgment (ADR 0009). The objective screens `curve_compare` prints
//! (beat-rate smoothness — the Verituner criterion) can *reject* a bad curve
//! but cannot crown a winner. This harness produces the missing selection
//! evidence in the only form that can decide it: a **human perceptual A/B**.
//!
//! It is a **thin driver over [`tuner_core::synth`]** (the reusable, headless
//! additive-resynthesis engine). Its own job is the *comparison*: render the
//! same musical material under every engine's curve, loudness-matched, so the
//! audible differences between the WAVs are literally the different
//! coincident-partial beat rates each curve prescribes. This does not break
//! n = 1 discipline — the output is audio for a person to compare, not a
//! statistic — and nothing touches the hot path.
//!
//! ## Test material (where coincident-partial beats are audible)
//!
//!   1. octaves walked up the compass (2:1/4:2/6:3 beats),
//!   2. fifths walked up (3:2 beat),
//!   3. an A–E–A chord (octave + two fifths interacting),
//!   4. a chromatic run (timbre/pitch reference),
//!   5. a slow single-octave sweep — the Verituner smoothly-progressing-beats
//!      test in aural form.
//!
//! ## Usage
//!
//!   cargo run --release --example regenerate_partials -- diagnostics > p2.json
//!   cargo run --release --example auralize -- p2.json --out auralize_out
//!
//! Writes `auralize_out/<engine>.wav` (a, b, c×{Low,Mean,High ρ}, d, d-pure-12ths)
//! and prints a short material-specific beat-rate screen. Consumes the piano-2
//! regenerated partials (the timbres) + each engine's curve (the targets);
//! **repeat captures are averaged per key** to denoise the timbre (see
//! [`load_profile`]). Validation-only data (n = 1); commit only when asked.

use std::collections::BTreeMap;

use tuner_core::algorithms::curves::{
    self, BALANCED_INTERVALS, CurveParams, PURE_TWELFTHS_INTERVALS, StretchPreset,
};
use tuner_core::algorithms::rigaud;
use tuner_core::audio::SAMPLE_RATE;
use tuner_core::models::{
    CurveInput, InharmonicityProfile, KeyMeasurement, NOTES, Partial, TuningCurve,
};
use tuner_core::synth::{self, EnvelopeParams, Note};

const NYQUIST_HZ: f64 = SAMPLE_RATE as f64 / 2.0;

// ─── Partials loader (averages repeat captures per key to denoise timbre) ────

/// One parsed capture entry from a `regenerate_partials` dump.
struct RawEntry {
    measured_f0: f64,
    calculated_b: Option<f64>,
    partials: Vec<(u32, f64, f64)>, // (n, frequency, amplitude)
}

/// Load a `regenerate_partials` JSON dump into a profile, **averaging repeat
/// captures per key** to denoise the timbre (unlike `curve_compare`, which
/// keeps the last capture). A single strike is a noisy sample of the string;
/// repeats shrink that noise ~√N (the reason the repeat set was captured —
/// ADR 0009). Aggregation per key:
///
///   * **B**: log-space median of the finite positive per-capture B (repeat
///     noise of B is multiplicative — ADR 0009 A1);
///   * **partials**: grouped by number n, a partial is kept only if it appears
///     in a **majority** of the key's captures (drops one-off spurious lines),
///     and its frequency/amplitude are the **medians** across the captures
///     that carry it;
///   * `measured_f0`: median (unused by the curve — `CurveInput` derives F₀
///     from the partials via Eq. 20 — but kept sensible).
fn load_profile(path: &str) -> InharmonicityProfile {
    let text = std::fs::read_to_string(path).expect("read partials JSON");
    let raw: Vec<serde_json::Value> = serde_json::from_str(&text).expect("parse JSON");

    let mut by_key: BTreeMap<u8, Vec<RawEntry>> = BTreeMap::new();
    for e in &raw {
        let key_index = e["key_index"].as_u64().expect("key_index") as u8;
        let partials: Vec<(u32, f64, f64)> = e["partials"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .map(|p| {
                        (
                            p["number"].as_u64().unwrap_or(0) as u32,
                            p["frequency"].as_f64().unwrap_or(0.0),
                            p["amplitude"].as_f64().unwrap_or(0.0),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        by_key.entry(key_index).or_default().push(RawEntry {
            measured_f0: e["measured_f0"].as_f64().unwrap_or(0.0),
            calculated_b: e["calculated_b"].as_f64(),
            partials,
        });
    }

    let mut profile = InharmonicityProfile::default();
    let mut reps: Vec<usize> = Vec::new();
    for (key_index, entries) in &by_key {
        reps.push(entries.len());
        profile.record(aggregate_key(*key_index, entries));
    }
    reps.sort_unstable();
    let total: usize = reps.iter().sum();
    let med_reps = reps.get(reps.len() / 2).copied().unwrap_or(0);
    println!(
        "loaded {total} captures across {} keys — averaged per key \
         (median {med_reps} repeats/key; log-median B, majority-present partials, \
         median amplitude)",
        by_key.len()
    );
    profile
}

/// Collapse a key's repeat captures into one denoised [`KeyMeasurement`]
/// (see [`load_profile`] for the rules).
fn aggregate_key(key_index: u8, entries: &[RawEntry]) -> KeyMeasurement {
    let n_reps = entries.len();

    let lnb: Vec<f64> = entries
        .iter()
        .filter_map(|e| e.calculated_b)
        .filter(|b| b.is_finite() && *b > 0.0)
        .map(f64::ln)
        .collect();
    let calculated_b = (!lnb.is_empty()).then(|| median(lnb).exp() as f32);

    let f0s: Vec<f64> = entries
        .iter()
        .map(|e| e.measured_f0)
        .filter(|f| *f > 0.0)
        .collect();
    let measured_f0 = if f0s.is_empty() {
        0.0
    } else {
        median(f0s) as f32
    };

    let mut by_n: BTreeMap<u32, (Vec<f64>, Vec<f64>)> = BTreeMap::new();
    for e in entries {
        for &(n, f, a) in &e.partials {
            if n > 0 && f > 0.0 {
                let slot = by_n.entry(n).or_default();
                slot.0.push(f);
                slot.1.push(a);
            }
        }
    }
    let partials: Vec<Partial> = by_n
        .into_iter()
        .filter(|(_, (freqs, _))| freqs.len() * 2 >= n_reps) // majority-present
        .map(|(n, (freqs, amps))| Partial {
            number: n,
            frequency: median(freqs) as f32,
            amplitude: median(amps) as f32,
        })
        .collect();

    KeyMeasurement {
        key_index,
        measured_f0,
        partials,
        calculated_b,
        last_captured: String::new(),
        captured_in_auto: true, // honest provenance: auto-mode validation data
    }
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

// ─── Test-material program (shared by every engine) ──────────────────────────

/// Build the note timeline. Returns the notes and the total duration (s).
fn build_program() -> (Vec<Note>, f64) {
    let mut notes = Vec::new();
    let mut t = 0.0f64;
    let gap = 0.35; // silence between sections
    let g = 0.7; // per-note intensity (mix loudness-matched afterwards)

    let dyad = |notes: &mut Vec<Note>, t: &mut f64, lo: u8, up: u8, dur: f64| {
        for key in [lo, up] {
            notes.push(Note {
                key,
                intensity: g,
                start_s: *t,
                dur_s: dur,
            });
        }
        *t += dur;
    };

    // 1. Octaves walked up the compass: (m, m+12), C1…C6 roots.
    for &lo in &[3u8, 15, 27, 39, 51, 63] {
        dyad(&mut notes, &mut t, lo, lo + 12, 2.2);
    }
    t += gap;

    // 2. Fifths walked up: (m, m+7), A1…A5 roots (A–E).
    for &lo in &[12u8, 24, 36, 48, 60] {
        dyad(&mut notes, &mut t, lo, lo + 7, 2.2);
    }
    t += gap;

    // 3. A–E–A chord (A3–E4–A4): octave + two fifths beating together.
    for &k in &[36u8, 43, 48] {
        notes.push(Note {
            key: k,
            intensity: g,
            start_s: t,
            dur_s: 3.2,
        });
    }
    t += 3.2 + gap;

    // 4. Chromatic run C4…C5 (timbre/pitch reference).
    for k in 39u8..=51 {
        notes.push(Note {
            key: k,
            intensity: g,
            start_s: t,
            dur_s: 0.34,
        });
        t += 0.32;
    }
    t += gap;

    // 5. Slow single-octave sweep: (m, m+12) walked chromatically through one
    //    octave (A2…A3 roots) — the Verituner smoothly-progressing-beats test.
    for lo in 24u8..=36 {
        dyad(&mut notes, &mut t, lo, lo + 12, 1.3);
    }
    t += gap;

    (notes, t)
}

// ─── Objective beat-rate screen (material-specific; sanity-rank aid) ─────────

/// Raw B where measured, else the instrument B_ξ fit — the physical-string B.
fn raw_b(input: &CurveInput, bxi: &rigaud::BXi, key: usize) -> f64 {
    input.keys[key]
        .as_ref()
        .map(|d| d.b)
        .unwrap_or_else(|| bxi.b_at_key(key))
}

/// For a coincident p:q pair over the given roots, the beat rate
/// |f_p(lo) − f_q(up)| per dyad (median / max / median jag between neighbours).
fn beat_screen(
    curve: &TuningCurve,
    input: &CurveInput,
    bxi: &rigaud::BXi,
    roots: impl Iterator<Item = usize>,
    k: usize,
    p: u32,
    q: u32,
) -> (f64, f64, f64) {
    let rates: Vec<f64> = roots
        .filter_map(|m| {
            let (bl, bu) = (raw_b(input, bxi, m), raw_b(input, bxi, m + k));
            let fp = synth::partial_freq(curve.target_f1(m as u8) as f64, bl, p);
            let fq = synth::partial_freq(curve.target_f1((m + k) as u8) as f64, bu, q);
            (fp < NYQUIST_HZ && fq < NYQUIST_HZ).then(|| (fp - fq).abs())
        })
        .collect();
    let jag: Vec<f64> = rates.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let max = rates.iter().cloned().fold(0.0, f64::max);
    (median(rates.clone()), max, median(jag))
}

// ─── main ────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "partials_current.json".into());
    let out_dir = args
        .iter()
        .position(|a| a == "--out")
        .and_then(|i| args.get(i + 1).cloned())
        .unwrap_or_else(|| "auralize_out".into());
    std::fs::create_dir_all(&out_dir).expect("create out dir");

    let profile = load_profile(&path);
    let input = CurveInput::from_profile_including_auto(&profile);
    let bxi = curves::instrument_b_fit(&input);
    println!(
        "auralize — additive resynthesis A/B/C/D on {path}\n\
         measured keys: {} of 88 (auto-mode provenance, validation only)\n",
        input.measured_count()
    );

    // Engine curves. ρ presets (Low/Mean/High) applied to engine (c), the
    // perceptual octave-type engine where stretch taste is the knob.
    let base = CurveParams::default();
    let preset = |p: StretchPreset| CurveParams { preset: p, ..base };
    let specs: Vec<(&str, String, TuningCurve)> = vec![
        (
            "a_rigaud-pure.wav",
            "a: Rigaud-pure".into(),
            curves::rigaud_pure(&input, &base),
        ),
        (
            "b_per-key-whittaker.wav",
            "b: per-key + Whittaker".into(),
            curves::per_key_smoothed(&input, &base),
        ),
        (
            "c_giordano-low.wav",
            "c: Giordano-calibrated (Low ρ)".into(),
            curves::giordano_calibrated(&input, &preset(StretchPreset::Low)),
        ),
        (
            "c_giordano-mean.wav",
            "c: Giordano-calibrated (Mean ρ)".into(),
            curves::giordano_calibrated(&input, &base),
        ),
        (
            "c_giordano-high.wav",
            "c: Giordano-calibrated (High ρ)".into(),
            curves::giordano_calibrated(&input, &preset(StretchPreset::High)),
        ),
        (
            "d_balanced.wav",
            "d: multi-interval (balanced)".into(),
            curves::multi_interval(&input, &base, BALANCED_INTERVALS, None),
        ),
        (
            "d_pure-12ths.wav",
            "d: multi-interval (pure 12ths)".into(),
            curves::multi_interval(&input, &base, PURE_TWELFTHS_INTERVALS, None),
        ),
    ];

    let (notes, total_s) = build_program();
    let env = EnvelopeParams::default();
    println!(
        "program: {} note events, {total_s:.1} s (octaves ▸ fifths ▸ A–E–A ▸ \
         chromatic ▸ octave-sweep)\n",
        notes.len()
    );

    // Any program keys missing a trusted timbre render silent — report them.
    let mut missing: Vec<u8> = notes
        .iter()
        .filter(|n| input.keys[n.key as usize].is_none())
        .map(|n| n.key)
        .collect();
    missing.sort_unstable();
    missing.dedup();
    if !missing.is_empty() {
        println!(
            "note: {} program key(s) have no trusted timbre and render silent: {:?}\n",
            missing.len(),
            missing
                .iter()
                .map(|&k| NOTES[k as usize].name.clone())
                .collect::<Vec<_>>()
        );
    }

    // Render every engine (tuner_core::synth), then loudness-match with one
    // global scale so the A/B compares curves, not levels.
    let buffers: Vec<(&str, String, Vec<f32>)> = specs
        .iter()
        .map(|(file, label, curve)| {
            (
                *file,
                label.clone(),
                synth::render(curve, &input, &notes, &env),
            )
        })
        .collect();
    let global_peak = buffers
        .iter()
        .map(|(_, _, b)| synth::peak(b))
        .fold(0.0f32, f32::max);
    let scale = if global_peak > 0.0 {
        0.9 / global_peak
    } else {
        1.0
    };
    for (file, label, buf) in &buffers {
        let path = format!("{out_dir}/{file}");
        synth::write_wav(&path, buf, scale).expect("write WAV");
        println!("wrote {path}  [{label}]");
    }

    // ── Objective beat-rate screen on the rendered material ──
    println!(
        "\n── beat-rate screen on rendered material (Hz; median / max / median jag) ──\n\
         (slow octave sweep = keys 24–36 dyads; fifths = A1–A5 roots)"
    );
    println!(
        "{:<32}{:>22}{:>22}",
        "engine", "2:1 oct (sweep)", "3:2 fifth"
    );
    for (_, label, curve) in &specs {
        let (om, ox, oj) = beat_screen(curve, &input, &bxi, 24..=36, 12, 2, 1);
        let (fm, fx, fj) = beat_screen(
            curve,
            &input,
            &bxi,
            [12, 24, 36, 48, 60].into_iter(),
            7,
            3,
            2,
        );
        println!(
            "{label:<32}{:>7.3}/{:>5.2}/{:>5.2}{:>10.3}/{:>5.2}/{:>4.2}",
            om, ox, oj, fm, fx, fj
        );
    }
    println!(
        "\nAll numbers are a sanity screen, not selection evidence (n = 1). The\n\
         listening comparison of the WAVs picks which curve to tune the piano to."
    );
}
