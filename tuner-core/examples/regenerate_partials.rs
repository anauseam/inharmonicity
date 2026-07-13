//! # Regenerate persisted partials from kept audio with the CURRENT Serial MAT
//!
//! The `diagnostics/key_*/analysis.json` files were written 2026-05-30 under the
//! **Simultaneous** MAT default (capped at `SIM_MAX_PARTIALS = 12`) and an earlier
//! `(f0, B)` estimator. Both their partial *count* (12) and their `calculated_b` are
//! therefore stale. Every key kept its `audio.raw`, so we can re-derive the partials
//! offline with today's shipped worker path (Serial MAT, up to `MAX_PARTIALS = 32`)
//! **without re-capturing on the instrument**.
//!
//! This harness reproduces the Worker's exact offline path (FFT sizing, Hann window,
//! CSPE one-sample shift, `detect_pitch_mat(Serial)`, amplitude-at-rounded-bin) and
//! emits one JSON array over all keys to **stdout** — it writes no repo files, so it
//! cannot clobber the validation captures. Redirect it where you like.
//!
//! Usage: cargo run --release --example regenerate_partials -- [diagnostics_dir] > out.json

use std::path::Path;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use tuner_core::algorithms::mat::{MAX_PARTIALS, MatOrder, detect_pitch_mat};
use tuner_core::algorithms::spectral::{cspe, fft, magnitude_spectrum};
use tuner_core::models::{NOTES, get_expected_beta};

fn largest_pow2_le(n: usize) -> usize {
    if n == 0 {
        return 0;
    }
    1usize << (usize::BITS - 1 - n.leading_zeros())
}

fn read_raw_f32(path: &Path) -> Option<Vec<f32>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() % 4 != 0 {
        return None;
    }
    let n = bytes.len() / 4;
    let mut out = vec![0.0f32; n];
    unsafe {
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), out.as_mut_ptr() as *mut u8, bytes.len());
    }
    Some(out)
}

fn process(dir: &Path) -> Option<serde_json::Value> {
    let jpath = dir.join("analysis.json");
    let apath = dir.join("audio.raw");
    if !jpath.exists() || !apath.exists() {
        return None;
    }
    let j: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&jpath).ok()?).ok()?;
    let meta = &j["metadata"];
    let key_index = meta["key_index"].as_u64()? as u8;
    let sample_rate = meta["sample_rate"].as_u64().unwrap_or(44100) as u32;
    let f0_et = NOTES[key_index as usize].frequency;
    // Mirror the shipped worker's seed plausibility gate
    // (`worker::MAT_SEED_TOLERANCE`): a recorded tracker seed outside ±10 %
    // of the named key's ET is tracker garbage (observed: deep-bass seeds of
    // 5–16 Hz on A0 captures) — seed from ET instead. This is also the
    // rescue path for dumps captured before the gate existed: the kept
    // audio is fine, only the recorded seed was bad.
    let seed = meta["measured_f0"]
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| *v > 0.0 && (*v / f0_et - 1.0).abs() <= tuner_core::worker::MAT_SEED_TOLERANCE)
        .unwrap_or(f0_et);

    let mut audio = read_raw_f32(&apath)?;
    let num_samples = audio.len();
    if num_samples < 2048 {
        return None;
    }
    let fft_size = largest_pow2_le(num_samples.max(2048));
    if audio.len() < fft_size + 1 {
        audio.resize(fft_size + 1, 0.0);
    }

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c: Arc<dyn RealToComplex<f32>> = planner.plan_fft_forward(fft_size);
    let mut tbuf = vec![0.0f32; fft_size];
    let mut fbuf = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut fbuf_sh = vec![Complex { re: 0.0, im: 0.0 }; fft_size / 2 + 1];
    let mut mags = vec![0.0f32; fft_size / 2];
    let mut cspe_map = vec![0.0f32; fft_size / 2];

    fft(&audio[..fft_size], &mut tbuf, &mut fbuf, &r2c, fft_size);
    magnitude_spectrum(&fbuf, fft_size, &mut mags);
    fft(
        &audio[1..fft_size + 1],
        &mut tbuf,
        &mut fbuf_sh,
        &r2c,
        fft_size,
    );
    cspe(&fbuf, &fbuf_sh, fft_size, sample_rate, &mut cspe_map);

    let hz_per_bin = sample_rate as f32 / fft_size as f32;
    let mut freqs = [0.0f32; MAX_PARTIALS];
    let mut ns = [0u32; MAX_PARTIALS];
    let est = detect_pitch_mat(
        &mags,
        &cspe_map,
        sample_rate,
        seed,
        MatOrder::Serial,
        &mut freqs,
        &mut ns,
    )?;

    let mut partials = Vec::new();
    for i in 0..est.partial_count {
        let bin = (freqs[i] / hz_per_bin).round() as usize;
        let amp = if bin < mags.len() { mags[bin] } else { 0.0 };
        partials.push(serde_json::json!({
            "number": ns[i],
            "frequency": freqs[i],
            "amplitude": amp,
        }));
    }

    Some(serde_json::json!({
        "key_index": key_index,
        // Which dump this entry came from — repeat-capture sets have many
        // dirs per key; downstream audits need the identity.
        "source_dir": dir.file_name().and_then(|n| n.to_str()),
        "measured_f0": seed,
        "mat_f0": est.f0,
        "calculated_b": est.b,
        "prior_b": get_expected_beta(key_index),
        "confidence": est.confidence,
        "partial_count": est.partial_count,
        "partials": partials,
    }))
}

fn main() {
    let root = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "diagnostics".into());
    let root = Path::new(&root);
    let mut dirs: Vec<_> = std::fs::read_dir(root)
        .expect("read diagnostics dir")
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

    let out: Vec<_> = dirs.iter().filter_map(|d| process(d)).collect();
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
    eprintln!("regenerated {} keys", out.len());
}
