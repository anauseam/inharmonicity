//! MAT's (f₀, B) recovery against known synthetic inharmonicity.
//!
//! Report 0006 states the numbers this asserts: MAT recovers known B to < 1 %
//! across the doubted regime (B to 25× the prior, missing fundamentals, parallel
//! strings) provided the f₀ seed is within ≈ ±10 % of true, and the only large
//! self-consistent error is an octave seed error, which yields exactly 4× B at a
//! low self-fit residual.
//!
//! MAT consumes a magnitude spectrum plus a CSPE per-bin frequency map, not a
//! peak list, so the fixture synthesizes time-domain sinusoids and runs the
//! exact Worker front end (two Hann FFTs → `cspe` → `detect_pitch_mat`), so the
//! recovery meets real FFT leakage.
//!
//! The full 1×–25× characterisation sweep this is drawn from is `cargo lab mat
//! recovery`; here the grid is only as wide as the assertions need.

use std::f32::consts::PI;
use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use tuner_core::algorithms::mat::{MAX_PARTIALS, MatEstimate, MatOrder, detect_pitch_mat};
use tuner_core::algorithms::spectral::{cspe, fft, magnitude_spectrum};
use tuner_core::models::{NOTES, get_expected_beta};

const SAMPLE_RATE: u32 = 44100;

/// The Worker's deep-bass FFT size: the largest power of two ≤ a full ~1.5 s
/// capture's sample count. The bass gets its best-case 0.673 Hz/bin here, so a
/// divergence is not a windowing artifact.
const FFT_SIZE: usize = 65536;

/// Partials above this are not synthesized (the `gen_frame` envelope cutoff).
const F_MAX: f32 = 9000.0;

/// Broadband white noise against partial RMS. 40 dB keeps MAT's §2.2
/// significance floor realistic without letting noise drive the result.
const SNR_DB: f32 = 40.0;

/// Realizations per cell. The medians report 0006 quotes are stable well below the
/// characterisation's 24; this is what keeps the test inside a plain
/// `cargo test`.
const SEEDS: usize = 5;

// ── Deterministic RNG: SplitMix64, the generator the synthetic harnesses share ──

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
}

/// One synthesis condition: the physical knobs report 0006's rows vary.
#[derive(Clone, Copy)]
struct Cond {
    f0: f32,
    b: f32,
    /// Unison strings, each an independent slightly-detuned series.
    n_strings: usize,
    /// String-to-string f₀ detune.
    spread_cents: f32,
    /// Envelope exponent `a_n ∝ n^(−alpha)`. Flatter in the bass, where the
    /// n² leverage on B lives.
    alpha: f32,
    /// Attenuate partials 1–3 — the soundboard-impedance missing fundamental.
    missing_fundamental: bool,
}

fn rms(signal: &[f32]) -> f32 {
    (signal.iter().map(|&x| x * x).sum::<f32>() / signal.len() as f32).sqrt()
}

/// Renders `cond` and returns the `(magnitudes, CSPE map)` `detect_pitch_mat`
/// consumes.
fn synth_spectrum(
    cond: &Cond,
    rng: &mut Rng,
    r2c: &Arc<dyn RealToComplex<f32>>,
) -> (Vec<f32>, Vec<f32>) {
    // One extra sample so the one-sample-shifted CSPE frame is fully populated.
    let mut signal = vec![0.0_f32; FFT_SIZE + 1];

    let mut f0_s = [cond.f0; 3];
    for f0_string in f0_s.iter_mut().take(cond.n_strings).skip(1) {
        *f0_string = cond.f0
            * 2f32.powf(rng.range(-cond.spread_cents / 2.0, cond.spread_cents / 2.0) / 1200.0);
    }

    let mut n = 1u32;
    while n <= MAX_PARTIALS as u32 * 2 {
        let n_f = n as f32;
        let stretch = (1.0 + cond.b * n_f * n_f).sqrt();
        if n_f * cond.f0 * stretch > F_MAX {
            break;
        }
        let mut a_n = n_f.powf(-cond.alpha) * (0.5 * rng.normal()).exp();
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

    let noise_rms = rms(&signal).max(1e-9) * 10f32.powf(-SNR_DB / 20.0);
    for sample in signal.iter_mut() {
        *sample += noise_rms * rng.normal();
    }

    // ── The Worker's spectral front end, unmodified ──
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

/// RMS relative residual of a fitted `(f0, B)` against its own located partials,
/// in ppm — self-consistency, not accuracy. Low here and wrong against truth
/// is the mis-association signature.
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

fn median(mut v: Vec<f32>) -> f32 {
    assert!(!v.is_empty(), "no MAT estimate was produced at all");
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

/// Median recovered/true B ratio and median self-residual over [`SEEDS`]
/// realizations, driven from `seed_f0` in the shipped `Serial` order.
fn recover(cond: Cond, salt: u64, seed_f0: f32) -> (f32, f32) {
    let mut planner = RealFftPlanner::<f32>::new();
    let r2c = planner.plan_fft_forward(FFT_SIZE);
    let mut ratios = Vec::new();
    let mut resids = Vec::new();
    for i in 0..SEEDS {
        let mut rng = Rng::new(
            salt ^ (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (cond.b.to_bits() as u64) << 11,
        );
        let (mags, cspe_map) = synth_spectrum(&cond, &mut rng, &r2c);
        let mut freqs = [0.0f32; MAX_PARTIALS];
        let mut ns = [0u32; MAX_PARTIALS];
        if let Some(est) = detect_pitch_mat(
            &mags,
            &cspe_map,
            SAMPLE_RATE,
            seed_f0,
            MatOrder::Serial,
            &mut freqs,
            &mut ns,
        ) {
            ratios.push(est.b / cond.b);
            resids.push(self_residual_ppm(
                &est,
                &freqs[..est.partial_count],
                &ns[..est.partial_count],
            ));
        }
    }
    (median(ratios), median(resids))
}

/// A0's condition at `ratio` × the Rigaud prior: single-strung, flat envelope,
/// the deep bass the 7–25× readings came from.
fn deep_bass(ratio: f32, missing_fundamental: bool) -> Cond {
    Cond {
        f0: NOTES[0].frequency,
        b: get_expected_beta(0) * ratio,
        n_strings: 1,
        spread_cents: 2.0,
        alpha: 0.6,
        missing_fundamental,
    }
}

/// report 0006: baseline at prior B, bass/mid/treble — 1.00×. If this fails the
/// fixture is wrong, not MAT.
#[test]
fn recovers_prior_b_across_the_compass() {
    for (key, n_strings, alpha) in [(0usize, 1, 0.6f32), (27, 3, 1.1), (60, 3, 1.1)] {
        let cond = Cond {
            f0: NOTES[key].frequency,
            b: get_expected_beta(key as u8),
            n_strings,
            spread_cents: 2.0,
            alpha,
            missing_fundamental: false,
        };
        let (ratio, _) = recover(cond, 0x0001 ^ key as u64, cond.f0);
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "{}: recovered/true B = {ratio:.4}, expected 1.00 ± 0.01",
            NOTES[key].name
        );
    }
}

/// report 0006: B swept to 25× the prior, with and without the fundamental —
/// 1.00× at every step. This is the row the deep-bass measurement rests on.
#[test]
fn recovers_high_bass_b_with_and_without_a_fundamental() {
    for ratio_to_prior in [1.0f32, 7.0, 25.0] {
        for missing in [false, true] {
            let cond = deep_bass(ratio_to_prior, missing);
            let (ratio, _) = recover(cond, 0x0200, cond.f0);
            assert!(
                (ratio - 1.0).abs() < 0.01,
                "A0 at {ratio_to_prior}× prior, missing fundamental {missing}: \
                 recovered/true B = {ratio:.4}, expected 1.00 ± 0.01"
            );
        }
    }
}

/// report 0006: the `MAT_SEED_TOLERANCE` cliff. Inside ±10 % the recovery is
/// unaffected. The tolerance has no margin either side and nothing downstream
/// catches a wrong B, so both edges are asserted.
#[test]
fn seed_error_within_ten_percent_does_not_move_b() {
    let cond = deep_bass(7.0, true);
    for seed_scale in [0.90f32, 1.0, 1.10] {
        let (ratio, _) = recover(cond, 0x0400, cond.f0 * seed_scale);
        assert!(
            (ratio - 1.0).abs() < 0.01,
            "seed {seed_scale}× true: recovered/true B = {ratio:.4}, expected 1.00 ± 0.01"
        );
    }
}

/// report 0006: an octave seed error is the one self-consistent failure. Every
/// other partial of a stiff string is itself a stiff series with f₀′ = 2f₀ and
/// B′ = 4B, so the fit is exact and the self-residual stays low — confidence
/// cannot catch it, which is why the seed tolerance is the guard.
#[test]
fn octave_seed_error_yields_four_times_b_at_a_low_residual() {
    let cond = deep_bass(1.0, false);
    let (on_seed_ratio, on_seed_resid) = recover(cond, 0x0800, cond.f0);
    let (ratio, resid) = recover(cond, 0x0800, cond.f0 * 2.0);
    assert!(
        (on_seed_ratio - 1.0).abs() < 0.01,
        "control: recovered/true B = {on_seed_ratio:.4} on an accurate seed"
    );
    assert!(
        (ratio - 4.0).abs() < 0.05,
        "octave seed: recovered/true B = {ratio:.4}, expected 4.00 ± 0.05"
    );
    assert!(
        resid < 10.0 * on_seed_resid.max(1.0),
        "octave seed self-residual {resid:.1} ppm is not low against the \
         on-seed {on_seed_resid:.1} ppm — the failure would be detectable, \
         which contradicts report 0006"
    );
}
