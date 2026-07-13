//! A/B: our spectral-sparsity ratio vs faithful Mounir NINOS² variants.
//!
//! Question (faithfulness-audit-05): the Gatekeeper's tonality gate (`metrics::ninos2`)
//! deviates from the cited Mounir 2021 NINOS² in four ways (linear magnitudes, all
//! bins, no energy factor, reciprocal orientation). Is ours actually the better
//! discriminator for the Gatekeeper's job — separating a note's transient from its
//! tonal steady state — or did we just assert that?
//!
//! Method: replay every `diagnostics/key_*/` capture. Ground-truth classes are
//! **time-anchored** (not derived from any metric, avoiding circularity):
//!   * onset sample n₀ = earliest |x| ≥ 1% of max|x| (Mounir 2021 Eq. 19 style),
//!   * TRANSIENT frames: window start ∈ [n₀ − N/2, n₀ + 90 ms] with frame RMS
//!     ≥ 5 % of the capture's max frame RMS (the strike itself, not lead-in),
//!   * STEADY frames: window start ∈ [n₀ + 300 ms, n₀ + 1000 ms] with the same
//!     RMS floor — so fast-decaying treble contributes only frames where the
//!     note is still sounding (a fixed time window would otherwise label the
//!     noise floor as "steady", biasing the comparison).
//! Per key and metric, compute the AUC (Mann–Whitney) of steady-vs-transient
//! separation, oriented so 1.0 = perfect in each metric's own direction (ours:
//! steady HIGH; Mounir ODFs: transient HIGH). Threshold-free, scale-free.
//!
//! Faithful variants implemented per Mounir et al. 2021 (EURASIP 2021:30):
//!   * preprocessing (Eqs. 4, 6–7): y = sorted-ascending log(1+|X_k|), k=1..N/2−1,
//!     keep lowest J = ⌊γ/100·(N/2−1)⌋ with γ = 95.5 (their tuned value);
//!   * NINOS² (ℓ₂ℓ₄), Eq. 13:  ‖y‖₂/(⁴√J−1) · (‖y‖₂/‖y‖₄ − 1);
//!   * INOS² (ℓ₁),   Eq. 14:  ‖y‖₁;
//!   * NINOS² (ℓ₁),  Eq. 15:  ‖y‖₂/(√J−1) · (‖y‖₁/‖y‖₂ − 1).
//!
//! Run: cargo run --release --example sparsity_ab

use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use std::fs;
use tuner_core::algorithms::{metrics, spectral};

const N: usize = 2048; // Gatekeeper analysis window
const HOP: usize = 1024;
const FS: f32 = 44_100.0;
const GAMMA: f32 = 95.5;
const TRANSIENT_MS: f32 = 90.0;
const STEADY_FROM_MS: f32 = 300.0;
const STEADY_TO_MS: f32 = 1000.0;

struct Faithful {
    ninos2_l2l4: f32,
    inos2_l1: f32,
    ninos2_l1: f32,
    /// Paper Eq. 12: normalized inverse-sparsity factor S̄ ∈ [0,1] (ℓ₂/ℓ₄),
    /// i.e. the ODF with its energy factor stripped — the level-independent
    /// quantity comparable to a tonality gate.
    sbar_l2l4: f32,
    /// The ℓ₁/ℓ₂ analog of Eq. 12 (paper §3.3's p=1,q=2 choice, normalized).
    sbar_l1l2: f32,
}

/// Mounir 2021 preprocessing + the three (N)INOS² variants for one frame.
fn faithful_ninos2(spectrum: &[Complex<f32>]) -> Faithful {
    // k = 1 .. N/2 − 1 (paper uses the one-sided grid without DC/Nyquist)
    let mut y: Vec<f32> = spectrum[1..N / 2]
        .iter()
        .map(|c| (1.0 + (c.re * c.re + c.im * c.im).sqrt()).ln())
        .collect();
    y.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let j = ((GAMMA / 100.0) * y.len() as f32).floor() as usize;
    let y = &y[..j];

    let (mut l1, mut l2sq, mut l4q) = (0.0f64, 0.0f64, 0.0f64);
    for &v in y {
        let v = v as f64;
        l1 += v;
        l2sq += v * v;
        l4q += v * v * v * v;
    }
    let l2 = l2sq.sqrt();
    let l4 = l4q.powf(0.25);
    let jf = j as f64;

    Faithful {
        ninos2_l2l4: if l4 > 0.0 {
            ((l2 / (jf.powf(0.25) - 1.0)) * (l2 / l4 - 1.0)) as f32
        } else {
            0.0
        },
        inos2_l1: l1 as f32,
        ninos2_l1: if l2 > 0.0 {
            ((l2 / (jf.sqrt() - 1.0)) * (l1 / l2 - 1.0)) as f32
        } else {
            0.0
        },
        sbar_l2l4: if l4 > 0.0 {
            (((l2 / l4) - 1.0) / (jf.powf(0.25) - 1.0)) as f32
        } else {
            0.0
        },
        sbar_l1l2: if l2 > 0.0 {
            (((l1 / l2) - 1.0) / (jf.sqrt() - 1.0)) as f32
        } else {
            0.0
        },
    }
}

/// AUC = P(a > b) + 0.5·P(a = b), a = class expected HIGH, b = class expected LOW.
fn auc(high: &[f32], low: &[f32]) -> f32 {
    if high.is_empty() || low.is_empty() {
        return f32::NAN;
    }
    let mut wins = 0.0f64;
    for &a in high {
        for &b in low {
            if a > b {
                wins += 1.0;
            } else if a == b {
                wins += 0.5;
            }
        }
    }
    (wins / (high.len() as f64 * low.len() as f64)) as f32
}

fn register(key_idx: usize) -> usize {
    if key_idx < 26 {
        0
    } else if key_idx < 59 {
        1
    } else {
        2
    }
}

fn main() {
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(N);
    let mut time_buf = vec![0.0f32; N];
    let mut spec = vec![
        Complex {
            re: 0.0f32,
            im: 0.0
        };
        N / 2 + 1
    ];

    // per metric: per register, (sum_auc, count, min_auc, n_below_95)
    const M: usize = 6;
    let names = [
        "ours N*(l2/l1)^2",
        "NINOS2(l2l4) Eq13",
        "INOS2(l1) Eq14",
        "NINOS2(l1) Eq15",
        "Sbar(l2l4) Eq12",
        "Sbar(l1l2) Eq12-l1",
    ];
    let mut agg = [[(0.0f64, 0usize, f32::MAX, 0usize); 3]; M];

    let mut dirs: Vec<_> = fs::read_dir("./diagnostics")
        .expect("run from repo root")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("key_"))
        .collect();
    dirs.sort();

    let mut keys_used = 0;
    for dir in &dirs {
        let key_idx: usize = dir.file_name().unwrap().to_string_lossy()[4..7]
            .parse()
            .unwrap();
        let raw = ["audio_full_event.raw", "audio.raw"]
            .iter()
            .map(|f| dir.join(f))
            .find(|p| p.exists());
        let Some(raw) = raw else { continue };
        let bytes = fs::read(&raw).unwrap();
        let audio: Vec<f32> = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        if audio.len() < N * 4 {
            continue;
        }

        // Onset: earliest |x| >= 1% of max |x| (Mounir Eq. 19 style, rho = 1).
        let max_abs = audio.iter().fold(0.0f32, |m, &x| m.max(x.abs()));
        let Some(n0) = audio.iter().position(|&x| x.abs() >= 0.01 * max_abs) else {
            continue;
        };

        let t_from = n0.saturating_sub(N / 2);
        let t_end = n0 + (TRANSIENT_MS / 1000.0 * FS) as usize;
        let s_from = n0 + (STEADY_FROM_MS / 1000.0 * FS) as usize;
        let s_to = n0 + (STEADY_TO_MS / 1000.0 * FS) as usize;

        // RMS floor: 5% of the loudest frame — keeps only frames where the note
        // is actually sounding (drops lead-in noise and decayed-to-silence tails).
        let mut max_frame_rms = 0.0f32;
        let mut cursor = 0usize;
        while cursor + N <= audio.len() {
            max_frame_rms = max_frame_rms.max(metrics::rms(&audio[cursor..cursor + N]));
            cursor += HOP;
        }
        let rms_floor = 0.05 * max_frame_rms;

        // vals[m][class]: class 0 = transient, 1 = steady
        let mut vals: [[Vec<f32>; 2]; M] = Default::default();
        let mut cursor = 0usize;
        while cursor + N <= audio.len() {
            let class = if cursor >= t_from && cursor <= t_end {
                Some(0)
            } else if cursor >= s_from && cursor <= s_to {
                Some(1)
            } else {
                None
            };
            let class = class.filter(|_| metrics::rms(&audio[cursor..cursor + N]) >= rms_floor);
            if let Some(class) = class {
                spectral::fft(
                    &audio[cursor..cursor + N],
                    &mut time_buf,
                    &mut spec,
                    &r2c,
                    N,
                );
                let ours = metrics::ninos2(&spec);
                let f = faithful_ninos2(&spec);
                for (m, v) in [
                    ours,
                    f.ninos2_l2l4,
                    f.inos2_l1,
                    f.ninos2_l1,
                    f.sbar_l2l4,
                    f.sbar_l1l2,
                ]
                .into_iter()
                .enumerate()
                {
                    vals[m][class].push(v);
                }
            }
            cursor += HOP;
        }

        if vals[0][0].is_empty() || vals[0][1].is_empty() {
            continue;
        }
        keys_used += 1;
        let reg = register(key_idx);
        for m in 0..M {
            // Orientation: ours is HIGH in steady; the Mounir ODFs are HIGH at onsets.
            let a = if m == 0 {
                auc(&vals[m][1], &vals[m][0])
            } else {
                auc(&vals[m][0], &vals[m][1])
            };
            let e = &mut agg[m][reg];
            e.0 += a as f64;
            e.1 += 1;
            e.2 = e.2.min(a);
            if a < 0.95 {
                e.3 += 1;
            }
        }
    }

    println!(
        "keys used: {keys_used}   (AUC oriented per metric; 1.0 = perfect transient/steady separation)"
    );
    println!(
        "{:<20} {:>21} {:>21} {:>21}",
        "metric", "bass mean/min(<.95)", "mid mean/min(<.95)", "treble mean/min(<.95)"
    );
    for m in 0..M {
        let cell = |r: usize| {
            let (s, c, mn, nb) = agg[m][r];
            format!("{:.4}/{:.3} ({})", s / c as f64, mn, nb)
        };
        println!(
            "{:<20} {:>21} {:>21} {:>21}",
            names[m],
            cell(0),
            cell(1),
            cell(2)
        );
    }
}
