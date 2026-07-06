//! # MAT (f₀, B) ground-truth recovery stress test
//!
//! Characterises the Worker's Median-Adjustive Trajectories estimator against **known**
//! synthetic inharmonicity, in the regime the real captures cast doubt on: **high bass B
//! with missing fundamentals** (ADR 0006 fix-path step 1; "Validation strategy" bullet).
//! The real deep-bass keys read 7–25× the Rigaud prior; this harness asks whether that is a
//! trustworthy measurement or a partial-**mis-association** artifact — by sweeping a *known*
//! B from 1× to 25× the prior and scoring MAT's recovery against truth.
//!
//! ## Why this is honest (the gotcha)
//!
//! MAT does not consume a peak list — it consumes a **magnitude spectrum + a CSPE per-bin
//! frequency map** (`detect_pitch_mat`). So we cannot reuse `mobo_evaluator`'s peak-domain
//! frames. Instead we **synthesize a time-domain signal** (sum of sinusoids at
//! `f_n = n·f0·√(1+B·n²)`, with the `gen_frame` amplitude / missing-fundamental / unison /
//! noise model), Hann-window it, and run the **exact** Worker path: two real FFTs (the frame
//! and the same frame advanced one sample) → `spectral::cspe` → `detect_pitch_mat`. Rendering
//! through real FFT leakage + CSPE (rather than placing ideal peaks) is what makes the
//! recovery test faithful — every hazard MAT meets in production (leakage skirts, sub-bin
//! refinement, beating unison clusters, the §2.2 significance floor) is reproduced.
//!
//! ## Experiments (recovered B vs known B)
//!
//! 1. **Baseline** — prior-level B across bass / mid / treble, full clean partials. MAT must
//!    nail these (sanity; if it fails here the harness, not MAT, is wrong).
//! 2. **B sweep** — a deep-bass f₀, known B swept 1×→25× the prior, full partials. Does
//!    recovered B track the diagonal up to high B, or saturate / diverge?
//! 3. **Missing-fundamental stress** (the key one) — the sweep with partials 1–3 attenuated
//!    (the real bass condition: soundboard impedance kills the lows). Does MAT still recover
//!    the true high B, or lock onto a wrong-but-self-consistent series (over / under-read)?
//! 4. **Parallel-string stress** — 2–3 detuned unison strings; at high n their partials
//!    diverge (n·Δf₀ grows) into resolved/beating clusters. Does serial's high-partial
//!    tracking bias B (the DAFx-09 Conclusion/§4 concern)?
//!
//! Both `MatOrder::Serial` (the shipped default) and `MatOrder::Simultaneous` are run
//! side-by-side, as `validate_mat` does. The ground-truth-free **self-fit residual** (does
//! the model explain its *own* located partials) is reported alongside the truth error so the
//! mis-association signature — *low self-residual yet wrong B* (the A#0→279× band-tightening
//! failure) — is visible as a number.
//!
//! Usage:  `cargo run --release --example mat_b_recovery`

use std::f32::consts::PI;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;
use std::sync::Arc;

use tuner_core::algorithms::mat::{MAX_PARTIALS, MatEstimate, MatOrder, detect_pitch_mat};
use tuner_core::algorithms::spectral::{cspe, fft, magnitude_spectrum};
use tuner_core::models::{NOTES, get_expected_beta};

const SAMPLE_RATE: u32 = 44100;
/// Production deep-bass FFT size: the Worker uses the largest power of two ≤ the stable
/// sample count, which for a full ~1.5 s capture (66150 samples) is 65536. The deep bass gets
/// its best-case resolution here (0.673 Hz/bin) — if MAT diverges even at this resolution the
/// finding is not a windowing artifact.
const FFT_SIZE: usize = 65536;
/// Partials above this are not synthesized (matches `gen_frame`'s 9 kHz envelope cutoff).
const F_MAX: f32 = 9000.0;
/// Signal-to-noise ratio (broadband white noise vs partial RMS). A sustained piano note's
/// decay is fairly clean; 40 dB keeps the §2.2 significance floor realistic without letting
/// noise, rather than B recovery, drive the result.
const SNR_DB: f32 = 40.0;

// ─────────────────────────── Deterministic RNG ───────────────────────────
// SplitMix64 — same generator `mobo_evaluator` uses, so the synthetic physics is byte-for-byte
// the same family of draws (envelope jitter, unison spread, missing-fundamental attenuation).

struct Rng(u64);
impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
    /// Standard normal via Box–Muller.
    fn normal(&mut self) -> f32 {
        let u1 = self.f32().max(1e-7);
        let u2 = self.f32();
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

// ─────────────────────────── Synthetic tone model ───────────────────────────

/// One synthesis condition: the physical knobs the experiments vary. The partial / envelope /
/// missing-fundamental / unison model mirrors `mobo_evaluator::gen_frame`; only the rendering
/// differs (time-domain sinusoids here, ideal peaks there).
#[derive(Clone, Copy)]
struct Cond {
    f0: f32,
    b: f32,
    /// Number of unison strings (1 low bass, 2 upper bass, 3 tenor+). Each is an independent,
    /// slightly-detuned partial series — the parallel-string hazard.
    n_strings: usize,
    /// Unison spread in cents (string-to-string f₀ detune). High values + high n diverge into
    /// resolved/beating clusters at the top of the series.
    spread_cents: f32,
    /// Envelope exponent: `a_n ∝ n^(−alpha)`. Flatter (smaller) in the bass — strong high
    /// partials, where B leverage ∝ n² lives.
    alpha: f32,
    /// Attenuate partials 1–3 (the soundboard-impedance missing-fundamental of the deep bass).
    missing_fundamental: bool,
    /// Fully remove every partial with `n <= drop_below` (a harder missing-fundamental than the
    /// graded `missing_fundamental` attenuation — used to find how many low partials can vanish
    /// before the trajectory loses its anchor and mis-numbers).
    drop_below: u32,
    /// Add a sympathetic / sub-harmonic decoy series (octave-below or fifth-above ringing
    /// through the bridge) — the "self-consistent wrong series" bait.
    sympathetic: bool,
}

/// Renders `cond` to a time-domain signal, then runs the exact Worker spectral front-end
/// (two Hann FFTs + CSPE) and returns the `(magnitude spectrum, CSPE frequency map)` that
/// `detect_pitch_mat` consumes — identical to `worker::process_payload` / `validate_mat`.
fn synth_spectrum(
    cond: &Cond,
    rng: &mut Rng,
    r2c: &Arc<dyn RealToComplex<f32>>,
) -> (Vec<f32>, Vec<f32>) {
    // One extra sample so the one-sample-shifted CSPE frame is fully populated.
    let mut signal = vec![0.0_f32; FFT_SIZE + 1];

    // Per-string fundamentals (string 0 at f0; others detuned within ±spread/2).
    let mut f0_s = [cond.f0; 3];
    for f0_string in f0_s.iter_mut().take(cond.n_strings).skip(1) {
        *f0_string =
            cond.f0 * 2f32.powf(rng.range(-cond.spread_cents / 2.0, cond.spread_cents / 2.0) / 1200.0);
    }

    // Sum the inharmonic series for every string. Each (string, partial) sinusoid gets a random
    // phase so unison pairs beat / null exactly as in `emit_partial_cluster`.
    let mut n = 1u32;
    while n <= MAX_PARTIALS as u32 * 2 {
        let n_f = n as f32;
        let stretch = (1.0 + cond.b * n_f * n_f).sqrt();
        let f_n_nominal = n_f * cond.f0 * stretch;
        if f_n_nominal > F_MAX {
            break;
        }
        if n <= cond.drop_below {
            n += 1;
            continue;
        }

        // Envelope + lognormal per-partial jitter (gen_frame: exp(0.5·N(0,1))).
        let mut a_n = n_f.powf(-cond.alpha) * (0.5 * rng.normal()).exp();
        // Missing fundamentals: drop 1–3 with the gen_frame attenuation ladder.
        if cond.missing_fundamental {
            match n {
                1 => a_n *= rng.range(0.0, 0.15),
                2 => a_n *= rng.range(0.1, 0.6),
                3 => a_n *= rng.range(0.3, 1.0),
                _ => {}
            }
        }

        for &f0_string in f0_s.iter().take(cond.n_strings) {
            let f_ns = n_f * f0_string * stretch;
            if f_ns >= SAMPLE_RATE as f32 / 2.0 {
                continue;
            }
            let amp = a_n * (1.0 + 0.1 * rng.normal()).max(0.05);
            let phi = rng.range(0.0, 2.0 * PI);
            let w = 2.0 * PI * f_ns / SAMPLE_RATE as f32;
            for (i, sample) in signal.iter_mut().enumerate() {
                *sample += amp * (w * i as f32 + phi).sin();
            }
        }
        n += 1;
    }

    // Sympathetic / sub-harmonic decoy (the dense "wrong series" energy): octave-below or
    // fifth-above, ~−20…−35 dB, its own slight detune. This is the bait a mis-associating
    // trajectory could lock onto when the true fundamental is absent.
    if cond.sympathetic {
        let sig_rms = rms(&signal);
        let rel = if rng.chance(0.6) { 0.5 } else { 1.5 };
        let f0_sym = cond.f0 * rel * 2f32.powf(rng.range(-10.0, 10.0) / 1200.0);
        let n_sym = 3 + (rng.f32() * 6.0) as usize;
        for m in 1..=n_sym {
            let f = m as f32 * f0_sym;
            if f >= SAMPLE_RATE as f32 / 2.0 || f > F_MAX {
                break;
            }
            let amp = sig_rms * 10f32.powf(rng.range(-35.0, -20.0) / 20.0);
            let phi = rng.range(0.0, 2.0 * PI);
            let w = 2.0 * PI * f / SAMPLE_RATE as f32;
            for (i, sample) in signal.iter_mut().enumerate() {
                *sample += amp * (w * i as f32 + phi).sin();
            }
        }
    }

    // Broadband white noise at the configured SNR (raises the §2.2 mean-magnitude floor).
    let sig_rms = rms(&signal).max(1e-9);
    let noise_rms = sig_rms * 10f32.powf(-SNR_DB / 20.0);
    for sample in signal.iter_mut() {
        *sample += noise_rms * rng.normal();
    }

    // ── Exact Worker spectral front-end ──
    let mut time = vec![0.0_f32; FFT_SIZE];
    let mut x0 = vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE / 2 + 1];
    let mut x1 = vec![Complex { re: 0.0, im: 0.0 }; FFT_SIZE / 2 + 1];
    let mut mags = vec![0.0_f32; FFT_SIZE / 2];
    let mut cspe_map = vec![0.0_f32; FFT_SIZE / 2];

    fft(&signal[..FFT_SIZE], &mut time, &mut x0, r2c, FFT_SIZE);
    fft(&signal[1..FFT_SIZE + 1], &mut time, &mut x1, r2c, FFT_SIZE);
    magnitude_spectrum(&x0, FFT_SIZE, &mut mags);
    cspe(&x0, &x1, FFT_SIZE, SAMPLE_RATE, &mut cspe_map);

    (mags, cspe_map)
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|&x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
}

// ─────────────────────────── Scoring ───────────────────────────

/// RMS relative residual of a fitted `(f0, B)` against its located partials (validate_mat's
/// goodness-of-fit, in ppm). The *self-consistency* metric: low here + wrong B vs truth = the
/// mis-association signature.
fn self_residual_ppm(est: &MatEstimate, freqs: &[f32], ns: &[u32]) -> f32 {
    let mut sumsq = 0.0_f32;
    let mut count = 0u32;
    for (&f, &n) in freqs.iter().zip(ns) {
        let n_f = n as f32;
        let predicted = n_f * est.f0 * (1.0 + est.b * n_f * n_f).max(0.0).sqrt();
        if predicted > 0.0 {
            let rel = (f - predicted) / predicted;
            sumsq += rel * rel;
            count += 1;
        }
    }
    if count == 0 {
        0.0
    } else {
        (sumsq / count as f32).sqrt() * 1e6
    }
}

/// One MAT run's outcome on one realization.
#[derive(Clone, Copy, Default)]
struct Run {
    b: f32,
    f0: f32,
    confidence: f32,
    partials: usize,
    max_n: u32,
    self_resid_ppm: f32,
    /// Estimate produced at all (false only when `detect_pitch_mat` returned `None`).
    ok: bool,
}

fn run_mat(mags: &[f32], cspe: &[f32], seed_f0: f32, order: MatOrder) -> Run {
    let mut freqs = [0.0f32; MAX_PARTIALS];
    let mut ns = [0u32; MAX_PARTIALS];
    match detect_pitch_mat(mags, cspe, SAMPLE_RATE, seed_f0, order, &mut freqs, &mut ns) {
        Some(est) => Run {
            b: est.b,
            f0: est.f0,
            confidence: est.confidence,
            partials: est.partial_count,
            max_n: ns[..est.partial_count].iter().copied().max().unwrap_or(0),
            self_resid_ppm: self_residual_ppm(&est, &freqs[..est.partial_count], &ns[..est.partial_count]),
            ok: true,
        },
        None => Run::default(),
    }
}

// ─────────────────────────── Cell aggregation ───────────────────────────

/// Median + spread summary of a (condition, B, order) cell over `seeds` realizations.
struct Stat {
    n_ok: usize,
    med_b: f32,
    med_ratio_to_true: f32, // recovered / true
    within_20: f32,         // fraction with |recovered−true|/true ≤ 0.20
    med_conf: f32,
    med_partials: f32,
    med_max_n: f32,
    med_self_ppm: f32,
    med_f0_err_pct: f32,
}

fn median(v: &mut [f32]) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

fn summarize(runs: &[Run], true_b: f32, true_f0: f32) -> Stat {
    let ok: Vec<&Run> = runs.iter().filter(|r| r.ok).collect();
    if ok.is_empty() {
        return Stat {
            n_ok: 0,
            med_b: f32::NAN,
            med_ratio_to_true: f32::NAN,
            within_20: 0.0,
            med_conf: f32::NAN,
            med_partials: f32::NAN,
            med_max_n: f32::NAN,
            med_self_ppm: f32::NAN,
            med_f0_err_pct: f32::NAN,
        };
    }
    let mut bs: Vec<f32> = ok.iter().map(|r| r.b).collect();
    let mut ratios: Vec<f32> = ok.iter().map(|r| r.b / true_b).collect();
    let mut confs: Vec<f32> = ok.iter().map(|r| r.confidence).collect();
    let mut parts: Vec<f32> = ok.iter().map(|r| r.partials as f32).collect();
    let mut maxn: Vec<f32> = ok.iter().map(|r| r.max_n as f32).collect();
    let mut sresid: Vec<f32> = ok.iter().map(|r| r.self_resid_ppm).collect();
    let mut f0err: Vec<f32> = ok.iter().map(|r| 100.0 * (r.f0 - true_f0).abs() / true_f0).collect();
    let within = ok.iter().filter(|r| (r.b - true_b).abs() / true_b <= 0.20).count() as f32 / ok.len() as f32;
    Stat {
        n_ok: ok.len(),
        med_b: median(&mut bs),
        med_ratio_to_true: median(&mut ratios),
        within_20: within,
        med_conf: median(&mut confs),
        med_partials: median(&mut parts),
        med_max_n: median(&mut maxn),
        med_self_ppm: median(&mut sresid),
        med_f0_err_pct: median(&mut f0err),
    }
}

/// Runs `seeds` realizations of `cond` (parallel over seeds), returning the per-order stats.
/// The MAT seed f₀ is the true f₀ (the production case when the Goertzel tracker locks);
/// seed sensitivity is probed separately.
fn run_cell(cond: Cond, seeds: usize, salt: u64) -> (Stat, Stat) {
    run_cell_seeded(cond, seeds, salt, cond.f0)
}

fn run_cell_seeded(cond: Cond, seeds: usize, salt: u64, seed_f0: f32) -> (Stat, Stat) {
    let nthreads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1);
    let chunk = seeds.div_ceil(nthreads).max(1);
    let (ser, sim): (Vec<Run>, Vec<Run>) = std::thread::scope(|sc| {
        let handles: Vec<_> = (0..seeds)
            .collect::<Vec<_>>()
            .chunks(chunk)
            .map(|idxs| {
                let idxs = idxs.to_vec();
                sc.spawn(move || {
                    // One FFT plan per worker thread, reused across its seeds.
                    let mut planner = RealFftPlanner::<f32>::new();
                    let r2c = planner.plan_fft_forward(FFT_SIZE);
                    let mut out_ser = Vec::new();
                    let mut out_sim = Vec::new();
                    for &i in &idxs {
                        let mut rng = Rng::new(
                            salt ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
                                ^ (cond.b.to_bits() as u64) << 11,
                        );
                        let (mags, cspe) = synth_spectrum(&cond, &mut rng, &r2c);
                        out_ser.push(run_mat(&mags, &cspe, seed_f0, MatOrder::Serial));
                        out_sim.push(run_mat(&mags, &cspe, seed_f0, MatOrder::Simultaneous));
                    }
                    (out_ser, out_sim)
                })
            })
            .collect();
        let mut ser = Vec::new();
        let mut sim = Vec::new();
        for h in handles {
            let (s, m) = h.join().unwrap();
            ser.extend(s);
            sim.extend(m);
        }
        (ser, sim)
    });
    (summarize(&ser, cond.b, cond.f0), summarize(&sim, cond.b, cond.f0))
}

// ─────────────────────────── Reporting ───────────────────────────

fn sweep_header(title: &str) {
    println!("\n── {title} ──");
    println!(
        "{:>5} {:>9} | {:>8} {:>7} {:>6} {:>5} {:>4} {:>4} {:>8} {:>6} | {:>8} {:>7} {:>6} {:>5} {:>4} {:>4} {:>8} {:>6}",
        "B×pr", "true_B",
        "B_ser", "rat/tru", "in20%", "conf", "pt", "maxn", "self_ppm", "f0e%",
        "B_sim", "rat/tru", "in20%", "conf", "pt", "maxn", "self_ppm", "f0e%",
    );
}

fn sweep_row(ratio_to_prior: f32, true_b: f32, ser: &Stat, sim: &Stat) {
    let cell = |s: &Stat| {
        if s.n_ok == 0 {
            format!("{:>8} {:>7} {:>6} {:>5} {:>4} {:>4} {:>8} {:>6}", "None", "-", "-", "-", "-", "-", "-", "-")
        } else {
            format!(
                "{:>8.2e} {:>7.2} {:>5.0}% {:>5.2} {:>4.0} {:>4.0} {:>8.0} {:>6.2}",
                s.med_b, s.med_ratio_to_true, 100.0 * s.within_20, s.med_conf,
                s.med_partials, s.med_max_n, s.med_self_ppm, s.med_f0_err_pct,
            )
        }
    };
    println!("{:>5.1} {:>9.2e} | {} | {}", ratio_to_prior, true_b, cell(ser), cell(sim));
}

fn main() {
    println!("MAT (f₀, B) ground-truth recovery — FFT {FFT_SIZE} ({:.3} Hz/bin), SNR {SNR_DB} dB, SR {SAMPLE_RATE}",
        SAMPLE_RATE as f32 / FFT_SIZE as f32);
    println!("Columns per order: med recovered_B, med(recovered/true), %within±20%, med conf, med partials, med max-n, med self-fit residual (ppm), med |f0 err|%.");
    println!("Mis-association signature = low self_ppm but rat/tru far from 1.0 (self-consistent yet wrong).");

    let seeds = 24;

    // ── Experiment 1: Baseline (prior-level B, full clean single-ish partials) ──
    println!("\n========== EXPERIMENT 1 — Baseline (prior B, full partials) — sanity ==========");
    sweep_header("bass A0 / mid C3 / treble A5 at prior B");
    for &key in &[0usize, 27, 60] {
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        let alpha = if key < 27 { 0.6 } else { 1.1 };
        let n_strings = if key < 8 { 1 } else if key < 26 { 2 } else { 3 };
        let cond = Cond {
            f0,
            b: prior,
            n_strings,
            spread_cents: 2.0,
            alpha,
            missing_fundamental: false,
            sympathetic: false,
            drop_below: 0,
        };
        let (ser, sim) = run_cell(cond, seeds, 0x0001 ^ key as u64);
        print!("{:<5} ", NOTES[key].name);
        sweep_row(1.0, prior, &ser, &sim);
    }

    // The deep-bass key the sweeps center on (A0 — deepest, single-string, most missing-
    // fundamental doubt; prior B ≈ 1.0e-4). C1 is also one of the real broken keys.
    let sweep_keys: [(usize, &str); 2] = [(0, "A0"), (3, "C1")];
    let ratios = [1.0f32, 2.0, 3.0, 5.0, 7.0, 10.0, 14.0, 18.0, 22.0, 25.0];

    // ── Experiment 2: B sweep, full partials ──
    println!("\n========== EXPERIMENT 2 — B sweep, FULL partials (no missing fundamental) ==========");
    for &(key, name) in &sweep_keys {
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        let n_strings = if key < 8 { 1 } else if key < 26 { 2 } else { 3 };
        sweep_header(&format!("{name} (f0 {f0:.2} Hz, prior B {prior:.2e}), full partials"));
        for &ratio in &ratios {
            let cond = Cond {
                f0,
                b: prior * ratio,
                n_strings,
                spread_cents: 2.0,
                alpha: 0.6,
                missing_fundamental: false,
                sympathetic: false,
                drop_below: 0,
            };
            let (ser, sim) = run_cell(cond, seeds, 0x0200 ^ key as u64);
            sweep_row(ratio, prior * ratio, &ser, &sim);
        }
    }

    // ── Experiment 3: B sweep, missing fundamental (THE key one) ──
    println!("\n========== EXPERIMENT 3 — B sweep, MISSING FUNDAMENTAL (partials 1–3 attenuated) ==========");
    for &(key, name) in &sweep_keys {
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        let n_strings = if key < 8 { 1 } else if key < 26 { 2 } else { 3 };
        sweep_header(&format!("{name} (f0 {f0:.2} Hz, prior B {prior:.2e}), missing fundamental + sympathetic decoy"));
        for &ratio in &ratios {
            let cond = Cond {
                f0,
                b: prior * ratio,
                n_strings,
                spread_cents: 2.0,
                alpha: 0.6,
                missing_fundamental: true,
                sympathetic: true,
                drop_below: 0,
            };
            let (ser, sim) = run_cell(cond, seeds, 0x0300 ^ key as u64);
            sweep_row(ratio, prior * ratio, &ser, &sim);
        }
    }

    // ── Experiment 4: Parallel-string stress ──
    // Upper-bass key with 2 unison strings, wide spread → high-n divergence into resolved /
    // beating clusters (DAFx-09 Conclusion/§4). D2 (key 17) is one of the real broken keys.
    println!("\n========== EXPERIMENT 4 — Parallel-string stress (2 strings, 12¢ spread, high-n divergence) ==========");
    {
        let key = 17usize;
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        sweep_header(&format!("D2 (f0 {f0:.2} Hz, prior B {prior:.2e}), 2 strings @ 12¢, full partials"));
        for &ratio in &[1.0f32, 3.0, 7.0, 14.0, 25.0] {
            let cond = Cond {
                f0,
                b: prior * ratio,
                n_strings: 2,
                spread_cents: 12.0,
                alpha: 0.6,
                missing_fundamental: false,
                sympathetic: false,
                drop_below: 0,
            };
            let (ser, sim) = run_cell(cond, seeds, 0x0400 ^ key as u64);
            sweep_row(ratio, prior * ratio, &ser, &sim);
        }
        // Same key/B, single string @ 2¢ — the isolation control for the parallel-string bias.
        sweep_header("D2 control: 1 string @ 2¢ (isolates the parallel-string effect above)");
        for &ratio in &[1.0f32, 3.0, 7.0, 14.0, 25.0] {
            let cond = Cond {
                f0,
                b: prior * ratio,
                n_strings: 1,
                spread_cents: 2.0,
                alpha: 0.6,
                missing_fundamental: false,
                sympathetic: false,
                drop_below: 0,
            };
            let (ser, sim) = run_cell(cond, seeds, 0x0401 ^ key as u64);
            sweep_row(ratio, prior * ratio, &ser, &sim);
        }
    }

    // ── Experiment 5: Deep missing-fundamental — how many low partials can vanish? ──
    // A0, high B (18×), good seed: fully remove partials 1..=k. The graded gen_frame model only
    // touches 1–3; this pushes further to find where the trajectory loses its anchor and the
    // surviving partials get mis-numbered (the worst real bass hazard, taken past the model).
    println!("\n========== EXPERIMENT 5 — Deep missing-fundamental (drop partials 1..=k), A0 B=18×prior ==========");
    {
        let key = 0usize;
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        let true_b = prior * 18.0;
        sweep_header(&format!("A0 (true B {true_b:.2e}, good seed); 'B×pr' column shows k = lowest present partial − 1"));
        for &k in &[0u32, 3, 5, 6, 8, 10, 12] {
            let cond = Cond {
                f0,
                b: true_b,
                n_strings: 1,
                spread_cents: 2.0,
                alpha: 0.6,
                missing_fundamental: false,
                sympathetic: false,
                drop_below: k,
            };
            let (ser, sim) = run_cell(cond, seeds, 0x0600 ^ key as u64 ^ (k as u64) << 8);
            // Reuse the ratio column to print k (partials 1..=k removed).
            sweep_row(k as f32, true_b, &ser, &sim);
        }
    }

    // ── Seed-sensitivity probe: does a wrong f₀ seed trigger mis-association? ──
    // A0, high B (18×) — the worst real cell — seeded off true f₀ across a wide range, to map
    // the cliff between "robust" and "locks onto a wrong-but-self-consistent series". Run both
    // WITH the missing fundamental and WITHOUT it, to isolate whether the hazard is the missing
    // fundamental or purely the seed. (A wrong-octave seed is what the Goertzel tracker can
    // report when the true fundamental is absent.)
    println!("\n========== SEED SENSITIVITY — A0, B=18×prior (maps the octave-mis-association cliff) ==========");
    {
        let key = 0usize;
        let f0 = NOTES[key].frequency;
        let prior = get_expected_beta(key as u8);
        let true_b = prior * 18.0;
        let seed_mults = [0.5f32, 0.75, 0.9, 0.94, 0.97, 1.0, 1.03, 1.06, 1.1, 1.25, 1.5, 1.75, 2.0];
        for &missing in &[true, false] {
            let cond = Cond {
                f0,
                b: true_b,
                n_strings: 1,
                spread_cents: 2.0,
                alpha: 0.6,
                missing_fundamental: missing,
                sympathetic: missing,
                drop_below: 0,
            };
            sweep_header(&format!(
                "A0 seed sweep, missing_fundamental={missing} (true f0 {f0:.2} Hz, true B {true_b:.2e})"
            ));
            for &mult in &seed_mults {
                let (ser, sim) = run_cell_seeded(cond, seeds, 0x0500 ^ key as u64, f0 * mult);
                print!("s{:>4.2} ", mult);
                sweep_row(18.0, true_b, &ser, &sim);
            }
        }
    }

    println!("\n(Interpretation: see the writeup. ±20% band is the accuracy gate; rat/tru≈1 with");
    println!(" the diagonal tracked up to 25× ⇒ deep-bass readings trustworthy ⇒ Prompt 3 founded.)");
}
