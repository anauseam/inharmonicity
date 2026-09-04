//! # What the panel displays, measured against the reference
//!
//! The displayed reading and every candidate rule for producing it, scored
//! against [`crate::truth`]'s independent estimate: the tracker as shipped, the
//! bounded spectral read, which partial the register table selects, how far off
//! pitch the read still holds, and where the regime boundary chatters.

use std::path::{Path, PathBuf};

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use crate::truth::*;
use tuner_core::algorithms::curves;
use tuner_core::algorithms::peaks;
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::models::{NOTES, get_expected_beta};
use tuner_core::strobe::MAX_STROBE_REFS;

fn process_capture(dir: &Path, planner: &mut RealFftPlanner<f32>) -> Option<u8> {
    let key = crate::capture::key_from_dirname(dir.file_name()?.to_str()?)?;
    let signal = crate::raw::read(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let name = &NOTES[key as usize].name;

    let truth = dtft_truth(&signal, f_et, planner);
    let yin_f0 = yin(&signal, f_et * 0.7, f_et * 1.5);
    let app = run_engine(&signal, key, 0.001, planner);

    let fmt = |o: Option<f32>| match o {
        Some(f) if f.is_finite() && f > 0.0 => format!("{:>8.3}Hz {:>+6.1}¢", f, cents(f, f_et)),
        _ => "     --        ".to_string(),
    };
    let app_f0 = app.as_ref().map(|a| a.f0);
    print!(
        "{:<3} {:>4} f_ET={:>8.3}  truth {}  yin {}  app {}",
        key,
        name,
        f_et,
        fmt(truth),
        fmt(yin_f0),
        fmt(app_f0),
    );
    if let (Some(a), Some(t)) = (app_f0, truth)
        && a.is_finite()
    {
        print!("   app−truth {:>+6.1}¢", cents(a, f_et) - cents(t, f_et));
    }
    if let Some(a) = &app {
        print!(
            "   lock={:?} gate={:.0}% jitter=±{:.1}¢",
            a.locked_key,
            a.gated_frac * 100.0,
            a.cents_jitter
        );
    }
    if let Some((mean, jit)) = band_slope_cents(&signal, f_et) {
        print!("   BAND-slope {mean:+.1}¢ jitter=±{jit:.1}¢");
    }
    println!();
    Some(key)
}

/// Validates the arbiter (`truth`) and `yin` against *known* detunings before
/// we trust either on real audio. A biased estimator here disqualifies its
/// column on the real captures.
fn selftest(planner: &mut RealFftPlanner<f32>) {
    let amps = [0.30f32, 1.0, 0.7, 0.45, 0.25]; // weak-fundamental (wound-string) worst case
    let keys: [(u8, &str); 6] = [
        (19, "E2"),
        (24, "A2"),
        (29, "D3"),
        (34, "G3"),
        (38, "B3"),
        (43, "E4"),
    ];
    println!("SELF-TEST — recovered − true cents on synthetic tones (decay, weak fundamental).");
    println!("Any nonzero here is estimator bias, not a real reading.\n");
    for (b_name, b) in [("B=0", 0.0f32), ("B=3e-4", 3e-4)] {
        println!("── {b_name} ──   (columns: true¢ → truth bias / yin bias)");
        for (key, name) in keys {
            let f_et = NOTES[key as usize].frequency;
            let mut row = String::new();
            for &c in &[-5.0f32, 0.0, 5.0] {
                let f_true = f_et * 2f32.powf(c / 1200.0);
                let sig = synth_tone(f_true, b, &amps, SAMPLE_RATE as usize * 3 / 2, 0.6);
                let tb = dtft_truth(&sig, f_et, planner)
                    .map(|f| cents(f, f_et) - c)
                    .unwrap_or(f32::NAN);
                let yb = yin(&sig, f_et * 0.7, f_et * 1.5)
                    .map(|f| cents(f, f_et) - c)
                    .unwrap_or(f32::NAN);
                row.push_str(&format!("{c:+.0}→[{tb:+5.2}/{yb:+5.2}] "));
            }
            println!("  {name:<3} {f_et:>7.2}Hz  {row}");
        }
        println!();
    }
}

/// **#2 test.** Quantifies YIN (whole-signal) sharpness vs inharmonicity `B`
/// and partial richness. Mechanism claim: a partial-weighted estimate reads
/// sharp by ≈ `866·B·⟨n²⟩`, where `⟨n²⟩` is the power-weighted mean-square
/// partial index — because partial n implies a fundamental `f₀·√(1+Bn²)`,
/// sharp by `866·B·n²` cents. A *harmonic* signal (B=0) has an exact period
/// ⇒ zero drift at any partial count (missing-fundamental recovered). Our
/// strobe reads n=1 ⇒ ⟨n²⟩=1 ⇒ B-immune.
fn inharm_sweep() {
    let f0 = 110.0f32; // A2
    println!("YIN sharpness vs B and partial richness (f0=110 Hz, aₙ=1/n, decay).");
    println!("drift = 1200·log₂(yin/f0) cents; pred ≈ 866·B·⟨n²⟩.");
    println!("B=0 must give ≈0 at every K (the 'independent of inharmonicity' check).\n");
    let bs = [0.0f32, 5e-5, 1e-4, 3e-4, 1e-3];
    print!("{:<20}", "");
    for b in bs {
        print!("  B={b:>7.0e}    ");
    }
    println!();
    for k in [5usize, 10, 15, 20] {
        let mut amps = [0.0f32; 24];
        for (i, a) in amps.iter_mut().enumerate().take(k) {
            *a = 1.0 / (i + 1) as f32;
        }
        let num: f32 = (0..k)
            .map(|i| amps[i] * amps[i] * ((i + 1) * (i + 1)) as f32)
            .sum();
        let den: f32 = (0..k).map(|i| amps[i] * amps[i]).sum();
        let mn2 = num / den;
        print!("K={k:<2} ⟨n²⟩={mn2:5.1}      ");
        for b in bs {
            let sig = synth_tone(f0, b, &amps[..k], SAMPLE_RATE as usize * 3 / 2, 0.6);
            let drift = yin(&sig, f0 * 0.7, f0 * 1.5)
                .map(|y| 1200.0 * (y / f0).log2())
                .unwrap_or(f32::NAN);
            let pred = 866.0 * b * mn2;
            print!("{drift:+5.2}(p{pred:+5.2}) ");
        }
        println!();
    }
}

/// **Fit-window sweep.** Does a shorter baseline rescue the fast-decaying
/// treble, where the ~0.5 s window finds no long-enough ungated run? Reports
/// the longest ungated run (the note's usable life at `f_ref`) and the reading
/// at several window lengths, with its jitter — the accuracy/responsiveness
/// trade the CRLB predicts (variance ~1/T³).
fn window_sweep(dir: &Path, planner: &mut RealFftPlanner<f32>) {
    let Some(key) = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)
    else {
        return;
    };
    let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
        return;
    };
    let f_et = NOTES[key as usize].frequency;
    let Some(f_true) = dtft_truth(&signal, f_et, planner) else {
        return;
    };
    let hop_ms = 1000.0 * HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let true_cents = 1200.0 * (f_true / f_et).log2();
    print!(
        "{:<4} f_true={f_true:>9.2} true={true_cents:>+7.1}¢ |",
        NOTES[key as usize].name
    );
    for win in [21usize, 12, 6, 3] {
        match band_slope_cents_win(&signal, f_et, win) {
            Some((m, j, run)) => print!(
                "  w{:>2}({:>3.0}ms,run{:>3}): {:>+6.1}±{:<4.1}",
                win,
                win as f32 * hop_ms,
                run,
                m,
                j
            ),
            None => print!("  w{win:>2}: {:>18}", "--"),
        }
    }
    println!();
}

/// **Out-of-range test.** For each capture, place the strobe reference a known
/// `Δ` Hz below the string's true pitch and read the band-slope back. It should
/// track the true detuning up to ≈ [`ALIAS_HZ`], then break (alias) — the case
/// the regime-aware D4 routing must guard against.
fn alias_sweep(dir: &Path, planner: &mut RealFftPlanner<f32>) {
    println!("Band-slope vs reference offset — 'what happens out of tune'.");
    println!("readable range = ±{ALIAS_HZ:.1} Hz (hop/unwrap limit). Δ = string − reference.\n");
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname);
    let Some(key) = key else { return };
    let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
        return;
    };
    let f_et = NOTES[key as usize].frequency;
    let Some(f_true) = dtft_truth(&signal, f_et, planner) else {
        return;
    };
    println!("{} f_true={f_true:.2} Hz", NOTES[key as usize].name);
    println!(
        "{:>7}  {:>10}  {:>12}  {:>8}",
        "Δ(Hz)", "true¢", "band-read¢", "error¢"
    );
    for d_hz in [0.0f32, 5.0, 10.0, 15.0, 18.0, 21.0, 24.0, 30.0, 40.0] {
        let f_ref = f_true - d_hz;
        let true_cents = 1200.0 * (f_true / f_ref).log2();
        match band_slope_cents(&signal, f_ref) {
            Some((read, _)) => {
                let err = read - true_cents;
                let flag = if d_hz > ALIAS_HZ {
                    " ← past limit"
                } else {
                    ""
                };
                println!("{d_hz:>7.0}  {true_cents:>+10.1}  {read:>+12.1}  {err:>+8.1}{flag}");
            }
            None => println!(
                "{d_hz:>7.0}  {true_cents:>+10.1}  {:>12}  {:>8}",
                "--", "--"
            ),
        }
    }
    println!();
}

// ─── Three-way readout comparison (Prompt N) ─────────────────────────────────
//
// The prompt's decision experiment: **tracker as-is** vs **tracker with the
// Defect-1 register window** vs **bounded spectral peak + jacobsen**, scored on
// availability (fraction of hops yielding any value — the treble's real
// limit), accuracy vs `truth`, and jitter. Availability is measured over the
// note's WHOLE life, not the settled tail: a fast-decaying treble note is
// already dead by the tail, so a tail-only measurement scores its availability
// as 0 for reasons that have nothing to do with the estimator.

/// **The three-way comparison.** Per capture, every candidate readout on the
/// same audio and the same reference, so the columns are directly comparable.
fn readout_compare(dir: &Path, planner: &mut RealFftPlanner<f32>, span_cents: f32, min_bins: f32) {
    let Some(key) = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)
    else {
        return;
    };
    let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
        return;
    };
    if signal.len() < BASS_WINDOW_SIZE {
        return;
    }
    let f_et = NOTES[key as usize].frequency;
    let noise_floor = 0.001;
    // Every method is seeded/referenced at the SAME place the live app would
    // have it: the ET target of the key the user selected.
    let seed = f_et;
    let truth_c = dtft_truth(&signal, f_et, planner).map(|f| cents(f, f_et));

    // The tracker seeded at the string's ACTUAL pitch. The ET-seeded rows can
    // fail two different ways — the seed is too far off to unwrap (aliasing,
    // an accuracy failure) or the partial is too weak/short to gate through (an
    // availability failure). Only a perfectly-seeded tracker separates them:
    // whatever it still cannot do is not a seeding problem. (The live engine
    // seeds from Stage B's refined scale, which lands between these two rows.)
    let ideal_seed = dtft_truth(&signal, f_et, planner).unwrap_or(seed);

    let win = register_window(seed);
    let rows: [(&str, Vec<Option<f32>>); 6] = [
        ("trk1024", tracker_series(&signal, seed, 1024, noise_floor)),
        ("trk4096", tracker_series(&signal, seed, 4096, noise_floor)),
        (
            if win == 4096 { "trkFIX*" } else { "trkFIX " },
            tracker_series(&signal, seed, win, noise_floor),
        ),
        (
            "trkTRU",
            tracker_series(&signal, ideal_seed, win, noise_floor),
        ),
        (
            "pk2048",
            spectral_series(
                &signal,
                seed,
                seed,
                2048,
                noise_floor,
                span_cents,
                min_bins,
                Gate::Ambient,
                planner,
            )
            .0,
        ),
        (
            "pk8192",
            spectral_series(
                &signal,
                seed,
                seed,
                8192,
                noise_floor,
                span_cents,
                min_bins,
                Gate::Ambient,
                planner,
            )
            .0,
        ),
    ];

    print!("{:<4} ", NOTES[key as usize].name);
    match truth_c {
        Some(t) => print!("true {t:>+7.1}¢ |"),
        None => print!("true      --  |"),
    }
    for (name, series) in &rows {
        let s = score(series, f_et);
        let err = truth_c.map(|t| s.median_cents - t).unwrap_or(f32::NAN);
        if s.median_cents.is_finite() {
            print!(
                "  {name} av{:>3.0}/{:>3.0}% e{:>+6.1} j{:>5.1}",
                s.avail * 100.0,
                s.avail_early * 100.0,
                err,
                s.jitter
            );
        } else {
            print!("  {name} av{:>3.0}/{:>3.0}%       --      ", 0.0, 0.0);
        }
    }
    println!();
}

// ─── Partial-centered bass read (the deep-bass question) ─────────────────────
//
// Every n = 1 method is junk below ≈ E1: the fundamental is not acoustically
// present, and even a 32k-sample DFT disagrees with itself across repeat
// captures of the same key. But the string's mistuning is observable on ANY
// partial, exactly: f_n = n·f₀·√(1+Bn²) is linear in f₀, so scaling the string
// by x cents scales every partial by x cents. Partial-relative cents IS
// string-relative cents, with no B correction at display time. This mode asks
// whether a bounded search centered on a *strong* partial reads cleanly where
// the fundamental cannot.

/// Pre-registered success criterion, fixed before the first run so the result
/// is a verdict rather than a curve-fit: at least one partial n ∈ 2..=6 must
/// reach **≥ 90 % availability**, **|median − truth_n| ≤ 2 ¢**, and
/// **jitter ≤ 10 ¢** on the deep-bass keys.
const BASS_PASS_AVAIL: f32 = 0.90;
const BASS_PASS_ERR_CENTS: f32 = 2.0;
const BASS_PASS_JITTER_CENTS: f32 = 10.0;

/// Per-partial reading of one capture. Returns whether any partial in
/// `2..=MAX_BASS_PARTIAL` met the pre-registered criterion.
fn bass_partials(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
) -> Option<bool> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)?;
    let signal = crate::raw::read(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    // The prior B is what a cold-start app has: no capture has been measured
    // for this key yet, so the reference partials come from the Rigaud prior.
    let b = get_expected_beta(key);

    // One reference set, all partials, in the shipped convention and installed
    // exactly as the live app does.
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let count = strobe_refs(f_et, b, MAX_BASS_PARTIAL, &mut refs);
    let angles = strobe_angles(&signal, &refs, count);

    println!(
        "{:<4} key {:>2}  f_ET={:>9.3}  B_prior={:.3e}  ({} partials)",
        NOTES[key as usize].name, key, f_et, b, count
    );

    let mut any_pass = false;
    for n in 1..=count {
        let r_n = refs[n - 1];
        let truth_n = dtft_truth(&signal, r_n, planner).map(|f| cents(f, r_n));
        let (series, no_ref, med_ref) = spectral_series(
            &signal, r_n, f_et, // spacing ≈ f₀ — NOT r_n (the n>1 trap)
            8192, 0.001, span_cents, min_bins, gate, planner,
        );
        let s = score(&series, r_n);
        let err = truth_n.map(|t| s.median_cents - t);
        let band = slope_from_angles(&angles[n - 1], r_n, BAND_WIN_HOPS);

        // The criterion applies to partials above the fundamental only: n = 1
        // is the thing already known not to work down here.
        let pass = n >= 2
            && s.avail >= BASS_PASS_AVAIL
            && err.is_some_and(|e| e.abs() <= BASS_PASS_ERR_CENTS)
            && s.jitter <= BASS_PASS_JITTER_CENTS;
        any_pass |= pass;

        print!("   n={n} r={r_n:>9.2}  ");
        match truth_n {
            Some(t) => print!("truth {t:>+7.1}¢  "),
            None => print!("truth      --   "),
        }
        print!("pk8192 av{:>4.0}% ", s.avail * 100.0);
        match err {
            Some(e) if s.median_cents.is_finite() => print!("e{e:>+7.1} j{:>6.1}", s.jitter),
            _ => print!("e     -- j    --"),
        }
        if med_ref > 0 || no_ref > 0 {
            print!(
                "  [ref {med_ref:>3} bins, no-ref {:>3.0}%]",
                100.0 * no_ref as f32 / series.len().max(1) as f32
            );
        }
        match band {
            Some((m, j, run)) => print!("  band {m:>+7.1}¢ ±{j:<5.1} run{run:>4}"),
            None => print!("  band        --            "),
        }
        if pass {
            print!("  ✓PASS");
        }
        println!();
    }
    Some(any_pass)
}

/// **Measurement A — fixed n\* vs strongest-partial-per-hop.**
///
/// Both policies exploit the equal-cents identity: because `fₙ = n·f₀·√(1+Bn²)`
/// is linear in f₀, scaling the string by x cents scales every partial by
/// exactly x cents, so *any* partial's deviation from its own reference is the
/// string's deviation. The policies differ only in which partial supplies it.
///
/// The strongest policy picks, each hop, the admitted partial with the largest
/// CFAR margin (comparable across partials — each is normalized by its own
/// local noise). Reported per policy: availability, median cents, jitter; and
/// for the strongest policy the **switch rate** — the fraction of consecutive
/// admitted hops that changed partial, each of which steps the displayed
/// number by the reference error `866·ΔB·(n₂²−n₁²)` if B is imperfect.
#[allow(clippy::too_many_arguments)]
fn partial_policy(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    gate: Gate,
    use_measured_b: bool,
) -> Option<()> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)?;
    let signal = crate::raw::read(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let b_prior = get_expected_beta(key);

    // Optionally re-derive B from the capture's own partial truths, so the
    // references are right and the partial comparison is not confounded by the
    // prior's error (which the earlier run fitted at ≈ 4.7× on this piano).
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let n_refs = strobe_refs(f_et, b_prior, MAX_BASS_PARTIAL, &mut refs);
    let b_used = if use_measured_b {
        let mut obs: Vec<(usize, f32)> = Vec::new();
        for (i, r) in refs.iter().take(n_refs).enumerate() {
            if let Some(f) = dtft_truth(&signal, *r, planner) {
                obs.push((i + 1, f));
            }
        }
        fit_f0_b(&obs).map(|(_, b)| b).unwrap_or(b_prior)
    } else {
        b_prior
    };
    let count = strobe_refs(f_et, b_used, MAX_BASS_PARTIAL, &mut refs);

    let n_star = curves::default_display_partials()[key as usize] as usize;
    let series = multi_partial_series(
        &signal, &refs, count, f_et, 8192, 0.001, span_cents, min_bins, gate, planner,
    );

    // Truth in string-relative cents: any partial's own deviation works, so
    // take the median across partials of cents(truth_n, r_n).
    let mut truths: Vec<f32> = (0..count)
        .filter_map(|i| dtft_truth(&signal, refs[i], planner).map(|f| cents(f, refs[i])))
        .collect();
    truths.sort_by(f32::total_cmp);
    let truth_c = truths.get(truths.len() / 2).copied();

    // Fixed n* policy.
    let fixed: Vec<Option<f32>> = series
        .iter()
        .map(|hop| {
            hop.get(n_star - 1)
                .and_then(|(f, _)| f.map(|f| cents(f, refs[n_star - 1])))
        })
        .collect();

    // Strongest-margin policy, tracking partial switches AND the cents step
    // each switch puts on screen — the user-visible cost, whose analytic form
    // is 866·ΔB·(n₂²−n₁²) when the reference B is imperfect. A switch rate
    // alone cannot say whether switching is harmless or a visible jump.
    let mut strongest: Vec<Option<f32>> = Vec::with_capacity(series.len());
    let mut switches = 0usize;
    let mut admitted = 0usize;
    let mut steps: Vec<f32> = Vec::new();
    let mut prev_pick: Option<usize> = None;
    let mut prev_cents: Option<f32> = None;
    for hop in &series {
        let pick = hop
            .iter()
            .enumerate()
            .filter(|(_, (f, _))| f.is_some())
            .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
            .map(|(i, _)| i);
        match pick {
            Some(i) => {
                admitted += 1;
                let c = hop[i].0.map(|f| cents(f, refs[i]));
                if let Some(p) = prev_pick
                    && p != i
                {
                    switches += 1;
                    if let (Some(a), Some(b)) = (prev_cents, c) {
                        steps.push((b - a).abs());
                    }
                }
                prev_pick = Some(i);
                prev_cents = c;
                strongest.push(c);
            }
            None => {
                prev_pick = None;
                prev_cents = None;
                strongest.push(None);
            }
        }
    }
    steps.sort_by(f32::total_cmp);
    let (med_step, max_step) = if steps.is_empty() {
        (f32::NAN, f32::NAN)
    } else {
        (steps[steps.len() / 2], *steps.last().unwrap())
    };

    let stat = |s: &[Option<f32>]| -> (f32, f32, f32) {
        let v: Vec<f32> = s
            .iter()
            .filter_map(|x| *x)
            .filter(|x| x.is_finite())
            .collect();
        let avail = v.len() as f32 / s.len().max(1) as f32;
        if v.is_empty() {
            return (avail, f32::NAN, f32::NAN);
        }
        let mut sorted = v.clone();
        sorted.sort_by(f32::total_cmp);
        let med = sorted[sorted.len() / 2];
        let mean = v.iter().sum::<f32>() / v.len() as f32;
        let jit = (v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / v.len() as f32).sqrt();
        (avail, med, jit)
    };

    let (fa, fm, fj) = stat(&fixed);
    let (sa, sm, sj) = stat(&strongest);
    let e = |m: f32| truth_c.map(|t| m - t).unwrap_or(f32::NAN);

    print!(
        "{:<4} key {:>2}  B={:.3e}{}  n*={n_star}  ",
        NOTES[key as usize].name,
        key,
        b_used,
        if use_measured_b { "*" } else { " " }
    );
    match truth_c {
        Some(t) => print!("truth {t:>+7.1}¢ | "),
        None => print!("truth      --  | "),
    }
    println!(
        "fixed av{:>4.0}% e{:>+7.1} j{:>6.1} | strongest av{:>4.0}% e{:>+7.1} j{:>6.1} sw{:>5.1}% step med{:>6} max{:>6}",
        fa * 100.0,
        e(fm),
        fj,
        sa * 100.0,
        e(sm),
        sj,
        100.0 * switches as f32 / admitted.max(1) as f32,
        if med_step.is_finite() {
            format!("{med_step:.1}¢")
        } else {
            "--".into()
        },
        if max_step.is_finite() {
            format!("{max_step:.1}¢")
        } else {
            "--".into()
        }
    );
    Some(())
}

/// **Measurement B — per-partial scores with references built from `b`.**
/// Same shape as [`bass_partials`] but takes the B to use, so the fixed-n
/// table can be re-run with the capture's own measured B.
fn partial_table_row(
    dir: &Path,
    planner: &mut RealFftPlanner<f32>,
    span_cents: f32,
    min_bins: f32,
    use_measured_b: bool,
) -> Option<Vec<(usize, f32, f32, f32)>> {
    let key = dir
        .file_name()
        .and_then(|n| n.to_str())
        .and_then(crate::capture::key_from_dirname)?;
    let signal = crate::raw::read(&dir.join("audio.raw"))?;
    if signal.len() < BASS_WINDOW_SIZE {
        return None;
    }
    let f_et = NOTES[key as usize].frequency;
    let b_prior = get_expected_beta(key);
    let mut refs = [0.0f32; MAX_STROBE_REFS];
    let n0 = strobe_refs(f_et, b_prior, MAX_BASS_PARTIAL, &mut refs);
    let b_used = if use_measured_b {
        let mut obs: Vec<(usize, f32)> = Vec::new();
        for (i, r) in refs.iter().take(n0).enumerate() {
            if let Some(f) = dtft_truth(&signal, *r, planner) {
                obs.push((i + 1, f));
            }
        }
        fit_f0_b(&obs).map(|(_, b)| b).unwrap_or(b_prior)
    } else {
        b_prior
    };
    let count = strobe_refs(f_et, b_used, MAX_BASS_PARTIAL, &mut refs);

    let mut rows = Vec::new();
    for n in 1..=count {
        let r_n = refs[n - 1];
        let truth_n = dtft_truth(&signal, r_n, planner).map(|f| cents(f, r_n));
        let (series, _, _) = spectral_series(
            &signal,
            r_n,
            f_et,
            8192,
            0.001,
            span_cents,
            min_bins,
            Gate::Ambient,
            planner,
        );
        let s = score(&series, r_n);
        let err = truth_n.map(|t| s.median_cents - t).unwrap_or(f32::NAN);
        rows.push((n, s.avail, err, s.jitter));
    }
    Some(rows)
}

/// **Reference-offset reach.** The live substitute for detuning a real piano:
/// hold the capture fixed and move the *reference* instead. A reference `x` ¢
/// below the string is indistinguishable, to the bounded search, from a string
/// `x` ¢ above the reference — so this measures how far off pitch the coarse
/// read still works, on real audio, without touching an instrument.
///
/// Reports availability and |read − DFT truth| per offset. The band's own limit
/// is printed alongside: it hands over at `BAND_READABLE_HZ` = 18 Hz, which in
/// cents is ≈ 37200/f, so the coarse read only *adds* range where that is narrow.
/// **T6 — regime-switch chatter.** `main_view` shows the band-slope read while
/// `!gated && !out_of_range && band_cents.is_some()`, and the coarse read
/// otherwise. `out_of_range` is decided from the **coarse** read's cents,
/// converted to Hz at the displayed reference and compared with
/// `BAND_READABLE_HZ` — so near the boundary the decision is made by an
/// estimator whose own treble error is comparable to the 3.5 Hz margin it
/// protects, and the displayed *source* can flip hop to hop.
///
/// Sweeps the reference offset across each key's boundary and reports how often
/// the source changes between consecutive hops. Assumes the band is ungated and
/// filled, which is the worst case for chatter and the normal case in a treble
/// sustain; a `None` coarse read leaves `out_of_range` false and so shows the
/// band, exactly as shipped, and is counted as a source change too.
fn switch_chatter(caps: &[PathBuf], planner: &mut RealFftPlanner<f32>) {
    const BAND_READABLE_HZ: f32 = 18.0; // mirrors tuner-gui/src/views/main_view.rs
    println!(
        "Readout-source chatter at the band/coarse boundary. Offsets are relative to each\n\
         key's own boundary (1.0 = exactly at it). flip% = consecutive hops that changed\n\
         source; drop% = hops with no coarse read (which shows the band by default).\n"
    );
    println!(
        "{:>4} {:>5} {:>8} | {:>34} | {:>6}",
        "key", "note", "bound ¢", "flip% | aliased-hold% : none/8,8/1,8/2,8/4,8", "drop%"
    );

    let table = curves::default_display_partials();
    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::capture::key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
        let f_et = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        let n_coarse = curves::coarse_read_partial(key) as usize;
        let n_disp = table[key as usize] as usize;
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut refs);
        if n_coarse > count || n_disp > count {
            continue;
        }
        let coarse_center = refs[n_coarse - 1];
        let disp_center = refs[n_disp - 1];
        let spacing = f_et / (1.0 + b).sqrt();
        // The boundary in cents: the offset whose Hz-equivalent at the displayed
        // reference equals BAND_READABLE_HZ.
        let bound_c = 1200.0 * (1.0 + BAND_READABLE_HZ / disp_center).log2();

        let fftp = planner.plan_fft_forward(BASS_WINDOW_SIZE);
        let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
        let mut spec = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
        let mut mag = vec![0.0f32; BASS_WINDOW_SIZE / 2];
        let mut scratch = vec![0.0f32; BASS_WINDOW_SIZE / 2];

        // Verdicts are pooled over offsets straddling the boundary — the state
        // the chatter lives in. Each offset is replayed independently.
        // (hops to switch TO coarse, hops to switch BACK to the band)
        const VARIANTS: [(usize, usize); 5] = [(1, 1), (8, 8), (1, 8), (2, 8), (4, 8)];
        let mut flips = [0usize; VARIANTS.len()];
        let mut stale = [0usize; VARIANTS.len()];
        let mut pairs = 0usize;
        let (mut drops, mut hops_total) = (0usize, 0usize);
        for mult in [0.9f32, 1.0, 1.1] {
            let off = mult * bound_c;
            let c_off = coarse_center * 2f32.powf(-off / 1200.0);
            let d_off = disp_center * 2f32.powf(-off / 1200.0);
            let s_off = spacing * 2f32.powf(-off / 1200.0);
            let mut verdicts: Vec<bool> = Vec::new();
            let mut cursor = 0usize;
            while cursor + BASS_WINDOW_SIZE <= signal.len() {
                let end = cursor + BASS_WINDOW_SIZE;
                cursor += HOP_SIZE;
                fft(
                    &signal[end - BASS_WINDOW_SIZE..end],
                    &mut time,
                    &mut spec,
                    &fftp,
                    BASS_WINDOW_SIZE,
                );
                magnitude_spectrum(&spec, BASS_WINDOW_SIZE, &mut mag);
                let read = peaks::coarse_read(
                    &mag,
                    &spec,
                    BASS_WINDOW_SIZE,
                    SAMPLE_RATE,
                    c_off,
                    s_off,
                    &mut scratch,
                );
                hops_total += 1;
                // The shipped predicate, verbatim: no coarse read ⇒ in range.
                verdicts.push(match read {
                    Some(hz) => {
                        let cents_off = 1200.0 * (hz / c_off).log2();
                        (d_off * ((cents_off / 1200.0).exp2() - 1.0)).abs() >= BAND_READABLE_HZ
                    }
                    None => {
                        drops += 1;
                        false
                    }
                });
            }
            if verdicts.len() < 2 {
                continue;
            }
            pairs += verdicts.len() - 1;
            // Debounce, in three symmetries. `(m_out, m_back)` = hops of opposing
            // evidence needed to switch *to* coarse and back *to* the band.
            // Asymmetric variants matter because holding the band past the
            // boundary means displaying an aliased number, while holding the
            // coarse read merely means displaying a jitterier true one.
            for (i, &(m_out, m_back)) in VARIANTS.iter().enumerate() {
                let mut state = verdicts[0];
                let mut run = 0usize;
                for &v in &verdicts[1..] {
                    if v == state {
                        run = 0;
                    } else {
                        run += 1;
                        let need = if v { m_out } else { m_back };
                        if run >= need {
                            state = v;
                            run = 0;
                            flips[i] += 1;
                        }
                    }
                }
                // Exposure: hops displaying the band while the verdict says the
                // band is aliased — the cost the out-ward debounce buys.
                let mut state = verdicts[0];
                let mut run = 0usize;
                for &v in &verdicts[1..] {
                    if v == state {
                        run = 0;
                    } else {
                        run += 1;
                        let need = if v { m_out } else { m_back };
                        if run >= need {
                            state = v;
                            run = 0;
                        }
                    }
                    if v && !state {
                        stale[i] += 1;
                    }
                }
            }
        }
        if pairs == 0 {
            continue;
        }
        let pct = |i: usize| 100.0 * flips[i] as f32 / pairs as f32;
        let st = |i: usize| 100.0 * stale[i] as f32 / pairs as f32;
        println!(
            "{:>4} {:>5} {:>7.1} | {} | {:>5.0}",
            key,
            NOTES[key as usize].name,
            bound_c,
            (0..5)
                .map(|i| format!("{:>5.1}|{:<5.1}", pct(i), st(i)))
                .collect::<Vec<_>>()
                .join(" "),
            100.0 * drops as f32 / hops_total.max(1) as f32
        );
    }
}

fn reach_sweep(caps: &[PathBuf], planner: &mut RealFftPlanner<f32>) {
    println!(
        "Reference-offset reach — how far off pitch the coarse read still reads.\n\
              Offsetting the reference == detuning the string, on real capture audio.\n"
    );
    let offsets = [0.0f32, 10.0, 25.0, 50.0, 75.0, 100.0, 150.0];
    print!("{:>4} {:>5} {:>9} |", "key", "note", "band to");
    for o in offsets {
        print!(" {:>13}", format!("{o:.0} c"));
    }
    println!("\n{:->4} {:->5} {:->9} |{:->98}", "", "", "", "");

    for dir in caps {
        let Some(key) = dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(crate::capture::key_from_dirname)
        else {
            continue;
        };
        let Some(signal) = crate::raw::read(&dir.join("audio.raw")) else {
            continue;
        };
        if signal.len() < BASS_WINDOW_SIZE {
            continue;
        }
        let f_et = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        let n_star = curves::coarse_read_partial(key) as usize;
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        let count = strobe_refs(f_et, b, MAX_STROBE_REFS, &mut refs);
        if n_star > count {
            continue;
        }
        let center = refs[n_star - 1];
        let spacing = f_et / (1.0 + b).sqrt();
        let Some(truth) = dtft_truth(&signal, center, planner) else {
            continue;
        };
        let band_c = 1200.0 * ((center - 18.0).max(1.0) / center).log2();

        print!(
            "{:>4} {:>5} {:>8.0}c |",
            key, NOTES[key as usize].name, band_c
        );
        for &off in &offsets {
            // Reference moved DOWN by `off` cents == string `off` cents sharp of it.
            let c_off = center * 2f32.powf(-off / 1200.0);
            let s_off = spacing * 2f32.powf(-off / 1200.0);
            let fftp = planner.plan_fft_forward(BASS_WINDOW_SIZE);
            let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
            let mut spec = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
            let mut mag = vec![0.0f32; BASS_WINDOW_SIZE / 2];
            let mut scratch = vec![0.0f32; BASS_WINDOW_SIZE / 2];
            let (mut hits, mut hops, mut err) = (0usize, 0usize, Vec::new());
            let mut cursor = 0usize;
            while cursor + BASS_WINDOW_SIZE <= signal.len() {
                let end = cursor + BASS_WINDOW_SIZE;
                fft(
                    &signal[end - BASS_WINDOW_SIZE..end],
                    &mut time,
                    &mut spec,
                    &fftp,
                    BASS_WINDOW_SIZE,
                );
                magnitude_spectrum(&spec, BASS_WINDOW_SIZE, &mut mag);
                cursor += HOP_SIZE;
                hops += 1;
                if let Some(hz) = peaks::coarse_read(
                    &mag,
                    &spec,
                    BASS_WINDOW_SIZE,
                    SAMPLE_RATE,
                    c_off,
                    s_off,
                    &mut scratch,
                ) {
                    hits += 1;
                    err.push((1200.0 * (hz / truth).log2()).abs());
                }
            }
            if hits == 0 {
                print!("      --     ");
                continue;
            }
            err.sort_by(f32::total_cmp);
            print!(
                " {:>4.0}% {:>5.1}c",
                100.0 * hits as f32 / hops as f32,
                err[err.len() / 2]
            );
        }
        println!();
    }
}

/// **Q4 — does a readout survive a fast-moving string?** Synthesizes a note
/// whose f₀ glides at a known rate (turning a peg) and scores each method
/// against the known instantaneous truth **at the newest sample** — "what is
/// the string doing now", the only epoch a live readout is judged on.
///
/// Two effects separate in this table. Every method is centred on its own
/// analysis window, so it necessarily lags the newest sample by `win/2`
/// samples (1024 → 11.6 ms, 2048 → 23.2, 4096 → 46.4, 8192 → 92.9); against a
/// glide of `R` ¢/s that shows up as a floor of `R·win/(2·fs)` cents, and that
/// floor **is** the latency cost of the window (Prompt N open question 2).
/// Errors far above the floor are the second effect: the adaptive tracker's
/// EMA (α = 0.05, τ ≈ 0.46 s) losing the string, whereupon `|f_live − f_target|`
/// passes the ±21.5 Hz unwrap limit and the reading aliases. A fixed-reference
/// spectral search carries no such state and cannot fail that way.
fn detune_sweep(span_cents: f32, min_bins: f32, gate: Gate, planner: &mut RealFftPlanner<f32>) {
    println!("Fast-detune survival — synthetic glide from the reference, 2.0 s per run.");
    println!(
        "Error = median |read − true at the newest sample| (¢); (%) = availability.\n\
         Expected floor = window group delay × rate: at 100 ¢/s that is 1.2 ¢ (1024), \
         2.3 (2048), 4.6 (4096), 9.3 (8192).\n\
         Errors far above the floor = the adaptive tracker aliasing after its EMA lost the string.\n"
    );
    let secs = 2.0f32;
    let len = (SAMPLE_RATE as f32 * secs) as usize;
    let amps = [0.6f32, 1.0, 0.7, 0.45, 0.25];

    for &(key, name) in &[(19u8, "E2"), (43, "E4"), (48, "A4")] {
        let f_ref = NOTES[key as usize].frequency;
        println!("── {name} (ref {f_ref:.2} Hz) ──");
        println!(
            "{:>10}  {:>22}  {:>22}  {:>22}  {:>22}",
            "rate ¢/s", "trkFIX (adaptive)", "pk2048", "pk8192", "dual (tier-1)"
        );
        for &rate in &[0.0f32, 50.0, 100.0, 200.0, 400.0] {
            // f(t) = f_ref·2^(rate·t/1200); phase = ∫2π f dt integrated exactly.
            let k = rate / 1200.0;
            let signal: Vec<f32> = (0..len)
                .map(|i| {
                    let t = i as f32 / SAMPLE_RATE as f32;
                    let mut s = 0.0;
                    for (j, &a) in amps.iter().enumerate() {
                        let n = (j + 1) as f32;
                        // ∫₀ᵗ f_ref·2^(k·u) du = f_ref·(2^(k t) − 1)/(k·ln2)
                        let ph = if k.abs() < 1e-9 {
                            f_ref * t
                        } else {
                            f_ref * (2f32.powf(k * t) - 1.0) / (k * std::f32::consts::LN_2)
                        };
                        s += a * (TAU * n * ph).sin();
                    }
                    0.1 * s
                })
                .collect();

            let win = register_window(f_ref);
            let s8: Vec<Option<f32>> = spectral_series(
                &signal, f_ref, f_ref, 8192, 0.001, span_cents, min_bins, gate, planner,
            )
            .0;
            let s2: Vec<Option<f32>> = spectral_series(
                &signal, f_ref, f_ref, 2048, 0.001, span_cents, min_bins, gate, planner,
            )
            .0;
            // ── Tier-1 dual-window selection ──────────────────────────────
            // Both spectra are computed every hop anyway, so read both and
            // prefer 8192 when it is admitted, falling back to 2048. No
            // constants and no state: 8192's bins smear when the tone moves,
            // so its own availability collapse IS the motion signal. `churn`
            // counts hops whose source differs from the previous hop — the
            // mixed-source artifact, where a 93 ms-lagged read can sit beside
            // a 23 ms one.
            let mut dual: Vec<Option<f32>> = Vec::with_capacity(s8.len());
            let (mut churn, mut prev_src, mut picks) = (0usize, None::<u8>, 0usize);
            // Per-source error, so a surprising pooled median can be attributed:
            // an accounting bug would show each source matching its own column,
            // whereas a *selection* effect shows the 2048-sourced subset worse
            // than 2048's own column — those are exactly the hops where 8192
            // rejected, i.e. where the tone was smearing hardest.
            let (mut e8, mut e2): (Vec<f32>, Vec<f32>) = (Vec::new(), Vec::new());
            for (h, (a, b)) in s8.iter().zip(s2.iter()).enumerate() {
                let (v, src) = match (a, b) {
                    (Some(x), _) => (Some(*x), Some(0u8)),
                    (None, Some(y)) => (Some(*y), Some(1u8)),
                    (None, None) => (None, None),
                };
                if let (Some(f), Some(sc)) = (v, src) {
                    let t_now = (h * HOP_SIZE + BASS_WINDOW_SIZE) as f32 / SAMPLE_RATE as f32;
                    let f_true = f_ref * 2f32.powf(k * t_now);
                    let err = (cents(f, f_ref) - cents(f_true, f_ref)).abs();
                    if sc == 0 { e8.push(err) } else { e2.push(err) }
                }
                if let Some(sc) = src {
                    picks += 1;
                    if prev_src.is_some_and(|p| p != sc) {
                        churn += 1;
                    }
                    prev_src = Some(sc);
                } else {
                    prev_src = None;
                }
                dual.push(v);
            }
            let churn_pct = 100.0 * churn as f32 / picks.max(1) as f32;
            let med = |v: &mut Vec<f32>| -> f32 {
                if v.is_empty() {
                    return f32::NAN;
                }
                v.sort_by(f32::total_cmp);
                v[v.len() / 2]
            };
            let (n8, n2) = (e8.len(), e2.len());
            let (m8, m2) = (med(&mut e8), med(&mut e2));
            // Percentiles of the pooled dual errors: a pooled median far above
            // both source medians can only come from overlapping spreads, and
            // the spread is what a readout actually shows the user.
            let mut pooled: Vec<f32> = e8.iter().chain(e2.iter()).copied().collect();
            pooled.sort_by(f32::total_cmp);
            let pct = |v: &[f32], q: f32| -> f32 {
                if v.is_empty() {
                    return f32::NAN;
                }
                v[(((v.len() - 1) as f32) * q).round() as usize]
            };
            let (p10, p50, p90) = (pct(&pooled, 0.1), pct(&pooled, 0.5), pct(&pooled, 0.9));
            let (s2p10, s2p90) = (pct(&e2, 0.1), pct(&e2, 0.9));
            let series: [(&str, Vec<Option<f32>>); 4] = [
                ("trkFIX", tracker_series(&signal, f_ref, win, 0.001)),
                (
                    "pk2048",
                    spectral_series(
                        &signal, f_ref, f_ref, 2048, 0.001, span_cents, min_bins, gate, planner,
                    )
                    .0,
                ),
                ("pk8192", s8.clone()),
                ("dual", dual),
            ];
            print!("{rate:>10.0}");
            let _ = churn_pct;
            for (_, s) in &series {
                // Truth at the hop's window centre — the estimate's own epoch.
                let mut errs: Vec<f32> = Vec::new();
                let mut hits = 0usize;
                for (h, v) in s.iter().enumerate() {
                    let Some(f) = v else { continue };
                    hits += 1;
                    // The newest sample in this hop's COLA window — every
                    // method saw exactly this much audio, so scoring here
                    // charges each its own group delay rather than a shared one.
                    let t_now = (h * HOP_SIZE + BASS_WINDOW_SIZE) as f32 / SAMPLE_RATE as f32;
                    let f_true = f_ref * 2f32.powf(k * t_now);
                    errs.push((cents(*f, f_ref) - cents(f_true, f_ref)).abs());
                }
                if errs.is_empty() {
                    print!("  {:>22}", "-- (0%)");
                } else {
                    errs.sort_by(f32::total_cmp);
                    let med = errs[errs.len() / 2];
                    print!(
                        "  {:>13}{:>9}",
                        format!("{med:.1}¢"),
                        format!("({:.0}%)", 100.0 * hits as f32 / s.len() as f32)
                    );
                }
            }
            println!(
                "   churn {churn_pct:>4.0}%  src: 8192 {n8}@{}  2048 {n2}@{}  \
                 2048spread[{}..{}]  pooled p10/p50/p90 {}/{}/{}",
                if m8.is_finite() {
                    format!("{m8:.1}")
                } else {
                    "--".into()
                },
                if m2.is_finite() {
                    format!("{m2:.1}")
                } else {
                    "--".into()
                },
                if s2p10.is_finite() {
                    format!("{s2p10:.1}")
                } else {
                    "--".into()
                },
                if s2p90.is_finite() {
                    format!("{s2p90:.1}")
                } else {
                    "--".into()
                },
                if p10.is_finite() {
                    format!("{p10:.1}")
                } else {
                    "--".into()
                },
                if p50.is_finite() {
                    format!("{p50:.1}")
                } else {
                    "--".into()
                },
                if p90.is_finite() {
                    format!("{p90:.1}")
                } else {
                    "--".into()
                },
            );
        }
        println!();
    }
}

// ── The ambient-σ gates, measured (ADR 0015) ────────────────────────────────

// ── Mode entry points ────────────────────────────────────────────────────────
//
// Each reproduces one of the harness's modes: the same header, the same
// population, the same call.

use anyhow::Result;

use crate::capture;
use crate::strobe::ReadOpts;

/// The captures a readout mode runs over, honouring `--keys`.
fn population(o: &ReadOpts) -> Result<Vec<PathBuf>> {
    let mut caps = capture::find_or_single(&o.root);
    if let Some(keys) = &o.keys {
        capture::retain_keys(&mut caps, keys);
    }
    if caps.is_empty() {
        anyhow::bail!("No captures found under {}", o.root.display());
    }
    Ok(caps)
}

pub(super) fn truth(o: &ReadOpts) -> Result<()> {
    println!(
        "Pitch ground-truth audit — app (our hot path) vs truth (hi-res DFT) vs yin (autocorr).\n\
         cents shown are vs equal temperament (f_ET). Negative = flat.\n\
         The decisive column is app−truth; yin is the 'other tuners' family.\n"
    );
    let caps = population(o)?;
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &caps {
        process_capture(cap, &mut planner);
    }
    println!("\n{} capture(s).", caps.len());
    Ok(())
}

pub(super) fn selftest_mode() -> Result<()> {
    selftest(&mut RealFftPlanner::<f32>::new());
    Ok(())
}

pub(super) fn inharm_mode() -> Result<()> {
    inharm_sweep();
    Ok(())
}

pub(super) fn detune_mode(o: &ReadOpts, flank_hz: Option<f32>) -> Result<()> {
    detune_sweep(
        o.span,
        o.min_bins,
        shipping_gate_hz(flank_hz.unwrap_or(FLANK_MIN_HZ)),
        &mut RealFftPlanner::<f32>::new(),
    );
    Ok(())
}

pub(super) fn alias(o: &ReadOpts) -> Result<()> {
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &population(o)? {
        alias_sweep(cap, &mut planner);
    }
    Ok(())
}

pub(super) fn window(o: &ReadOpts) -> Result<()> {
    println!("Fit-window sweep — longest ungated run and the band read per window length.\n");
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &population(o)? {
        window_sweep(cap, &mut planner);
    }
    Ok(())
}

pub(super) fn readout(o: &ReadOpts) -> Result<()> {
    let span_cents = o.span;
    let min_bins = o.min_bins;
    println!(
        "Three-way readout comparison — tracker as-is / tracker + Defect-1 window / \
         bounded spectral peak.\n\
         av = availability over the whole note / over its first third (%); \
         e = median reading − truth (¢); j = jitter (¢).\n\
         * marks a key where the register rule selects the long tracker window. \
         Search band ±{span_cents:.0} ¢, floor {min_bins:.0} bins.\n"
    );
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &population(o)? {
        readout_compare(cap, &mut planner, span_cents, min_bins);
    }
    Ok(())
}

pub(super) fn chatter(o: &ReadOpts) -> Result<()> {
    println!("T6 — does the band/coarse regime switch chatter near its boundary?\n");
    switch_chatter(&population(o)?, &mut RealFftPlanner::<f32>::new());
    Ok(())
}

pub(super) fn policy(o: &ReadOpts, measured_b: bool) -> Result<()> {
    println!(
        "Partial-selection policy — fixed n* (register table) vs strongest-margin-per-hop.\n\
         Both read the same 8192 spectra; cents are string-relative via the equal-cents \
         identity. References use {} B.\n\
         switch%% = fraction of consecutive admitted hops that changed partial (each one \
         steps the displayed number when B is imperfect).\n",
        if measured_b {
            "the capture's own MEASURED"
        } else {
            "the Rigaud prior"
        }
    );
    let gate = shipping_gate();
    let mut planner = RealFftPlanner::<f32>::new();
    for cap in &population(o)? {
        partial_policy(cap, &mut planner, o.span, o.min_bins, gate, measured_b);
    }
    Ok(())
}

pub(super) fn fixed_n(o: &ReadOpts) -> Result<()> {
    println!(
        "Fixed-n table with the capture's own MEASURED B (fitted from the partial truths) \
         vs the Rigaud prior.\n\
         Does the D5 register table's deep-bass n* = 6 clean up once its reference is right?\n"
    );
    let caps = population(o)?;
    let mut planner = RealFftPlanner::<f32>::new();
    let mut agg: Vec<Vec<(f32, f32)>> = vec![Vec::new(); MAX_BASS_PARTIAL];
    for cap in &caps {
        for use_meas in [false, true] {
            if let Some(rows) = partial_table_row(cap, &mut planner, o.span, o.min_bins, use_meas) {
                for (n, _av, e, j) in rows {
                    if use_meas && e.is_finite() {
                        agg[n - 1].push((e.abs(), j));
                    }
                }
            }
        }
    }
    println!("  MEASURED-B aggregate over {} captures:", caps.len());
    for (i, v) in agg.iter().enumerate() {
        if v.is_empty() {
            continue;
        }
        let mut es: Vec<f32> = v.iter().map(|x| x.0).collect();
        let mut js: Vec<f32> = v.iter().map(|x| x.1).collect();
        es.sort_by(f32::total_cmp);
        js.sort_by(f32::total_cmp);
        println!(
            "    n={}  median|e| {:>6.2}¢   median j {:>7.2}¢   ({} rows)",
            i + 1,
            es[es.len() / 2],
            js[js.len() / 2],
            v.len()
        );
    }
    Ok(())
}

pub(super) fn bass_partials_mode(o: &ReadOpts) -> Result<()> {
    println!(
        "Partial-centered bass read — 8192 bounded search at each partial's PRIOR-B target.\n\
         Reference partials installed as ONE strobe ref set (the live path). Search band \
         ±{:.0} ¢, floor {:.0} bins, neighbour cap f₀/2.\n\
         Pre-registered criterion (fixed before the run): some n ≥ 2 with availability \
         ≥ {:.0} %, |median − truth| ≤ {:.0} ¢, jitter ≤ {:.0} ¢.\n",
        o.span,
        o.min_bins,
        BASS_PASS_AVAIL * 100.0,
        BASS_PASS_ERR_CENTS,
        BASS_PASS_JITTER_CENTS
    );
    let mut planner = RealFftPlanner::<f32>::new();
    let mut passed = 0usize;
    let mut total = 0usize;
    for cap in &population(o)? {
        if let Some(p) = bass_partials(cap, &mut planner, o.span, o.min_bins, Gate::Ambient) {
            total += 1;
            passed += usize::from(p);
        }
    }
    println!(
        "\nCRITERION: {passed}/{total} captures had at least one partial n ≥ 2 meeting all three bars."
    );
    Ok(())
}

pub(super) fn reach(o: &ReadOpts) -> Result<()> {
    reach_sweep(&population(o)?, &mut RealFftPlanner::<f32>::new());
    Ok(())
}
