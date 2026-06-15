//! # MOBO Evaluator — Synthetic Dataset Generator + Discovery Fitness Harness
//!
//! Phase 3 of `.agents/claude_implementation_plan.md` (ADR 0001 methodology,
//! ADR 0005 split discovery). Generates the fixed-seed synthetic piano dataset
//! entirely in the peak domain (no FFT — peak *detection* is a different
//! problem), applies the production `mask_peaks` selection exactly once, and
//! evaluates the shared `discovery` search against ground truth.
//!
//! Physics grounding (all constants cited in `.agents/claude_implementation_plan.md`):
//! - Partial model f_n = n·f0·√(1+Bn²), B from the Rigaud two-bridge curve the
//!   engine already uses (`get_expected_beta`), treble bridge fixed across
//!   pianos, bass bridge piano-dependent.       [Rigaud, David, Daudet DAFx-11]
//! - Per-note B scatter: ×(1+N(0,σ)), σ ≈ 0.157 (A0–B4) / 0.116 (C4–C8).
//!   [Rigaud Fig. 3, 5 pianos]
//! - B↔f0 coupling under detuning: ΔB/B = −2·Δf0/f0.        [Rigaud fn. 1]
//! - Baseline tuning = ET × Railsback stretch via the ρ-type-octave recursion
//!   (m0≈64, α≈24, K≈4.51; per-piano variants).  [Rigaud §4.2, Fig. 4–5]
//! - Unison spread 0→15 cents (concert 0–2, neglected 5–15), string count by
//!   register (1 low bass / 2 upper bass / 3 from tenor). Split peaks, merges
//!   and beating dropouts EMERGE from per-string phasor sums vs. the 5.38 Hz/bin
//!   resolution — never injected as flags.
//!
//! Labels: the generated key, by construction. A frame is SCORED only when its
//! total ET-deviation |D| ≤ 45 cents (own basin strictly nearest); beyond that
//! it lands in the reported "ambiguous" bucket (pitch-raise regime, v2).
//!
//! Determinism: hand-rolled SplitMix64 (not `rand::StdRng`, whose stream is not
//! guaranteed stable across crate versions). Same seed → byte-identical dataset,
//! forever. The harness asserts this at startup by double-generating.

use std::time::Instant;

use tuner_core::algorithms::discovery::{self, TOP_K};
use tuner_core::algorithms::peaks::{SpectralPeak, mask_peaks};
use tuner_core::algorithms::twm::TwmConfig;
use tuner_core::engine::KeyProfile;
use tuner_core::models::get_expected_beta;

const FIXED_SEED: u64 = 0x1AB4_2026_0612_5EED;
const BASE_FRAMES_PER_KEY: usize = 100;
const HARD_FRAMES_PER_KEY: usize = 20;
/// Keys excluded from targeted "hard" oversampling: the historical real
/// false-lock pairs (D2/E1, F#3/A0, D#4/A#0) are held out as validation.
const HOLDOUT_KEYS: [usize; 6] = [17, 7, 33, 0, 42, 1];
/// FFT bin width of the production bass window (44100 / 8192).
const HZ_PER_BIN: f32 = 44100.0 / 8192.0;
/// Scored-set clamp: |D| ≤ 45 cents keeps the labeled key strictly nearest on
/// the ET grid with margin inside the ±80-cent refinement window.
const AMBIGUOUS_CENTS: f32 = 45.0;

// ─────────────────────────── Deterministic RNG ───────────────────────────

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed)
    }
    /// SplitMix64 (Steele, Lea & Flood 2014) — tiny, fast, stable by construction.
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    /// Uniform in [0, 1).
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
        (-2.0 * u1.ln()).sqrt() * (2.0 * core::f32::consts::PI * u2).cos()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

/// Abramowitz & Stegun 7.1.26 (|err| < 1.5e-7) — std has no erf.
fn erf(x: f32) -> f32 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly =
        ((((1.061_405_4 * t - 1.453_152_1) * t + 1.421_413_8) * t - 0.284_496_72) * t
            + 0.254_829_6)
            * t;
    sign * (1.0 - poly * (-x * x).exp())
}

fn et_freq(key: usize) -> f32 {
    27.5 * 2f32.powf(key as f32 / 12.0)
}

// ───────────────────────── Synthetic piano model ─────────────────────────

struct Piano {
    /// Smooth per-instrument B curve (the tuner-visible model, pre-scatter).
    b_curve: [f32; 88],
    /// Railsback stretch + global detune, in cents from ET, per key.
    stretch_cents: [f32; 88],
}

fn synth_piano(rng: &mut Rng) -> Piano {
    // Two-bridge B model in the engine's key convention (n = key + 1; the
    // engine's treble term equals Rigaud's fixed sT=0.0926 after the A0 index
    // shift). Treble bridge FIXED across pianos; bass bridge {slope, intercept}
    // is the piano-dependent part (Rigaud §3.2): slope ±10%, intercept spread
    // ~×1.5 (1σ) covering the cross-piano bass range.
    let s_b = -0.066 * (1.0 + 0.10 * rng.normal());
    let y_b = -9.211 + 0.40 * rng.normal();
    const S_T: f32 = 0.0926;
    const Y_T: f32 = -11.788;

    let mut b_curve = [0.0f32; 88];
    for (k, b) in b_curve.iter_mut().enumerate() {
        let n = k as f32 + 1.0;
        *b = (s_b * n + y_b).exp() + (S_T * n + Y_T).exp();
    }

    // ρ-type-octave stretch model (Rigaud Eq. 9): mean fit m0≈64, α≈24, K≈4.51;
    // low/high stretching variants were ρ±1, covered here by K ∈ [3.5, 5.5].
    let k_stretch = rng.range(3.5, 5.5);
    let m0 = 64.0 + 3.0 * rng.normal();
    let alpha = (24.0 + 3.0 * rng.normal()).max(10.0);
    let rho = |key: usize| -> f32 {
        let m = key as f32 + 21.0; // MIDI index
        (k_stretch / 2.0) * (1.0 - erf((m - m0) / alpha)) + 1.0
    };

    // Global detune (regularly-tuned pianos drift 1–3 cents/yr, neglected 5–15;
    // beyond ~25 cents is pitch-raise territory → lands in the ambiguous bucket
    // by design via the |D| clamp).
    let dg = (8.0 * rng.normal()).clamp(-25.0, 25.0);

    // Octave recursion on the A keys from A4 outward (Rigaud Eq. 8), in the
    // flexible-string F0 domain; f1 = F0·√(1+B).
    let b = &b_curve;
    let mut f1 = [0.0f32; 88];
    let mut f0 = [0.0f32; 88];
    f0[48] = 440.0 * 2f32.powf(dg / 1200.0) / (1.0 + b[48]).sqrt(); // A4 anchor
    for a in [60usize, 72, 84] {
        let r = rho(a);
        f0[a] = 2.0 * f0[a - 12] * ((1.0 + b[a - 12] * 4.0 * r * r) / (1.0 + b[a] * r * r)).sqrt();
    }
    for a in [36usize, 24, 12, 0] {
        let r = rho(a + 12);
        f0[a] =
            f0[a + 12] / (2.0 * ((1.0 + b[a] * 4.0 * r * r) / (1.0 + b[a + 12] * r * r)).sqrt());
    }
    for a in [0usize, 12, 24, 36, 48, 60, 72, 84] {
        f1[a] = f0[a] * (1.0 + b[a]).sqrt();
    }

    // Semitone fill inside each A–A octave (Rigaud Eq. 12–14):
    // λ = 24·ln(f1(a+12)/(2·f1(a))) / Σ B(a+p);  f1(m+1) = f1(m)·(2+λ·B(m+1))^(1/12)
    let mut last_lambda = 0.0f32;
    for a in [0usize, 12, 24, 36, 48, 60, 72] {
        let b_sum: f32 = (1..=12).map(|p| b[a + p]).sum();
        let lambda = 24.0 * (f1[a + 12] / (2.0 * f1[a])).ln() / b_sum.max(1e-9);
        last_lambda = lambda;
        for p in 1..12 {
            f1[a + p] = f1[a + p - 1] * (2.0 + lambda * b[a + p]).powf(1.0 / 12.0);
        }
    }
    for k in 85..88 {
        f1[k] = f1[k - 1] * (2.0 + last_lambda * b[k]).powf(1.0 / 12.0);
    }

    let mut stretch_cents = [0.0f32; 88];
    for k in 0..88 {
        stretch_cents[k] = 1200.0 * (f1[k] / et_freq(k)).log2();
    }

    Piano {
        b_curve,
        stretch_cents,
    }
}

// ───────────────────────────── Frame generation ─────────────────────────────

struct Frame {
    key: u8,
    /// Ground-truth total deviation from ET in cents (stretch + error).
    d_cents: f32,
    /// |D| > 45 cents: generated and reported, excluded from objectives.
    ambiguous: bool,
    /// From the targeted confusable oversampling set.
    hard: bool,
    /// Masked, frequency-ascending peaks (the exact TWM input contract).
    peaks: Vec<SpectralPeak>,
}

/// Raw (pre-mask) peak emission for one string-set partial cluster.
/// Per-string phasor sum: splits, merges, and beating dropouts fall out of the
/// spacing vs. HZ_PER_BIN and the random relative phases — no artifact flags.
fn emit_partial_cluster(
    rng: &mut Rng,
    freqs: &[f32],   // per-string frequency of this partial
    amps: &[f32],    // per-string amplitude
    out: &mut Vec<SpectralPeak>,
) {
    // Greedy clustering by gap: strings whose components sit within 1.5 bins
    // cannot form separate local maxima under a 4-bin Hann mainlobe.
    let n = freqs.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| freqs[a].partial_cmp(&freqs[b]).unwrap());

    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (freqs[order[j + 1]] - freqs[order[j]]).abs() < 1.5 * HZ_PER_BIN {
            j += 1;
        }
        // Coherent phasor sum of the merged group (random relative phases →
        // occasional destructive nulls = the NINOS2-visible unison dropouts).
        let (mut re, mut im, mut wsum, mut fsum) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for &s in &order[i..=j] {
            let phi = rng.range(0.0, 2.0 * core::f32::consts::PI);
            re += amps[s] * phi.cos();
            im += amps[s] * phi.sin();
            wsum += amps[s];
            fsum += amps[s] * freqs[s];
        }
        let mag = (re * re + im * im).sqrt();
        if mag > 1e-6 && wsum > 0.0 {
            // Jacobsen sub-bin jitter stand-in: ~N(0, 0.2 Hz).
            let freq = fsum / wsum + 0.2 * rng.normal();
            if freq > 10.0 {
                out.push(SpectralPeak {
                    frequency: freq,
                    magnitude: mag,
                });
            }
        }
        i = j + 1;
    }
}

fn gen_frame(rng: &mut Rng, piano: &Piano, key: usize, hard: bool) -> Frame {
    let sigma_b = if key <= 50 { 0.157 } else { 0.116 }; // Rigaud Fig. 3 split
    let b_note = (piano.b_curve[key] * (1.0 + sigma_b * rng.normal())).max(1e-7);

    // Tuning error on top of the stretch: low-weighted sweep 0→25 cents
    // (u² mass near 0: in-tune 0–3, service 3–10, neglected 10–25). Hard frames
    // bias toward the adjacent-basin regime seen in the Phase 2 baseline.
    let u = rng.f32();
    let mut err = 25.0 * u * u * if rng.chance(0.5) { 1.0 } else { -1.0 };
    if hard {
        err = rng.range(25.0, 44.0) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    }
    let d_cents = piano.stretch_cents[key] + err;
    let ambiguous = d_cents.abs() > AMBIGUOUS_CENTS;

    // B↔f0 coupling (Rigaud fn. 1): ΔB/B = −2·Δf0/f0.
    let df_rel = 2f32.powf(d_cents / 1200.0) - 1.0;
    let b_actual = (b_note * (1.0 - 2.0 * df_rel)).max(1e-7);
    let f0 = et_freq(key) * 2f32.powf(d_cents / 1200.0);

    // Unison string count by register (scale-design convention: single wound
    // strings in the low bass, doubles in the upper bass, triples from tenor).
    let n_strings = if key < 8 {
        1
    } else if key < 26 {
        2
    } else {
        3
    };
    // Unison spread sweep 0→15 cents, low-weighted (concert 0–2, neglected 5–15).
    let v = rng.f32();
    let spread = 15.0 * v * v;
    let mut offsets = [0.0f32; 3];
    for o in offsets.iter_mut().take(n_strings).skip(1) {
        *o = rng.range(-spread / 2.0, spread / 2.0);
    }
    let mut f0_s = [0.0f32; 3];
    for s in 0..n_strings {
        f0_s[s] = f0 * 2f32.powf(offsets[s] / 1200.0);
    }

    // Spectral envelope a_n = n^(−α) with per-partial lognormal jitter. Bass
    // strings carry strong high partials (soundboard impedance kills the lows
    // instead): flatter α in the bass.
    let alpha_env = if key < 27 {
        rng.range(0.4, 0.9)
    } else {
        rng.range(0.8, 1.6)
    };

    let mut raw: Vec<SpectralPeak> = Vec::with_capacity(96);
    let mut n = 1usize;
    while n <= 64 {
        let f_nominal = (n as f32) * f0 * (1.0 + b_actual * (n * n) as f32).sqrt();
        if f_nominal > 9000.0 {
            break;
        }
        let mut a_n = (n as f32).powf(-alpha_env) * (0.5 * rng.normal()).exp();
        // Missing fundamentals in the low bass (soundboard impedance): drop
        // partials 1–3 with high probability for the lowest keys.
        if key < 15 && (key < 10 || rng.chance(0.7)) {
            match n {
                1 => a_n *= rng.range(0.0, 0.15),
                2 => a_n *= rng.range(0.1, 0.6),
                3 => a_n *= rng.range(0.3, 1.0),
                _ => {}
            }
        }
        let mut freqs = [0.0f32; 3];
        let mut amps = [0.0f32; 3];
        for s in 0..n_strings {
            freqs[s] = (n as f32) * f0_s[s] * (1.0 + b_actual * (n * n) as f32).sqrt();
            amps[s] = a_n * (1.0 + 0.1 * rng.normal()).max(0.05);
        }
        emit_partial_cluster(rng, &freqs[..n_strings], &amps[..n_strings], &mut raw);
        n += 1;
    }

    let a_max = raw
        .iter()
        .map(|p| p.magnitude)
        .fold(1e-6f32, f32::max);

    // Sympathetic resonance at musically related intervals (octave below /
    // fifth above ring through the bridge): the energy that makes dense
    // sub-harmonic impostors cheap. −20…−35 dB rel. max, own slight detune.
    if rng.chance(if hard { 0.8 } else { 0.45 }) {
        let rel = if rng.chance(0.6) { 0.5 } else { 1.5 };
        let f0_sym = f0 * rel * 2f32.powf(rng.range(-10.0, 10.0) / 1200.0);
        let n_sym = 3 + (rng.f32() * 6.0) as usize;
        for m in 1..=n_sym {
            let f = (m as f32) * f0_sym;
            if f > 9000.0 {
                break;
            }
            let db = rng.range(-35.0, -20.0);
            raw.push(SpectralPeak {
                frequency: f + 0.2 * rng.normal(),
                magnitude: a_max * 10f32.powf(db / 20.0),
            });
        }
    }

    // Broadband noise peaks (pink-weighted, log-uniform frequency).
    let n_noise = 3 + (rng.f32() * 12.0) as usize;
    for _ in 0..n_noise {
        let f = 25.0 * (9000.0f32 / 25.0).powf(rng.f32());
        let db = rng.range(-45.0, -25.0) - 6.0 * (f / 1000.0).max(1.0).log2(); // pinkish tilt
        raw.push(SpectralPeak {
            frequency: f,
            magnitude: a_max * 10f32.powf(db / 20.0),
        });
    }
    // Treble attack archetype (Phase 2 baseline: treble keys failed on dense
    // 69–128-peak post-attack frames of broadband low/mid content).
    if key >= 55 && rng.chance(0.35) {
        let n_attack = 30 + (rng.f32() * 40.0) as usize;
        for _ in 0..n_attack {
            let f = 50.0 * (4000.0f32 / 50.0).powf(rng.f32());
            let db = rng.range(-40.0, -22.0);
            raw.push(SpectralPeak {
                frequency: f,
                magnitude: a_max * 10f32.powf(db / 20.0),
            });
        }
    }

    // ── Production peak-selection contract ──
    // extract_peaks hands the engine the top-64 peaks by magnitude;
    // mask_peaks (Gómez/Cano) then filters and returns ascending order.
    raw.sort_by(|a, b| b.magnitude.partial_cmp(&a.magnitude).unwrap());
    raw.truncate(64);
    let valid = mask_peaks(&mut raw);
    raw.truncate(valid);

    Frame {
        key: key as u8,
        d_cents,
        ambiguous,
        hard,
        peaks: raw,
    }
}

fn generate_dataset(seed: u64) -> Vec<Frame> {
    let mut rng = Rng::new(seed);
    let mut frames =
        Vec::with_capacity(88 * BASE_FRAMES_PER_KEY + 82 * HARD_FRAMES_PER_KEY);
    for key in 0..88 {
        for _ in 0..BASE_FRAMES_PER_KEY {
            let piano = synth_piano(&mut rng);
            frames.push(gen_frame(&mut rng, &piano, key, false));
        }
        if !HOLDOUT_KEYS.contains(&key) {
            for _ in 0..HARD_FRAMES_PER_KEY {
                let piano = synth_piano(&mut rng);
                frames.push(gen_frame(&mut rng, &piano, key, true));
            }
        }
    }
    frames
}

fn dataset_fingerprint(frames: &[Frame]) -> u64 {
    let mut h = 0xCBF2_9CE4_8422_2325u64; // FNV-ish fold
    for f in frames {
        h = h.wrapping_mul(0x100_0000_01B3) ^ (f.key as u64);
        h = h.wrapping_mul(0x100_0000_01B3) ^ (f.d_cents.to_bits() as u64);
        for p in &f.peaks {
            h = h.wrapping_mul(0x100_0000_01B3) ^ (p.frequency.to_bits() as u64);
            h = h.wrapping_mul(0x100_0000_01B3) ^ (p.magnitude.to_bits() as u64);
        }
    }
    h
}

/// Stage A design diagnostic. For each scored frame, computes the rank of the
/// TRUE key (1 = best) two ways: (a) the current Stage A — discrete scoring at
/// scale=1.0, and (b) a scale-AWARE variant — each key scored at the min over
/// its 9-point ±80c pre-grid. Stage B can only rescue keys that survive Stage
/// A's top-K, so this decides whether K just needs widening or whether Stage A
/// itself must become detuning-aware (a potential ADR 0005 revision).
fn stage_a_rank_study(frames: &[Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig) {
    use tuner_core::algorithms::twm::score_candidate;
    let pre_grid = |peaks: &[SpectralPeak], profile: &KeyProfile| -> f32 {
        let mut best = f32::MAX;
        for i in 0..9 {
            let c = -80.0 + (i as f32) * 20.0;
            best = best.min(score_candidate(peaks, profile, (c / 1200.0).exp2(), cfg));
        }
        best
    };
    // ranks[register][variant] -> Vec of true-key ranks (1..=88)
    let mut flat: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
    let mut treble: [Vec<u32>; 2] = [Vec::new(), Vec::new()];

    for f in frames.iter().filter(|f| !f.ambiguous) {
        let true_e_flat = score_candidate(&f.peaks, &profiles[f.key as usize], 1.0, cfg);
        let true_e_grid = pre_grid(&f.peaks, &profiles[f.key as usize]);
        let (mut r_flat, mut r_grid) = (1u32, 1u32);
        for (k, profile) in profiles.iter().enumerate() {
            if k == f.key as usize {
                continue;
            }
            if score_candidate(&f.peaks, profile, 1.0, cfg) < true_e_flat {
                r_flat += 1;
            }
            if pre_grid(&f.peaks, profile) < true_e_grid {
                r_grid += 1;
            }
        }
        flat[0].push(r_flat);
        flat[1].push(r_grid);
        if f.key >= 60 {
            treble[0].push(r_flat);
            treble[1].push(r_grid);
        }
    }

    let k_for = |ranks: &[u32], target: f32| -> u32 {
        let mut sorted = ranks.to_vec();
        sorted.sort_unstable();
        let idx = ((sorted.len() as f32 * target).ceil() as usize).min(sorted.len()) - 1;
        sorted[idx]
    };
    println!("── Stage A rank study (K needed for recall; true-key rank in 88-scan) ──");
    for (label, set) in [("overall", &flat), ("treble", &treble)] {
        for (vi, vlabel) in [(0usize, "scale=1.0 (current)"), (1, "pre-grid (scale-aware)")] {
            let r = &set[vi];
            println!(
                "  {label:<8} {vlabel:<24} K@95%={:<3} K@99%={:<3} K@99.9%={:<3} (n={})",
                k_for(r, 0.95),
                k_for(r, 0.99),
                k_for(r, 0.999),
                r.len()
            );
        }
    }
}

// ───────────────────────────── Evaluation ─────────────────────────────

#[derive(Default)]
struct Bucket {
    n: usize,
    /// MOBO Objective A. K-independent: the true key's refined error is NOT the
    /// minimum over {top-K impostors ∪ true key}. Isolates the constants' job
    /// (separability) from the search's job (recall) — the same principle as
    /// "don't bake grid-compensation into the constants". Uses synthetic
    /// ground-truth to refine the true key even when Stage A drops it.
    sep_losses: usize,
    /// Production realism: what `discover()` ships at the current TOP_K (recall
    /// miss ⇒ guaranteed false lock). `prod_false_locks ≥ sep_losses`; the gap
    /// is the cost the current K imposes under these constants.
    prod_false_locks: usize,
    /// True key absent from Stage A top-K (then unrecoverable in production).
    recall_misses: usize,
    fidelity: Vec<f32>, // |ŝ − D| cents, separability-correct locks (refined mode)
}

struct Report {
    overall: Bucket,
    bass: Bucket,    // keys 0–26
    mid: Bucket,     // keys 27–59
    treble: Bucket,  // keys 60–87
    hard: Bucket,
}

fn evaluate(frames: &[Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig, refine: bool) -> Report {
    let mut rep = Report {
        overall: Bucket::default(),
        bass: Bucket::default(),
        mid: Bucket::default(),
        treble: Bucket::default(),
        hard: Bucket::default(),
    };
    for f in frames.iter().filter(|f| !f.ambiguous) {
        let key = f.key as usize;
        let top = discovery::stage_a(&f.peaks, profiles, cfg);
        let recall_miss = !top[..TOP_K].iter().any(|&(k, _)| k == key);

        // Production behavior at the shipped TOP_K.
        let res = discovery::discover(&f.peaks, profiles, cfg, refine);
        let prod_wrong = res.key_index != f.key;

        // K-independent separability: refine the true key plus each top-K
        // impostor; the true key must own the minimum refined error.
        let score_key = |k: usize| -> (f32, f32) {
            if refine {
                discovery::refine_scale(&f.peaks, &profiles[k], cfg)
            } else {
                (1.0, tuner_core::algorithms::twm::score_candidate(&f.peaks, &profiles[k], 1.0, cfg))
            }
        };
        let (true_scale, true_err) = score_key(key);
        let mut sep_loss = false;
        for &(k, _) in &top[..TOP_K] {
            if k == key {
                continue;
            }
            if score_key(k).1 < true_err {
                sep_loss = true;
                break;
            }
        }
        let fid = if !sep_loss && refine {
            Some((1200.0 * true_scale.log2() - f.d_cents).abs())
        } else {
            None
        };

        let tally = |b: &mut Bucket| {
            b.n += 1;
            b.sep_losses += sep_loss as usize;
            b.prod_false_locks += prod_wrong as usize;
            b.recall_misses += recall_miss as usize;
            if let Some(d) = fid {
                b.fidelity.push(d);
            }
        };
        tally(&mut rep.overall);
        match f.key {
            0..=26 => tally(&mut rep.bass),
            27..=59 => tally(&mut rep.mid),
            _ => tally(&mut rep.treble),
        }
        if f.hard {
            tally(&mut rep.hard);
        }
    }
    rep
}

/// One-off dense-scan oracle (plan: validates the pre-grid/golden-section
/// constants): on a subsample, compare `refine_scale` against a 0.5-cent dense
/// scan of the TRUE key's profile. Separates SEARCH error (golden vs dense —
/// should be ≲1 cent) from the PHYSICS floor (dense vs ground truth — B-scatter
/// compensation, unison centroids, jitter), which |ŝ−D| alone conflates.
fn dense_scan_oracle(frames: &[Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig) {
    let mut search_err: Vec<f32> = Vec::new();
    let mut physics_err: Vec<f32> = Vec::new();
    let mut excess_count = 0usize;
    for f in frames.iter().filter(|f| !f.ambiguous).take(400) {
        let profile = &profiles[f.key as usize];
        let (s_ref, e_ref) = discovery::refine_scale(&f.peaks, profile, cfg);
        if e_ref == f32::MAX {
            continue;
        }
        let mut best_c = 0.0f32;
        let mut best_e = f32::MAX;
        let mut c = -80.0f32;
        while c <= 80.0 {
            let e = tuner_core::algorithms::twm::score_candidate(
                &f.peaks,
                profile,
                (c / 1200.0).exp2(),
                cfg,
            );
            if e < best_e {
                best_e = e;
                best_c = c;
            }
            c += 0.5;
        }
        let ref_c = 1200.0 * s_ref.log2();
        search_err.push((ref_c - best_c).abs());
        physics_err.push((best_c - f.d_cents).abs());
        if e_ref > best_e + 0.01 * best_e.abs().max(0.1) {
            excess_count += 1;
        }
    }
    let med = |v: &mut Vec<f32>| -> (f32, f32) {
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        (v[v.len() / 2], v[(v.len() * 95) / 100])
    };
    let (s_med, s_p95) = med(&mut search_err);
    let (p_med, p_p95) = med(&mut physics_err);
    println!("── Dense-scan oracle (n={}, true-key profiles) ──", search_err.len());
    println!(
        "  search  (golden vs dense): med {s_med:.2}c p95 {s_p95:.2}c, error-excess frames {excess_count}"
    );
    println!(
        "  physics (dense vs truth) : med {p_med:.2}c p95 {p_p95:.2}c   <- B-scatter/unison/jitter floor"
    );
}

fn pct(num: usize, den: usize) -> f32 {
    if den == 0 { 0.0 } else { 100.0 * num as f32 / den as f32 }
}

fn print_bucket(name: &str, b: &Bucket, refined: bool) {
    let mut line = format!(
        "  {:<8} n={:<6} sep-loss(ObjA) {:>6.2}%  prod-FL@K={} {:>6.2}%  stageA-miss {:>5.2}%",
        name,
        b.n,
        pct(b.sep_losses, b.n),
        TOP_K,
        pct(b.prod_false_locks, b.n),
        pct(b.recall_misses, b.n)
    );
    if refined && !b.fidelity.is_empty() {
        let mut f = b.fidelity.clone();
        f.sort_by(|a, b| a.partial_cmp(b).unwrap());
        line += &format!(
            "  fidelity med {:.2}c p90 {:.2}c",
            f[f.len() / 2],
            f[(f.len() * 9) / 10]
        );
    }
    println!("{line}");
}

fn main() {
    let t0 = Instant::now();
    let frames = generate_dataset(FIXED_SEED);
    let fp = dataset_fingerprint(&frames);
    // Determinism gate: byte-identical regeneration, every run.
    let fp2 = dataset_fingerprint(&generate_dataset(FIXED_SEED));
    assert_eq!(fp, fp2, "dataset generation is non-deterministic!");

    let ambiguous = frames.iter().filter(|f| f.ambiguous).count();
    let scored = frames.len() - ambiguous;
    let avg_peaks: f32 =
        frames.iter().map(|f| f.peaks.len() as f32).sum::<f32>() / frames.len() as f32;
    println!("── Dataset ──");
    println!(
        "frames {} (scored {}, ambiguous {} = {:.1}%), avg peaks/frame {:.1}, fingerprint {fp:016x}, gen {:.2?}",
        frames.len(),
        scored,
        ambiguous,
        pct(ambiguous, frames.len()),
        avg_peaks,
        t0.elapsed()
    );

    let mut profiles_vec = Vec::with_capacity(88);
    for i in 0..88 {
        profiles_vec.push(KeyProfile::new(et_freq(i), get_expected_beta(i as u8)));
    }
    let profiles: Box<[KeyProfile; 88]> = Box::new(profiles_vec.try_into().unwrap());
    let cfg = TwmConfig::default();

    dense_scan_oracle(&frames, &profiles, &cfg);
    stage_a_rank_study(&frames, &profiles, &cfg);

    for refine in [false, true] {
        let t = Instant::now();
        let rep = evaluate(&frames, &profiles, &cfg, refine);
        println!(
            "── {} (default constants) — {:.2?} ──",
            if refine { "REFINED" } else { "DISCRETE" },
            t.elapsed()
        );
        print_bucket("overall", &rep.overall, refine);
        print_bucket("bass", &rep.bass, refine);
        print_bucket("mid", &rep.mid, refine);
        print_bucket("treble", &rep.treble, refine);
        print_bucket("hard", &rep.hard, refine);

        // ── Pre-optimization gate (plan, Verification): the dataset must
        // CONTAIN the failure mode under discrete defaults.
        if !refine {
            let rate = pct(rep.hard.sep_losses, rep.hard.n);
            if rep.hard.sep_losses == 0 {
                eprintln!("PRE-OPT GATE: FAIL — no separability losses on the hard subset; the dataset does not reproduce the failure mode.");
                std::process::exit(1);
            }
            println!("PRE-OPT GATE: PASS (hard-subset separability-loss rate {rate:.2}% > 0)");
        }
    }
}
