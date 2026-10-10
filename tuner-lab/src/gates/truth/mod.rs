//! Synthetic truth for the gate studies: the AWGN control (T1) and known partials
//! in real noise (T2), written as dump directories in the capture format with the
//! exact noise beside each capture, so an arm's estimate is scored against the
//! noise actually present at every bin of every hop.
//!
//! A synthetic set is one directory of `key_<index>_<note>_<n>/` captures, each
//! holding `audio.raw` (the record an arm reads), `audio_full_event.raw` (the
//! same with the pre-roll in front, as the app writes it), `noise.raw` (the
//! noise alone, pre-roll included) and the `analysis.json` the lab's loaders read,
//! plus one `truth.json` at the root in the `mat regen` schema, each row carrying
//! a `synthetic` object with the generator's parameters. `gates ambient` and
//! `gates arms` load it as they load a regen dump.

pub mod inject;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};

use crate::raw;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};
use tuner_core::models::{NOTES, get_expected_beta};
use tuner_core::pipeline::CAPTURE_DEFAULT_SAMPLES;

/// Hops of pre-roll before the onset in the control. The app's diagnostic
/// record opens with about 348 ms (15 hops); the control takes a second, so its
/// silence population is large enough to bound a 10⁻³ rate.
const T1_PREROLL_HOPS: usize = 43;

/// The keys the control is written on: two or three per register of the
/// four-band split, so every register has a row.
const T1_KEYS: [u8; 10] = [3, 15, 27, 39, 51, 63, 71, 78, 83, 87];

/// Partials present in the control, and the highest declared one.
const T1_PRESENT: usize = 4;
const T1_DECLARED: usize = 8;

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// T1, the AWGN control: white Gaussian noise of known σ with partials 1–4
    /// present at a stated per-bin SNR and 5–8 declared absent (report 0015 §4).
    T1 {
        /// Directory to write; refused if it exists.
        out: std::path::PathBuf,
        /// Time-domain noise σ.
        #[arg(long, default_value_t = 1.0e-3)]
        sigma: f32,
        /// Per-bin SNR of each present partial in the 8192-point Hann frame, dB.
        #[arg(long, default_value_t = 30.0)]
        snr_db: f32,
        /// Captures per key.
        #[arg(long, default_value_t = 10)]
        per_key: usize,
        /// Hops of pure noise before the onset.
        #[arg(long, default_value_t = T1_PREROLL_HOPS)]
        preroll_hops: usize,
        /// Seed of the generator.
        #[arg(long, default_value_t = 20261008)]
        seed: u64,
    },
    /// T2, known partials in real noise: each real capture's measured series
    /// re-synthesised into a room bed or the capture's own residual, with the
    /// fidelity tables.
    T2(inject::Args),
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::T1 {
            out,
            sigma,
            snr_db,
            per_key,
            preroll_hops,
            seed,
        } => t1(&out, sigma, snr_db, per_key, preroll_hops, seed),
        Mode::T2(a) => inject::run(a),
    }
}

/// xorshift64* with a Box–Muller output: deterministic Gaussian noise without a
/// dependency.
pub(crate) struct Gaussian {
    state: u64,
    spare: Option<f32>,
}

impl Gaussian {
    pub(crate) fn new(seed: u64) -> Self {
        Self {
            state: seed.max(1),
            spare: None,
        }
    }

    fn uniform(&mut self) -> f64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        let v = self.state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((v >> 11) as f64 + 0.5) / (1u64 << 53) as f64
    }

    /// One standard normal sample.
    pub(crate) fn next(&mut self) -> f32 {
        if let Some(s) = self.spare.take() {
            return s;
        }
        let (u1, u2) = (self.uniform(), self.uniform());
        let r = (-2.0 * u1.ln()).sqrt();
        let th = std::f64::consts::TAU * u2;
        self.spare = Some((r * th.sin()) as f32);
        (r * th.cos()) as f32
    }
}

/// Amplitude of a sinusoid whose peak bin stands `snr_db` above the noise in a
/// Hann frame of `n` samples with time-domain noise σ: the bin's mean noise
/// power is `σ²·0.375n` and a tone of amplitude `A` peaks at `A·n/4`.
pub(super) fn tone_amplitude(sigma: f32, snr_db: f32, n: usize) -> f32 {
    let noise_bin = sigma * sigma * 0.375 * n as f32;
    (noise_bin * 10f32.powf(snr_db / 10.0)).sqrt() * 4.0 / n as f32
}

/// The noise's median magnitude per 8192-point raw Hann bin for time-domain σ:
/// `√(σ²·0.375·N·ln 2)`.
pub(super) fn awgn_median_bin(sigma: f32) -> f32 {
    (sigma * sigma * 0.375 * BASS_WINDOW_SIZE as f32 * std::f32::consts::LN_2).sqrt()
}

fn t1(
    out: &Path,
    sigma: f32,
    snr_db: f32,
    per_key: usize,
    preroll_hops: usize,
    seed: u64,
) -> Result<()> {
    if out.exists() {
        bail!(
            "{} exists; T1 is written to a fresh directory",
            out.display()
        );
    }
    if per_key == 0 {
        bail!("--per-key must be at least 1");
    }
    if preroll_hops * HOP_SIZE < BASS_WINDOW_SIZE + 4 * HOP_SIZE {
        bail!(
            "--preroll-hops must cover one 8192-sample window and the four-hop guard: at least 12"
        );
    }
    fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let pre = preroll_hops * HOP_SIZE;
    let total = pre + CAPTURE_DEFAULT_SAMPLES;
    let amp = tone_amplitude(sigma, snr_db, BASS_WINDOW_SIZE);
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let mut rng = Gaussian::new(seed);
    let mut rows = Vec::new();
    let mut serial = 1_000_000u64;

    for &key in &T1_KEYS {
        let f0 = NOTES[key as usize].frequency;
        let b = get_expected_beta(key);
        for _ in 0..per_key {
            serial += 1;
            let dir_name = format!("key_{key:03}_{}_{serial}", NOTES[key as usize].name);
            let dir = out.join(&dir_name);
            fs::create_dir(&dir)?;

            let noise: Vec<f32> = (0..total).map(|_| sigma * rng.next()).collect();
            let mut audio = noise[pre..].to_vec();
            let mut partials = Vec::new();
            for n in 1..=T1_DECLARED {
                let f_n = n as f32 * f0 * (1.0 + b * (n * n) as f32).sqrt();
                // A declared partial needs its lobe inside the band.
                if f_n >= nyquist - 2.0 * SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32 {
                    break;
                }
                let present = n <= T1_PRESENT;
                if present {
                    let phase = std::f64::consts::TAU * rng.uniform();
                    for (t, s) in audio.iter_mut().enumerate() {
                        let arg = std::f64::consts::TAU * f_n as f64 * t as f64
                            / SAMPLE_RATE as f64
                            + phase;
                        *s += amp * arg.sin() as f32;
                    }
                }
                partials.push(serde_json::json!({
                    "number": n,
                    "frequency": f_n,
                    "amplitude": if present { amp } else { 0.0 },
                }));
            }
            let mut full = noise[..pre].to_vec();
            full.extend_from_slice(&audio);
            // The threshold the app would have recorded: its calibration takes
            // the loudest hop RMS of the room and multiplies by 1.5, so the
            // Gatekeeper's screen sits above the noise it screens. The true σ
            // stays in the truth; `A@1e-3` is the arm at the true σ.
            let recorded = 1.5
                * noise[..pre]
                    .as_chunks::<HOP_SIZE>()
                    .0
                    .iter()
                    .map(|h| (h.iter().map(|x| x * x).sum::<f32>() / HOP_SIZE as f32).sqrt())
                    .fold(0.0f32, f32::max);

            raw::write(&dir.join("audio.raw"), &audio)?;
            raw::write(&dir.join("audio_full_event.raw"), &full)?;
            raw::write(&dir.join("noise.raw"), &noise)?;
            let analysis = serde_json::json!({
                "key_index": key,
                "metadata": {
                    "key_index": key,
                    "sample_rate": SAMPLE_RATE,
                    "measured_f0": f0,
                    "f0_et": f0,
                    "noise_floor": recorded,
                    "synthetic": "t1",
                },
            });
            fs::write(
                dir.join("analysis.json"),
                serde_json::to_string_pretty(&analysis)?,
            )?;
            let present: Vec<usize> = (1..=T1_PRESENT.min(partials.len())).collect();
            let absent: Vec<usize> = (T1_PRESENT + 1..=partials.len()).collect();
            rows.push(serde_json::json!({
                "key_index": key,
                "source_dir": dir_name,
                "measured_f0": f0,
                "mat_f0": f0,
                "calculated_b": b,
                "prior_b": b,
                "partials": partials,
                "synthetic": {
                    "kind": "t1",
                    "sigma": sigma,
                    "recorded_threshold": recorded,
                    "snr_db": snr_db,
                    "amplitude": amp,
                    "present": present,
                    "absent": absent,
                    "noise_file": "noise.raw",
                    "onset_sample": pre,
                    "median_bin_8192": awgn_median_bin(sigma),
                },
            }));
        }
    }
    fs::write(out.join("truth.json"), serde_json::to_string_pretty(&rows)?)?;
    println!(
        "T1: {} captures on {} keys in {}; σ = {sigma:.3e}, partials 1–{T1_PRESENT} at {snr_db} dB \
         per bin (amplitude {amp:.3e}), 5–{T1_DECLARED} declared absent; {preroll_hops} hops of \
         pre-roll; the recorded threshold is the calibration's 1.5 × loudest hop RMS; truth.json \
         in the regen schema",
        rows.len(),
        T1_KEYS.len(),
        out.display()
    );
    Ok(())
}

/// A synthetic capture's truth, read from `truth.json`'s `synthetic` object.
#[derive(Debug, Clone)]
pub(super) struct Synthetic {
    pub(super) kind: String,
    /// Declared partial numbers that are absent from the audio: the T5(iii)
    /// population.
    pub(super) absent: Vec<usize>,
    /// Sample of `noise.raw` at which `audio.raw` begins.
    pub(super) onset_sample: usize,
    /// The noise alone, pre-roll included: `noise.raw`.
    pub(super) noise: Vec<f32>,
    /// The noise's median magnitude per 8192-point bin where the generator
    /// knows it in closed form (white noise); `None` where the bed's own hops
    /// must supply it.
    pub(super) level_bin_8192: Option<f32>,
}

/// The synthetic truth of every capture in a set, keyed by directory name, or
/// `None` where the regen file carries no `synthetic` objects (a real set).
pub(super) fn load(
    regen: &Path,
    root: &Path,
) -> Option<std::collections::HashMap<String, Synthetic>> {
    let text = fs::read_to_string(regen).ok()?;
    let rows: Vec<serde_json::Value> = serde_json::from_str(&text).ok()?;
    let mut out = std::collections::HashMap::new();
    for r in &rows {
        let Some(s) = r.get("synthetic") else {
            continue;
        };
        let dir = r["source_dir"].as_str()?.to_string();
        let noise_file = s["noise_file"].as_str().unwrap_or("noise.raw");
        let noise = raw::read(&root.join(&dir).join(noise_file))?;
        out.insert(
            dir,
            Synthetic {
                kind: s["kind"].as_str().unwrap_or("").to_string(),
                absent: s["absent"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_u64().map(|n| n as usize))
                            .collect()
                    })
                    .unwrap_or_default(),
                onset_sample: s["onset_sample"].as_u64().unwrap_or(0) as usize,
                noise,
                level_bin_8192: s["median_bin_8192"].as_f64().map(|v| v as f32),
            },
        );
    }
    (!out.is_empty()).then_some(out)
}
