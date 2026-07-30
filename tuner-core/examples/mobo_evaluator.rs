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
use tuner_core::algorithms::peaks::mask_peaks;
use tuner_core::algorithms::twm::{self, TwmConfig};
use tuner_core::models::KeyProfile;
use tuner_core::models::SpectralPeak;
use tuner_core::models::get_expected_beta;

const FIXED_SEED: u64 = 0x1AB4_2026_0612_5EED;
const BASE_FRAMES_PER_KEY: usize = 100;
const HARD_FRAMES_PER_KEY: usize = 20;
/// Keys excluded from targeted "hard" oversampling: the historical real
/// false-lock pairs (D2/E1, F#3/A0, D#4/A#0) are held out as validation.
const HOLDOUT_KEYS: [usize; 6] = [17, 7, 33, 0, 42, 1];
/// FFT bin width of the production bass window (44100 / 8192).
const HZ_PER_BIN: f32 = 44100.0 / 8192.0;
/// Scored-set clamp (cents from ET). Raised to 78 (just inside the ±80-cent
/// refinement window) so the heavily-detuned / pitch-raise stratum is *scored*
/// and labeled by the intended key — the side-search must reach it. Frames whose
/// total deviation exceeds the window are unreachable and stay in the ambiguous
/// bucket (the extreme treble where stretch alone is large — accepted failures).
const AMBIGUOUS_CENTS: f32 = 78.0;

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
    let poly = ((((1.061_405_4 * t - 1.453_152_1) * t + 1.421_413_8) * t - 0.284_496_72) * t
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
    /// The TRUE inharmonicity B the partials were generated with (after per-note
    /// scatter + f0 coupling). The scorer never sees this — it uses the Rigaud prior
    /// `get_expected_beta`. Used only by the oracle-B diagnostic (B-residual gate).
    b_actual: f32,
    /// Masked, frequency-ascending peaks (the exact TWM input contract).
    peaks: Vec<SpectralPeak>,
}

/// Raw (pre-mask) peak emission for one string-set partial cluster.
/// Per-string phasor sum: splits, merges, and beating dropouts fall out of the
/// spacing vs. HZ_PER_BIN and the random relative phases — no artifact flags.
fn emit_partial_cluster(
    rng: &mut Rng,
    freqs: &[f32], // per-string frequency of this partial
    amps: &[f32],  // per-string amplitude
    out: &mut Vec<SpectralPeak>,
) {
    // Greedy clustering by gap: strings whose components sit within 1.5 bins
    // cannot form separate local maxima under a 4-bin Hann mainlobe.
    let n = freqs.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| freqs[a].total_cmp(&freqs[b]));

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
    // OUR synthetic-calibration constants and split point (mis-cited to "Rigaud
    // Fig. 3" pre-audit; the paper contains no scatter statistics — see
    // docs/audits/faithfulness-audit-06-b-prior.md).
    let sigma_b = if key <= 50 { 0.157 } else { 0.116 };
    let b_note = (piano.b_curve[key] * (1.0 + sigma_b * rng.normal())).max(1e-7);

    // Tuning-state distribution: drift ON TOP of the Railsback stretch. Most
    // notes near in-tune, with a meaningful pitch-raise tail (a tuner works on
    // detuned pianos — this is the regime the side-search must earn its keep in).
    let drift = {
        let s = rng.f32();
        if s < 0.30 {
            rng.range(0.0, 5.0) // freshly tuned
        } else if s < 0.80 {
            rng.range(5.0, 25.0) // typical service drift
        } else {
            rng.range(25.0, 70.0) // neglected / pitch-raise
        }
    };
    let mut err = drift * if rng.chance(0.5) { 1.0 } else { -1.0 };
    if hard {
        // Confusable-pair frames: bias to the harder detuning band.
        err = rng.range(20.0, 70.0) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    }
    let d_cents = piano.stretch_cents[key] + err;
    // Scored iff reachable within the refinement window (label = intended key).
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

    let a_max = raw.iter().map(|p| p.magnitude).fold(1e-6f32, f32::max);

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
    raw.sort_by(|a, b| b.magnitude.total_cmp(&a.magnitude));
    raw.truncate(64);
    let valid = mask_peaks(&mut raw);
    raw.truncate(valid);

    Frame {
        key: key as u8,
        d_cents,
        ambiguous,
        hard,
        b_actual,
        peaks: raw,
    }
}

fn generate_dataset(seed: u64) -> Vec<Frame> {
    let mut rng = Rng::new(seed);
    let mut frames = Vec::with_capacity(88 * BASE_FRAMES_PER_KEY + 82 * HARD_FRAMES_PER_KEY);
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
        for (vi, vlabel) in [
            (0usize, "scale=1.0 (current)"),
            (1, "pre-grid (scale-aware)"),
        ] {
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

/// Mergeable, `Copy` trial accumulator (one per thread, summed at the end).
/// Registers: 0 bass (0–26), 1 mid (27–59), 2 treble (60–87), 3 hard subset.
#[derive(Default, Clone, Copy)]
struct Acc {
    n: usize,
    false_locks: usize, // separability = K=88 argmin (diagnostic since 2026-06-20)
    prod_false_locks: usize, // production K=3 via discover() — the OPTIMIZED objA
    /// Production false-locks at K ∈ {2,3,4,5} (diagnostic K-robustness sweep).
    /// objA optimizes the *shipped* K=3 (== prod_false_locks, here index 1), but we
    /// report neighbouring K so the chosen constants' K-sensitivity is a measured
    /// number, not an assumption — K=3 is empirical (best of {3,88} on n=1), not
    /// proven optimal. Computed only in refine mode (0 in discrete).
    prod_fl_k: [usize; 4],
    margin_sum: f64,
    fid_sum: f64,
    fid_n: usize,
    reg_n: [usize; 4],
    reg_fl: [usize; 4],
    // ── Degeneracy probes (diagnostic only; do NOT feed the objectives) ──
    /// Frames whose per-frame median error hit the 1e-3 normalizer floor — i.e.
    /// ≥ half the 88 keys scored ≈ 0. High ⇒ objB is inflated by a collapsed
    /// denominator, not real confidence.
    floor_hits: usize,
    /// Sum of RAW (unnormalized) winning margin (best_impostor − true). Mean near
    /// zero ⇒ the true key "wins" by float noise, not discrimination.
    raw_margin_sum: f64,
    /// Sum of per-frame median error. Mean near zero ⇒ error surface collapsed.
    median_sum: f64,
    /// Frames where the top-2 errors differ by < 1e-4 (a near-tie argmin) ⇒ objA
    /// is decided by tie-break/float noise rather than separability.
    tie_hits: usize,
    /// Detuning strata by |deviation from ET|: 0:<15¢ 1:15–35 2:35–55 3:55–78.
    det_n: [usize; 4],
    det_fl: [usize; 4],
}

impl Acc {
    fn merge(&mut self, o: &Acc) {
        self.n += o.n;
        self.false_locks += o.false_locks;
        self.prod_false_locks += o.prod_false_locks;
        for i in 0..4 {
            self.prod_fl_k[i] += o.prod_fl_k[i];
        }
        self.margin_sum += o.margin_sum;
        self.fid_sum += o.fid_sum;
        self.fid_n += o.fid_n;
        for i in 0..4 {
            self.det_n[i] += o.det_n[i];
            self.det_fl[i] += o.det_fl[i];
        }
        for i in 0..4 {
            self.reg_n[i] += o.reg_n[i];
            self.reg_fl[i] += o.reg_fl[i];
        }
        self.floor_hits += o.floor_hits;
        self.raw_margin_sum += o.raw_margin_sum;
        self.median_sum += o.median_sum;
        self.tie_hits += o.tie_hits;
    }
}

/// Fold one frame's two objectives into `acc`:
/// - **Objective A:** production false lock — `discover()` at the shipped TOP_K
///   returns the wrong key. Minimize.
/// - **Objective B:** ordinal confidence — fraction of the 87 impostors the true
///   key out-scores (rank-based; immune to error-scale inflation AND compression,
///   the failure that let the prior run exploit a degenerate low-λ regime).
///   Maximize. The all-88 error scan it needs also feeds the diagnostic probes.
fn process_frame(
    f: &Frame,
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    refine: bool,
    acc: &mut Acc,
) {
    let key = f.key as usize;
    let mut errs = [0f32; 88];
    let mut true_scale = 1.0f32;
    for (k, e) in errs.iter_mut().enumerate() {
        if refine {
            let (s, err) = discovery::refine_scale(&f.peaks, &profiles[k], cfg);
            *e = err;
            if k == key {
                true_scale = s;
            }
        } else {
            *e = twm::score_candidate(&f.peaks, &profiles[k], 1.0, cfg);
        }
    }

    let true_err = errs[key];
    let mut argmin = 0usize;
    let mut best_imp = f32::MAX;
    for (k, &e) in errs.iter().enumerate() {
        if e < errs[argmin] {
            argmin = k;
        }
        if k != key && e < best_imp {
            best_imp = e;
        }
    }
    // SEPARABILITY = K=88 argmin (does the true key win among ALL candidates).
    // DEMOTED to a diagnostic on 2026-06-20: in refine mode this is exactly the
    // K=88 exhaustive-refinement regime ADR 0006 Finding #1 measured as the *worst*
    // real setting, and its optimum is degenerate (floor-collapse) — so optimizing
    // it was an active mis-direction. Kept only for the objA↔production gap report.
    let false_lock = argmin != key;

    // Objective A (OPTIMIZED, since 2026-06-20) = PRODUCTION false-lock at the shipped
    // TOP_K=3 (discover() = Stage A top-K → Stage B refine → argmin). This optimizes
    // the architecture that actually ships, not the rejected K=88 regime.
    let prod_false_lock = discovery::discover(&f.peaks, profiles, cfg, refine).key_index != f.key;

    // K-robustness diagnostic: production false-lock at K ∈ {2,3,4,5}, reusing the
    // already-computed refined `errs` (no extra refinement) and the unrefined
    // Stage-A scores for top-K ranking — exactly discover()'s selection rule. Lets us
    // SEE whether the chosen constants are K-robust or K=3-overfit, since K=3 is
    // empirical, not proven optimal. Refine mode only (production refines).
    let mut prod_fl_k = [false; 4];
    if refine {
        let mut stage = [0f32; 88];
        for (k, s) in stage.iter_mut().enumerate() {
            *s = twm::score_candidate(&f.peaks, &profiles[k], 1.0, cfg);
        }
        let mut order: [usize; 88] = core::array::from_fn(|i| i);
        order.sort_by(|&a, &b| stage[a].total_cmp(&stage[b]));
        for (ki, &kk) in [2usize, 3, 4, 5].iter().enumerate() {
            let mut best_k = order[0];
            let mut best_err = f32::MAX;
            for &cand in order.iter().take(kk) {
                if errs[cand] < best_err {
                    best_err = errs[cand];
                    best_k = cand;
                }
            }
            prod_fl_k[ki] = best_k != key;
        }
    }

    // ── Objective B: ORDINAL confidence (replaces the median-normalized margin).
    // Fraction of the 87 impostors the true key out-scores. Purely rank-based, so
    // it is immune to BOTH error-scale inflation AND compression — the failure
    // that let the previous run exploit a degenerate low-λ regime. 1.0 = true key
    // beats everything; 0 = true key is worst. Maximize.
    let mut beaten = 0usize;
    for (k, &e) in errs.iter().enumerate() {
        if k != key && e > true_err {
            beaten += 1;
        }
    }
    let ordinal = beaten as f64 / 87.0;

    // Degeneracy probes (diagnostic only — to confirm objB is no longer gamed).
    let mut sorted = errs;
    sorted.sort_unstable_by(|a, b| a.total_cmp(b));
    let median = 0.5 * (sorted[43] + sorted[44]);

    acc.n += 1;
    acc.false_locks += false_lock as usize; // separability K=88 (diagnostic)
    acc.prod_false_locks += prod_false_lock as usize; // production K=3 = OPTIMIZED objA
    for (a, &fl) in acc.prod_fl_k.iter_mut().zip(prod_fl_k.iter()) {
        *a += fl as usize; // K∈{2,3,4,5} robustness diagnostic
    }
    acc.margin_sum += ordinal; // objB accumulator (ordinal)
    acc.raw_margin_sum += (best_imp - true_err) as f64;
    acc.median_sum += median as f64;
    if median <= 1e-3 {
        acc.floor_hits += 1;
    }
    if sorted[1] - sorted[0] < 1e-4 {
        acc.tie_hits += 1;
    }
    if !false_lock && refine {
        acc.fid_sum += (1200.0 * true_scale.log2() - f.d_cents).abs() as f64;
        acc.fid_n += 1;
    }
    // Detuning strata (by |deviation from ET|), using the PRODUCTION outcome so the
    // strata reveal the Stage-A recall cap at high detuning (the pitch-raise issue).
    let det = f.d_cents.abs();
    let di = if det < 15.0 {
        0
    } else if det < 35.0 {
        1
    } else if det < 55.0 {
        2
    } else {
        3
    };
    acc.det_n[di] += 1;
    acc.det_fl[di] += prod_false_lock as usize;
    let reg = match f.key {
        0..=26 => 0,
        27..=59 => 1,
        _ => 2,
    };
    acc.reg_n[reg] += 1;
    acc.reg_fl[reg] += prod_false_lock as usize;
    if f.hard {
        acc.reg_n[3] += 1;
        acc.reg_fl[3] += prod_false_lock as usize;
    }
}

/// One trial = score every scored frame under `cfg`/`mode`. Embarrassingly
/// parallel over frames (read-only shared data), chunked across the available
/// cores via `thread::scope` — at TOP_K=88 a single trial is ~16 s
/// single-threaded, so this is what keeps the MOBO loop in the hours, not days.
fn run_trial(scored: &[&Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig, refine: bool) -> Acc {
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk = scored.len().div_ceil(nthreads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = scored
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    let mut acc = Acc::default();
                    for &f in c {
                        process_frame(f, profiles, cfg, refine, &mut acc);
                    }
                    acc
                })
            })
            .collect();
        let mut total = Acc::default();
        for h in handles {
            total.merge(&h.join().unwrap());
        }
        total
    })
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
        v.sort_by(|a, b| a.total_cmp(b));
        (v[v.len() / 2], v[(v.len() * 95) / 100])
    };
    let (s_med, s_p95) = med(&mut search_err);
    let (p_med, p_p95) = med(&mut physics_err);
    println!(
        "── Dense-scan oracle (n={}, true-key profiles) ──",
        search_err.len()
    );
    println!(
        "  search  (golden vs dense): med {s_med:.2}c p95 {s_p95:.2}c, error-excess frames {excess_count}"
    );
    println!(
        "  physics (dense vs truth) : med {p_med:.2}c p95 {p_p95:.2}c   <- B-scatter/unison/jitter floor"
    );
}

/// Oracle-B diagnostic (ADR 0005 revisit #3 / mobo-methodology §8.2 gate): how much
/// of the bass false-lock is a *wrong-B-template* problem vs a peak-domain limit?
///
/// For each scored frame we measure SEPARABILITY (does the true key win the all-88
/// argmin — the ranking question B affects, isolated from K-recall): once with the
/// shipped prior-B templates, and once giving the TRUE key its TRUE generated B
/// (`f.b_actual`) while impostors stay at the prior. The drop = the ceiling on what
/// joint (f₀,B) estimation could buy.
///
/// CAVEAT (lower bound): on the synthetic the true B is distributed *around* the
/// prior (per-instrument bridge variation + zero-mean scatter), NOT the
/// one-directional ~7× bass gap Review 1 found on the real piano. So a large drop
/// here ⇒ B is definitely a real lever (the real gap is bigger); a small drop is
/// inconclusive for real (the real gap is under-represented here).
fn b_oracle_diagnostic(frames: &[Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig) {
    let scored: Vec<&Frame> = frames.iter().filter(|f| !f.ambiguous).collect();
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk = scored.len().div_ceil(nthreads).max(1);
    // per register [bass, mid, treble]: [n, fl_prior, fl_oracle, sum(b_true/b_prior)]
    let tally = std::thread::scope(|s| {
        let handles: Vec<_> = scored
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    let mut t = [[0.0f64; 4]; 3];
                    for &f in c {
                        let key = f.key as usize;
                        let reg = if key <= 26 {
                            0
                        } else if key <= 59 {
                            1
                        } else {
                            2
                        };
                        let mut errs = [f32::MAX; 88];
                        for (k, e) in errs.iter_mut().enumerate() {
                            *e = discovery::refine_scale(&f.peaks, &profiles[k], cfg).1;
                        }
                        let mut am = 0usize;
                        for k in 1..88 {
                            if errs[k] < errs[am] {
                                am = k;
                            }
                        }
                        let oracle = KeyProfile::new(et_freq(key), f.b_actual);
                        let oe = discovery::refine_scale(&f.peaks, &oracle, cfg).1;
                        let mut amo = 0usize;
                        let mut best = f32::MAX;
                        for (k, &err) in errs.iter().enumerate() {
                            let e = if k == key { oe } else { err };
                            if e < best {
                                best = e;
                                amo = k;
                            }
                        }
                        t[reg][0] += 1.0;
                        t[reg][1] += (am != key) as i32 as f64;
                        t[reg][2] += (amo != key) as i32 as f64;
                        t[reg][3] += (f.b_actual / get_expected_beta(f.key)) as f64;
                    }
                    t
                })
            })
            .collect();
        let mut total = [[0.0f64; 4]; 3];
        for h in handles {
            let t = h.join().unwrap();
            for r in 0..3 {
                for i in 0..4 {
                    total[r][i] += t[r][i];
                }
            }
        }
        total
    });
    println!("── Oracle-B diagnostic (separability; true key gets its true B) ──");
    println!("  register | n     | sep_FL prior | sep_FL oracle |   Δ    | mean B_true/B_prior");
    for (r, name) in ["bass  ", "mid   ", "treble"].iter().enumerate() {
        let n = tally[r][0].max(1.0);
        let flp = tally[r][1] / n;
        let flo = tally[r][2] / n;
        let ratio = tally[r][3] / n;
        println!(
            "  {name}   | {:5.0} |    {:.4}    |    {:.4}     | {:+.4} |   {:.2}×",
            tally[r][0],
            flp,
            flo,
            flo - flp,
            ratio
        );
    }
    println!("  (synthetic true-B is distributed AROUND the prior → this Δ is a LOWER BOUND");
    println!("   on the real wrong-B cost, which Review 1 found one-directional ~7× in the bass.)");
}

fn pct(num: usize, den: usize) -> f32 {
    if den == 0 {
        0.0
    } else {
        100.0 * num as f32 / den as f32
    }
}

fn build_profiles() -> Box<[KeyProfile; 88]> {
    let mut v = Vec::with_capacity(88);
    for i in 0..88 {
        v.push(KeyProfile::new(et_freq(i), get_expected_beta(i as u8)));
    }
    Box::new(v.try_into().unwrap())
}

/// One JSON metrics line per trial to stdout — the orchestrator's contract.
/// objA = PRODUCTION K=3 false-lock rate (minimize); objB = mean ORDINAL confidence
/// (maximize). `sep_fl` (K=88 separability) and `prod_fl_k*` are diagnostics.
fn emit_json(acc: &Acc) {
    let n = acc.n.max(1) as f64;
    let obj_a = acc.prod_false_locks as f64 / n; // production K=3 = OPTIMIZED objA
    let sep_fl = acc.false_locks as f64 / n; // K=88 separability (diagnostic)
    let prod_fl = acc.prod_false_locks as f64 / n; // == objA (kept for back-compat)
    let plk = |i: usize| acc.prod_fl_k[i] as f64 / n; // K∈{2,3,4,5} diagnostic
    let obj_b = acc.margin_sum / n; // ordinal: mean fraction of impostors beaten
    let fl = |i: usize| acc.reg_fl[i] as f64 / acc.reg_n[i].max(1) as f64;
    let dfl = |i: usize| acc.det_fl[i] as f64 / acc.det_n[i].max(1) as f64;
    let fid = if acc.fid_n > 0 {
        acc.fid_sum / acc.fid_n as f64
    } else {
        0.0
    };
    let floor_frac = acc.floor_hits as f64 / n;
    let tie_frac = acc.tie_hits as f64 / n;
    println!(
        "{{\"objA\":{obj_a:.6},\"objB\":{obj_b:.6},\"prod_fl\":{prod_fl:.6},\"sep_fl\":{sep_fl:.6},\"prod_fl_k2\":{:.6},\"prod_fl_k3\":{:.6},\"prod_fl_k4\":{:.6},\"prod_fl_k5\":{:.6},\"n\":{},\"fl_bass\":{:.6},\"fl_mid\":{:.6},\"fl_treble\":{:.6},\"fl_hard\":{:.6},\"fl_det0\":{:.6},\"fl_det1\":{:.6},\"fl_det2\":{:.6},\"fl_det3\":{:.6},\"fidelity_mean\":{fid:.4},\"floor_frac\":{floor_frac:.4},\"tie_frac\":{tie_frac:.4}}}",
        plk(0),
        plk(1),
        plk(2),
        plk(3),
        acc.n,
        fl(0),
        fl(1),
        fl(2),
        fl(3),
        dfl(0),
        dfl(1),
        dfl(2),
        dfl(3)
    );
    use std::io::Write;
    std::io::stdout().flush().ok();
}

/// Persistent server: dataset built once, then one trial per stdin line
/// `mode p q r rho lambda [nonpeak smoothness]` (mode ∈ {discrete, refine};
/// lambda may be `inf`). The two trailing structural coefficients are optional
/// (default 0 = off) so the co-tuned structural arms can vary them as free params
/// while older 6-token callers still work. stdout carries ONLY JSON lines.
fn serve(frames: &[Frame], profiles: &[KeyProfile; 88]) {
    let scored: Vec<&Frame> = frames.iter().filter(|f| !f.ambiguous).collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    eprintln!(
        "ready: {} scored frames, fingerprint {:016x}, K={}, {threads} threads",
        scored.len(),
        dataset_fingerprint(frames),
        TOP_K
    );
    let mut line = String::new();
    let stdin = std::io::stdin();
    loop {
        line.clear();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF — orchestrator closed the pipe
        }
        let t = line.trim();
        if t.is_empty() {
            continue;
        }
        let p: Vec<&str> = t.split_whitespace().collect();
        let parsed = (|| -> Option<(bool, TwmConfig)> {
            if p.len() != 6 && p.len() != 8 {
                return None;
            }
            let refine = match p[0] {
                "refine" => true,
                "discrete" => false,
                _ => return None,
            };
            // Optional trailing structural coeffs (8-token form); default off.
            let (nonpeak, smoothness) = if p.len() == 8 {
                (p[6].parse().ok()?, p[7].parse().ok()?)
            } else {
                (0.0, 0.0)
            };
            Some((
                refine,
                TwmConfig {
                    p: p[1].parse().ok()?,
                    q: p[2].parse().ok()?,
                    r: p[3].parse().ok()?,
                    rho: p[4].parse().ok()?,
                    lambda_penalty: p[5].parse().ok()?, // "inf" parses to f32::INFINITY
                    nonpeak_penalty: nonpeak,
                    smoothness_penalty: smoothness,
                    ..TwmConfig::default()
                },
            ))
        })();
        match parsed {
            Some((refine, cfg)) => emit_json(&run_trial(&scored, profiles, &cfg, refine)),
            None => eprintln!("skip: malformed trial line: {t:?}"),
        }
    }
}

fn print_summary(label: &str, acc: &Acc, refine: bool) {
    println!("── {label} ──");
    let fl = |i| pct(acc.reg_fl[i], acc.reg_n[i]);
    let dfl = |i| pct(acc.det_fl[i], acc.det_n[i]);
    print!(
        "  sep-loss(objA) {:.2}%  prod-FL@K=3 {:.2}%  | bass {:.2}%  mid {:.2}%  treble {:.2}%  hard {:.2}%   objB {:.4}",
        pct(acc.false_locks, acc.n),
        pct(acc.prod_false_locks, acc.n),
        fl(0),
        fl(1),
        fl(2),
        fl(3),
        acc.margin_sum / acc.n.max(1) as f64,
    );
    if refine && acc.fid_n > 0 {
        print!("   fidelity {:.2}c", acc.fid_sum / acc.fid_n as f64);
    }
    println!();
    // Side-search signal: false-lock by detuning stratum (|cents from ET|).
    println!(
        "    by detune  <15¢ {:.1}% (n={})  15-35 {:.1}% ({})  35-55 {:.1}% ({})  55-78 {:.1}% ({})   [floor {:.1}% tie {:.1}%]",
        dfl(0),
        acc.det_n[0],
        dfl(1),
        acc.det_n[1],
        dfl(2),
        acc.det_n[2],
        dfl(3),
        acc.det_n[3],
        pct(acc.floor_hits, acc.n),
        pct(acc.tie_hits, acc.n),
    );
}

/// Human-facing validation mode (default): dataset banner, oracle + rank-study
/// diagnostics, default-constant summary in both modes, and the pre-opt gate.
fn report(frames: &[Frame], profiles: &[KeyProfile; 88]) {
    let ambiguous = frames.iter().filter(|f| f.ambiguous).count();
    let avg_peaks: f32 =
        frames.iter().map(|f| f.peaks.len() as f32).sum::<f32>() / frames.len() as f32;
    println!("── Dataset ──");
    println!(
        "frames {} (scored {}, ambiguous {} = {:.1}%), avg peaks/frame {:.1}, fingerprint {:016x}",
        frames.len(),
        frames.len() - ambiguous,
        ambiguous,
        pct(ambiguous, frames.len()),
        avg_peaks,
        dataset_fingerprint(frames),
    );

    let cfg = TwmConfig::default();
    dense_scan_oracle(frames, profiles, &cfg);
    b_oracle_diagnostic(frames, profiles, &cfg);
    stage_a_rank_study(frames, profiles, &cfg);

    let scored: Vec<&Frame> = frames.iter().filter(|f| !f.ambiguous).collect();
    for refine in [false, true] {
        let t = Instant::now();
        let acc = run_trial(&scored, profiles, &cfg, refine);
        let elapsed = t.elapsed();
        let mode = if refine { "REFINED" } else { "DISCRETE" };
        print_summary(
            &format!("{mode} (default constants, K={TOP_K})"),
            &acc,
            refine,
        );
        println!("  (trial wall time {elapsed:.2?})");

        if !refine {
            let rate = pct(acc.reg_fl[3], acc.reg_n[3]);
            if acc.reg_fl[3] == 0 {
                eprintln!(
                    "PRE-OPT GATE: FAIL — no false locks on the hard subset; the dataset does not reproduce the failure mode."
                );
                std::process::exit(1);
            }
            println!("PRE-OPT GATE: PASS (hard-subset false-lock rate {rate:.2}% > 0)");
        }
    }
}

fn main() {
    let serve_mode = std::env::args().any(|a| a == "--serve");

    let frames = generate_dataset(FIXED_SEED);
    // Determinism gate: byte-identical regeneration (catches any nondeterminism
    // before a multi-hour sweep trusts the dataset).
    assert_eq!(
        dataset_fingerprint(&frames),
        dataset_fingerprint(&generate_dataset(FIXED_SEED)),
        "dataset generation is non-deterministic!"
    );
    let profiles = build_profiles();

    if serve_mode {
        serve(&frames, &profiles);
    } else {
        report(&frames, &profiles);
    }
}
