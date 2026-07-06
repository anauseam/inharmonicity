//! # MAT validation harness
//!
//! Re-runs the Worker's Median-Adjustive Trajectories (f0, B) estimator over the real
//! captures in `diagnostics/key_*/` and reports the measured inharmonicity per key against
//! the Rigaud prior. The FFT path mirrors `worker::process_payload`: largest power-of-two
//! window ≤ the stable sample count, Hann window, magnitude spectrum.
//!
//! Usage:
//!   cargo run --release --example validate_mat -- [diagnostics_dir]   (default: diagnostics)

use anyhow::{Context, Result};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::env;
use std::fs;
use std::path::Path;

use tuner_core::algorithms::mat::{MAX_PARTIALS, MatOrder, detect_pitch_mat};
use tuner_core::algorithms::spectral::{cspe, fft, magnitude_spectrum};
use tuner_core::models::{NOTES, get_expected_beta};

/// Largest power of two ≤ `n` (matches the Worker's FFT sizing).
fn largest_pow2_le(n: usize) -> usize {
    1usize << (usize::BITS - 1 - n.max(1).leading_zeros())
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

fn main() -> Result<()> {
    let root = env::args()
        .nth(1)
        .unwrap_or_else(|| "diagnostics".to_string());
    let root = Path::new(&root);

    let mut dirs: Vec<_> = fs::read_dir(root)
        .with_context(|| format!("read dir {}", root.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| n.starts_with("key_"))
                    .unwrap_or(false)
        })
        .collect();
    dirs.sort();

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
