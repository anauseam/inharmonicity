//! Stage B: the room's per-bin distribution. The Gatekeeper-screened pre-roll of
//! each capture set, and a room recording, read per bin of the 8192-point Hann
//! frame and pooled per octave band, each quantity against the law it takes for
//! stationary Gaussian noise seen through the same overlapping frames.
//!
//! A window is one frame on the pipeline's hop grid, and `P = |X|²` its power per
//! bin in `spectral::fft`'s unnormalised units. A block is one capture's screened
//! pre-roll in time order, or one stretch of a recording; the set is the pool of
//! its blocks.

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, anyhow, bail};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;

use super::ambient;
use super::arms::{analytic_nu_half, preroll_offset};
use super::stats;
use super::truth::Gaussian;
use crate::gatekeeper::replay::{self as gate_replay, Overrides, Source, Thresholds};
use crate::truth::{SAMPLE_RATE, hann_vec};
use crate::{capture, raw};
use tuner_core::algorithms::spectral;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::gatekeeper::SignalState;

/// Bins of the 8192-point frame below Nyquist. Bin 0 is a real value with a χ²₁
/// law, not an exponential one, and is left out of every statistic.
const BINS: usize = BASS_WINDOW_SIZE / 2;

/// Band edges, Hz: octaves from 43 Hz. The bins under the first edge, the DC
/// blocker's 35 Hz corner and below, are the rumble band, reported apart.
const BAND_EDGES_HZ: [f64; 10] = [
    43.0, 86.0, 172.0, 344.0, 688.0, 1376.0, 2752.0, 5504.0, 11008.0, 22050.0,
];

/// Hops between a pre-roll window's end and the onset, as the gate studies'
/// silence keeps them.
const ONSET_GUARD_HOPS: usize = 4;

/// Frames this many hops apart share no samples, so their spectra of white noise
/// are independent.
const DISJOINT_HOPS: usize = BASS_WINDOW_SIZE / HOP_SIZE;

/// Bins of one frame this far apart are uncorrelated in power: the Hann window's
/// squared spectrum spans two bins either side (1.5 × 10⁻¹⁰ at three).
const DISJOINT_BINS: usize = 3;

/// The within-block law is read at the upper `p`-points of `F(2, 2M)`.
const LAW_P: [f64; 4] = [0.5, 0.1, 0.01, 0.001];

/// The tail is read at the exponential law's points `−ln p`.
const TAIL_P: [f64; 3] = [0.1, 0.01, 0.001];

/// Fewest pooled values behind a row at `p = 10⁻³`: a hundred expected
/// exceedances.
const MIN_VALUES_AT_1E3: u64 = 100_000;

/// Largest block whose frame-correlation matrix is diagonalised exactly. A
/// longer contiguous block takes the circulant form, which differs from the
/// exact matrix only in the corners coupling its first and last seven frames, a
/// perturbation of rank fourteen.
const EXACT_MAX_FRAMES: usize = 128;

/// The finite-variance test's jump: an increment of `C(k, 1)` above this many
/// times the median absolute increment. The paper finds the peaks of the
/// increments with MATLAB's `findpeaks` and states no parameters; this rule is
/// ours, and the simulated null runs through it unchanged.
const JUMP_Q: f64 = 10.0;

/// Shortest segment `A(1)` is fitted on, and the fallback tail, as a fraction of
/// `n` (Skowronek, Zimroz & Wyłomańska 2024, §2.1, step 3).
const SEGMENT_FRACTION: f64 = 0.1;

/// Draws of the exponential null per acceptance region: fifty beyond each of the
/// 2.5 % and 97.5 % points.
const NULL_SAMPLES: usize = 2000;

/// Fewest values the finite-variance test runs on: the smallest sample the
/// paper's power curves report (n = 500, their Fig. 19).
const MIN_FV_VALUES: usize = 500;

/// Most values per band for the finite-variance test; past it the bin stride
/// widens, keeping the time order.
const MAX_FV_VALUES: usize = 100_000;

/// Seed of the finite-variance null.
const NULL_SEED: u64 = 0x5EED_B500;

/// The kurtosis bands' level: band 1 is `±Φ⁻¹(1 − α/2)` standard deviations.
const SK_ALPHA: f64 = 0.05;

/// Towsey's histogram: 100 bins from the minimum to the maximum, a centred
/// 5-point moving average, and a modal bin in the top 5 % held at the 95th.
const MODE_BINS: usize = 100;
const MODE_SMOOTH: usize = 5;

/// Fewest blocks of one length for a measured `M` at that length.
const MIN_GROUP_BLOCKS: usize = 10;

/// A recording's blocks, seconds, before the whole record.
const RECORDING_BLOCKS_S: [f64; 2] = [1.0, 10.0];

/// The calibration-length check: the app's 2 s calibration against longer cuts.
const CALIBRATION_S: [f64; 4] = [2.0, 5.0, 10.0, 30.0];

/// `10 / ln 10`: natural-log spread to dB of power.
const DB_PER_NEPER: f64 = 4.342_944_819_032_518;

// ── Pools of windows ──────────────────────────────────────────────────────────

/// One window: its hop on the record's grid and its row in the power table.
#[derive(Clone, Copy)]
struct Frame {
    hop: usize,
    row: usize,
}

/// Blocks of windows in time order, over a power table of `BINS` values a row.
struct Pool<'a> {
    power: &'a [f32],
    blocks: Vec<Vec<Frame>>,
}

impl Pool<'_> {
    fn frames(&self) -> usize {
        self.blocks.iter().map(Vec::len).sum()
    }

    fn row(&self, f: Frame) -> &[f32] {
        &self.power[f.row * BINS..(f.row + 1) * BINS]
    }
}

/// A block's hops relative to its first.
fn geometry(block: &[Frame]) -> Vec<usize> {
    block.iter().map(|f| f.hop - block[0].hop).collect()
}

/// One reporting band: bins `lo..hi`.
struct Band {
    label: String,
    lo: usize,
    hi: usize,
    rumble: bool,
}

/// The rumble band and the octaves from 43 Hz, at `bin_hz` Hz per bin. A bin
/// belongs to the band its centre frequency falls in.
fn bands(bin_hz: f64) -> Vec<Band> {
    let first = |f: f64| ((f / bin_hz).ceil() as usize).clamp(1, BINS);
    let mut out = vec![Band {
        label: format!("rumble <{:.0}", BAND_EDGES_HZ[0]),
        lo: 1,
        hi: first(BAND_EDGES_HZ[0]),
        rumble: true,
    }];
    for e in BAND_EDGES_HZ.windows(2) {
        let (lo, hi) = (first(e[0]), first(e[1]));
        if hi > lo {
            out.push(Band {
                label: format!("{:.0}–{:.0}", e[0], e[1]),
                lo,
                hi,
                rumble: false,
            });
        }
    }
    out
}

// ── The frames' correlation, and exact laws over it ───────────────────────────

/// A bin's complex correlation between two frames `m` hops apart,
/// `Σ w(n) w(n + mH) / Σ w²`, for `m` = 0…7. For white Gaussian noise it is the
/// whole covariance of a bin across frames, up to a phase per lag that leaves
/// every power statistic unchanged.
fn frame_rho() -> &'static [f64; DISJOINT_HOPS] {
    static TABLE: OnceLock<[f64; DISJOINT_HOPS]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let w = hann_vec(BASS_WINDOW_SIZE);
        let w2: f64 = w.iter().map(|&x| f64::from(x) * f64::from(x)).sum();
        let mut table = [0.0; DISJOINT_HOPS];
        for (m, slot) in table.iter_mut().enumerate() {
            let shift = m * HOP_SIZE;
            let s: f64 = w[..BASS_WINDOW_SIZE - shift]
                .iter()
                .zip(&w[shift..])
                .map(|(&a, &b)| f64::from(a) * f64::from(b))
                .sum();
            *slot = s / w2;
        }
        table
    })
}

fn rho(hops_apart: usize) -> f64 {
    frame_rho().get(hops_apart).copied().unwrap_or(0.0)
}

/// `K² / Σ_{i,j} ρ(h_i − h_j)^q` over frames at ascending hops `hops`.
fn frame_count(hops: &[usize], q: i32) -> f64 {
    if hops.is_empty() {
        return 0.0;
    }
    let mut sum = hops.len() as f64;
    for (i, &a) in hops.iter().enumerate() {
        for &b in hops[i + 1..].iter().take_while(|&&b| b - a < DISJOINT_HOPS) {
            sum += 2.0 * rho(b - a).powi(q);
        }
    }
    (hops.len() as f64).powi(2) / sum
}

/// Equivalent independent periodograms in the mean of frames at `hops`:
/// Percival & Walden's EDOF/2, `K² / Σ_{i,j} ρ²`, the power correlating as the
/// complex correlation squared. `analytic_nu_half` for a contiguous run.
fn nu_half_at(hops: &[usize]) -> f64 {
    frame_count(hops, 2)
}

/// The spectral kurtosis estimator's count of independent frames at `hops`,
/// `K² / Σ_{i,j} ρ⁴`. About the stationary point the estimator is the mean of
/// `P² − 4P + 2` (unit mean power), whose correlation between two frames of a
/// circular Gaussian pair is `|ρ|⁴` where the power's is `|ρ|²`; so its variance
/// is `4/K` at this count, not at ν/2.
fn sk_frames_at(hops: &[usize]) -> f64 {
    frame_count(hops, 4)
}

/// Eigenvalues and eigenvectors of the symmetric `n × n` matrix `a` (row-major)
/// by cyclic Jacobi rotations (Golub & Van Loan, *Matrix Computations*, §8.5).
/// Returns the eigenvalues and `V` row-major, eigenvector `i` in column `i`.
fn jacobi(mut a: Vec<f64>, n: usize) -> (Vec<f64>, Vec<f64>) {
    let mut v = vec![0.0; n * n];
    for i in 0..n {
        v[i * n + i] = 1.0;
    }
    let scale: f64 = a.iter().map(|x| x * x).sum();
    for _ in 0..100 {
        let off: f64 = a
            .iter()
            .enumerate()
            .filter(|(idx, _)| idx / n != idx % n)
            .map(|(_, x)| x * x)
            .sum();
        if off <= 1e-30 * scale {
            break;
        }
        for p in 0..n {
            for q in p + 1..n {
                let apq = a[p * n + q];
                if apq == 0.0 {
                    continue;
                }
                let theta = (a[q * n + q] - a[p * n + p]) / (2.0 * apq);
                let t = theta.signum() / (theta.abs() + (theta * theta + 1.0).sqrt());
                let c = 1.0 / (t * t + 1.0).sqrt();
                let s = t * c;
                for r in 0..n {
                    let (x, y) = (a[r * n + p], a[r * n + q]);
                    a[r * n + p] = c * x - s * y;
                    a[r * n + q] = s * x + c * y;
                }
                for r in 0..n {
                    let (x, y) = (a[p * n + r], a[q * n + r]);
                    a[p * n + r] = c * x - s * y;
                    a[q * n + r] = s * x + c * y;
                }
                for r in 0..n {
                    let (x, y) = (v[r * n + p], v[r * n + q]);
                    v[r * n + p] = c * x - s * y;
                    v[r * n + q] = s * x + c * y;
                }
            }
        }
    }
    ((0..n).map(|i| a[i * n + i]).collect(), v)
}

/// The eigen-decomposition of a block's frame-correlation matrix
/// `R_ij = ρ(h_i − h_j)`.
struct FrameSpectrum {
    lam: Vec<f64>,
    /// `V_ki²`, row-major K × K; `None` for the circulant form, where every one is
    /// `1/K`.
    v2: Option<Vec<f64>>,
}

impl FrameSpectrum {
    fn of(hops: &[usize]) -> Self {
        let k = hops.len();
        let contiguous = hops.windows(2).all(|w| w[1] == w[0] + 1);
        if k > EXACT_MAX_FRAMES && contiguous {
            let lam = (0..k)
                .map(|j| {
                    let x = std::f64::consts::TAU * j as f64 / k as f64;
                    rho(0)
                        + 2.0
                            * (1..DISJOINT_HOPS)
                                .map(|m| rho(m) * (x * m as f64).cos())
                                .sum::<f64>()
                })
                .collect();
            return Self { lam, v2: None };
        }
        let mut a = vec![0.0; k * k];
        for (i, &hi) in hops.iter().enumerate() {
            for (j, &hj) in hops.iter().enumerate() {
                a[i * k + j] = rho(hi.abs_diff(hj));
            }
        }
        let (lam, vecs) = jacobi(a, k);
        Self {
            lam,
            v2: Some(vecs.iter().map(|x| x * x).collect()),
        }
    }

    fn weight(&self, k: usize, i: usize) -> f64 {
        match &self.v2 {
            Some(v) => v[k * self.lam.len() + i],
            None => 1.0 / self.lam.len() as f64,
        }
    }
}

/// Frame spectra by block geometry: blocks of one shape share one.
#[derive(Default)]
struct SpectrumCache(HashMap<Vec<usize>, FrameSpectrum>);

impl SpectrumCache {
    fn get(&mut self, hops: &[usize]) -> &FrameSpectrum {
        self.0
            .entry(hops.to_vec())
            .or_insert_with(|| FrameSpectrum::of(hops))
    }
}

/// The exact probability that `c·P_k > s·Σ_j P_j` for circular Gaussian noise
/// whose frames correlate as `own` within frame `k`'s block, with `all_lam` the
/// correlation eigenvalues of every frame the sum runs over (`own`'s among them;
/// other blocks are independent of it), `c > s > 0`.
///
/// With `X ~ CN(0, R)` the event is `X^H Q X > 0`, `Q = c e_k e_kᵀ − s I`, and
/// `R^½ Q R^½ = c r rᵀ − s R` has one positive eigenvalue `μ₊`, the root of the
/// secular equation `1 = c Σ_i z_i² / (μ + s λ_i)`, `z_i² = λ_i V_ki²`. Writing the
/// form as `Σ μ_j E_j` with `E_j` independent `Exp(1)`, the probability is
/// `Π_{μ_j<0} (1 + |μ_j|/μ₊)⁻¹`, which the characteristic polynomial gives as
/// `1 / [μ₊ f′(μ₊) Π_i (1 + s λ_i / μ₊)]`, `f′(μ) = c Σ z_i² / (μ + s λ_i)²`.
fn ratio_exceedance(own: &FrameSpectrum, k: usize, all_lam: &[f64], c: f64, s: f64) -> f64 {
    let z2: Vec<f64> = own
        .lam
        .iter()
        .enumerate()
        .map(|(i, &l)| l * own.weight(k, i))
        .collect();
    let secular = |mu: f64| {
        1.0 - c * z2
            .iter()
            .zip(&own.lam)
            .map(|(&z, &l)| z / (mu + s * l))
            .sum::<f64>()
    };
    // The root lies in (0, c]: f(0) = 1 − c/s < 0, and f(c) ≥ 0 as Σ z_i² = R_kk = 1.
    let (mut lo, mut hi) = (0.0f64, c);
    for _ in 0..100 {
        let mid = 0.5 * (lo + hi);
        if secular(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let mu = 0.5 * (lo + hi);
    let slope = c * z2
        .iter()
        .zip(&own.lam)
        .map(|(&z, &l)| z / (mu + s * l).powi(2))
        .sum::<f64>();
    let log_prod: f64 = all_lam.iter().map(|&l| (s * l / mu).ln_1p()).sum();
    (-(mu * slope).ln() - log_prod).exp()
}

// ── 1. The within-block law ───────────────────────────────────────────────────

/// Every window's power standardised by the mean of the other windows of its
/// block, `r = P_k / mean_{j≠k} P_j`, at the upper `p`-points of `F(2, 2M)`, `M`
/// = ν/2 of the others: the law of a periodogram standardised by an average it
/// does not share samples with (Sulis, Mary & Bigot 2017, Eqs. 9 and 17; Talbot
/// & Thrane 2020). The others overlap the window here, so each rate is read
/// against the exact law of the same ratio through the same frames.
struct Law {
    /// Exceedances per bin, per point of [`LAW_P`].
    counts: Vec<Vec<u64>>,
    /// Values per bin: the windows of blocks of two or more.
    per_bin: u64,
    /// The exact law's rate at each point, over those windows.
    nominal: Vec<f64>,
    /// `M` at the median block.
    m_median: f64,
}

fn within_block(pool: &Pool, cache: &mut SpectrumCache) -> Law {
    let mut counts = vec![vec![0u64; BINS]; LAW_P.len()];
    let mut nominal = vec![0.0f64; LAW_P.len()];
    let mut memo: HashMap<Vec<usize>, Vec<f64>> = HashMap::new();
    let mut per_bin = 0u64;
    let mut ms = Vec::new();
    let mut sum = vec![0.0f64; BINS];
    for block in &pool.blocks {
        let k = block.len();
        if k < 2 {
            continue;
        }
        let others = (k - 1) as f64;
        let m = f64::from(analytic_nu_half(k - 1));
        ms.push(m);
        let points: Vec<f64> = LAW_P
            .iter()
            .map(|&p| stats::f_2_2m_threshold(m, p))
            .collect();
        let geo = geometry(block);
        let rates = memo.entry(geo.clone()).or_insert_with(|| {
            let spec = cache.get(&geo);
            points
                .iter()
                .map(|&t| {
                    let (c, s) = (1.0 + t / others, t / others);
                    match spec.v2 {
                        None => k as f64 * ratio_exceedance(spec, 0, &spec.lam, c, s),
                        Some(_) => (0..k)
                            .map(|w| ratio_exceedance(spec, w, &spec.lam, c, s))
                            .sum(),
                    }
                })
                .collect()
        });
        for (acc, &r) in nominal.iter_mut().zip(rates.iter()) {
            *acc += r;
        }
        sum.fill(0.0);
        for &f in block {
            for (s, &p) in sum.iter_mut().zip(pool.row(f)) {
                *s += f64::from(p);
            }
        }
        for &f in block {
            for (bin, (&s, &p)) in sum.iter().zip(pool.row(f)).enumerate().skip(1) {
                let p = f64::from(p);
                let r = p * others / (s - p);
                if !r.is_finite() {
                    continue;
                }
                for (row, &t) in counts.iter_mut().zip(&points) {
                    if r > t {
                        row[bin] += 1;
                    }
                }
            }
        }
        per_bin += k as u64;
    }
    if per_bin > 0 {
        nominal.iter_mut().for_each(|r| *r /= per_bin as f64);
    }
    Law {
        counts,
        per_bin,
        nominal,
        m_median: median_of(&mut ms),
    }
}

// ── 2. The level's wander ─────────────────────────────────────────────────────

/// Per bin, each block's mean power is one reading of the level; across blocks,
/// Oppenheimer's steadiness `M = μ²/σ²` (Oppenheimer 2016, Eq. 6: the background
/// power is `χ²` with `2M` degrees of freedom) against the blocks' own ν/2, which
/// a stationary level reproduces. `M` below ν/2 is a level that moves between
/// blocks.
struct Wander {
    /// Per bin, measured `M`.
    m: Vec<f64>,
    /// Per bin, `M` over [`Wander::nu_ref`].
    ratio: Vec<f64>,
    /// Per bin, the standard deviation across blocks of `10 lg` of the block mean.
    level_sd_db: Vec<f64>,
    /// The `M` a stationary level gives: the harmonic mean of the blocks' ν/2, the
    /// mean of `1/ν_b` being the block means' relative variance.
    nu_ref: f64,
    /// The `10 lg` spread a stationary level gives, `4.343 · √(mean ψ′(ν_b))`, a
    /// block mean being near `Gamma(ν_b)` in law.
    expected_sd_db: f64,
    blocks: usize,
}

fn wander(pool: &Pool) -> Option<Wander> {
    let blocks: Vec<&Vec<Frame>> = pool.blocks.iter().filter(|b| !b.is_empty()).collect();
    if blocks.len() < 3 {
        return None;
    }
    let nb = blocks.len() as f64;
    let mut means = vec![0.0f64; blocks.len() * BINS];
    for (bi, block) in blocks.iter().enumerate() {
        let row = &mut means[bi * BINS..(bi + 1) * BINS];
        for &f in block.iter() {
            for (m, &p) in row.iter_mut().zip(pool.row(f)) {
                *m += f64::from(p);
            }
        }
        row.iter_mut().for_each(|m| *m /= block.len() as f64);
    }
    let nus: Vec<f64> = blocks.iter().map(|b| nu_half_at(&geometry(b))).collect();
    let nu_ref = nb / nus.iter().map(|v| 1.0 / v).sum::<f64>();
    let expected_sd_db =
        DB_PER_NEPER * (nus.iter().map(|&v| stats::trigamma(v)).sum::<f64>() / nb).sqrt();
    let mut m = vec![f64::NAN; BINS];
    let mut ratio = vec![f64::NAN; BINS];
    let mut level_sd_db = vec![f64::NAN; BINS];
    for bin in 1..BINS {
        let col = |bi: usize| means[bi * BINS + bin];
        let mu = (0..blocks.len()).map(col).sum::<f64>() / nb;
        let var = (0..blocks.len())
            .map(|bi| (col(bi) - mu).powi(2))
            .sum::<f64>()
            / (nb - 1.0);
        if var > 0.0 {
            m[bin] = mu * mu / var;
            ratio[bin] = m[bin] / nu_ref;
        }
        let db: Vec<f64> = (0..blocks.len())
            .map(col)
            .filter(|&v| v > 0.0)
            .map(|v| 10.0 * v.log10())
            .collect();
        if db.len() >= 2 {
            let mean = db.iter().sum::<f64>() / db.len() as f64;
            let v = db.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (db.len() - 1) as f64;
            level_sd_db[bin] = v.sqrt();
        }
    }
    Some(Wander {
        m,
        ratio,
        level_sd_db,
        nu_ref,
        expected_sd_db,
        blocks: blocks.len(),
    })
}

/// `M` measured per block length, over the blocks of each length that occurs
/// [`MIN_GROUP_BLOCKS`] times or more: (windows a block, blocks, ν/2 analytic for
/// a contiguous block, ν/2 of these blocks, median `M` over bins from 43 Hz).
fn wander_by_length(pool: &Pool, bands: &[Band]) -> Vec<(usize, usize, f64, f64, f64)> {
    let mut lengths: Vec<usize> = pool
        .blocks
        .iter()
        .map(Vec::len)
        .filter(|&k| k > 0)
        .collect();
    lengths.sort_unstable();
    lengths.dedup();
    let lo = bands.iter().find(|b| !b.rumble).map_or(1, |b| b.lo);
    let mut out = Vec::new();
    for k in lengths {
        let group = Pool {
            power: pool.power,
            blocks: pool
                .blocks
                .iter()
                .filter(|b| b.len() == k)
                .cloned()
                .collect(),
        };
        if group.blocks.len() < MIN_GROUP_BLOCKS {
            continue;
        }
        let Some(w) = wander(&group) else {
            continue;
        };
        let mut ms: Vec<f64> = w.m[lo..]
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        out.push((
            k,
            group.blocks.len(),
            f64::from(analytic_nu_half(k)),
            w.nu_ref,
            median_of(&mut ms),
        ));
    }
    out
}

// ── 3. Heavy tails: the finite-variance test ──────────────────────────────────

/// `A(1)` and `C(n, 1)` of Skowronek, Zimroz & Wyłomańska (2024): the empirical
/// cumulative second moment `C(k, 1) = (1/k) Σ_{i≤k} x_i²` (their Eq. 1 at
/// N = 1), split into segments at the jumps of its increments; `A(1)` is the
/// least-squares slope of `C(k, 1)` on `k` over the last segment at least a tenth
/// of `n` long, or over the last tenth where there is no jump or no such segment.
/// `c` is scratch.
fn ecem(x: &[f64], c: &mut Vec<f64>) -> (f64, f64) {
    let n = x.len();
    c.clear();
    let mut acc = 0.0;
    for (i, &v) in x.iter().enumerate() {
        acc += v * v;
        c.push(acc / (i + 1) as f64);
    }
    let mut inc: Vec<f64> = c.windows(2).map(|w| (w[1] - w[0]).abs()).collect();
    let bar = if inc.is_empty() {
        f64::INFINITY
    } else {
        let mid = inc.len() / 2;
        JUMP_Q * *inc.select_nth_unstable_by(mid, f64::total_cmp).1
    };
    let min_len = ((SEGMENT_FRACTION * n as f64).ceil() as usize)
        .max(2)
        .min(n);
    let mut seg_end = n;
    let mut jumped = false;
    let mut chosen = None;
    for i in (0..n.saturating_sub(1)).rev() {
        if c[i + 1] - c[i] > bar {
            jumped = true;
            if seg_end - (i + 1) >= min_len {
                chosen = Some((i + 1, seg_end));
                break;
            }
            seg_end = i + 1;
        }
    }
    if chosen.is_none() && jumped && seg_end >= min_len {
        chosen = Some((0, seg_end));
    }
    let (a, b) = chosen.unwrap_or((n - min_len, n));
    let len = (b - a) as f64;
    let t_mean = (a + b + 1) as f64 / 2.0;
    let c_mean = c[a..b].iter().sum::<f64>() / len;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (j, &cv) in c[a..b].iter().enumerate() {
        let dt = (a + j + 1) as f64 - t_mean;
        sxy += dt * (cv - c_mean);
        sxx += dt * dt;
    }
    let slope = if sxx > 0.0 { sxy / sxx } else { 0.0 };
    (slope, c[n - 1])
}

/// SplitMix64's output function: a well-mixed seed from a counter (Steele, Lea &
/// Flood 2014).
fn splitmix64(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// The acceptance regions of `A(1)` and `C(n, 1)` for `n` values: the 2.5 % and
/// 97.5 % points over [`NULL_SAMPLES`] samples of `Exp(1) − 1`, the standardised
/// power of a stationary exponential bin, through the same jump rule. One
/// generator per draw, so the region does not depend on the thread count.
fn finite_variance_null(n: usize) -> NullRegion {
    let threads = std::thread::available_parallelism()
        .map(|t| t.get())
        .unwrap_or(1)
        .clamp(1, 32);
    let chunk = NULL_SAMPLES.div_ceil(threads);
    let draws: Vec<(f64, f64)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    let mut x = vec![0.0f64; n];
                    let mut c = Vec::with_capacity(n);
                    let mut out = Vec::with_capacity(chunk);
                    for draw in t * chunk..((t + 1) * chunk).min(NULL_SAMPLES) {
                        let mut g =
                            Gaussian::new(splitmix64(NULL_SEED ^ ((n as u64) << 24) ^ draw as u64));
                        for v in x.iter_mut() {
                            let (a, b) = (f64::from(g.next()), f64::from(g.next()));
                            // Half the sum of two squared standard normals is Exp(1).
                            *v = 0.5 * (a * a + b * b) - 1.0;
                        }
                        out.push(ecem(&x, &mut c));
                    }
                    out
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("a null thread panicked"))
            .collect()
    });
    let region = |mut v: Vec<f64>| {
        v.sort_by(f64::total_cmp);
        let q = |p: f64| v[((v.len() - 1) as f64 * p).round() as usize];
        (q(0.025), q(0.975))
    };
    NullRegion {
        a1: region(draws.iter().map(|d| d.0).collect()),
        cn: region(draws.iter().map(|d| d.1).collect()),
    }
}

/// The finite-variance test's acceptance regions for one `n`: `A(1)`'s and
/// `C(n, 1)`'s 2.5 % and 97.5 % points.
#[derive(Clone, Copy)]
struct NullRegion {
    a1: (f64, f64),
    cn: (f64, f64),
}

/// The test's verdict for one band.
struct FiniteVariance {
    n: usize,
    bin_stride: usize,
    a1: f64,
    cn: f64,
    region: NullRegion,
}

/// Indices in `block` of frames at least [`DISJOINT_HOPS`] apart, the first kept.
fn spaced(block: &[Frame]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    for (i, f) in block.iter().enumerate() {
        if out
            .last()
            .is_none_or(|&l| f.hop >= block[l].hop + DISJOINT_HOPS)
        {
            out.push(i);
        }
    }
    out
}

/// The finite-variance sample's centre under stationary Gaussian noise: `E[x²]`
/// for `x = P/mean − 1` with the mean over every window of the pool, as the
/// data's is, averaged over the frames [`spaced`] keeps. The mean shares the
/// sampled window and the windows it overlaps, so the centre sits near `1 − 2/ν`
/// rather than at the exponential law's 1. From the exact law of `r = P/mean`,
/// `E[r^m] = ∫ m x^{m−1} P(r > x) dx`, by Simpson's rule up to `x = 60`, where
/// the tail is below `e⁻⁶⁰`.
fn fv_centre(pool: &Pool, cache: &mut SpectrumCache) -> f64 {
    const PANELS: usize = 1200;
    let n = pool.frames();
    if n < 2 {
        return f64::NAN;
    }
    let blocks: Vec<&Vec<Frame>> = pool.blocks.iter().filter(|b| !b.is_empty()).collect();
    for block in &blocks {
        cache.get(&geometry(block));
    }
    let all_lam: Vec<f64> = blocks
        .iter()
        .flat_map(|b| cache.0[&geometry(b)].lam.iter().copied())
        .collect();
    let x_max = 60.0f64.min(n as f64 * (1.0 - 1e-9));
    let h = x_max / PANELS as f64;
    let mut memo: HashMap<(Vec<usize>, usize), f64> = HashMap::new();
    let (mut sum, mut count) = (0.0, 0usize);
    for block in &blocks {
        let geo = geometry(block);
        let spec = &cache.0[&geo];
        for i in spaced(block) {
            let k = if spec.v2.is_none() { 0 } else { i };
            let centre = *memo.entry((geo.clone(), k)).or_insert_with(|| {
                let (mut m1, mut m2) = (0.0, 0.0);
                for i in 0..=PANELS {
                    let x = i as f64 * h;
                    let weight = if i == 0 || i == PANELS {
                        1.0
                    } else if i.is_multiple_of(2) {
                        2.0
                    } else {
                        4.0
                    };
                    let tail = ratio_exceedance(spec, k, &all_lam, 1.0, x / n as f64);
                    m1 += weight * tail;
                    m2 += weight * 2.0 * x * tail;
                }
                (m2 - 2.0 * m1) * h / 3.0 + 1.0
            });
            sum += centre;
            count += 1;
        }
    }
    sum / count.max(1) as f64
}

/// The finite-variance test on `band`: `x = P/mean − 1` at bins
/// [`DISJOINT_BINS`] apart and windows [`DISJOINT_HOPS`] apart in each block, in
/// time order, so the values are independent as the test requires; `C(n, 1)`'s
/// region is the exponential null's moved onto `centre` ([`fv_centre`]). `Err(n)`
/// where fewer than [`MIN_FV_VALUES`] values exist.
fn finite_variance(
    pool: &Pool,
    means: &[f64],
    band: &Band,
    centre: f64,
    nulls: &mut HashMap<usize, NullRegion>,
) -> std::result::Result<FiniteVariance, usize> {
    let picked: Vec<Frame> = pool
        .blocks
        .iter()
        .flat_map(|b| spaced(b).into_iter().map(|i| b[i]))
        .collect();
    let span = band.hi - band.lo;
    let mut stride = DISJOINT_BINS;
    while picked.len() * span.div_ceil(stride) > MAX_FV_VALUES {
        stride += DISJOINT_BINS;
    }
    let mut x = Vec::with_capacity(picked.len() * span.div_ceil(stride));
    for &f in &picked {
        let row = pool.row(f);
        for bin in (band.lo..band.hi).step_by(stride) {
            if means[bin] > 0.0 {
                x.push(f64::from(row[bin]) / means[bin] - 1.0);
            }
        }
    }
    if x.len() < MIN_FV_VALUES {
        return Err(x.len());
    }
    let (a1, cn) = ecem(&x, &mut Vec::new());
    let mut region = *nulls
        .entry(x.len())
        .or_insert_with(|| finite_variance_null(x.len()));
    // C(n, 1)'s region moves onto the sample's own centre; A(1), a slope, keeps
    // its null's.
    if centre.is_finite() {
        region.cn = (region.cn.0 + centre - 1.0, region.cn.1 + centre - 1.0);
    }
    Ok(FiniteVariance {
        n: x.len(),
        bin_stride: stride,
        a1,
        cn,
        region,
    })
}

// ── 4. The kurtosis bands ─────────────────────────────────────────────────────

/// Antoni's spectral kurtosis in its STFT form, `SK = ⟨P²⟩ / ⟨P⟩² − 2` per bin
/// over the pool's windows (Antoni 2006, Eq. 40): zero for stationary Gaussian
/// noise with variance `4/K` (Eq. 44), and `16/K` for a Gaussian-driven level that
/// moves (Eq. 43). The bands are `±Φ⁻¹(1 − α/2)·2/√K` and twice that.
struct Kurtosis {
    sk: Vec<f64>,
    /// `K` for the variance: [`sk_frames_at`] summed over blocks.
    frames_sk: f64,
    /// ν/2 summed over blocks, for comparison.
    frames_nu: f64,
    /// Band 1's half-width.
    band: f64,
}

fn kurtosis(pool: &Pool) -> Kurtosis {
    let mut s2 = vec![0.0f64; BINS];
    let mut s4 = vec![0.0f64; BINS];
    for block in &pool.blocks {
        for &f in block {
            for ((a, b), &p) in s2.iter_mut().zip(s4.iter_mut()).zip(pool.row(f)) {
                let p = f64::from(p);
                *a += p;
                *b += p * p;
            }
        }
    }
    let n = pool.frames() as f64;
    let sk = s2
        .iter()
        .zip(&s4)
        .enumerate()
        .map(|(bin, (&a, &b))| {
            if bin == 0 || a <= 0.0 {
                f64::NAN
            } else {
                (b / n) / (a / n).powi(2) - 2.0
            }
        })
        .collect();
    let frames_sk: f64 = pool
        .blocks
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| sk_frames_at(&geometry(b)))
        .sum();
    let frames_nu: f64 = pool
        .blocks
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| nu_half_at(&geometry(b)))
        .sum();
    Kurtosis {
        sk,
        frames_sk,
        frames_nu,
        band: stats::normal_quantile(1.0 - SK_ALPHA / 2.0) * 2.0 / frames_sk.sqrt(),
    }
}

// ── 5. The tail ───────────────────────────────────────────────────────────────

/// `P / mean`, with the bin's mean over every window of the pool, against the
/// exponential law's points `−ln p`. The nominal is the exact law of the same
/// ratio through the same frames, the mean including the window itself and the
/// windows it overlaps.
struct Tail {
    /// Exceedances per bin, per point of [`TAIL_P`].
    counts: Vec<Vec<u64>>,
    per_bin: u64,
    nominal: Vec<f64>,
    /// Per band, the quantile ratio `q_{1−p} / (−ln p)` per point, where the band
    /// has the values for it.
    ratios: Vec<Vec<Option<f64>>>,
}

fn tail(pool: &Pool, means: &[f64], bands: &[Band], cache: &mut SpectrumCache) -> Tail {
    let n = pool.frames();
    let mut counts = vec![vec![0u64; BINS]; TAIL_P.len()];
    let points: Vec<f64> = TAIL_P.iter().map(|p| -p.ln()).collect();
    for block in &pool.blocks {
        for &f in block {
            for (bin, &p) in pool.row(f).iter().enumerate().skip(1) {
                if means[bin] <= 0.0 {
                    continue;
                }
                let r = f64::from(p) / means[bin];
                for (row, &t) in counts.iter_mut().zip(&points) {
                    if r > t {
                        row[bin] += 1;
                    }
                }
            }
        }
    }

    // The nominal: every block's eigenvalues enter the mean; each window's own
    // block carries its correlation with the mean.
    let mut shapes: HashMap<Vec<usize>, usize> = HashMap::new();
    for block in pool.blocks.iter().filter(|b| !b.is_empty()) {
        *shapes.entry(geometry(block)).or_default() += 1;
        cache.get(&geometry(block));
    }
    let all_lam: Vec<f64> = pool
        .blocks
        .iter()
        .filter(|b| !b.is_empty())
        .flat_map(|b| cache.0[&geometry(b)].lam.iter().copied())
        .collect();
    let mut nominal = vec![0.0f64; TAIL_P.len()];
    if n > 1 {
        for (geo, &count) in &shapes {
            let spec = &cache.0[geo];
            for (acc, &x) in nominal.iter_mut().zip(&points) {
                let s = x / n as f64;
                let per_block = match spec.v2 {
                    None => geo.len() as f64 * ratio_exceedance(spec, 0, &all_lam, 1.0, s),
                    Some(_) => (0..geo.len())
                        .map(|k| ratio_exceedance(spec, k, &all_lam, 1.0, s))
                        .sum(),
                };
                *acc += count as f64 * per_block;
            }
        }
        nominal.iter_mut().for_each(|r| *r /= n as f64);
    }

    let frames: Vec<Frame> = pool.blocks.iter().flatten().copied().collect();
    let mut values: Vec<f32> = Vec::new();
    let ratios = bands
        .iter()
        .map(|band| {
            values.clear();
            for &f in &frames {
                let row = pool.row(f);
                values.extend(
                    (band.lo..band.hi)
                        .filter(|&bin| means[bin] > 0.0)
                        .map(|bin| (f64::from(row[bin]) / means[bin]) as f32),
                );
            }
            TAIL_P
                .iter()
                .zip(&points)
                .map(|(&p, &x)| {
                    if values.is_empty() || (p < 0.01 && (values.len() as u64) < MIN_VALUES_AT_1E3)
                    {
                        return None;
                    }
                    let idx = ((values.len() - 1) as f64 * (1.0 - p)).round() as usize;
                    let q = *values.select_nth_unstable_by(idx, f32::total_cmp).1;
                    Some(f64::from(q) / x)
                })
                .collect()
        })
        .collect();
    Tail {
        counts,
        per_bin: n as u64,
        nominal,
        ratios,
    }
}

// ── 6. The location three ways ────────────────────────────────────────────────

/// Towsey's histogram mode of `values_db`: 100 bins from the minimum to the
/// maximum, a centred 5-point moving average, the first highest bin's centre,
/// held at the 95th bin where it falls in the top five (Towsey 2017, the noise
/// profile of a recording's frames). An exponential power's dB values have the
/// density `e^{y − e^y}` in `y = ln(P/μ)`, peaked at `y = 0`, so the mode is
/// `10 lg μ` without bias.
fn log_mode(values_db: &[f64]) -> f64 {
    if values_db.is_empty() {
        return f64::NAN;
    }
    let (lo, hi) = values_db
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &v| {
            (a.min(v), b.max(v))
        });
    if hi <= lo {
        return lo;
    }
    let width = (hi - lo) / MODE_BINS as f64;
    let mut counts = [0.0f64; MODE_BINS];
    for &v in values_db {
        counts[(((v - lo) / width) as usize).min(MODE_BINS - 1)] += 1.0;
    }
    let half = MODE_SMOOTH / 2;
    let smoothed: Vec<f64> = (0..MODE_BINS)
        .map(|i| {
            counts[i.saturating_sub(half)..(i + half + 1).min(MODE_BINS)]
                .iter()
                .sum::<f64>()
                / MODE_SMOOTH as f64
        })
        .collect();
    let first_max = smoothed
        .iter()
        .enumerate()
        .fold(0, |best, (i, &v)| if v > smoothed[best] { i } else { best });
    let cap = MODE_BINS * 95 / 100;
    let k = if first_max >= cap { cap - 1 } else { first_max };
    lo + (k as f64 + 0.5) * width
}

/// Per bin: `10 lg` of the mean power, of `median / ln 2` and the log-domain mode,
/// three estimates of the same level under the exponential law.
struct Location {
    mean_db: Vec<f64>,
    median_db: Vec<f64>,
    mode_db: Vec<f64>,
}

fn location(pool: &Pool) -> Location {
    let frames: Vec<Frame> = pool.blocks.iter().flatten().copied().collect();
    let mut out = Location {
        mean_db: vec![f64::NAN; BINS],
        median_db: vec![f64::NAN; BINS],
        mode_db: vec![f64::NAN; BINS],
    };
    if frames.is_empty() {
        return out;
    }
    let mut col: Vec<f64> = Vec::with_capacity(frames.len());
    let mut db: Vec<f64> = Vec::with_capacity(frames.len());
    for bin in 1..BINS {
        col.clear();
        col.extend(
            frames
                .iter()
                .map(|f| f64::from(pool.power[f.row * BINS + bin])),
        );
        db.clear();
        db.extend(col.iter().filter(|&&p| p > 0.0).map(|&p| 10.0 * p.log10()));
        let mean = col.iter().sum::<f64>() / col.len() as f64;
        let mid = col.len() / 2;
        let median = *col.select_nth_unstable_by(mid, f64::total_cmp).1;
        out.mean_db[bin] = 10.0 * mean.log10();
        out.median_db[bin] = 10.0 * (median / std::f64::consts::LN_2).log10();
        out.mode_db[bin] = log_mode(&db);
    }
    out
}

// ── Pooled helpers ────────────────────────────────────────────────────────────

/// The upper median of the finite values of `v`; NaN where there are none.
fn median_of(v: &mut Vec<f64>) -> f64 {
    v.retain(|x| x.is_finite());
    if v.is_empty() {
        return f64::NAN;
    }
    let mid = v.len() / 2;
    *v.select_nth_unstable_by(mid, f64::total_cmp).1
}

/// The `p`-quantile of the finite values of `v`, by the nearest rank.
fn quantile_of(v: &[f64], p: f64) -> f64 {
    let mut w: Vec<f64> = v.iter().copied().filter(|x| x.is_finite()).collect();
    if w.is_empty() {
        return f64::NAN;
    }
    w.sort_by(f64::total_cmp);
    w[((w.len() - 1) as f64 * p).round() as usize]
}

/// Per-bin mean power over the pool.
fn bin_means(pool: &Pool) -> Vec<f64> {
    let mut m = vec![0.0f64; BINS];
    for block in &pool.blocks {
        for &f in block {
            for (a, &p) in m.iter_mut().zip(pool.row(f)) {
                *a += f64::from(p);
            }
        }
    }
    let n = pool.frames().max(1) as f64;
    m.iter_mut().for_each(|a| *a /= n);
    m
}

/// A pooled count's dispersion against the binomial: the variance of each band's
/// total, from the per-bin counts' autocovariance at lags 0–2 across bins (the
/// span of the Hann window's power correlation), summed over the bands from
/// 43 Hz and divided by `Σ n p̂(1 − p̂)`. Overlapping windows cluster exceedances
/// in time, and neighbouring bins share them, so a Clopper–Pearson interval on
/// the raw count is too narrow by `√φ`.
fn dispersion(counts: &[u64], per_bin: u64, bands: &[Band]) -> f64 {
    let (mut var, mut binom) = (0.0f64, 0.0f64);
    for band in bands.iter().filter(|b| !b.rumble && b.hi - b.lo >= 3) {
        let c: Vec<f64> = counts[band.lo..band.hi].iter().map(|&k| k as f64).collect();
        let mean = c.iter().sum::<f64>() / c.len() as f64;
        let d: Vec<f64> = c.iter().map(|v| v - mean).collect();
        let lag = |l: usize| d.iter().zip(&d[l..]).map(|(a, b)| a * b).sum::<f64>();
        var += lag(0) + 2.0 * lag(1) + 2.0 * lag(2);
        let p = mean / per_bin as f64;
        binom += c.len() as f64 * per_bin as f64 * p * (1.0 - p);
    }
    if binom > 0.0 { var / binom } else { f64::NAN }
}

/// A rate's 95 % Clopper–Pearson interval on the effective count `n/φ`, `φ`
/// floored at 1, and whether `nominal` lies in it.
fn interval(k: u64, n: u64, phi: f64, nominal: f64) -> (f64, f64, &'static str) {
    let phi = if phi.is_finite() { phi.max(1.0) } else { 1.0 };
    let n_eff = ((n as f64 / phi).round() as u64).max(1);
    let k_eff = ((k as f64 / phi).round() as u64).min(n_eff);
    let (lo, hi) = stats::clopper_pearson(k_eff, n_eff, 0.05);
    let verdict = if !nominal.is_finite() {
        ""
    } else if nominal < lo {
        "ABOVE"
    } else if nominal > hi {
        "BELOW"
    } else {
        "inside"
    };
    (lo, hi, verdict)
}

// ── Reading the data ──────────────────────────────────────────────────────────

/// Where a capture's silence threshold came from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum SigmaFrom {
    CommandLine,
    Logged,
    Candidates,
    GateDefault,
}

impl SigmaFrom {
    fn label(self) -> &'static str {
        match self {
            SigmaFrom::CommandLine => "the command line",
            SigmaFrom::Logged => "the logged noise_floor",
            SigmaFrom::Candidates => "calibration_candidates.json setting_primary",
            SigmaFrom::GateDefault => "the gate's default (nothing logged)",
        }
    }
}

/// An external store's silence threshold: `calibration_candidates.json`'s
/// `setting_primary.value`, the set's one σ.
fn candidates_sigma(dir: &Path) -> Option<f32> {
    let text = std::fs::read_to_string(dir.join("calibration_candidates.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json["setting_primary"]["value"]
        .as_f64()
        .map(|v| v as f32)
        .filter(|v| *v > 0.0)
}

/// The screen's thresholds for the capture in `dir`: σ from the command line,
/// else the capture's logged value, else an external store's candidate, else the
/// gate's default.
fn thresholds_for(dir: &Path, sigma: Option<f32>) -> (Thresholds, SigmaFrom) {
    let with = |s: f32| {
        Thresholds::resolve(
            dir,
            &Overrides {
                silence: Some(s),
                ..Overrides::default()
            },
        )
    };
    if let Some(s) = sigma {
        return (with(s), SigmaFrom::CommandLine);
    }
    let logged = Thresholds::resolve(dir, &Overrides::default());
    if logged.silence == Source::Logged {
        return (logged, SigmaFrom::Logged);
    }
    match candidates_sigma(dir) {
        Some(s) => (with(s), SigmaFrom::Candidates),
        None => (logged, SigmaFrom::GateDefault),
    }
}

/// The 8192-point Hann frame's power, `|X|²`, one row per window.
struct Spectra {
    plan: std::sync::Arc<dyn realfft::RealToComplex<f32>>,
    time: Vec<f32>,
    spec: Vec<Complex<f32>>,
    mag: Vec<f32>,
}

impl Spectra {
    fn new() -> Self {
        Self {
            plan: RealFftPlanner::<f32>::new().plan_fft_forward(BASS_WINDOW_SIZE),
            time: vec![0.0; BASS_WINDOW_SIZE],
            spec: vec![Complex::new(0.0, 0.0); BINS + 1],
            mag: vec![0.0; BINS],
        }
    }

    /// Appends the power of `samples`, one window, to `power`.
    fn push(&mut self, samples: &[f32], power: &mut Vec<f32>) {
        spectral::fft(
            samples,
            &mut self.time,
            &mut self.spec,
            &self.plan,
            BASS_WINDOW_SIZE,
        );
        spectral::magnitude_spectrum(&self.spec, BASS_WINDOW_SIZE, &mut self.mag);
        power.extend(self.mag.iter().map(|&m| m * m));
    }
}

/// One capture set's pre-roll, read and screened.
struct SetData {
    label: String,
    root: PathBuf,
    power: Vec<f32>,
    /// Every pre-roll window, per capture in time order.
    all: Vec<Vec<Frame>>,
    /// The windows the Gatekeeper called silence, per capture.
    screened: Vec<Vec<Frame>>,
    found: usize,
    skipped: Vec<(String, &'static str)>,
    sigmas: Vec<(SigmaFrom, f32)>,
    /// Windows, all and screened, that the −20 dB onset removed from the pre-roll
    /// the kept record's start alone would give.
    after_onset: (usize, usize),
}

/// Reads every capture under `root` in recorded order: the onset, the earlier of
/// `audio.raw`'s place in the full event and the first hop within 20 dB of the
/// loudest (where that falls inside the record before it, or alone where there is
/// no `audio.raw`); the pre-roll's windows on the hop grid, ending
/// [`ONSET_GUARD_HOPS`] hops before the onset; and the Gatekeeper's verdict on each.
fn load_set(label: &str, root: &Path, sigma: Option<f32>) -> Result<SetData> {
    let mut dirs = capture::find(root)?;
    let serial = |d: &Path| {
        d.file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.rsplit('_').next())
            .and_then(|s| s.parse::<u64>().ok())
    };
    dirs.sort_by_key(|d| (serial(d), d.clone()));
    let mut spectra = Spectra::new();
    let mut set = SetData {
        label: label.to_string(),
        root: root.to_path_buf(),
        power: Vec::new(),
        all: Vec::new(),
        screened: Vec::new(),
        found: dirs.len(),
        skipped: Vec::new(),
        sigmas: Vec::new(),
        after_onset: (0, 0),
    };
    for dir in &dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();
        let Some(full) = raw::full_event(dir) else {
            set.skipped.push((name, "no audio_full_event.raw"));
            continue;
        };
        let lead = match raw::stable(dir) {
            Some(kept) => match preroll_offset(&full, &kept) {
                Some(l) => l,
                None => {
                    set.skipped
                        .push((name, "audio.raw is not inside the full event"));
                    continue;
                }
            },
            None => ambient::onset_sample(&full),
        };
        let (thresholds, from) = thresholds_for(dir, sigma);
        set.sigmas.push((from, thresholds.config.silence_threshold));
        let verdicts = gate_replay::run(&full, thresholds.config);
        // The kept record begins at the Gatekeeper's Stable verdict, hops after
        // the strike, so its start alone lets the strike into the pre-roll.
        let onset = Some(ambient::onset_sample(&full))
            .filter(|&o| o > 0 && o < lead)
            .unwrap_or(lead);
        let usable = lead.saturating_sub(ONSET_GUARD_HOPS * HOP_SIZE);
        let clean = onset.saturating_sub(ONSET_GUARD_HOPS * HOP_SIZE);
        let mut all = Vec::new();
        let mut screened = Vec::new();
        let mut end = BASS_WINDOW_SIZE;
        while end <= usable {
            let hop = (end - BASS_WINDOW_SIZE) / HOP_SIZE;
            let silent = verdicts.get(hop).map(|h| h.result.state) == Some(SignalState::Silence);
            if end > clean {
                set.after_onset.0 += 1;
                set.after_onset.1 += usize::from(silent);
                end += HOP_SIZE;
                continue;
            }
            let frame = Frame {
                hop,
                row: set.power.len() / BINS,
            };
            spectra.push(&full[end - BASS_WINDOW_SIZE..end], &mut set.power);
            all.push(frame);
            if silent {
                screened.push(frame);
            }
            end += HOP_SIZE;
        }
        set.all.push(all);
        set.screened.push(screened);
    }
    Ok(set)
}

/// A `cargo lab audio record` directory, every window of it.
struct RecordingData {
    dir: PathBuf,
    fs: f64,
    rate_logged: bool,
    label: Option<String>,
    samples: usize,
    power: Vec<f32>,
    frames: usize,
}

fn load_recording(dir: &Path) -> Result<RecordingData> {
    let path = dir.join("audio.raw");
    let audio = raw::read(&path).ok_or_else(|| anyhow!("cannot read {}", path.display()))?;
    let meta: Option<serde_json::Value> = std::fs::read_to_string(dir.join("recording.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok());
    let logged = meta.as_ref().and_then(|m| m["sample_rate_hz"].as_f64());
    let mut spectra = Spectra::new();
    let mut power = Vec::new();
    let mut end = BASS_WINDOW_SIZE;
    while end <= audio.len() {
        spectra.push(&audio[end - BASS_WINDOW_SIZE..end], &mut power);
        end += HOP_SIZE;
    }
    Ok(RecordingData {
        dir: dir.to_path_buf(),
        fs: logged.unwrap_or(f64::from(SAMPLE_RATE)),
        rate_logged: logged.is_some(),
        label: meta
            .as_ref()
            .and_then(|m| m["label"].as_str().map(str::to_string)),
        samples: audio.len(),
        frames: power.len() / BINS,
        power,
    })
}

/// The recording's windows in blocks of `seconds` (each window wholly inside one
/// block, incomplete last blocks dropped), or one block for the whole record.
fn recording_blocks(rec: &RecordingData, seconds: Option<f64>) -> Vec<Vec<Frame>> {
    let frame = |h: usize| Frame { hop: h, row: h };
    let Some(t) = seconds else {
        return vec![(0..rec.frames).map(frame).collect()];
    };
    let len = (t * rec.fs).round() as usize;
    let complete = rec.samples / len.max(1);
    let mut blocks = vec![Vec::new(); complete];
    for h in 0..rec.frames {
        let (start, end) = (h * HOP_SIZE, h * HOP_SIZE + BASS_WINDOW_SIZE);
        let b = start / len;
        if b < complete && end <= (b + 1) * len {
            blocks[b].push(frame(h));
        }
    }
    blocks.retain(|b| !b.is_empty());
    blocks
}

// ── The tables ────────────────────────────────────────────────────────────────

fn print_bands(bands: &[Band], bin_hz: f64) {
    println!(
        "  bands, Hz (bin width {bin_hz:.3} Hz; bin 0, DC, left out): {}",
        bands
            .iter()
            .map(|b| format!("{} (bins {}–{})", b.label, b.lo, b.hi - 1))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

fn fmt_p(p: f64) -> String {
    if p >= 0.1 {
        format!("{p}")
    } else {
        format!("{p:.0e}")
    }
}

/// The rate rows of a law: per band and point, the realised exceedance with its
/// interval against the exact nominal and against `p`. Returns (rows inside, rows).
fn print_rates(
    bands: &[Band],
    ps: &[f64],
    counts: &[Vec<u64>],
    per_bin: u64,
    nominal: &[f64],
    phis: &[f64],
    ratios: Option<&[Vec<Option<f64>>]>,
) -> (usize, usize) {
    println!(
        "  {:<14} {:>6} {:>11} {:>9} {:>11}  {:<24} {:>10}  {:<7}{}",
        "band (Hz)",
        "p",
        "values",
        "exceed",
        "rate",
        "95 % CP on n/φ",
        "nominal",
        "verdict",
        if ratios.is_some() {
            "  q₁₋ₚ ÷ (−ln p)"
        } else {
            ""
        }
    );
    let (mut inside, mut rows) = (0, 0);
    for (bi, band) in bands.iter().enumerate() {
        let values = per_bin * (band.hi - band.lo) as u64;
        for (pi, &p) in ps.iter().enumerate() {
            if p < 0.01 && values < MIN_VALUES_AT_1E3 {
                continue;
            }
            let k: u64 = counts[pi][band.lo..band.hi].iter().sum();
            let rate = if values > 0 {
                k as f64 / values as f64
            } else {
                f64::NAN
            };
            let (lo, hi, verdict) = interval(k, values, phis[pi], nominal[pi]);
            rows += 1;
            if verdict == "inside" {
                inside += 1;
            }
            let ratio = ratios
                .and_then(|r| r[bi][pi])
                .map(|v| format!("  {v:>6.3}"))
                .unwrap_or_default();
            println!(
                "  {:<14} {:>6} {:>11} {:>9} {:>11.4e}  [{:.3e}, {:.3e}] {:>11.4e}  {:<7}{}",
                band.label,
                fmt_p(p),
                values,
                k,
                rate,
                lo,
                hi,
                nominal[pi],
                verdict,
                ratio
            );
        }
    }
    (inside, rows)
}

fn print_law(law: &Law, bands: &[Band]) {
    let phis: Vec<f64> = law
        .counts
        .iter()
        .map(|c| dispersion(c, law.per_bin, bands))
        .collect();
    println!(
        "\n  ── 1. Within-block law: r = P_k ÷ mean of the block's other windows, at the upper p-point of F(2, 2M), M = ν/2 of the others ({:.2} at the median block); nominal is the exact law of r for stationary Gaussian noise through these overlapping frames (p for independent ones) ──",
        law.m_median
    );
    println!(
        "  dispersion φ, count variance across bins ÷ binomial (intervals use max(1, φ)): {}",
        LAW_P
            .iter()
            .zip(&phis)
            .map(|(p, f)| format!("p={} {f:.2}", fmt_p(*p)))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    println!(
        "  exact nominal at the four points: {}  (p: {})",
        law.nominal
            .iter()
            .map(|v| format!("{v:.4e}"))
            .collect::<Vec<_>>()
            .join(", "),
        LAW_P
            .iter()
            .map(|p| fmt_p(*p))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if law.per_bin == 0 {
        println!("  no block holds two windows");
        return;
    }
    let (inside, rows) = print_rates(
        bands,
        &LAW_P,
        &law.counts,
        law.per_bin,
        &law.nominal,
        &phis,
        None,
    );
    println!(
        "  {inside} of {rows} rows inside their interval; about {:.1} outside expected by chance at 95 %",
        0.05 * rows as f64
    );
}

fn print_wander(w: Option<&Wander>, by_length: &[(usize, usize, f64, f64, f64)], bands: &[Band]) {
    println!(
        "\n  ── 2. Level wander: per bin, each block's mean power is one reading; M = μ²/σ² across blocks (Oppenheimer 2016, Eq. 6) against the blocks' ν/2 (1.0 = a stationary level; below it, a level that moves between blocks) ──"
    );
    let Some(w) = w else {
        println!("  fewer than three blocks: not measurable");
        return;
    };
    println!(
        "  {} blocks; ν/2 of the blocks (harmonic mean) {:.3}; the 10 lg spread of a stationary level {:.2} dB",
        w.blocks, w.nu_ref, w.expected_sd_db
    );
    println!(
        "  {:<14} {:>6} {:>10} {:>18} {:>12}",
        "band (Hz)", "bins", "M/(ν/2)", "[5 %, 95 %] bins", "level SD dB"
    );
    for band in bands {
        let r = &w.ratio[band.lo..band.hi];
        let mut rv = r.to_vec();
        let mut sd = w.level_sd_db[band.lo..band.hi].to_vec();
        println!(
            "  {:<14} {:>6} {:>10.3} [{:>6.3}, {:>6.3}]  {:>12.2}",
            band.label,
            band.hi - band.lo,
            median_of(&mut rv),
            quantile_of(r, 0.05),
            quantile_of(r, 0.95),
            median_of(&mut sd)
        );
    }
    if !by_length.is_empty() {
        println!(
            "  by block length (lengths with ≥ {MIN_GROUP_BLOCKS} blocks; M is the median over bins from 43 Hz):"
        );
        println!(
            "  {:>14} {:>7} {:>13} {:>14} {:>11} {:>8}",
            "windows/block", "blocks", "ν/2 analytic", "ν/2 of these", "M measured", "M/(ν/2)"
        );
        for &(k, n, nu_a, nu, m) in by_length {
            println!(
                "  {k:>14} {n:>7} {nu_a:>13.3} {nu:>14.3} {m:>11.3} {:>8.3}",
                m / nu
            );
        }
    }
}

fn print_finite_variance(
    pool: &Pool,
    means: &[f64],
    bands: &[Band],
    cache: &mut SpectrumCache,
    nulls: &mut HashMap<usize, NullRegion>,
) {
    let centre = fv_centre(pool, cache);
    println!(
        "\n  ── 3. Finite variance (Skowronek, Zimroz & Wyłomańska 2024; N = 1): x = P/mean − 1, bins ≥ {DISJOINT_BINS} apart and windows ≥ {DISJOINT_HOPS} hops apart, in time order; region = 2.5–97.5 % of {NULL_SAMPLES} Exp(1) − 1 samples of the same n, C(n,1)'s moved onto the sample's exact centre; a jump is an increment > {JUMP_Q} × the median |increment| ──"
    );
    println!(
        "  C(n,1)'s centre for these windows under stationary Gaussian noise (the mean taken from them): {centre:.4}"
    );
    println!(
        "  {:<14} {:>7} {:>6} {:>12} {:<26} {:<8} {:>9} {:<20} {:<8}",
        "band (Hz)", "n", "stride", "A(1)", "region", "", "C(n,1)", "region", ""
    );
    for band in bands {
        match finite_variance(pool, means, band, centre, nulls) {
            Err(n) => println!(
                "  {:<14} {n:>7}  fewer than {MIN_FV_VALUES} values",
                band.label
            ),
            Ok(fv) => {
                let tag = |v: f64, (lo, hi): (f64, f64)| {
                    if v < lo || v > hi {
                        "OUTSIDE"
                    } else {
                        "inside"
                    }
                };
                println!(
                    "  {:<14} {:>7} {:>6} {:>12.3e} [{:>10.3e}, {:>10.3e}] {:<8} {:>9.4} [{:>7.4}, {:>7.4}]  {:<8}",
                    band.label,
                    fv.n,
                    fv.bin_stride,
                    fv.a1,
                    fv.region.a1.0,
                    fv.region.a1.1,
                    tag(fv.a1, fv.region.a1),
                    fv.cn,
                    fv.region.cn.0,
                    fv.region.cn.1,
                    tag(fv.cn, fv.region.cn)
                );
            }
        }
    }
}

fn print_kurtosis(k: &Kurtosis, bands: &[Band]) {
    println!(
        "\n  ── 4. Spectral kurtosis per bin over the pool's windows (Antoni 2006, Eq. 40); band 1 = ±Φ⁻¹({:.3})·2/√K = ±{:.4} (Eq. 44), band 2 twice it (Eq. 43); the normal law's shares 95 % / 5 % / 0.01 %, SK's right skew at finite K adding to the upper side ──",
        1.0 - SK_ALPHA / 2.0,
        k.band
    );
    println!(
        "  K = {:.1}, SK's own count of independent frames (Σ K²/Σρ⁴ over blocks); ν/2 = {:.1} would widen the bands by {:.0} %",
        k.frames_sk,
        k.frames_nu,
        100.0 * ((k.frames_sk / k.frames_nu).sqrt() - 1.0)
    );
    println!(
        "  {:<14} {:>6} {:>9} {:>32} {:>26} {:>10}",
        "band (Hz)",
        "bins",
        "inside",
        "between (level not stationary)",
        "outside (not Gaussian)",
        "median SK"
    );
    for band in bands {
        let sk: Vec<f64> = k.sk[band.lo..band.hi]
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        let n = sk.len().max(1) as f64;
        let inside = sk.iter().filter(|v| v.abs() <= k.band).count() as f64 / n;
        let outside = sk.iter().filter(|v| v.abs() > 2.0 * k.band).count() as f64 / n;
        let mut sv = sk.clone();
        println!(
            "  {:<14} {:>6} {:>8.2}% {:>31.2}% {:>25.2}% {:>10.4}",
            band.label,
            sk.len(),
            100.0 * inside,
            100.0 * (1.0 - inside - outside),
            100.0 * outside,
            median_of(&mut sv)
        );
    }
}

fn print_tail(t: &Tail, bands: &[Band]) {
    let phis: Vec<f64> = t
        .counts
        .iter()
        .map(|c| dispersion(c, t.per_bin, bands))
        .collect();
    println!(
        "\n  ── 5. Tail: P ÷ the bin's mean over the pool, at the exponential law's −ln p; nominal is the exact law of that ratio with the mean taken from these windows; q₁₋ₚ ÷ (−ln p) is the multiplier a known-mean threshold needs (1.0 = exponential) ──"
    );
    println!(
        "  dispersion φ: {}",
        TAIL_P
            .iter()
            .zip(&phis)
            .map(|(p, f)| format!("p={} {f:.2}", fmt_p(*p)))
            .collect::<Vec<_>>()
            .join(" · ")
    );
    let (inside, rows) = print_rates(
        bands,
        &TAIL_P,
        &t.counts,
        t.per_bin,
        &t.nominal,
        &phis,
        Some(&t.ratios),
    );
    println!(
        "  {inside} of {rows} rows inside their interval; about {:.1} outside expected by chance at 95 %",
        0.05 * rows as f64
    );
}

/// Per band, the median over bins of `a − b`.
fn band_median_diff(a: &[f64], b: &[f64], band: &Band) -> f64 {
    let mut d: Vec<f64> = a[band.lo..band.hi]
        .iter()
        .zip(&b[band.lo..band.hi])
        .map(|(x, y)| x - y)
        .collect();
    median_of(&mut d)
}

fn print_location(screened: &Location, unscreened: Option<&Location>, bands: &[Band]) {
    println!(
        "\n  ── 6. Location three ways, dB re the bin's mean (0 = the exponential law's value): median ÷ ln 2 and the log-domain mode (Towsey's 100-bin histogram of 10 lg P, 5-point smoothing) ──"
    );
    match unscreened {
        Some(u) => {
            println!(
                "  {:<14} {:>14} {:>9}   {:>14} {:>9}   {:>36}",
                "band (Hz)",
                "screened med",
                "mode",
                "unscreened med",
                "mode",
                "screen's effect, dB: mean · med · mode"
            );
            for band in bands {
                println!(
                    "  {:<14} {:>14.2} {:>9.2}   {:>14.2} {:>9.2}   {:>14.2} {:>9.2} {:>9.2}",
                    band.label,
                    band_median_diff(&screened.median_db, &screened.mean_db, band),
                    band_median_diff(&screened.mode_db, &screened.mean_db, band),
                    band_median_diff(&u.median_db, &u.mean_db, band),
                    band_median_diff(&u.mode_db, &u.mean_db, band),
                    band_median_diff(&screened.mean_db, &u.mean_db, band),
                    band_median_diff(&screened.median_db, &u.median_db, band),
                    band_median_diff(&screened.mode_db, &u.mode_db, band)
                );
            }
        }
        None => {
            println!("  {:<14} {:>9} {:>9}", "band (Hz)", "med", "mode");
            for band in bands {
                println!(
                    "  {:<14} {:>9.2} {:>9.2}",
                    band.label,
                    band_median_diff(&screened.median_db, &screened.mean_db, band),
                    band_median_diff(&screened.mode_db, &screened.mean_db, band)
                );
            }
        }
    }
}

fn print_summary(
    nu_median: f64,
    k_median: usize,
    wander: Option<&Wander>,
    tail: &Tail,
    bands: &[Band],
) {
    let lo = bands.iter().find(|b| !b.rumble).map_or(1, |b| b.lo);
    let m = wander.map_or(f64::NAN, |w| {
        let mut v = w.m[lo..].to_vec();
        median_of(&mut v)
    });
    let p3 = TAIL_P
        .iter()
        .position(|&p| p == 0.001)
        .unwrap_or(TAIL_P.len() - 1);
    println!(
        "\n  summary: ν/2 analytic for the median block ({k_median} windows) {nu_median:.3}; measured M median {m:.3}; 10⁻³ multiplier per band (q₀.₉₉₉ ÷ 6.908): {}",
        bands
            .iter()
            .zip(&tail.ratios)
            .map(|(b, r)| format!(
                "{} {}",
                b.label,
                r[p3].map_or("—".to_string(), |v| format!("{v:.3}"))
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
}

/// Per-bin statistics to `<dir>/distribution_<label>.csv`.
#[allow(clippy::too_many_arguments)]
fn write_csv(
    dir: &Path,
    label: &str,
    bin_hz: f64,
    bands: &[Band],
    law: &Law,
    wander: Option<&Wander>,
    kurt: &Kurtosis,
    tail: &Tail,
    loc: &Location,
    unscreened: Option<&Location>,
) -> Result<()> {
    let path = dir.join(format!("distribution_{label}.csv"));
    let mut w =
        BufWriter::new(File::create(&path).with_context(|| format!("create {}", path.display()))?);
    writeln!(
        w,
        "set,bin,hz,band,mean_power_db,median_ln2_re_mean_db,mode_re_mean_db,unscreened_mean_power_db,unscreened_median_ln2_re_mean_db,unscreened_mode_re_mean_db,sk,m,m_over_nu,level_sd_db,loo_values,loo_exceed_p0.5,loo_exceed_p0.1,loo_exceed_p0.01,loo_exceed_p0.001,tail_values,tail_exceed_p0.1,tail_exceed_p0.01,tail_exceed_p0.001"
    )?;
    let f = |v: f64| {
        if v.is_finite() {
            format!("{v:.6e}")
        } else {
            String::new()
        }
    };
    for bin in 1..BINS {
        let Some(band) = bands.iter().find(|b| (b.lo..b.hi).contains(&bin)) else {
            continue;
        };
        let (um, umed, umode) = match unscreened {
            Some(u) => (
                f(u.mean_db[bin]),
                f(u.median_db[bin] - u.mean_db[bin]),
                f(u.mode_db[bin] - u.mean_db[bin]),
            ),
            None => (String::new(), String::new(), String::new()),
        };
        writeln!(
            w,
            "{label},{bin},{:.3},{},{},{},{},{um},{umed},{umode},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            bin as f64 * bin_hz,
            band.label,
            f(loc.mean_db[bin]),
            f(loc.median_db[bin] - loc.mean_db[bin]),
            f(loc.mode_db[bin] - loc.mean_db[bin]),
            f(kurt.sk[bin]),
            wander.map_or(String::new(), |w| f(w.m[bin])),
            wander.map_or(String::new(), |w| f(w.ratio[bin])),
            wander.map_or(String::new(), |w| f(w.level_sd_db[bin])),
            law.per_bin,
            law.counts[0][bin],
            law.counts[1][bin],
            law.counts[2][bin],
            law.counts[3][bin],
            tail.per_bin,
            tail.counts[0][bin],
            tail.counts[1][bin],
            tail.counts[2][bin]
        )?;
    }
    w.flush()?;
    println!("  per-bin statistics: {}", path.display());
    Ok(())
}

// ── The two inputs ────────────────────────────────────────────────────────────

fn report_set(
    set: &SetData,
    nulls: &mut HashMap<usize, NullRegion>,
    out: Option<&Path>,
) -> Result<()> {
    let bin_hz = f64::from(SAMPLE_RATE) / BASS_WINDOW_SIZE as f64;
    let bands = bands(bin_hz);
    println!("\n════ gates distribution: {} ════", set.label);
    println!("  root {}", set.root.display());
    let sig: Vec<f64> = set.sigmas.iter().map(|s| f64::from(s.1)).collect();
    let sources = [
        SigmaFrom::CommandLine,
        SigmaFrom::Logged,
        SigmaFrom::Candidates,
        SigmaFrom::GateDefault,
    ]
    .iter()
    .filter_map(|&from| {
        let n = set.sigmas.iter().filter(|s| s.0 == from).count();
        (n > 0).then(|| format!("{} for {n}", from.label()))
    })
    .collect::<Vec<_>>()
    .join(", ");
    println!(
        "  σ (the screen's silence threshold): median {:.4e}, range {:.4e} … {:.4e}; from {sources}",
        median_of(&mut sig.clone()),
        quantile_of(&sig, 0.0),
        quantile_of(&sig, 1.0)
    );
    let with_preroll = set.all.iter().filter(|b| !b.is_empty()).count();
    let n_all: usize = set.all.iter().map(Vec::len).sum();
    let n_screened: usize = set.screened.iter().map(Vec::len).sum();
    let mut per_block: Vec<f64> = set
        .screened
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| b.len() as f64)
        .collect();
    println!(
        "  {} captures found, {} skipped, {with_preroll} with pre-roll; {n_all} pre-roll windows, {n_screened} screened silent; screened windows per capture: median {}, range {} … {}",
        set.found,
        set.skipped.len(),
        median_of(&mut per_block.clone()),
        quantile_of(&per_block, 0.0),
        quantile_of(&per_block, 1.0)
    );
    println!(
        "  left out as within {ONSET_GUARD_HOPS} hops of the −20 dB onset, which precedes the kept record: {} windows, {} of them screened silent",
        set.after_onset.0, set.after_onset.1
    );
    for (name, why) in set.skipped.iter().take(5) {
        println!("    skipped {name}: {why}");
    }
    if set.skipped.len() > 5 {
        println!("    … and {} more", set.skipped.len() - 5);
    }
    print_bands(&bands, bin_hz);
    let screened = Pool {
        power: &set.power,
        blocks: set
            .screened
            .iter()
            .filter(|b| !b.is_empty())
            .cloned()
            .collect(),
    };
    let unscreened = Pool {
        power: &set.power,
        blocks: set.all.iter().filter(|b| !b.is_empty()).cloned().collect(),
    };
    if screened.frames() == 0 {
        println!("  no screened windows: nothing to measure");
        return Ok(());
    }
    let mut cache = SpectrumCache::default();
    let means = bin_means(&screened);
    let law = within_block(&screened, &mut cache);
    print_law(&law, &bands);
    let wander = wander(&screened);
    let by_length = wander_by_length(&screened, &bands);
    print_wander(wander.as_ref(), &by_length, &bands);
    print_finite_variance(&screened, &means, &bands, &mut cache, nulls);
    let kurt = kurtosis(&screened);
    print_kurtosis(&kurt, &bands);
    let tail = tail(&screened, &means, &bands, &mut cache);
    print_tail(&tail, &bands);
    let loc_s = location(&screened);
    let loc_u = location(&unscreened);
    print_location(&loc_s, Some(&loc_u), &bands);
    let k_median = median_of(&mut per_block) as usize;
    print_summary(
        f64::from(analytic_nu_half(k_median)),
        k_median,
        wander.as_ref(),
        &tail,
        &bands,
    );
    if let Some(dir) = out {
        write_csv(
            dir,
            &set.label,
            bin_hz,
            &bands,
            &law,
            wander.as_ref(),
            &kurt,
            &tail,
            &loc_s,
            Some(&loc_u),
        )?;
    }
    Ok(())
}

/// The calibration-length check: per bin, the level from consecutive cuts of
/// `seconds` against the whole record's, as the standard deviation over the cuts
/// of `10 lg(level_cut ÷ level_whole)`, the median over each band's bins. Returns
/// per length: (seconds, cuts, windows a cut, the stationary spread
/// `4.343·√ψ′(ν/2)`, per band the median spread); lengths with fewer than two cuts
/// are left out.
fn calibration_check(
    rec: &RecordingData,
    bands: &[Band],
) -> Vec<(f64, usize, usize, f64, Vec<f64>)> {
    let whole = Pool {
        power: &rec.power,
        blocks: recording_blocks(rec, None),
    };
    let whole_mean = bin_means(&whole);
    let mut out = Vec::new();
    for &t in &CALIBRATION_S {
        let cuts = recording_blocks(rec, Some(t));
        if cuts.len() < 2 {
            continue;
        }
        let windows = cuts.iter().map(Vec::len).min().unwrap_or(0);
        let nu = nu_half_at(&geometry(&cuts[0]));
        let mut db: Vec<Vec<f64>> = vec![Vec::new(); BINS];
        for cut in &cuts {
            let m = bin_means(&Pool {
                power: &rec.power,
                blocks: vec![cut.clone()],
            });
            for (bin, (&a, &b)) in m.iter().zip(&whole_mean).enumerate().skip(1) {
                if a > 0.0 && b > 0.0 {
                    db[bin].push(10.0 * (a / b).log10());
                }
            }
        }
        let sd: Vec<f64> = db
            .iter()
            .map(|v| {
                if v.len() < 2 {
                    return f64::NAN;
                }
                let mean = v.iter().sum::<f64>() / v.len() as f64;
                (v.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (v.len() - 1) as f64).sqrt()
            })
            .collect();
        let per_band = bands
            .iter()
            .map(|b| {
                let mut v = sd[b.lo..b.hi].to_vec();
                median_of(&mut v)
            })
            .collect();
        out.push((
            t,
            cuts.len(),
            windows,
            DB_PER_NEPER * stats::trigamma(nu).sqrt(),
            per_band,
        ));
    }
    out
}

fn report_recording(
    rec: &RecordingData,
    nulls: &mut HashMap<usize, NullRegion>,
    out: Option<&Path>,
) -> Result<()> {
    let bin_hz = rec.fs / BASS_WINDOW_SIZE as f64;
    let bands = bands(bin_hz);
    println!("\n════ gates distribution: recording ════");
    println!(
        "  {}; {:.1} s at {:.0} Hz ({}); label {}; {} windows, every one (a recording carries no σ, so no screen)",
        rec.dir.display(),
        rec.samples as f64 / rec.fs,
        rec.fs,
        if rec.rate_logged {
            "recording.json"
        } else {
            "no recording.json: assumed"
        },
        rec.label.as_deref().unwrap_or("—"),
        rec.frames
    );
    print_bands(&bands, bin_hz);
    if rec.frames == 0 {
        println!("  shorter than one window: nothing to measure");
        return Ok(());
    }
    let mut cache = SpectrumCache::default();
    let mut lengths = Vec::new();
    for seconds in RECORDING_BLOCKS_S
        .iter()
        .map(|&s| Some(s))
        .chain(std::iter::once(None))
    {
        let pool = Pool {
            power: &rec.power,
            blocks: recording_blocks(rec, seconds),
        };
        println!(
            "\n  ════ blocks of {}: {} blocks, {} windows ════",
            seconds.map_or("the whole record".to_string(), |s| format!("{s} s")),
            pool.blocks.len(),
            pool.frames()
        );
        if pool.frames() == 0 {
            println!("  no complete block");
            continue;
        }
        let law = within_block(&pool, &mut cache);
        print_law(&law, &bands);
        let w = wander(&pool);
        print_wander(w.as_ref(), &[], &bands);
        let nu = pool
            .blocks
            .iter()
            .map(|b| nu_half_at(&geometry(b)))
            .sum::<f64>()
            / pool.blocks.len() as f64;
        lengths.push((seconds, nu, law, w));
    }
    let whole = Pool {
        power: &rec.power,
        blocks: recording_blocks(rec, None),
    };
    println!("\n  ════ the whole record, pooled ════");
    let means = bin_means(&whole);
    print_finite_variance(&whole, &means, &bands, &mut cache, nulls);
    let kurt = kurtosis(&whole);
    print_kurtosis(&kurt, &bands);
    let tail = tail(&whole, &means, &bands, &mut cache);
    print_tail(&tail, &bands);
    let loc = location(&whole);
    print_location(&loc, None, &bands);

    println!(
        "\n  ── Calibration length: per bin, the level from consecutive cuts against the whole record's; the SD over cuts of 10 lg(cut ÷ whole), median over each band's bins, against a stationary level's 4.343·√ψ′(ν/2) ──"
    );
    let check = calibration_check(rec, &bands);
    if check.is_empty() {
        println!("  the record holds fewer than two cuts of every length");
    } else {
        print!("  {:<14}", "band (Hz)");
        for (t, cuts, windows, expect, _) in &check {
            print!(
                " {:>22}",
                format!("{t:.0} s ×{cuts} ({windows} w) {expect:.2}")
            );
        }
        println!();
        for (bi, band) in bands.iter().enumerate() {
            print!("  {:<14}", band.label);
            for (_, _, _, _, per_band) in &check {
                print!(" {:>22.2}", per_band[bi]);
            }
            println!();
        }
    }
    let lo = bands.iter().find(|b| !b.rumble).map_or(1, |b| b.lo);
    let p3 = TAIL_P.len() - 1;
    println!(
        "\n  summary: ν/2 analytic and measured M (median over bins from 43 Hz) per block length: {}; 10⁻³ multiplier per band (q₀.₉₉₉ ÷ 6.908): {}",
        lengths
            .iter()
            .map(|(seconds, nu, _, w)| {
                let m = w
                    .as_ref()
                    .map_or(f64::NAN, |w| median_of(&mut w.m[lo..].to_vec()));
                format!(
                    "{} {nu:.3} / {m:.3}",
                    seconds.map_or("whole".to_string(), |s| format!("{s} s"))
                )
            })
            .collect::<Vec<_>>()
            .join(", "),
        bands
            .iter()
            .zip(&tail.ratios)
            .map(|(b, r)| format!(
                "{} {}",
                b.label,
                r[p3].map_or("—".to_string(), |v| format!("{v:.3}"))
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // The CSV carries the shortest blocks' law and wander: the whole record is
    // one block and has no wander to read.
    if let (Some(dir), Some((_, _, law, w))) = (out, lengths.first()) {
        write_csv(
            dir,
            "recording",
            bin_hz,
            &bands,
            law,
            w.as_ref(),
            &kurt,
            &tail,
            &loc,
            None,
        )?;
    }
    Ok(())
}

/// Stage B over every `--set LABEL ROOT SIGMA` (σ a number, or `-` for each
/// capture's own) and any `--recording DIR`, with a CSV of per-bin statistics per
/// input in `out`.
pub(super) fn run(sets: &[String], recording: Option<&Path>, out: Option<&Path>) -> Result<()> {
    if sets.is_empty() && recording.is_none() {
        bail!("give at least one --set LABEL ROOT SIGMA, or a --recording DIR");
    }
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let mut nulls = HashMap::new();
    for chunk in sets.chunks(3) {
        let [label, root, sigma] = chunk else {
            bail!("--set takes LABEL ROOT SIGMA");
        };
        let sigma = match sigma.as_str() {
            "-" => None,
            s => Some(
                s.parse::<f32>()
                    .with_context(|| format!("σ for {label}: {s} is neither a number nor -"))?,
            ),
        };
        let set = load_set(label, Path::new(root), sigma)?;
        report_set(&set, &mut nulls, out)?;
    }
    if let Some(dir) = recording {
        let rec = load_recording(dir)?;
        report_recording(&rec, &mut nulls, out)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One `Exp(1)` draw: half the sum of two squared standard normals.
    fn exp1(g: &mut Gaussian) -> f64 {
        let (a, b) = (f64::from(g.next()), f64::from(g.next()));
        0.5 * (a * a + b * b)
    }

    /// White Gaussian noise in `blocks` blocks of `per_block` contiguous windows,
    /// as a power table and the pool's blocks.
    fn white_blocks(blocks: usize, per_block: usize, seed: u64) -> (Vec<f32>, Vec<Vec<Frame>>) {
        let mut g = Gaussian::new(seed);
        let mut spectra = Spectra::new();
        let mut power = Vec::new();
        let mut out = Vec::new();
        for _ in 0..blocks {
            let x: Vec<f32> = (0..BASS_WINDOW_SIZE + (per_block - 1) * HOP_SIZE)
                .map(|_| 1e-3 * g.next())
                .collect();
            let mut block = Vec::new();
            for h in 0..per_block {
                block.push(Frame {
                    hop: h,
                    row: power.len() / BINS,
                });
                spectra.push(
                    &x[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE],
                    &mut power,
                );
            }
            out.push(block);
        }
        (power, out)
    }

    #[test]
    fn the_new_laws_reproduce_known_values() {
        assert!((stats::normal_quantile(0.975) - 1.959_964).abs() < 1e-6);
        assert!((stats::normal_cdf(1.0) - 0.841_344_746).abs() < 1e-9);
        assert!((stats::trigamma(1.0) - std::f64::consts::PI.powi(2) / 6.0).abs() < 1e-9);
        assert!((stats::trigamma(0.5) - std::f64::consts::PI.powi(2) / 2.0).abs() < 1e-9);
    }

    /// The frame count is Percival & Walden's EDOF/2 wherever the lab already
    /// computes it, and the 2 s calibration's 79 frames give ν/2 = 20.84 and a
    /// stationary level spread of 0.96 dB, the ±1 dB the record expects at 2 s.
    #[test]
    fn frame_counts_match_the_analytic_hann_value() {
        for k in 1..=100 {
            let hops: Vec<usize> = (0..k).collect();
            let ratio = nu_half_at(&hops) / f64::from(analytic_nu_half(k));
            assert!((ratio - 1.0).abs() < 1e-5, "{k}: {ratio}");
        }
        let hops: Vec<usize> = (0..79).collect();
        assert!((nu_half_at(&hops) - 20.836).abs() < 1e-3);
        let spread = DB_PER_NEPER * stats::trigamma(nu_half_at(&hops)).sqrt();
        assert!((spread - 0.963).abs() < 2e-3, "{spread}");
        // SK's count exceeds ν/2: 32 contiguous frames give 11.96 against 8.63.
        let hops: Vec<usize> = (0..32).collect();
        assert!((sk_frames_at(&hops) - 11.955).abs() < 1e-2);
    }

    /// Independent frames reduce the law to the F(2, 2n) tail and to n·Beta(1, n − 1);
    /// 32 overlapping frames reproduce numpy's eigenvalues of the same 32 × 32
    /// quadratic forms, far from the F law's p.
    #[test]
    fn the_ratio_law_is_exact() {
        let indep: Vec<usize> = (0..6).map(|i| i * DISJOINT_HOPS).collect();
        let spec = FrameSpectrum::of(&indep);
        let n = 5.0;
        for p in LAW_P {
            let t = stats::f_2_2m_threshold(n, p);
            let e = ratio_exceedance(&spec, 2, &spec.lam, 1.0 + t / n, t / n);
            assert!((e / p - 1.0).abs() < 1e-9, "{p}: {e}");
        }
        let x = 3.0;
        let e = ratio_exceedance(&spec, 0, &spec.lam, 1.0, x / 6.0);
        assert!((e - (1.0 - x / 6.0f64).powi(5)).abs() < 1e-12, "{e}");

        let contiguous: Vec<usize> = (0..32).collect();
        let spec = FrameSpectrum::of(&contiguous);
        let m = f64::from(analytic_nu_half(31));
        let rate = |p: f64| {
            let t = stats::f_2_2m_threshold(m, p);
            (0..32)
                .map(|k| ratio_exceedance(&spec, k, &spec.lam, 1.0 + t / 31.0, t / 31.0))
                .sum::<f64>()
                / 32.0
        };
        for (p, numpy) in [
            (0.5, 5.091_477_18e-1),
            (0.1, 7.154_716_52e-2),
            (0.01, 1.131_419_82e-3),
            (0.001, 7.709_217_56e-6),
        ] {
            let r = rate(p);
            assert!((r / numpy - 1.0).abs() < 1e-5, "{p}: {r} against {numpy}");
        }
    }

    #[test]
    fn the_circulant_form_matches_the_exact_one() {
        let k = 160;
        let hops: Vec<usize> = (0..k).collect();
        let circulant = FrameSpectrum::of(&hops);
        assert!(circulant.v2.is_none());
        let mut a = vec![0.0; k * k];
        for i in 0..k {
            for j in 0..k {
                a[i * k + j] = rho(i.abs_diff(j));
            }
        }
        let (lam, vecs) = jacobi(a, k);
        let exact = FrameSpectrum {
            lam,
            v2: Some(vecs.iter().map(|x| x * x).collect()),
        };
        let others = (k - 1) as f64;
        let t = stats::f_2_2m_threshold(f64::from(analytic_nu_half(k - 1)), 0.1);
        let (c, s) = (1.0 + t / others, t / others);
        let e_exact = (0..k)
            .map(|w| ratio_exceedance(&exact, w, &exact.lam, c, s))
            .sum::<f64>()
            / k as f64;
        let e_circulant = ratio_exceedance(&circulant, 0, &circulant.lam, c, s);
        assert!(
            (e_circulant / e_exact - 1.0).abs() < 0.01,
            "{e_circulant} against {e_exact}"
        );
    }

    /// 2000 windows of white noise in 50 blocks of 40: the leave-one-out rate at
    /// the F(2, 2M) point of p = 0.1 lands on the exact law (numpy: 0.0775), not
    /// on 0.1; the kurtosis bands hold 95 % at SK's own count and about 98 % at ν/2.
    #[test]
    fn white_noise_meets_the_exact_law_and_the_kurtosis_bands() {
        let (power, blocks) = white_blocks(50, 40, 20_261_009);
        let pool = Pool {
            power: &power,
            blocks,
        };
        let lo = bands(f64::from(SAMPLE_RATE) / BASS_WINDOW_SIZE as f64)[1].lo;
        let law = within_block(&pool, &mut SpectrumCache::default());
        assert!(
            (law.nominal[1] / 7.747_960_27e-2 - 1.0).abs() < 1e-5,
            "{}",
            law.nominal[1]
        );
        let k: u64 = law.counts[1][lo..].iter().sum();
        let rate = k as f64 / (law.per_bin * (BINS - lo) as u64) as f64;
        assert!(
            (rate / law.nominal[1] - 1.0).abs() < 0.02,
            "{rate} against {}",
            law.nominal[1]
        );

        let kurt = kurtosis(&pool);
        let sk = &kurt.sk[lo..];
        let share =
            |half: f64| sk.iter().filter(|v| v.abs() <= half).count() as f64 / sk.len() as f64;
        let inside = share(kurt.band);
        let outside = 1.0 - share(2.0 * kurt.band);
        assert!((0.93..=0.97).contains(&inside), "inside {inside}");
        assert!(outside < 0.005, "outside {outside}");
        let at_nu = share(kurt.band * (kurt.frames_sk / kurt.frames_nu).sqrt());
        assert!(at_nu > 0.97, "{at_nu}");
    }

    /// On exponential powers each statistic leaves its region about 5 % of the
    /// time; a symmetric Pareto sample of tail index 1.5 (infinite variance) leaves
    /// C(n, 1)'s region almost always.
    #[test]
    fn the_finite_variance_test_holds_its_size_and_rejects_infinite_variance() {
        let n = MIN_FV_VALUES;
        let region = finite_variance_null(n);
        let outside = |v: f64, (lo, hi): (f64, f64)| v < lo || v > hi;
        let mut g = Gaussian::new(7);
        let mut c = Vec::new();
        let trials = 300;
        let (mut size_a, mut size_c, mut power_c) = (0, 0, 0);
        for _ in 0..trials {
            let x: Vec<f64> = (0..n).map(|_| exp1(&mut g) - 1.0).collect();
            let (a1, cn) = ecem(&x, &mut c);
            size_a += usize::from(outside(a1, region.a1));
            size_c += usize::from(outside(cn, region.cn));
            let y: Vec<f64> = (0..n)
                .map(|_| {
                    let magnitude = (exp1(&mut g) / 1.5).exp() - 1.0;
                    if g.next() < 0.0 {
                        -magnitude
                    } else {
                        magnitude
                    }
                })
                .collect();
            power_c += usize::from(outside(ecem(&y, &mut c).1, region.cn));
        }
        let rate = |k: usize| k as f64 / trials as f64;
        assert!(
            (0.01..=0.10).contains(&rate(size_a)),
            "A(1) size {}",
            rate(size_a)
        );
        assert!(
            (0.01..=0.10).contains(&rate(size_c)),
            "C(n,1) size {}",
            rate(size_c)
        );
        assert!(rate(power_c) > 0.9, "C(n,1) power {}", rate(power_c));
    }

    /// `C(n, 1)`'s centre below 1 when the mean comes from the sampled windows:
    /// 0.99112 for one 20 s record (numpy, the exact Toeplitz eigenvectors; the
    /// circulant form here), and T1's 100 blocks of 32 against a simulation of
    /// their noise (0.9982 ± 0.0016).
    #[test]
    fn the_finite_variance_centre_is_exact() {
        let record = Pool {
            power: &[],
            blocks: vec![(0..854).map(|h| Frame { hop: h, row: h }).collect()],
        };
        let c = fv_centre(&record, &mut SpectrumCache::default());
        assert!((c - 0.991_12).abs() < 5e-4, "{c}");
        let t1 = Pool {
            power: &[],
            blocks: (0..100)
                .map(|b| {
                    (0..32)
                        .map(|h| Frame {
                            hop: h,
                            row: b * 32 + h,
                        })
                        .collect()
                })
                .collect(),
        };
        let c = fv_centre(&t1, &mut SpectrumCache::default());
        assert!((c - 0.9982).abs() < 3e-3, "{c}");
    }

    /// A screened block with gaps: the frames' indices, not their hops, address
    /// the block's eigenvectors.
    #[test]
    fn a_block_with_gaps_has_a_centre_and_a_law() {
        let hops = [0usize, 1, 3, 9, 10, 18];
        let block: Vec<Frame> = hops
            .iter()
            .enumerate()
            .map(|(row, &hop)| Frame { hop, row })
            .collect();
        assert_eq!(spaced(&block), vec![0, 3, 5]);
        let pool = Pool {
            power: &[],
            blocks: vec![block; 40],
        };
        let c = fv_centre(&pool, &mut SpectrumCache::default());
        assert!(c > 0.95 && c < 1.0, "{c}");
        let spec = FrameSpectrum::of(&hops);
        let t = stats::f_2_2m_threshold(f64::from(analytic_nu_half(5)), 0.1);
        for k in 0..hops.len() {
            let e = ratio_exceedance(&spec, k, &spec.lam, 1.0 + t / 5.0, t / 5.0);
            assert!(e > 0.0 && e < 0.1, "{k}: {e}");
        }
    }

    /// The histogram mode of an exponential power's dB values sits on the mean
    /// power (the BA-1 toy: −0.10 dB at 4140 frames).
    #[test]
    fn the_log_mode_estimates_the_mean_power() {
        let mut g = Gaussian::new(11);
        let mut modes: Vec<f64> = (0..200)
            .map(|_| {
                let db: Vec<f64> = (0..4140).map(|_| 10.0 * exp1(&mut g).log10()).collect();
                log_mode(&db)
            })
            .collect();
        let m = median_of(&mut modes);
        assert!(m.abs() < 0.3, "{m}");
    }

    /// A 20 s AWGN recording through every block length, the pooled quantities
    /// and the calibration-length check, written beside the run's other output
    /// and removed after.
    #[test]
    fn a_synthetic_recording_runs_through_every_block_length() {
        let run = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../docs/old/quasi-periodic-research/runs/QP-03/out/distribution");
        let parent = if run.is_dir() {
            run
        } else {
            std::env::temp_dir()
        };
        let dir = parent.join(format!("test_recording_{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create the test recording's directory");
        let mut g = Gaussian::new(3);
        let audio: Vec<f32> = (0..20 * SAMPLE_RATE as usize)
            .map(|_| 1e-3 * g.next())
            .collect();
        raw::write(&dir.join("audio.raw"), &audio).expect("write audio.raw");
        std::fs::write(
            dir.join("recording.json"),
            r#"{"sample_rate_hz": 44100, "label": "synthetic AWGN"}"#,
        )
        .expect("write recording.json");
        let rec = load_recording(&dir);
        let outcome = rec
            .as_ref()
            .map_err(|e| anyhow!("{e}"))
            .and_then(|r| report_recording(r, &mut HashMap::new(), None));
        let check = rec
            .as_ref()
            .map(|r| calibration_check(r, &bands(r.fs / BASS_WINDOW_SIZE as f64)));
        std::fs::remove_dir_all(&dir).expect("remove the test recording");
        outcome.expect("the recording's report");
        let check = check.expect("the calibration check");
        assert_eq!(
            check.iter().map(|c| c.0).collect::<Vec<_>>(),
            vec![2.0, 5.0, 10.0]
        );
        let (_, cuts, _, expect, per_band) = &check[0];
        assert_eq!(*cuts, 10);
        assert!((expect - 0.963).abs() < 0.01, "{expect}");
        for v in &per_band[5..] {
            assert!((v / expect - 1.0).abs() < 0.25, "{v} against {expect}");
        }
    }
}
