//! # Asymmetric, prior-regularized, per-candidate (f₀, B) refinement — Stage-B diagnostic
//!
//! ADR 0006 fix-path **step 3 / "Prompt 3"**: an *offline* test of whether refining β
//! **per top-K candidate** in Stage B — bounded, pulled toward that candidate's Rigaud
//! prior — reduces false-locks vs the shipped fixed-prior-β baseline **without**
//! strengthening the octave / sub-harmonic bass impostors. This is a *diagnostic, not a
//! build*: it decides whether the hot-path port (3b) is worth it, on evidence.
//!
//! ## What the baseline is, and what changes
//!
//! Shipped discovery = Stage A (88-key discrete scan, prior templates, scale 1.0) →
//! Stage B (`refine_scale`: per-candidate ±80¢ **scale** golden search, β **frozen** at
//! the Rigaud prior) → argmin. β is *shape*, f₀ is *scale*, and they are **orthogonal**
//! (ADR 0006 Mechanism): Stage B today can fix a wrong f₀ but never a wrong β. This
//! harness adds a **bounded joint (f₀, β)** Stage B: for each top-K candidate it also
//! searches β on a log-grid within ±n·σ_B of *that candidate's* prior, with a quadratic
//! regularizer pulling β toward the prior, then re-scores and takes the argmin. The
//! baseline is the **n_σ=0 / nb=1 special case** (asserted at startup).
//!
//! ## The decisive danger this must measure (why a "better" β can *break* discovery)
//!
//! `mat_b_recovery` proved the even partials of (f₀,B) are exactly the partials of
//! (2f₀,4B). So an **octave-above impostor**, given per-candidate B freedom, can find a
//! low-residual (2f₀,4B) fit and its **forward** error collapses. The only guards left:
//! (a) TWM's **reverse** error — the true note's *odd* partials have no match in the
//! octave template (weakest in the missing-fundamental bass); and (b) the
//! **regularizer**, whose pull is register-dependent. Whether those hold is empirical —
//! this harness reports the octave/twelfth win-rate, the forward-vs-reverse margin
//! decomposition, and the regularizer's effective pull, per register, baseline vs joint.
//!
//! ## Decision gate (printed explicitly at the end)
//!
//! Proceed to the hot-path port (3b) **only if** per-candidate B beats fixed-β **on real**
//! AND does **not** increase bass octave/sub-harmonic false-locks. If it wins on synthetic
//! but not real (the standing pattern), or strengthens the octave impostors, **stop** — the
//! bass-B lever is not realizable via scoring-time B (a valid, money-saving result).
//! Standing discipline: real is **validation-only**, n=1 cannot select.
//!
//! Usage:
//!   cargo run --release --example joint_b_refine_diagnostic               # synthetic + octave + real
//!   cargo run --release --example joint_b_refine_diagnostic -- --synthetic-only
//!   cargo run --release --example joint_b_refine_diagnostic -- --real-only

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use tuner_core::algorithms::discovery::{self, TOP_K};
use tuner_core::algorithms::peaks::{extract_peaks, mask_peaks};
use tuner_core::algorithms::spectral::{fft, magnitude_spectrum};
use tuner_core::algorithms::twm::{self, TwmConfig};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::gatekeeper::{Gatekeeper, SignalState};
use tuner_core::models::SpectralPeak;
use tuner_core::models::{KeyProfile, NOTES, get_expected_beta};
use tuner_core::pipeline::ProcessingFrame;

// ─────────────────────────── Registers ───────────────────────────
// bass 0–26, mid 27–59, treble 60–87 (the ADR 0006 split).
const REG_NAMES: [&str; 3] = ["bass  ", "mid   ", "treble"];
fn reg_of(key: usize) -> usize {
    if key <= 26 {
        0
    } else if key <= 59 {
        1
    } else {
        2
    }
}
/// Per-note relative B scatter σ_B — OUR calibration constants, the same values
/// `gen_frame` draws the synthetic per-note B with (mis-cited to "Rigaud Fig. 3"
/// pre-audit; see docs/audits/faithfulness-audit-06-rigaud.md). The ±n·σ_B
/// refinement bound is expressed in these units so the search window matches the
/// prior's own uncertainty model.
fn sigma_b(key: usize) -> f32 {
    if key <= 50 { 0.157 } else { 0.116 }
}

// ─────────────────────────── Joint-refine config ───────────────────────────

/// One refinement policy. `n_sigma == 0` (or `nb == 1`) ⇒ the shipped fixed-β
/// baseline (β pinned to the prior); the joint search degenerates to `refine_scale`.
#[derive(Clone, Copy)]
struct JointCfg {
    label: &'static str,
    /// Half-width of the β search, in units of σ_B (relative). Tight by design.
    n_sigma: f32,
    /// Regularizer strength γ (absolute score units): penalty `γ·d²`, d in σ_B units.
    /// γ→∞ pins β to the prior (→ baseline); γ=0 = unregularized within the bound
    /// (the dangerous case the octave impostor can exploit).
    gamma: f32,
    /// β grid points across the ±n_sigma window.
    nb: usize,
}

const BASELINE: JointCfg = JointCfg {
    label: "fixed-β (baseline)",
    n_sigma: 0.0,
    gamma: 0.0,
    nb: 1,
};

/// The policies compared. Baseline first; then the **tight, shippable** candidates
/// (±1–2 σ_B, the prompt's design point) at varied regularizer strength; then **wide
/// probes** (n_σ≈20 ⇒ β ∈ ~[0.04×, 23×] prior in log space) that deliberately reach the
/// real bass 7–25× regime — NOT shippable, but they answer the decisive question: when β
/// CAN reach the real value, does the true bass key benefit, or do the octave/sub-harmonic
/// impostors (which also get that freedom) win? The wide-γ=0 case is the maximal-danger
/// probe; wide-γ tests whether the regularizer still holds when β can roam.
fn joint_configs() -> Vec<JointCfg> {
    vec![
        BASELINE,
        JointCfg {
            label: "joint nσ=1 γ=2",
            n_sigma: 1.0,
            gamma: 2.0,
            nb: 9,
        },
        JointCfg {
            label: "joint nσ=2 γ=2",
            n_sigma: 2.0,
            gamma: 2.0,
            nb: 13,
        },
        JointCfg {
            label: "joint nσ=2 γ=0 (unreg)",
            n_sigma: 2.0,
            gamma: 0.0,
            nb: 13,
        },
        JointCfg {
            label: "joint nσ=2 γ=8 (strong)",
            n_sigma: 2.0,
            gamma: 8.0,
            nb: 13,
        },
        JointCfg {
            label: "WIDE nσ=20 γ=0 (≤23×)",
            n_sigma: 20.0,
            gamma: 0.0,
            nb: 25,
        },
        JointCfg {
            label: "WIDE nσ=20 γ=2 (reg)",
            n_sigma: 20.0,
            gamma: 2.0,
            nb: 25,
        },
    ]
}

/// Result of one candidate's bounded joint (f₀, β) refinement.
#[derive(Clone, Copy)]
#[allow(dead_code)] // `raw` retained for readers / future per-frame dumps
struct Joint {
    scale: f32,
    beta: f32,
    /// Best *raw* (unregularized) TWM error at the argmin.
    raw: f32,
    /// Best *regularized* error = raw + γ·d²  (the value candidates are ranked by).
    reg: f32,
    /// Chosen β offset in σ_B units (the regularizer's realized pull; ±n_sigma at the bound).
    d: f32,
    /// True when the argmin sat at the ±n_sigma edge (the bound is binding — the
    /// candidate "wants" more β shift than the prior-scatter window allows).
    pinned: bool,
}

/// Bounded, prior-regularized joint (f₀, β) refinement of ONE candidate, reusing the
/// existing Stage-B scale search at each trial β. Offline: a fresh `KeyProfile` is built
/// per β (no real-time constraint). The peak↔partial associations are recomputed inside
/// `score_candidate` exactly as Stage B already does — there is no *peak* re-extraction;
/// the masked peak list is fixed input.
///
/// At `nb == 1` this is byte-for-byte `refine_scale` on the prior template (the baseline).
fn refine_joint(
    peaks: &[SpectralPeak],
    f0_et: f32,
    beta_prior: f32,
    sig: f32,
    cfg: &TwmConfig,
    jc: &JointCfg,
) -> Joint {
    let mut best = Joint {
        scale: 1.0,
        beta: beta_prior,
        raw: f32::MAX,
        reg: f32::MAX,
        d: 0.0,
        pinned: false,
    };
    let nb = jc.nb.max(1);
    // β is searched in LOG space (the prior model is exponential; log-symmetry keeps the
    // bound positive even for wide probes that reach the real bass 7–25×). Half-width
    // `h = n_sigma·σ_B` in ln-units; the regularizer measures deviation `d = ln(β/prior)/σ_B`
    // in σ_B units, so `d²` is register-normalized by the prior's own scatter.
    let h = jc.n_sigma * sig;
    for i in 0..nb {
        let t = if nb == 1 {
            0.0
        } else {
            -1.0 + 2.0 * (i as f32) / (nb as f32 - 1.0) // t ∈ [-1, 1]
        };
        let ln_ratio = t * h;
        let beta = (beta_prior * ln_ratio.exp()).max(1e-7);
        let d = ln_ratio / sig; // deviation in σ_B units
        let prof = KeyProfile::new(f0_et, beta);
        let (scale, raw) = discovery::refine_scale(peaks, &prof, cfg);
        if raw == f32::MAX {
            continue;
        }
        let reg = raw + jc.gamma * d * d;
        if reg < best.reg {
            best = Joint {
                scale,
                beta,
                raw,
                reg,
                d,
                pinned: nb > 1 && (t.abs() >= 1.0 - 1e-4),
            };
        }
    }
    best
}

/// Outcome of a full joint discovery pass: production K=3 winner.
#[derive(Clone, Copy)]
#[allow(dead_code)] // `scale`/`beta` retained for readers / future per-frame dumps
struct JointDiscovery {
    key_index: u8,
    scale: f32,
    beta: f32,
    reg: f32,
}

/// Production K=3 joint discovery: identical Stage A (prior templates, scale 1.0,
/// top-`TOP_K`) so **recall is unchanged from the baseline** — the *only* difference
/// is Stage B refining β per candidate. Candidates are ranked by the **regularized**
/// error (a wrong-β impostor pays the regularizer; that is the whole defense).
fn discover_joint(
    peaks: &[SpectralPeak],
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    jc: &JointCfg,
) -> JointDiscovery {
    let top = discovery::stage_a(peaks, profiles, cfg);
    let mut best = JointDiscovery {
        key_index: top[0].0 as u8,
        scale: 1.0,
        beta: profiles[top[0].0].beta,
        reg: f32::MAX,
    };
    if top[0].1 == f32::MAX {
        return best;
    }
    for &(k, stage_err) in &top {
        if stage_err == f32::MAX {
            continue;
        }
        let j = refine_joint(
            peaks,
            profiles[k].f0_et,
            profiles[k].beta,
            sigma_b(k),
            cfg,
            jc,
        );
        if j.reg < best.reg {
            best = JointDiscovery {
                key_index: k as u8,
                scale: j.scale,
                beta: j.beta,
                reg: j.reg,
            };
        }
    }
    best
}

// ─────────────── Forward/reverse error decomposition (the octave discriminator) ───────────────

/// Splits the TWM total into its **forward** (`Err_{p-m}/N`) and **reverse**
/// (`Err_{m-p}/K`) components — mirrors `twm::score_candidate` Eq. (1)–(3) for the
/// shipped default (the experimental nonpeak/smoothness/deadzone terms are 0). Total =
/// `fwd + cfg.rho * rev`. The reverse term is the **octave discriminator**: an
/// octave-up template predicts nothing near the true note's *odd* partials, so those
/// measured peaks charge `Err_{m-p}`. A `debug_assert` guards against drift from the
/// canonical scorer.
fn score_split(
    peaks: &[SpectralPeak],
    profile: &KeyProfile,
    scale: f32,
    cfg: &TwmConfig,
) -> (f32, f32) {
    let valid_count = profile.valid_partial_count;
    if valid_count == 0 || peaks.is_empty() {
        return (f32::MAX, f32::MAX);
    }
    let mut a_max = 0.0_f32;
    let mut max_obs_freq = 0.0_f32;
    for peak in peaks {
        if peak.magnitude > a_max {
            a_max = peak.magnitude;
        }
        if peak.frequency > max_obs_freq {
            max_obs_freq = peak.frequency;
        }
    }
    a_max = a_max.max(1e-6);

    let cutoff_freq = max_obs_freq + profile.f0_et * scale;
    let mut active_predicted = 0_usize;
    for &p_freq in &profile.predicted_partials[..valid_count] {
        if p_freq * scale <= cutoff_freq {
            active_predicted += 1;
        } else {
            break;
        }
    }
    if active_predicted == 0 {
        active_predicted = 1;
    }
    let predicted = &profile.predicted_partials[..active_predicted];

    let fweight = |f: f32| -> f32 {
        if cfg.p == 0.5 {
            1.0 / f.max(1.0).sqrt()
        } else {
            f.max(1.0).powf(-cfg.p)
        }
    };

    // Forward Err_{p-m}.
    let mut err_pm = 0.0_f32;
    let mut j = 0;
    for &p_freq in predicted {
        let f_n = p_freq * scale;
        while j + 1 < peaks.len()
            && (peaks[j + 1].frequency - f_n).abs() <= (peaks[j].frequency - f_n).abs()
        {
            j += 1;
        }
        let delta = (peaks[j].frequency - f_n).abs();
        let w = fweight(f_n);
        let amp = peaks[j].magnitude / a_max;
        err_pm += delta * w + amp * (cfg.q * delta * w - cfg.r);
    }

    // Reverse Err_{m-p} (λ-capped).
    let mut err_mp = 0.0_f32;
    let mut i = 0;
    for peak in peaks {
        let f_k = peak.frequency;
        while i + 1 < predicted.len()
            && (predicted[i + 1] * scale - f_k).abs() <= (predicted[i] * scale - f_k).abs()
        {
            i += 1;
        }
        let delta = (predicted[i] * scale - f_k).abs();
        let w = fweight(f_k);
        let amp = peak.magnitude / a_max;
        let term = (delta * w + amp * (cfg.q * delta * w - cfg.r)).min(cfg.lambda_penalty);
        err_mp += term;
    }

    let n = active_predicted as f32;
    let k = peaks.len() as f32;
    let fwd = err_pm / n;
    let rev = err_mp / k;

    debug_assert!(
        cfg.nonpeak_penalty == 0.0 && cfg.smoothness_penalty == 0.0 && cfg.b_deadzone == 0.0,
        "score_split mirrors only the default (no experimental terms)"
    );
    debug_assert!({
        let total = fwd + cfg.rho * rev;
        let canon = twm::score_candidate(peaks, profile, scale, cfg);
        (total - canon).abs() <= 1e-2 * (1.0 + canon.abs())
    });
    (fwd, rev)
}

// ─────────────────────────── Deterministic RNG (mobo_evaluator parity) ───────────────────────────

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
    fn normal(&mut self) -> f32 {
        let u1 = self.f32().max(1e-7);
        let u2 = self.f32();
        (-2.0 * u1.ln()).sqrt() * (2.0 * core::f32::consts::PI * u2).cos()
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f32() < p
    }
}

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

// ─────────────────── Synthetic piano (verbatim from mobo_evaluator) ───────────────────

const FIXED_SEED: u64 = 0x1AB4_2026_0612_5EED;
const BASE_FRAMES_PER_KEY: usize = 100;
const HARD_FRAMES_PER_KEY: usize = 20;
const HOLDOUT_KEYS: [usize; 6] = [17, 7, 33, 0, 42, 1];
const HZ_PER_BIN: f32 = 44100.0 / 8192.0;
const AMBIGUOUS_CENTS: f32 = 78.0;

struct Piano {
    b_curve: [f32; 88],
    stretch_cents: [f32; 88],
}

fn synth_piano(rng: &mut Rng) -> Piano {
    let s_b = -0.066 * (1.0 + 0.10 * rng.normal());
    let y_b = -9.211 + 0.40 * rng.normal();
    const S_T: f32 = 0.0926;
    const Y_T: f32 = -11.788;

    let mut b_curve = [0.0f32; 88];
    for (k, b) in b_curve.iter_mut().enumerate() {
        let n = k as f32 + 1.0;
        *b = (s_b * n + y_b).exp() + (S_T * n + Y_T).exp();
    }

    let k_stretch = rng.range(3.5, 5.5);
    let m0 = 64.0 + 3.0 * rng.normal();
    let alpha = (24.0 + 3.0 * rng.normal()).max(10.0);
    let rho = |key: usize| -> f32 {
        let m = key as f32 + 21.0;
        (k_stretch / 2.0) * (1.0 - erf((m - m0) / alpha)) + 1.0
    };

    let dg = (8.0 * rng.normal()).clamp(-25.0, 25.0);
    let b = &b_curve;
    let mut f1 = [0.0f32; 88];
    let mut f0 = [0.0f32; 88];
    f0[48] = 440.0 * 2f32.powf(dg / 1200.0) / (1.0 + b[48]).sqrt();
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

#[allow(dead_code)] // `d_cents`/`hard` carried for parity with gen_frame; not all read here
struct Frame {
    key: u8,
    d_cents: f32,
    ambiguous: bool,
    hard: bool,
    b_actual: f32,
    peaks: Vec<SpectralPeak>,
}

fn emit_partial_cluster(rng: &mut Rng, freqs: &[f32], amps: &[f32], out: &mut Vec<SpectralPeak>) {
    let n = freqs.len();
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| freqs[a].total_cmp(&freqs[b]));
    let mut i = 0;
    while i < n {
        let mut j = i;
        while j + 1 < n && (freqs[order[j + 1]] - freqs[order[j]]).abs() < 1.5 * HZ_PER_BIN {
            j += 1;
        }
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
    let sig = if key <= 50 { 0.157 } else { 0.116 };
    let b_note = (piano.b_curve[key] * (1.0 + sig * rng.normal())).max(1e-7);
    let drift = {
        let s = rng.f32();
        if s < 0.30 {
            rng.range(0.0, 5.0)
        } else if s < 0.80 {
            rng.range(5.0, 25.0)
        } else {
            rng.range(25.0, 70.0)
        }
    };
    let mut err = drift * if rng.chance(0.5) { 1.0 } else { -1.0 };
    if hard {
        err = rng.range(20.0, 70.0) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    }
    let d_cents = piano.stretch_cents[key] + err;
    let ambiguous = d_cents.abs() > AMBIGUOUS_CENTS;
    let df_rel = 2f32.powf(d_cents / 1200.0) - 1.0;
    let b_actual = (b_note * (1.0 - 2.0 * df_rel)).max(1e-7);
    let f0 = et_freq(key) * 2f32.powf(d_cents / 1200.0);
    let n_strings = if key < 8 {
        1
    } else if key < 26 {
        2
    } else {
        3
    };
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
    let n_noise = 3 + (rng.f32() * 12.0) as usize;
    for _ in 0..n_noise {
        let f = 25.0 * (9000.0f32 / 25.0).powf(rng.f32());
        let db = rng.range(-45.0, -25.0) - 6.0 * (f / 1000.0).max(1.0).log2();
        raw.push(SpectralPeak {
            frequency: f,
            magnitude: a_max * 10f32.powf(db / 20.0),
        });
    }
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

fn build_profiles() -> Box<[KeyProfile; 88]> {
    let mut v = Vec::with_capacity(88);
    for i in 0..88 {
        v.push(KeyProfile::new(et_freq(i), get_expected_beta(i as u8)));
    }
    Box::new(v.try_into().unwrap())
}

// ─────────────────────────── Synthetic evaluation ───────────────────────────

#[derive(Default, Clone, Copy)]
struct SynthAcc {
    reg_n: [usize; 3],
    /// production K=3 false-lock (discover_joint winner ≠ true key).
    prod_fl: [usize; 3],
    /// separability (all-88 argmin by regularized error) false-lock.
    sep_fl: [usize; 3],
    // ── bass octave / sub-harmonic impostor wins (production K=3) ──
    bass_n: usize,
    bass_oct_up_win: usize,  // winner == key+12
    bass_twelfth_win: usize, // winner == key+19
    bass_oct_dn_win: usize,  // winner == key-12 (sub-harmonic)
    // ── octave forward/reverse margin (octave-up minus true), bass frames ──
    // positive margin ⇒ octave is penalized (good). Measured at each policy's own
    // refined (scale,β) for the true key and the key+12 template.
    oct_marg_n: usize,
    oct_fwd_marg_sum: f64,
    oct_rev_marg_sum: f64, // already ρ-weighted
    // ── regularizer pull on the TRUE key, by register ──
    pull_n: [usize; 3],
    pull_absd_sum: [f64; 3],   // mean |d| chosen (σ_B units)
    pull_pinned: [usize; 3],   // frac that hit the ±n_sigma bound
    pull_bratio_sum: [f64; 3], // chosen β / prior β
    true_bratio_sum: [f64; 3], // actual β / prior β (the target)
}

impl SynthAcc {
    fn merge(&mut self, o: &SynthAcc) {
        for i in 0..3 {
            self.reg_n[i] += o.reg_n[i];
            self.prod_fl[i] += o.prod_fl[i];
            self.sep_fl[i] += o.sep_fl[i];
            self.pull_n[i] += o.pull_n[i];
            self.pull_absd_sum[i] += o.pull_absd_sum[i];
            self.pull_pinned[i] += o.pull_pinned[i];
            self.pull_bratio_sum[i] += o.pull_bratio_sum[i];
            self.true_bratio_sum[i] += o.true_bratio_sum[i];
        }
        self.bass_n += o.bass_n;
        self.bass_oct_up_win += o.bass_oct_up_win;
        self.bass_twelfth_win += o.bass_twelfth_win;
        self.bass_oct_dn_win += o.bass_oct_dn_win;
        self.oct_marg_n += o.oct_marg_n;
        self.oct_fwd_marg_sum += o.oct_fwd_marg_sum;
        self.oct_rev_marg_sum += o.oct_rev_marg_sum;
    }
}

fn eval_synthetic(
    scored: &[&Frame],
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    jc: &JointCfg,
) -> SynthAcc {
    let nthreads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let chunk = scored.len().div_ceil(nthreads).max(1);
    std::thread::scope(|s| {
        let handles: Vec<_> = scored
            .chunks(chunk)
            .map(|c| {
                s.spawn(move || {
                    let mut acc = SynthAcc::default();
                    for &f in c {
                        process_synth_frame(f, profiles, cfg, jc, &mut acc);
                    }
                    acc
                })
            })
            .collect();
        let mut total = SynthAcc::default();
        for h in handles {
            total.merge(&h.join().unwrap());
        }
        total
    })
}

fn process_synth_frame(
    f: &Frame,
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    jc: &JointCfg,
    acc: &mut SynthAcc,
) {
    let key = f.key as usize;
    let reg = reg_of(key);
    acc.reg_n[reg] += 1;

    // Production K=3 joint discovery.
    let win = discover_joint(&f.peaks, profiles, cfg, jc);
    let prod_fl = win.key_index as usize != key;
    acc.prod_fl[reg] += prod_fl as usize;

    // Separability: refine EVERY key jointly, argmin by regularized error.
    let mut best_reg = f32::MAX;
    let mut argmin = 0usize;
    for k in 0..88 {
        let j = refine_joint(
            &f.peaks,
            profiles[k].f0_et,
            profiles[k].beta,
            sigma_b(k),
            cfg,
            jc,
        );
        if j.reg < best_reg {
            best_reg = j.reg;
            argmin = k;
        }
    }
    acc.sep_fl[reg] += (argmin != key) as usize;

    // Regularizer pull on the TRUE key.
    let jt = refine_joint(
        &f.peaks,
        profiles[key].f0_et,
        profiles[key].beta,
        sigma_b(key),
        cfg,
        jc,
    );
    acc.pull_n[reg] += 1;
    acc.pull_absd_sum[reg] += jt.d.abs() as f64;
    acc.pull_pinned[reg] += jt.pinned as usize;
    acc.pull_bratio_sum[reg] += (jt.beta / profiles[key].beta) as f64;
    acc.true_bratio_sum[reg] += (f.b_actual / profiles[key].beta) as f64;

    // Bass octave / sub-harmonic impostor analysis.
    if key <= 26 {
        acc.bass_n += 1;
        let w = win.key_index as usize;
        if w == key + 12 {
            acc.bass_oct_up_win += 1;
        }
        if w == key + 19 {
            acc.bass_twelfth_win += 1;
        }
        if key >= 12 && w == key - 12 {
            acc.bass_oct_dn_win += 1;
        }
        // Forward/reverse margin: octave-up minus true at each policy's refined template.
        if key + 12 < 88 {
            let jo = refine_joint(
                &f.peaks,
                profiles[key + 12].f0_et,
                profiles[key + 12].beta,
                sigma_b(key + 12),
                cfg,
                jc,
            );
            let true_prof = KeyProfile::new(profiles[key].f0_et, jt.beta);
            let oct_prof = KeyProfile::new(profiles[key + 12].f0_et, jo.beta);
            let (tf, tr) = score_split(&f.peaks, &true_prof, jt.scale, cfg);
            let (of, or) = score_split(&f.peaks, &oct_prof, jo.scale, cfg);
            if tf < f32::MAX && of < f32::MAX {
                acc.oct_marg_n += 1;
                acc.oct_fwd_marg_sum += (of - tf) as f64;
                acc.oct_rev_marg_sum += (cfg.rho * (or - tr)) as f64;
            }
        }
    }
}

fn pct(num: usize, den: usize) -> f32 {
    if den == 0 {
        0.0
    } else {
        100.0 * num as f32 / den as f32
    }
}

fn report_synthetic(frames: &[Frame], profiles: &[KeyProfile; 88], cfg: &TwmConfig) {
    let scored: Vec<&Frame> = frames.iter().filter(|f| !f.ambiguous).collect();
    let ambiguous = frames.len() - scored.len();
    println!("\n==================== SYNTHETIC (gen_frame, ground-truthed) ====================");
    println!(
        "frames {} (scored {}, ambiguous {} = {:.1}%)   registers: bass 0–26 / mid 27–59 / treble 60–87",
        frames.len(),
        scored.len(),
        ambiguous,
        pct(ambiguous, frames.len())
    );

    // Sanity: the n_σ=0 baseline (`discover_joint` with BASELINE) must reproduce the
    // library `discover()` exactly — the joint search is a strict generalization, so the
    // baseline column is trustworthy as the reference.
    let mut checked = 0usize;
    for f in scored.iter().filter(|f| !f.peaks.is_empty()).take(2000) {
        let lib = discovery::discover(&f.peaks, profiles, cfg, true).key_index;
        let base = discover_joint(&f.peaks, profiles, cfg, &BASELINE).key_index;
        assert_eq!(
            lib, base,
            "baseline parity broke: discover() vs discover_joint(BASELINE)"
        );
        checked += 1;
    }
    println!(
        "(baseline-parity check: discover_joint(BASELINE) == discover() on {checked} frames ✓)"
    );

    let configs = joint_configs();
    let accs: Vec<(JointCfg, SynthAcc)> = configs
        .iter()
        .map(|jc| (*jc, eval_synthetic(&scored, profiles, cfg, jc)))
        .collect();

    println!("\n── Production K=3 false-lock by register (the headline) ──");
    println!(
        "  {:<26} {:>8} {:>8} {:>8} {:>8}",
        "policy", "bass", "mid", "treble", "TOTAL"
    );
    for (jc, a) in &accs {
        let tot_fl = a.prod_fl[0] + a.prod_fl[1] + a.prod_fl[2];
        let tot_n = a.reg_n[0] + a.reg_n[1] + a.reg_n[2];
        println!(
            "  {:<26} {:>7.2}% {:>7.2}% {:>7.2}% {:>7.2}%",
            jc.label,
            pct(a.prod_fl[0], a.reg_n[0]),
            pct(a.prod_fl[1], a.reg_n[1]),
            pct(a.prod_fl[2], a.reg_n[2]),
            pct(tot_fl, tot_n),
        );
    }

    println!("\n── Separability (all-88 argmin) false-lock by register ──");
    println!(
        "  {:<26} {:>8} {:>8} {:>8}",
        "policy", "bass", "mid", "treble"
    );
    for (jc, a) in &accs {
        println!(
            "  {:<26} {:>7.2}% {:>7.2}% {:>7.2}%",
            jc.label,
            pct(a.sep_fl[0], a.reg_n[0]),
            pct(a.sep_fl[1], a.reg_n[1]),
            pct(a.sep_fl[2], a.reg_n[2]),
        );
    }

    println!(
        "\n── Bass octave / sub-harmonic impostor WINS (production K=3; the decisive test) ──"
    );
    println!(
        "  {:<26} {:>10} {:>10} {:>10}  (of {} bass frames)",
        "policy", "oct-up+12", "12th+19", "sub-12", accs[0].1.bass_n
    );
    for (jc, a) in &accs {
        println!(
            "  {:<26} {:>9} {:>9} {:>9}",
            jc.label, a.bass_oct_up_win, a.bass_twelfth_win, a.bass_oct_dn_win
        );
    }

    println!("\n── Octave discriminator: mean margin (octave-up − true), bass frames ──");
    println!("  positive ⇒ octave penalized. fwd collapse + rev rescue is the predicted pattern.");
    println!(
        "  {:<26} {:>12} {:>12} {:>12}",
        "policy", "fwd-margin", "ρ·rev-margin", "total"
    );
    for (jc, a) in &accs {
        let n = a.oct_marg_n.max(1) as f64;
        let fwd = a.oct_fwd_marg_sum / n;
        let rev = a.oct_rev_marg_sum / n;
        println!(
            "  {:<26} {:>12.4} {:>12.4} {:>12.4}",
            jc.label,
            fwd,
            rev,
            fwd + rev
        );
    }

    println!("\n── Regularizer effective pull on the TRUE key, by register ──");
    println!(
        "  mean|d| in σ_B units (bound = n_σ), %pinned at bound, mean β_chosen/prior vs β_true/prior."
    );
    for (jc, a) in &accs {
        if jc.n_sigma == 0.0 {
            continue;
        }
        println!("  {}", jc.label);
        for r in 0..3 {
            let n = a.pull_n[r].max(1) as f64;
            println!(
                "    {:<7} mean|d|={:>4.2}σ  pinned={:>5.1}%  β_chosen/prior={:>5.2}×  (β_true/prior={:>5.2}×)",
                REG_NAMES[r],
                a.pull_absd_sum[r] / n,
                pct(a.pull_pinned[r], a.pull_n[r]),
                a.pull_bratio_sum[r] / n,
                a.true_bratio_sum[r] / n,
            );
        }
    }
}

// ─────────────────────────── Real-capture evaluation ───────────────────────────

/// Per-policy per-key lock outcome on the real captures, mirroring `test_engine_all.py`:
/// Stable frames only, first key that is the discovery winner for 3 consecutive Stable
/// frames is the lock. We run every policy on the SAME Stable-frame peak lists (the
/// gatekeeper path is policy-independent), so the comparison is apples-to-apples.
struct RealKeyResult {
    key: usize,
    /// lock key per policy (−1 = never locked).
    locked: Vec<i32>,
}

fn three_frame_lock(winners: &[i32]) -> i32 {
    let mut current = -1i32;
    let mut count = 0;
    for &w in winners {
        if w == current {
            count += 1;
        } else {
            current = w;
            count = 1;
        }
        if count >= 3 {
            return current;
        }
    }
    -1
}

fn read_raw_f32(path: &std::path::Path) -> Option<Vec<f32>> {
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

fn process_real_key(
    dir: &std::path::Path,
    key: usize,
    profiles: &[KeyProfile; 88],
    cfg: &TwmConfig,
    configs: &[JointCfg],
    r2c_bass: &Arc<dyn RealToComplex<f32>>,
    r2c_gate: &Arc<dyn RealToComplex<f32>>,
    bass_oct: &mut Vec<usize>,
) -> Option<RealKeyResult> {
    // noise_floor from analysis.json (same as diagnose_engine).
    let mut noise_floor = 0.0f32;
    if let Ok(s) = std::fs::read_to_string(dir.join("analysis.json"))
        && let Ok(j) = serde_json::from_str::<serde_json::Value>(&s)
        && let Some(nf) = j["metadata"]["noise_floor"].as_f64()
    {
        noise_floor = nf as f32;
    }
    if noise_floor <= 0.0 {
        noise_floor = 0.001;
    }

    let raw_path = {
        let a = dir.join("audio_full_event.raw");
        if a.exists() { a } else { dir.join("audio.raw") }
    };
    let audio = read_raw_f32(&raw_path)?;
    if audio.len() < BASS_WINDOW_SIZE {
        return None;
    }

    let sum_w2 = 0.375 * BASS_WINDOW_SIZE as f32;
    let p_bin = noise_floor * noise_floor * sum_w2;
    let min_magnitude = if p_bin > 0.0 {
        (-p_bin * 0.001_f32.ln()).sqrt()
    } else {
        0.0
    };

    let audio_pool = Arc::new(crossbeam_queue::ArrayQueue::new(1));
    let mut gatekeeper = Gatekeeper::new(audio_pool);
    gatekeeper.config.silence_threshold = noise_floor;
    gatekeeper.capture_mode_enabled = true; // traverse to Stable, as the 74/87 pipeline does

    let mut pf = ProcessingFrame::new();
    let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
    let mut freq = vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE / 2 + 1];
    let mut mags = vec![0.0f32; BASS_WINDOW_SIZE / 2];
    let mut peak_scratch = vec![SpectralPeak::default(); 128];

    // Per-policy winner sequences over Stable frames.
    let mut seqs: Vec<Vec<i32>> = vec![Vec::new(); configs.len()];

    let mut cursor = 0;
    while cursor + BASS_WINDOW_SIZE <= audio.len() {
        let frame_audio = &audio[cursor..cursor + BASS_WINDOW_SIZE];

        // Bass FFT → magnitudes → peaks (diagnose_engine path).
        fft(
            frame_audio,
            &mut time,
            &mut freq,
            r2c_bass,
            BASS_WINDOW_SIZE,
        );
        magnitude_spectrum(&freq, BASS_WINDOW_SIZE, &mut mags);

        // Gatekeeper FFT (newest WINDOW_SIZE samples).
        pf.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(frame_audio);
        let newest = BASS_WINDOW_SIZE - WINDOW_SIZE;
        fft(
            &pf.audio_buffer[newest..BASS_WINDOW_SIZE],
            &mut pf.time_buffer[..WINDOW_SIZE],
            &mut pf.frequency_buffer[..],
            r2c_gate,
            WINDOW_SIZE,
        );
        let gate = gatekeeper.process_frame(&pf);

        if gate.state == SignalState::Stable && !gate.is_transient_bypass {
            let count = extract_peaks(
                &mags,
                &freq,
                44100,
                BASS_WINDOW_SIZE,
                min_magnitude,
                &mut peak_scratch,
            );
            let k = count.min(64);
            let active = &mut peak_scratch[..k];
            let valid = mask_peaks(active);
            let peaks = &active[..valid];
            for (ci, jc) in configs.iter().enumerate() {
                if peaks.is_empty() {
                    seqs[ci].push(-1);
                    continue;
                }
                let w = discover_joint(peaks, profiles, cfg, jc).key_index as i32;
                seqs[ci].push(w);
                // Octave-impostor count on bass keys (winner is the octave/twelfth of true).
                if key <= 26 && (w as usize == key + 12 || w as usize == key + 19) {
                    bass_oct[ci] += 1;
                }
            }
        }
        cursor += HOP_SIZE;
    }

    let locked: Vec<i32> = seqs.iter().map(|s| three_frame_lock(s)).collect();
    Some(RealKeyResult { key, locked })
}

fn report_real(profiles: &[KeyProfile; 88], cfg: &TwmConfig) {
    let base = std::path::Path::new("diagnostics");
    if !base.exists() {
        println!("\n(real-capture skipped: ./diagnostics not found)");
        return;
    }
    let configs = joint_configs();

    let mut planner = RealFftPlanner::<f32>::new();
    let r2c_bass = planner.plan_fft_forward(BASS_WINDOW_SIZE);
    let r2c_gate = planner.plan_fft_forward(WINDOW_SIZE);

    let mut dirs: Vec<(usize, std::path::PathBuf)> = std::fs::read_dir(base)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if !name.starts_with("key_") {
                return None;
            }
            let key: usize = name.split('_').nth(1)?.parse().ok()?;
            Some((key, e.path()))
        })
        .collect();
    dirs.sort_by_key(|(k, _)| *k);

    println!(
        "\n==================== REAL CAPTURES (diagnose_engine path, 3-frame lock) ===================="
    );
    println!(
        "validation-only — n=1 cannot select. Reference: fixed-β baseline (the 74/87 program)."
    );

    let mut bass_oct = vec![0usize; configs.len()];
    let mut results: Vec<RealKeyResult> = Vec::new();
    for (key, dir) in &dirs {
        if let Some(r) = process_real_key(
            dir,
            *key,
            profiles,
            cfg,
            &configs,
            &r2c_bass,
            &r2c_gate,
            &mut bass_oct,
        ) {
            results.push(r);
        }
    }

    // Per-policy totals and per-register pass counts.
    println!("\n── Lock pass counts (correct 3-frame lock) ──");
    println!(
        "  {:<26} {:>10} {:>8} {:>8} {:>8}",
        "policy", "TOTAL/n", "bass", "mid", "treble"
    );
    let n_total = results.len();
    for (ci, jc) in configs.iter().enumerate() {
        let mut pass = 0;
        let mut reg_pass = [0usize; 3];
        let mut reg_n = [0usize; 3];
        for r in &results {
            let reg = reg_of(r.key);
            reg_n[reg] += 1;
            if r.locked[ci] == r.key as i32 {
                pass += 1;
                reg_pass[reg] += 1;
            }
        }
        println!(
            "  {:<26} {:>7}/{:<3} {:>3}/{:<3} {:>3}/{:<3} {:>3}/{:<3}",
            jc.label,
            pass,
            n_total,
            reg_pass[0],
            reg_n[0],
            reg_pass[1],
            reg_n[1],
            reg_pass[2],
            reg_n[2],
        );
    }

    println!("\n── Bass octave/twelfth impostor lock-frames (lower = safer) ──");
    for (ci, jc) in configs.iter().enumerate() {
        println!("  {:<26} {}", jc.label, bass_oct[ci]);
    }

    // Per-key diff vs baseline (which keys the policy fixes / breaks).
    println!("\n── Per-key changes vs fixed-β baseline (policy index 0) ──");
    for (ci, jc) in configs.iter().enumerate() {
        if ci == 0 {
            continue;
        }
        let mut fixed = Vec::new();
        let mut broke = Vec::new();
        for r in &results {
            let base_ok = r.locked[0] == r.key as i32;
            let pol_ok = r.locked[ci] == r.key as i32;
            if !base_ok && pol_ok {
                fixed.push(r.key);
            }
            if base_ok && !pol_ok {
                broke.push(r.key);
            }
        }
        let name = |ks: &[usize]| -> String {
            ks.iter()
                .map(|&k| NOTES[k].name.clone())
                .collect::<Vec<_>>()
                .join(",")
        };
        println!(
            "  {:<26} fixed {:>2} [{}]  broke {:>2} [{}]",
            jc.label,
            fixed.len(),
            name(&fixed),
            broke.len(),
            name(&broke)
        );
    }
}

// ─────────────────────────── Decision gate ───────────────────────────

fn print_decision_gate() {
    println!("\n==================== DECISION GATE (3a → 3b) ====================");
    println!(
        "PROCEED to the hot-path port (3b) ONLY IF, reading the tables above:
  (1) per-candidate B BEATS fixed-β ON REAL (total lock count up, no register regressed), AND
  (2) it does NOT raise bass octave/twelfth impostor locks on real (the 'sub-12 / oct-up' rows).

STOP (document, save the build) IF EITHER:
  • it wins on synthetic but not on real — the standing pattern (oracle-B promise that
    didn't transfer): the bass-B lever is not realizable via scoring-time B; OR
  • it raises the octave/sub-harmonic impostor wins — the (2f₀,4B) forward-collapse beat
    the reverse-error + regularizer guard (watch the 'fwd-margin' going negative without
    'ρ·rev-margin' compensating).

Cross-checks to weight the read:
  • Reg-pull table: if β_true/prior ≫ the n_σ bound in the bass (real wants 7–25×, the
    window only reaches 1±2σ≈±31%), the bound CANNOT deliver the true-key benefit — the
    lever is structurally out of reach at safe bounds, independent of γ. That is the
    β/f₀-orthogonality wall (ADR 0006 Mechanism), now quantified.
  • γ=0 (unreg) vs γ>0: if unregularized raises octave wins but regularized does not, the
    regularizer is doing its job; if even γ=8 can't hold the octaves, scoring-time B is unsafe.
  • Real is n=1 and validation-only: a real WIN here is necessary-not-sufficient (needs the
    second instrument); a real LOSS or octave-regression is decisive to STOP now."
    );
}

fn main() {
    let synthetic_only = std::env::args().any(|a| a == "--synthetic-only");
    let real_only = std::env::args().any(|a| a == "--real-only");

    let cfg = TwmConfig::default();
    let profiles = build_profiles();

    println!("Joint (f₀, B) Stage-B refinement diagnostic — ADR 0006 step 3 / 'Prompt 3'");
    println!(
        "TwmConfig default: p={} q={} r={} ρ={} λ={}   TOP_K={}",
        cfg.p, cfg.q, cfg.r, cfg.rho, cfg.lambda_penalty, TOP_K
    );

    if !real_only {
        let frames = generate_dataset(FIXED_SEED);
        report_synthetic(&frames, &profiles, &cfg);
    }
    if !synthetic_only {
        report_real(&profiles, &cfg);
    }
    print_decision_gate();
}
