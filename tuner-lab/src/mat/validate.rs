//! # MAT validation harness
//!
//! Re-runs the Worker's Median-Adjustive Trajectories (f0, B) estimator over the real
//! captures in `diagnostics/key_*/` and reports the measured inharmonicity per key against
//! the Rigaud prior. The FFT path mirrors `worker::process_payload`: largest power-of-two
//! window ≤ the stable sample count, Hann window, magnitude spectrum.
//!
//! `--offset-ms <list>` runs a second experiment instead: the same estimator over
//! same-length windows cut from `audio_full_event.raw` at several offsets from the
//! *physical* onset. The shipped capture begins at the gatekeeper's `Stable` verdict
//! (~116 ms), so the loudest part of the note is never measured; the literature's
//! reason for skipping it is that the attack's frequencies are unsettled and its
//! energy would drag the peak positions the B fit reads. This prices that, paired
//! per capture and read against the same set's repeat scatter (ADR 0009).
//!
//! Usage:
//!   cargo lab mat validate [diagnostics_dir]   (default: diagnostics)
//!   cargo lab mat offset diagnostics_piano2 --offsets 0,116,300

use anyhow::{Context, Result};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use tuner_core::algorithms::mat::{MAX_PARTIALS, MatOrder, detect_pitch_mat};
use tuner_core::algorithms::spectral::{cspe, fft, magnitude_spectrum};
use tuner_core::models::{NOTES, get_expected_beta};

/// Largest power of two ≤ `n` (matches the Worker's FFT sizing).
fn largest_pow2_le(n: usize) -> usize {
    1usize << (usize::BITS - 1 - n.max(1).leading_zeros())
}

/// Window for the offset sweep. 32768 samples (0.74 s), not the shipped 65536,
/// because the full-event dumps hold only ~1.15 s of note after the pre-roll.
/// The attack's share of the window is therefore twice what it is in production
/// — a conservative test against admitting it.
const OFFSET_FFT_SIZE: usize = 32768;

/// Where the shipped capture starts: the gatekeeper's `Stable` verdict, five
/// hops after the NHWRSF onset. Offsets are reported against it.
const SHIPPED_OFFSET_MS: u32 = 116;

/// Physical onset: the first 1 ms frame whose RMS clears both −20 dB re the
/// event's peak and 4× the pre-roll ambient. Deliberately independent of the
/// gatekeeper — the gate's own verdict is the quantity under test.
fn find_onset(x: &[f32], sample_rate: u32) -> Option<usize> {
    let (w, h) = (sample_rate as usize / 1000, sample_rate as usize / 2000);
    if x.len() <= w || h == 0 {
        return None;
    }
    let n = (x.len() - w) / h;
    let rms: Vec<f32> = (0..n)
        .map(|i| (x[i * h..i * h + w].iter().map(|v| v * v).sum::<f32>() / w as f32).sqrt())
        .collect();
    if rms.is_empty() {
        return None;
    }
    let amb_n = (0.25 * sample_rate as f32 / h as f32) as usize;
    let mut amb: Vec<f32> = rms[..amb_n.min(rms.len())].to_vec();
    amb.sort_by(f32::total_cmp);
    let amb = amb[amb.len() / 2];
    let (peak_i, peak) = rms
        .iter()
        .enumerate()
        .fold((0, 0.0f32), |m, (i, &v)| if v > m.1 { (i, v) } else { m });
    let thr = (0.1 * peak).max(4.0 * amb);
    (0..=peak_i).find(|&i| rms[i] > thr).map(|i| i * h)
}

/// One capture's fitted B and located-partial count at each swept offset;
/// `None` where the window ran past the record or MAT returned no usable fit.
struct OffsetRow {
    key: u8,
    by_offset: Vec<Option<(f32, usize)>>,
}

/// Median of `v`, sorting it in place. Callers guarantee non-empty.
fn median(v: &mut [f32]) -> f32 {
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Sample SD, or `None` below three points — the repeat scatter needs enough
/// captures of one key to mean anything.
fn stdev(v: &[f32]) -> Option<f32> {
    if v.len() < 3 {
        return None;
    }
    let n = v.len() as f32;
    let mean = v.iter().sum::<f32>() / n;
    Some((v.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / (n - 1.0)).sqrt())
}

fn offset_sweep(dirs: &[PathBuf], offsets: &[u32]) -> Result<()> {
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(OFFSET_FFT_SIZE);
    let mut time_buffer = vec![0.0f32; OFFSET_FFT_SIZE];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; OFFSET_FFT_SIZE / 2 + 1];
    let mut freq_buffer_shifted = vec![Complex { re: 0.0, im: 0.0 }; OFFSET_FFT_SIZE / 2 + 1];
    let mut magnitudes = vec![0.0f32; OFFSET_FFT_SIZE / 2];
    let mut cspe_map = vec![0.0f32; OFFSET_FFT_SIZE / 2];

    let mut rows: Vec<OffsetRow> = Vec::new();
    let mut skipped = 0u32;
    for dir in dirs {
        let (Some(audio), Ok(text)) = (
            crate::raw::full_event(dir),
            fs::read_to_string(dir.join("analysis.json")),
        ) else {
            skipped += 1;
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            skipped += 1;
            continue;
        };
        let Some(key) = json["metadata"]["key_index"].as_u64() else {
            skipped += 1;
            continue;
        };
        let key = key as u8;
        let sample_rate = json["metadata"]["sample_rate"].as_u64().unwrap_or(44100) as u32;
        let Some(onset) = find_onset(&audio, sample_rate) else {
            skipped += 1;
            continue;
        };
        // The ET seed, not `measured_f0`: one seed has to serve every offset,
        // and `measured_f0` is itself a product of the shipped window.
        let seed = NOTES[key as usize].frequency;

        let mut by_offset = Vec::with_capacity(offsets.len());
        for &off in offsets {
            let start = onset + (off as usize * sample_rate as usize) / 1000;
            if start + OFFSET_FFT_SIZE + 1 > audio.len() {
                by_offset.push(None);
                continue;
            }
            let seg = &audio[start..start + OFFSET_FFT_SIZE + 1];
            fft(
                &seg[..OFFSET_FFT_SIZE],
                &mut time_buffer,
                &mut freq_buffer,
                &r2c,
                OFFSET_FFT_SIZE,
            );
            magnitude_spectrum(&freq_buffer, OFFSET_FFT_SIZE, &mut magnitudes);
            fft(
                &seg[1..],
                &mut time_buffer,
                &mut freq_buffer_shifted,
                &r2c,
                OFFSET_FFT_SIZE,
            );
            cspe(
                &freq_buffer,
                &freq_buffer_shifted,
                OFFSET_FFT_SIZE,
                sample_rate,
                &mut cspe_map,
            );
            let mut freqs = [0.0f32; MAX_PARTIALS];
            let mut ns = [0u32; MAX_PARTIALS];
            by_offset.push(
                detect_pitch_mat(
                    &magnitudes,
                    &cspe_map,
                    sample_rate,
                    seed,
                    MatOrder::Serial,
                    &mut freqs,
                    &mut ns,
                )
                // Non-positive B carries no log ratio; deep-bass fits do land
                // there, so they are dropped and counted, not clamped.
                .filter(|e| e.b > 0.0)
                .map(|e| (e.b, e.partial_count)),
            );
        }
        rows.push(OffsetRow { key, by_offset });
    }

    offset_report(&rows, offsets, skipped);
    Ok(())
}

fn offset_report(rows: &[OffsetRow], offsets: &[u32], skipped: u32) {
    let ref_i = offsets
        .iter()
        .position(|&o| o == SHIPPED_OFFSET_MS)
        .unwrap_or(0);
    let ref_ms = offsets[ref_i];

    println!("── MAT against where the analysis window starts ──");
    println!(
        "   {} captures ({skipped} skipped: no full-event dump, no key, or no onset), each\n   \
         re-measured on a {OFFSET_FFT_SIZE}-sample window cut at each offset from the physical\n   \
         onset. ΔB is paired against the {ref_ms} ms reference — the gatekeeper's Stable\n   \
         verdict, where the shipped capture begins. Read ΔB against the repeat scatter\n   \
         printed below it: a shift smaller than the spread between two captures of the same\n   \
         key is not a shift.",
        rows.len()
    );
    println!(
        "\n  {:<8} {:>8} {:>9} {:>12} {:>9} {:>10} {:>10}",
        "register", "offset", "captures", "median ΔB %", "IQR %", "|ΔB|>5 %", "partials"
    );
    for reg in crate::capture::CURVE_REGISTERS {
        for (oi, &off) in offsets.iter().enumerate() {
            let mut shifts = Vec::new();
            let mut partials = Vec::new();
            for r in rows
                .iter()
                .filter(|r| crate::capture::curve_register(r.key) == reg)
            {
                let (Some((b, p)), Some((b_ref, _))) = (r.by_offset[oi], r.by_offset[ref_i]) else {
                    continue;
                };
                shifts.push(100.0 * (b / b_ref - 1.0));
                partials.push(p as f32);
            }
            if shifts.is_empty() {
                continue;
            }
            let n = shifts.len();
            let big = 100.0 * shifts.iter().filter(|s| s.abs() > 5.0).count() as f32 / n as f32;
            let mut sorted = shifts.clone();
            sorted.sort_by(f32::total_cmp);
            let iqr = sorted[(0.75 * n as f32) as usize % n] - sorted[(0.25 * n as f32) as usize];
            let tag = if oi == ref_i {
                format!("{off} ms *")
            } else {
                format!("{off} ms")
            };
            println!(
                "  {:<8} {:>8} {:>9} {:>12.2} {:>9.2} {:>9.1}% {:>10.1}",
                reg,
                tag,
                n,
                median(&mut shifts),
                iqr,
                big,
                median(&mut partials)
            );
        }
    }
    println!("  * the reference offset: its own row is ΔB against itself, and prices nothing.");

    // Repeat scatter at the reference offset — ADR 0009's yardstick, recomputed
    // here so the comparison is against this set rather than a quoted figure.
    let mut per_key: BTreeMap<u8, Vec<f32>> = BTreeMap::new();
    for r in rows {
        if let Some((b, _)) = r.by_offset[ref_i] {
            per_key.entry(r.key).or_default().push(b.ln());
        }
    }
    println!(
        "\n  repeat scatter of ln B at {ref_ms} ms (same key, different captures, keys with ≥3):"
    );
    for reg in crate::capture::CURVE_REGISTERS {
        let mut sds: Vec<f32> = per_key
            .iter()
            .filter(|(k, _)| crate::capture::curve_register(**k) == reg)
            .filter_map(|(_, v)| stdev(v))
            .collect();
        if sds.is_empty() {
            continue;
        }
        println!(
            "    {:<8} {:>6.2} %   over {} keys",
            reg,
            100.0 * median(&mut sds),
            sds.len()
        );
    }
}

/// One MAT order's outcome for a key, including the fitted model and its located partials.
struct ModeResult {
    b: Option<f32>,
    f0: f32,
    confidence: f32,
    partials: usize,
    /// Located partial frequencies (Hz) and indices, for goodness-of-fit cross-checks.
    pf: Vec<f32>,
    pn: Vec<u32>,
}

fn run_mode(mags: &[f32], cspe: &[f32], sr: u32, seed: f32, order: MatOrder) -> ModeResult {
    let mut freqs = [0.0f32; MAX_PARTIALS];
    let mut ns = [0u32; MAX_PARTIALS];
    match detect_pitch_mat(mags, cspe, sr, seed, order, &mut freqs, &mut ns) {
        Some(e) => ModeResult {
            b: Some(e.b),
            f0: e.f0,
            confidence: e.confidence,
            partials: e.partial_count,
            pf: freqs[..e.partial_count].to_vec(),
            pn: ns[..e.partial_count].to_vec(),
        },
        None => ModeResult {
            b: None,
            f0: seed,
            confidence: 0.0,
            partials: 0,
            pf: Vec::new(),
            pn: Vec::new(),
        },
    }
}

/// RMS relative residual of a fitted `(f0, B)` model against a set of measured partials:
/// how well the inharmonic series `n·f0·√(1+B·n²)` reproduces the located peak frequencies.
/// Lower = the model explains those partials better. Ground-truth-free goodness of fit.
fn fit_residual(f0: f32, b: f32, freqs: &[f32], ns: &[u32]) -> Option<f32> {
    let mut sumsq = 0.0_f32;
    let mut count = 0_u32;
    for (&f, &n) in freqs.iter().zip(ns) {
        let n_f = n as f32;
        let predicted = n_f * f0 * (1.0 + b * n_f * n_f).max(0.0).sqrt();
        if predicted > 0.0 {
            let rel = (f - predicted) / predicted;
            sumsq += rel * rel;
            count += 1;
        }
    }
    (count > 0).then(|| (sumsq / count as f32).sqrt())
}

struct KeyRow {
    key_index: u8,
    name: String,
    seed: f32,
    prior_b: f32,
    sim: ModeResult,
    ser: ModeResult,
}

fn process_capture(dir: &Path) -> Result<Option<KeyRow>> {
    let json_path = dir.join("analysis.json");
    let audio_path = dir.join("audio.raw");
    if !json_path.exists() || !audio_path.exists() {
        return Ok(None);
    }

    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&json_path).context("read analysis.json")?)
            .context("parse analysis.json")?;
    let meta = &json["metadata"];

    let key_index = meta["key_index"].as_u64().context("key_index")? as u8;
    let sample_rate = meta["sample_rate"].as_u64().unwrap_or(44100) as u32;
    // The Goertzel-tracked seed if present, else equal-temperament.
    let f0_et = NOTES[key_index as usize].frequency;
    let seed = meta["measured_f0"]
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| *v > 0.0)
        .unwrap_or(f0_et);

    // Load raw f32 audio.
    let bytes = fs::read(&audio_path).context("read audio.raw")?;
    let num_samples = bytes.len() / 4;
    if num_samples < 2048 {
        return Ok(None);
    }
    let mut audio = vec![0.0f32; num_samples];
    unsafe {
        std::ptr::copy_nonoverlapping(
            bytes.as_ptr(),
            audio.as_mut_ptr() as *mut u8,
            num_samples * 4,
        );
    }

    let fft_size = largest_pow2_le(num_samples.max(2048));
    // Need one extra sample for the CSPE one-sample-shifted frame (Hann zeroes the boundary).
    if audio.len() < fft_size + 1 {
        audio.resize(fft_size + 1, 0.0);
    }

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(fft_size);
    let mut time_buffer = vec![0.0f32; fft_size];
    let mut freq_buffer = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut freq_buffer_shifted = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut magnitudes = vec![0.0f32; fft_size / 2];
    let mut cspe_map = vec![0.0f32; fft_size / 2];

    fft(
        &audio[..fft_size],
        &mut time_buffer,
        &mut freq_buffer,
        &r2c,
        fft_size,
    );
    magnitude_spectrum(&freq_buffer, fft_size, &mut magnitudes);

    // CSPE per-bin frequency map from the frame and its one-sample-shifted twin (§2.3).
    fft(
        &audio[1..fft_size + 1],
        &mut time_buffer,
        &mut freq_buffer_shifted,
        &r2c,
        fft_size,
    );
    cspe(
        &freq_buffer,
        &freq_buffer_shifted,
        fft_size,
        sample_rate,
        &mut cspe_map,
    );

    let sim = run_mode(
        &magnitudes,
        &cspe_map,
        sample_rate,
        seed,
        MatOrder::Simultaneous,
    );
    let ser = run_mode(&magnitudes, &cspe_map, sample_rate, seed, MatOrder::Serial);

    Ok(Some(KeyRow {
        key_index,
        name: NOTES[key_index as usize].name.clone(),
        seed,
        prior_b: get_expected_beta(key_index),
        sim,
        ser,
    }))
}

pub fn run(root: &Path, offsets: Option<&[u32]>) -> Result<()> {
    let dirs = crate::capture::find(root)?;

    if let Some(offsets) = offsets {
        return offset_sweep(&dirs, offsets);
    }

    // Per-mode cell: (B, ratio-to-prior, confidence, partials).
    let fmt = |m: &ModeResult, prior: f32| -> (String, String) {
        match m.b {
            Some(b) => (format!("{b:.6}"), format!("{:.2}x", b / prior)),
            None => ("None".into(), "-".into()),
        }
    };

    println!(
        "{:>3} {:<5} {:>8} | {:>9} {:>6} {:>4} {:>3} | {:>9} {:>6} {:>4} {:>3}",
        "idx", "note", "seed", "B_simul", "ratio", "cf", "pt", "B_serial", "ratio", "cf", "pt"
    );
    println!("{}", "-".repeat(78));

    let mut rows = Vec::new();
    for dir in &dirs {
        match process_capture(dir) {
            Ok(Some(row)) => rows.push(row),
            Ok(None) => {}
            Err(e) => eprintln!("  [skip] {}: {e:#}", dir.display()),
        }
    }

    // Per-mode bass tallies + cross-mode comparison.
    let (mut sim_bass, mut ser_bass, mut sim_neg, mut ser_neg) = (0, 0, 0, 0);
    let (mut ser_extends, mut diverge) = (0, 0);
    // Goodness of fit over the BASS, in ppm of relative residual. The key discriminator:
    // `ser_on_clean` = serial's (f0,B) evaluated against simultaneous's clean low-mid
    // partials — if it stays as low as `sim_self`, serial's high partials did not corrupt it.
    // `sim_on_high` = simultaneous's model evaluated against serial's full high-partial set —
    // if it is large, simultaneous fails to explain the high partials serial captured.
    let (mut sim_self, mut ser_self, mut ser_on_clean, mut sim_on_high) =
        (0.0_f64, 0.0_f64, 0.0_f64, 0.0_f64);
    let mut fit_n = 0u32;
    let mut ser_consistent = 0u32; // bass keys where ser_on_clean ≤ 1.5× sim_self

    for r in &rows {
        let is_bass = r.key_index < 40;
        let (sb, sr) = fmt(&r.sim, r.prior_b);
        let (eb, er) = fmt(&r.ser, r.prior_b);

        if is_bass {
            if let Some(b) = r.sim.b {
                sim_bass += 1;
                if b < 0.0 {
                    sim_neg += 1;
                }
            }
            if let Some(b) = r.ser.b {
                ser_bass += 1;
                if b < 0.0 {
                    ser_neg += 1;
                }
            }
            // Cross-residuals (need both orders' fits + partials).
            if let (Some(sbv), Some(ebv)) = (r.sim.b, r.ser.b) {
                let ss = fit_residual(r.sim.f0, sbv, &r.sim.pf, &r.sim.pn);
                let es = fit_residual(r.ser.f0, ebv, &r.ser.pf, &r.ser.pn);
                let eoc = fit_residual(r.ser.f0, ebv, &r.sim.pf, &r.sim.pn);
                let soh = fit_residual(r.sim.f0, sbv, &r.ser.pf, &r.ser.pn);
                if let (Some(ss), Some(es), Some(eoc), Some(soh)) = (ss, es, eoc, soh) {
                    sim_self += ss as f64;
                    ser_self += es as f64;
                    ser_on_clean += eoc as f64;
                    sim_on_high += soh as f64;
                    fit_n += 1;
                    if eoc <= 1.5 * ss {
                        ser_consistent += 1;
                    }
                }
            }
        }
        if r.ser.partials > r.sim.partials {
            ser_extends += 1;
        }
        if let (Some(s), Some(e)) = (r.sim.b, r.ser.b)
            && (e - s).abs() > 0.25 * s.abs().max(1e-6)
        {
            diverge += 1;
        }

        println!(
            "{:>3} {:<5} {:>8.2} | {:>9} {:>6} {:>4.2} {:>3} | {:>9} {:>6} {:>4.2} {:>3}",
            r.key_index,
            r.name,
            r.seed,
            sb,
            sr,
            r.sim.confidence,
            r.sim.partials,
            eb,
            er,
            r.ser.confidence,
            r.ser.partials,
        );
    }

    println!("{}", "-".repeat(78));
    println!(
        "bass (<40): simultaneous measured {sim_bass} (neg {sim_neg})  |  serial measured {ser_bass} (neg {ser_neg})"
    );
    println!(
        "serial reached more partials than simultaneous on {ser_extends} key(s); B diverged >25% on {diverge} key(s)"
    );

    if fit_n > 0 {
        let n = fit_n as f64;
        let ppm = |x: f64| (x / n) * 1e6;
        println!("\nbass goodness-of-fit (mean RMS relative residual, ppm; lower = better):");
        println!(
            "  self-fit:        simultaneous {:>6.0}  |  serial {:>6.0}   (each model vs its own partials)",
            ppm(sim_self),
            ppm(ser_self)
        );
        println!(
            "  serial's (f0,B) vs simultaneous's clean low-mid partials: {:>6.0}   (vs sim self {:>6.0})",
            ppm(ser_on_clean),
            ppm(sim_self)
        );
        println!(
            "  simultaneous's (f0,B) vs serial's high-partial set:       {:>6.0}   (vs serial self {:>6.0})",
            ppm(sim_on_high),
            ppm(ser_self)
        );
        println!(
            "  serial stays consistent with the clean partials (≤1.5× sim self-fit) on {ser_consistent}/{fit_n} bass keys"
        );
    }

    Ok(())
}
