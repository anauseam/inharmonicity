//! # Isolation — what the shipped unison panel sees, against isolation truth
//!
//! The mute-isolation set is the only data in the project with **ground truth
//! for a unison split**: the same note recorded once per string with the others
//! damped, and once open. A difference of two independently measured solo f₀ is
//! not `2/T`-bound, so for the first time a reported split has an answer to
//! check against (`docs/internals/06-capture-sets.md`).
//!
//! This harness supplies the half of Prompt AD that only the shipped code can
//! answer — **what the panel reports** — and emits it as JSON for the scoring
//! step. The truth side (per-string (f₀, B), true splits, the B spread, the
//! coupling comparison) is post-processing of `regenerate_partials` output and
//! lives in `scripts/isolation_truth.py`, the same split as
//! `scripts/audit_captures.py`.
//!
//! Two pre-registered criteria are evaluated here (ADR 0014; the rule is
//! `docs/internals/07-evidence-and-methodology.md`):
//!
//! - **C2 — the false-beat positive control.** A capture with **one string
//!   sounding** that still resolves two lines is a false beat by construction,
//!   and the project had none of these before this set. Extra lines are
//!   *expected* in the bass (ADR 0013); the test is whether the discriminator
//!   ever asserts them as a second string.
//! - **C1's panel half** — the split the panel reports per open capture, written
//!   to JSON for comparison against solo truth.
//!
//! Run:
//! ```bash
//! cargo run --release --example regenerate_partials -- <dump_dir> > iso.json
//! cargo run --release --example isolation -- iso.json <dump_dir> [--json out.json]
//! ```

mod common;

use std::path::Path;

use common::{Capture, Resolved, load_regen, median, register, run_unison};
use tuner_core::algorithms::curves::default_display_partials;
use tuner_core::audio::HOP_RATE_HZ;
use tuner_core::strobe::MAX_STROBE_REFS;
use tuner_core::strobe::unison::UnisonVerdict;

/// One capture's panel reading at the partial the display shows.
struct Reading {
    cap: Capture,
    /// Displayed partial n\*, 1-indexed.
    n_star: usize,
    /// What that reference resolved at its best record.
    at_n_star: Resolved,
    /// The bank-wide verdict the panel would print.
    verdict: UnisonVerdict,
}

impl Reading {
    /// Widest pair the panel resolved at n\*, in Hz — the split it reports.
    fn reported_split_hz(&self) -> Option<f32> {
        let n = self.at_n_star.count as usize;
        if n < 2 {
            return None;
        }
        let offs = &self.at_n_star.lines[..n];
        let lo = offs
            .iter()
            .map(|l| l.offset_hz)
            .fold(f32::INFINITY, f32::min);
        let hi = offs
            .iter()
            .map(|l| l.offset_hz)
            .fold(f32::NEG_INFINITY, f32::max);
        (hi - lo).is_finite().then_some(hi - lo)
    }
}

fn verdict_name(v: UnisonVerdict) -> &'static str {
    match v {
        UnisonVerdict::Unison => "unison",
        UnisonVerdict::FalseBeat => "false_beat",
        UnisonVerdict::Undetermined => "undetermined",
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let regen = args
        .next()
        .expect("usage: isolation <regen.json> <dump_dir> [--json out]");
    let root = args
        .next()
        .expect("usage: isolation <regen.json> <dump_dir> [--json out]");
    let mut json_out = None;
    while let Some(a) = args.next() {
        if a == "--json" {
            json_out = args.next();
        }
    }
    let root = Path::new(&root);

    let (caps, dropped) = load_regen(Path::new(&regen));
    let declared: Vec<Capture> = caps.into_iter().filter(|c| c.sounding.is_some()).collect();
    println!(
        "loaded {} declared captures ({dropped} dropped as implausible)",
        declared.len()
    );

    let table = default_display_partials();
    let mut readings = Vec::new();
    let mut no_audio = 0;
    for cap in declared {
        let Some(audio) = cap.audio(root) else {
            no_audio += 1;
            continue;
        };
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        let count = cap.refs(&mut refs);
        let n_star = table[cap.key as usize] as usize;
        if count < n_star {
            continue; // the displayed partial was never measured on this capture
        }
        let (best, verdict) = run_unison(&audio, &refs, count, cap.f0, 3e-3);
        readings.push(Reading {
            n_star,
            at_n_star: best[n_star - 1],
            verdict,
            cap,
        });
    }
    println!(
        "replayed {} captures through the shipped bank ({no_audio} missing audio)\n",
        readings.len()
    );

    c2_false_beat_control(&readings);
    availability(&readings);
    if let Some(path) = json_out {
        write_json(&readings, Path::new(&path));
    }
}

/// **C2** — the false-beat positive control. Pre-registered pass: an
/// `Unison` verdict on ≤ 5 % of solo captures, anchored to the 4 % bass rate
/// ADR 0012 §5 measured on this instrument.
fn c2_false_beat_control(readings: &[Reading]) {
    println!("=== C2: the false-beat positive control (solo captures) ===");
    println!("A solo that resolves two lines is a false beat by construction.");
    println!("The test is whether the discriminator ever calls one a unison.\n");
    println!(
        "{:>14} {:>8} {:>9} {:>7} {:>7} {:>9} {:>10}",
        "population", "captures", "published", "≥2 lines", "≥3", "unison", "false beat"
    );

    let solos: Vec<&Reading> = readings
        .iter()
        .filter(|r| r.cap.sounding.is_some_and(|s| s.is_solo()))
        .collect();

    let report = |label: &str, rows: &[&Reading]| {
        if rows.is_empty() {
            return;
        }
        // Only captures whose ring actually published can testify: below the
        // record floor "one line" means the detector is blind, not that the
        // note is clean (ADR 0012 §3).
        let pub_rows: Vec<&&Reading> = rows.iter().filter(|r| r.at_n_star.record > 0).collect();
        let n = pub_rows.len().max(1) as f32;
        let pct = |k: usize| {
            100.0
                * pub_rows
                    .iter()
                    .filter(|r| r.at_n_star.count as usize >= k)
                    .count() as f32
                / n
        };
        let vpct = |v: UnisonVerdict| {
            100.0 * pub_rows.iter().filter(|r| r.verdict == v).count() as f32 / n
        };
        println!(
            "{label:>14} {:>8} {:>9} {:>6.0}% {:>6.0}% {:>8.1}% {:>9.0}%",
            rows.len(),
            pub_rows.len(),
            pct(2),
            pct(3),
            vpct(UnisonVerdict::Unison),
            vpct(UnisonVerdict::FalseBeat),
        );
    };

    // Single-strung keys needed no mute at all, so they carry no
    // mute-failure risk — reported apart from the muted solos for that reason.
    let single: Vec<&Reading> = solos
        .iter()
        .filter(|r| r.cap.sounding.is_some_and(|s| s.on_key == 1))
        .copied()
        .collect();
    let muted: Vec<&Reading> = solos
        .iter()
        .filter(|r| r.cap.sounding.is_some_and(|s| s.on_key > 1))
        .copied()
        .collect();
    report("single-strung", &single);
    report("muted solo", &muted);
    report("ALL SOLOS", &solos);

    let pub_all: Vec<&&Reading> = solos.iter().filter(|r| r.at_n_star.record > 0).collect();
    let unison = pub_all
        .iter()
        .filter(|r| r.verdict == UnisonVerdict::Unison)
        .count();
    let rate = 100.0 * unison as f32 / pub_all.len().max(1) as f32;
    println!(
        "\n  C2: {unison} of {} published solo captures called `Unison` = {rate:.1} %",
        pub_all.len()
    );
    println!(
        "  pre-registered bar ≤ 5.0 % → {}",
        if rate <= 5.0 { "PASS" } else { "FAIL" }
    );
    if unison > 0 {
        println!("\n  every solo called a unison (each one is a false positive):");
        for r in pub_all
            .iter()
            .filter(|r| r.verdict == UnisonVerdict::Unison)
        {
            println!(
                "    {:<34} key {:>2} {:<5} lines={} record={}",
                r.cap.dir,
                r.cap.key,
                register(r.cap.key),
                r.at_n_star.count,
                r.at_n_star.record
            );
        }
    }
}

/// How often the ring reaches a usable record at all, per register — the
/// denominator every other figure here is conditioned on.
fn availability(readings: &[Reading]) {
    println!("\n=== Availability at n* (the ring must publish before anything else) ===");
    println!(
        "{:>12} {:>9} {:>10} {:>9} {:>10}",
        "register", "captures", "published", "≥2 lines", "median 2/T"
    );
    for band in ["bass", "tenor", "treble", "high 76–87"] {
        let rows: Vec<&Reading> = readings
            .iter()
            .filter(|r| register(r.cap.key) == band)
            .collect();
        if rows.is_empty() {
            continue;
        }
        let pubd: Vec<&&Reading> = rows.iter().filter(|r| r.at_n_star.record > 0).collect();
        let two = pubd.iter().filter(|r| r.at_n_star.count >= 2).count();
        println!(
            "{band:>12} {:>9} {:>9.0}% {:>8.0}% {:>9.2}Hz",
            rows.len(),
            100.0 * pubd.len() as f32 / rows.len() as f32,
            100.0 * two as f32 / pubd.len().max(1) as f32,
            median(pubd.iter().map(|r| r.at_n_star.resolution_hz).collect()),
        );
    }
    println!(
        "\n  ring cap {:.2} s → 2/T = {:.3} Hz, one number for the whole compass",
        tuner_core::strobe::unison::UNISON_RING_SECS,
        2.0 * HOP_RATE_HZ / tuner_core::strobe::unison::UNISON_RING_HOPS as f32,
    );
}

/// Per-capture panel readings, for the scoring step to join against truth.
fn write_json(readings: &[Reading], path: &Path) {
    let rows: Vec<serde_json::Value> = readings
        .iter()
        .map(|r| {
            let s = r.cap.sounding.expect("declared");
            serde_json::json!({
                "dir": r.cap.dir,
                "key": r.cap.key,
                "on_key": s.on_key,
                "sounding": s.sounding,
                "is_open": s.is_open(),
                "is_solo": s.is_solo(),
                "n_star": r.n_star,
                "record_hops": r.at_n_star.record,
                "resolution_hz": r.at_n_star.resolution_hz,
                "line_count": r.at_n_star.count,
                "offsets_hz": r.at_n_star.lines[..r.at_n_star.count as usize]
                    .iter().map(|l| l.offset_hz).collect::<Vec<_>>(),
                "amplitudes": r.at_n_star.lines[..r.at_n_star.count as usize]
                    .iter().map(|l| l.relative_amplitude).collect::<Vec<_>>(),
                "reported_split_hz": r.reported_split_hz(),
                "ref_hz": r.cap.partials.get(r.n_star).copied().flatten().map(|(f, _)| f),
                "verdict": verdict_name(r.verdict),
            })
        })
        .collect();
    std::fs::write(path, serde_json::to_string_pretty(&rows).unwrap()).expect("write json");
    println!("\nwrote {} readings to {}", rows.len(), path.display());
}
