//! The peak-shape classifier of Zivanovic, Röbel & Rodet (2008) as the picker's
//! second stage: the shipped scalar admits a local maximum, and the classifier
//! keeps it only where its bandwidth, duration and frequency coherence lie
//! inside the sinusoidal class at the peak's own SNR.

use std::cell::Cell;
use std::f64::consts::{LN_2, PI, TAU};
use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use super::shipped::Shipped;
use super::{Arm, Availability, Block, CaptureCtx, Consumer, HopCtx, LOBE_HALF_BINS, SetCtx};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};

/// The analysis window's length `M`, in samples: the picker's frame.
const M: usize = BASS_WINDOW_SIZE;

/// The zero-padding factor `N/M` of the DFT the descriptors read. The paper asks
/// only that `N` keep the picket-fence effect out of a peak's shape, and used
/// 4096 points on a 1760-sample window.
// Ours, measured on a noise-free Hann tone and on the paper's worst case. At
// N/M = 1 the tone's leakage falls monotonically from bin to bin, so its peak
// has no minima and spans the spectrum (L = 4097, NBD → 0); at 2 a quarter-bin
// offset merges the main lobe into its side lobes (L = 178); at the paper's 2.33
// the worst case's L jumps between 9 and 12 with the carrier's place on the grid
// (NBD 0.103–0.136); at 4 every place gives L = 15 and NBD 0.1398–0.1401.
// report 0023
const PAD: usize = 4;

/// The DFT size `N`, in points.
const N: usize = PAD * M;

/// Eq. 11's amplitude-modulation depth `A_AM`.
const A_AM: f64 = 0.5;

/// Eq. 11's frequency-modulation depth `A_FM`, in radians of phase.
const A_FM: f64 = 10.0;

/// Eq. 11's vibrato period in analysis windows: `F_FM = 1/(4.2M)` cycles per
/// sample, and `F_AM = 2F_FM`.
const FM_PERIOD_WINDOWS: f64 = 4.2;

/// Table 2, Hanning: the phase `α_max` of Eq. 11 at which the sinusoidal peak
/// is widest.
const ALPHA_MAX: f64 = 0.75 * PI;

/// Table 2, Hanning: the phase `β_max` paired with [`ALPHA_MAX`].
const BETA_MAX: f64 = 0.5 * PI;

/// Eq. 13: the AM phase of the shortest signal pattern `x_dmin`, the envelope's
/// maximum at the window's centre.
const BETA_DMIN: f64 = -0.5 * PI;

/// Eq. 13: the AM phase of the longest signal pattern `x_dmax`, the envelope's
/// minimum at the window's centre.
const BETA_DMAX: f64 = 0.5 * PI;

// Table 3, Hanning: m_min = A0·SNR_p² + A1·SNR_p + A2 and
// m_max = B0·SNR_p² + B1·SNR_p + B2, SNR_p in dB, highest power first. The
// printed Σ a_i·SNR_p^i would put a₀ on the constant; Figure 3 (SNR_p = 10 dB)
// decides the order: r_dmax changes sign at u ≈ 0.16 of the window, which
// m_max = 1.98 gives (0.166) and the printed order's m_max = 84 does not (0.248),
// and r_dmin's crossings at 0.135 and 0.364 are m_min = 6.60's (0.137, 0.363).
// In this order the model's NDD_min and NDD_max at 10, 20 and 30 dB fall within
// 0.005 of the 2.5 % and 97.5 % points of the patterns in white noise; the
// printed order misses NDD_max at 10 dB by 0.026.
// report 0023

/// Table 3, Hanning: `a₀`, the coefficient of `SNR_p²` in `m_min`.
const A0: f64 = 0.0174;
/// Table 3, Hanning: `a₁`, the coefficient of `SNR_p` in `m_min`.
const A1: f64 = -0.5770;
/// Table 3, Hanning: `a₂`, the constant of `m_min`.
const A2: f64 = 10.6280;
/// Table 3, Hanning: `b₀`, the coefficient of `SNR_p²` in `m_max`.
const B0: f64 = -0.0006;
/// Table 3, Hanning: `b₁`, the coefficient of `SNR_p` in `m_max`.
const B1: f64 = 0.1211;
/// Table 3, Hanning: `b₂`, the constant of `m_max`.
const B2: f64 = 0.8279;

/// The carrier `F_o` of the stored patterns, in cycles per sample. The paper
/// finds the limits independent of it; a quarter of the sampling rate keeps the
/// modulated lobe far from DC and Nyquist.
const F_O: f64 = 0.25;

/// The lowest SNR_p the limits are evaluated at, in dB; a peak below it takes
/// its limits. The bottom of the range the paper fitted Table 3 on and checked
/// the model over (its Figure 5).
// Ours. Below 10 dB the polynomials carry no stated validity, and m_max falls to
// 1 at 1.4 dB, under which r_dmax loses the phase reversal the model rests on. It
// binds: 81 % of white noise's local maxima over Kay's 10⁻³ threshold at the true
// σ read under 10 dB and take its limits.
// report 0023
const SNR_P_FLOOR_DB: f64 = 10.0;

/// The highest SNR_p the limits are tabulated at, in dB: there the noise
/// patterns stand 10⁻⁴ of the signal's amplitude, and the limits equal the
/// noise-free ones to 2·10⁻⁴.
const SNR_P_TOP_DB: f64 = 80.0;

/// The step of the SNR_p grid the limits are tabulated on, in dB; a peak's
/// limits are linear between grid points.
const SNR_P_STEP_DB: f64 = 0.5;

/// Half-width in bins of the picker's frame over which the median magnitude
/// stands in for the noise floor a peak's SNR_p is read against.
// Ours: the paper's SNR_p is a user setting, and the driver hands a second stage
// no level.
// report 0023
const FLOOR_HALF_BINS: usize = 32;

/// Bins either side of the peak left out of that median: its Hann main lobe.
const FLOOR_GUARD_BINS: usize = 2;

/// The shipped scalar at each capture's recorded σ (arm A), then the peak-shape
/// classifier of Zivanovic, Röbel & Rodet (2008) on what it admits: the picker's
/// two stages, G1 only.
///
/// A local maximum of the 8192-point frame that clears arm A's threshold is
/// classified on the frame's DFT zero-padded to `N = 4M` points (`M` = 8192), by
/// the bins between the two minima that bound it there, both minima counted.
/// Three descriptors are read off that peak: the normalised bandwidth
/// `NBD = BW_rms/L` (Eqs. 1–2), the normalised duration `NDD = T_rms/M` from
/// the group delay and the magnitude's frequency derivative (Eqs. 5–8, from the
/// DFT under the time-weighted window), and the frequency coherence
/// `FCD = (N/2π)·min_k |Δω(k)|` from frequency reassignment (Eqs. 9–10, from the
/// DFT under the window's time derivative). Table 1's two-level tree then
/// calls the peak a side lobe where `FCD ≥ N/M`, and otherwise a sinusoid where
/// `NBD ≤ NBD_max` and `NDD_min ≤ NDD ≤ NDD_max`, noise where not; only a
/// sinusoid is kept.
///
/// The limits move with the peak's SNR_p, its maximum power over the noise
/// floor in dB, by the paper's model, tabulated once. `NBD_max` is the NBD of
/// the worst-case sinusoid (Eq. 11 at Table 2's phases) over the bins of its
/// main lobe that stand above a floor SNR_p dB under its maximum. `NDD_min` and
/// `NDD_max` are the NDDs of Eq. 13's signal patterns with Eq. 14's noise
/// patterns added: modulation indices from Table 3, and an amplitude `A` that
/// puts the noise pattern's spectral peak SNR_p dB under the signal pattern's.
/// Without noise they are 0.1398, 0.1349 and 0.1583 here. At the paper's own
/// geometry the same code gives `NDD_min` 0.1348 and `NDD_max` 0.1585 where the
/// paper's Figure 5 has 0.1345 and 0.1601, and a worst-case NBD of 0.103–0.136
/// as the carrier moves across a bin, which spans the figure's 0.1320.
///
/// The paper's SNR_p is set by its user; here it is read per peak, against the
/// noise's mean power per bin taken as `median²/ln 2` of the frame's magnitude
/// over ±32 bins of the peak with its own ±2 left out (the Rayleigh relation),
/// and held at 10 dB below the range the paper fitted. The zero padding, the
/// SNR_p reading and that floor are ours.
///
/// # References
///
/// - Zivanovic, M., Röbel, A. & Rodet, X. (2008). Adaptive threshold
///   determination for spectral peak classification. Computer Music Journal
///   32(2), 57–67. DOI: 10.1162/comj.2008.32.2.57. Eqs. 1–2, 5–11, 13–14;
///   Tables 1–3.
/// - Auger, F. & Flandrin, P. (1995). Improving the readability of
///   time-frequency and time-scale representations by the reassignment method.
///   IEEE Trans. Signal Processing 43(5), 1068–1089. The time-weighted and
///   time-derivative windows.
pub(super) struct PeakShape {
    first: Shipped,
    transforms: Transforms,
    /// Table 1's limits on the SNR_p grid, from [`SNR_P_FLOOR_DB`] up.
    limits: Vec<Limits>,
    /// The same without noise.
    noise_free: Limits,
    /// The hop whose transforms are held, which only that hop's peaks are
    /// classified on.
    held: Option<i64>,
    /// The verdicts on the peaks of the scored pre-roll, of the note within a
    /// measured partial's main lobe, and of the note elsewhere.
    preroll: Cell<Tally>,
    partial: Cell<Tally>,
    note: Cell<Tally>,
    /// Time spent on the transforms and the hops transformed; time spent
    /// classifying.
    transform_cost: Duration,
    hops: u64,
    classify_cost: Cell<Duration>,
}

/// Table 1's sine/noise limits at one SNR_p, as the paper's model moves them.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Limits {
    /// The right margin of the sinusoidal NBD distribution.
    nbd_max: f64,
    /// The left margin of the sinusoidal NDD distribution.
    ndd_min: f64,
    /// The right margin of the sinusoidal NDD distribution.
    ndd_max: f64,
}

/// A spectral peak's three descriptors.
#[derive(Clone, Copy, Debug)]
struct Descriptors {
    nbd: f64,
    ndd: f64,
    /// In bins of the padded DFT.
    fcd: f64,
}

/// What Table 1's two-level tree makes of a peak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    /// `FCD ≥ N/M`: a side lobe of a stronger component.
    SideLobe,
    /// `NBD > NBD_max`: noise, wider than a sinusoid at its SNR_p.
    Wide,
    /// NDD outside `[NDD_min, NDD_max]`: noise, shorter or longer than a
    /// sinusoid at its SNR_p.
    OffDuration,
    /// Inside both limits.
    Sinusoid,
}

/// One classified peak.
#[derive(Clone, Copy, Debug)]
struct Verdict {
    class: Class,
    descriptors: Descriptors,
    /// dB.
    snr_p: f64,
}

/// The classifier's verdicts over one population of hops.
#[derive(Clone, Copy, Debug, Default)]
struct Tally {
    asked: u64,
    side_lobe: u64,
    wide: u64,
    off_duration: u64,
    /// Peaks whose SNR_p fell below [`SNR_P_FLOOR_DB`].
    under_floor: u64,
    /// Σ SNR_p in dB, Σ NBD and Σ NDD, for the means.
    snr_p_sum: f64,
    nbd_sum: f64,
    ndd_sum: f64,
}

impl Tally {
    fn add(&mut self, verdict: &Verdict) {
        self.asked += 1;
        match verdict.class {
            Class::SideLobe => self.side_lobe += 1,
            Class::Wide => self.wide += 1,
            Class::OffDuration => self.off_duration += 1,
            Class::Sinusoid => {}
        }
        self.under_floor += u64::from(verdict.snr_p < SNR_P_FLOOR_DB);
        self.snr_p_sum += verdict.snr_p;
        self.nbd_sum += verdict.descriptors.nbd;
        self.ndd_sum += verdict.descriptors.ndd;
    }

    fn kept(&self) -> u64 {
        self.asked - self.side_lobe - self.wide - self.off_duration
    }
}

/// The zero-padded DFTs a peak's descriptors read, `m` samples padded to `n`
/// points: `X(k)` under the analysis window `w(n)`, the symmetric Hann
/// `½(1 − cos 2πn/(m − 1))` that `spectral::fft` applies; `X_t(k)` under the
/// time-weighted window `(n − (m − 1)/2)·w(n)`; and `X_dt(k)` under the
/// window's time derivative `w′(n) = (π/(m − 1))·sin 2πn/(m − 1)`, per sample.
struct Transforms {
    m: usize,
    plan: Arc<dyn RealToComplex<f64>>,
    window: Vec<f64>,
    ramp: Vec<f64>,
    derivative: Vec<f64>,
    time: Vec<f64>,
    scratch: Vec<Complex<f64>>,
    x: Vec<Complex<f64>>,
    xt: Vec<Complex<f64>>,
    xdt: Vec<Complex<f64>>,
    /// `|X(k)|²`.
    power: Vec<f64>,
}

impl Transforms {
    fn new(m: usize, n: usize) -> Self {
        let plan = RealFftPlanner::<f64>::new().plan_fft_forward(n);
        let scratch = plan.make_scratch_vec();
        let span = (m - 1) as f64;
        let window: Vec<f64> = (0..m)
            .map(|i| 0.5 * (1.0 - (TAU * i as f64 / span).cos()))
            .collect();
        let ramp = window
            .iter()
            .enumerate()
            .map(|(i, &w)| (i as f64 - span / 2.0) * w)
            .collect();
        let derivative = (0..m)
            .map(|i| PI / span * (TAU * i as f64 / span).sin())
            .collect();
        let bins = n / 2 + 1;
        Self {
            m,
            plan,
            window,
            ramp,
            derivative,
            time: vec![0.0; n],
            scratch,
            x: vec![Complex::new(0.0, 0.0); bins],
            xt: vec![Complex::new(0.0, 0.0); bins],
            xdt: vec![Complex::new(0.0, 0.0); bins],
            power: vec![0.0; bins],
        }
    }

    /// The three DFTs of the `m` samples `frame`.
    fn frame(&mut self, frame: &[f32]) {
        let frame = &frame[..self.m];
        let samples = || frame.iter().map(|&s| s as f64);
        let plan = &self.plan;
        let (time, scratch) = (&mut self.time, &mut self.scratch);
        dft(
            plan,
            time,
            scratch,
            &mut self.x,
            samples().zip(&self.window).map(|(s, &w)| s * w),
        );
        dft(
            plan,
            time,
            scratch,
            &mut self.xt,
            samples().zip(&self.ramp).map(|(s, &r)| s * r),
        );
        dft(
            plan,
            time,
            scratch,
            &mut self.xdt,
            samples().zip(&self.derivative).map(|(s, &d)| s * d),
        );
        self.refresh_power();
    }

    /// `X(k)` and `X_t(k)` of `m` samples that already carry their window, the
    /// form Eq. 14's untapered `r_dmax` needs: `X_t` is then the DFT of the
    /// samples weighted by `n − (m − 1)/2`. `X_dt` is left as it was.
    fn windowed(&mut self, samples: &[f64]) {
        let centre = (self.m - 1) as f64 / 2.0;
        let plan = &self.plan;
        let (time, scratch) = (&mut self.time, &mut self.scratch);
        dft(plan, time, scratch, &mut self.x, samples.iter().copied());
        let weighted = samples
            .iter()
            .enumerate()
            .map(|(i, &s)| (i as f64 - centre) * s);
        dft(plan, time, scratch, &mut self.xt, weighted);
        self.refresh_power();
    }

    /// `max_k |X(k)|` of `m` samples that already carry their window; the held
    /// transforms are left stale.
    fn peak_magnitude(&mut self, samples: &[f64]) -> f64 {
        dft(
            &self.plan,
            &mut self.time,
            &mut self.scratch,
            &mut self.x,
            samples.iter().copied(),
        );
        self.x.iter().map(|c| c.norm()).fold(0.0, f64::max)
    }

    fn refresh_power(&mut self) {
        for (p, c) in self.power.iter_mut().zip(&self.x) {
            *p = c.norm_sqr();
        }
    }
}

/// The forward DFT of the samples `signal` yields, zero-padded to the plan's
/// length, into `out`.
fn dft(
    plan: &Arc<dyn RealToComplex<f64>>,
    time: &mut [f64],
    scratch: &mut [Complex<f64>],
    out: &mut [Complex<f64>],
    signal: impl Iterator<Item = f64>,
) {
    // The transform uses its input as scratch, so the padding is rewritten
    // every time.
    time.fill(0.0);
    for (t, s) in time.iter_mut().zip(signal) {
        *t = s;
    }
    plan.process_with_scratch(time, out, scratch)
        .expect("buffers are sized to the plan");
}

/// The spectral peak at bin `k` of the power spectrum `power` as the paper
/// defines it, the bins between two neighbouring minima: from the local maximum
/// an ascent from `k` reaches, out to the first minimum on each side, both
/// minima counted in. Returns `(lo, top, hi)`.
// Counting the minima is the reading under which the paper's worst case at its
// own geometry stays near Table 1's NBD ≤ 0.13 (0.103–0.136 over the carrier's
// place on the grid); leaving them out gives 0.123–0.174.
fn peak_bounds(power: &[f64], k: usize) -> (usize, usize, usize) {
    let last = power.len() - 1;
    let mut top = k.min(last);
    while top < last && power[top + 1] > power[top] {
        top += 1;
    }
    while top > 0 && power[top - 1] > power[top] {
        top -= 1;
    }
    let mut lo = top;
    while lo > 0 && power[lo - 1] < power[lo] {
        lo -= 1;
    }
    let mut hi = top;
    while hi < last && power[hi + 1] < power[hi] {
        hi += 1;
    }
    (lo, top, hi)
}

/// The bin of the largest value of `power`.
fn argmax(power: &[f64]) -> usize {
    power
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map_or(0, |(k, _)| k)
}

/// Eqs. 1–2: the normalised bandwidth of the peak on bins `lo..=hi` of the
/// power spectrum `|X(k)|²`, `NBD = BW_rms/L`, with
/// `BW_rms = √(Σ(k − k̄)²|X(k)|² / Σ|X(k)|²)` and `k̄ = Σk|X(k)|² / Σ|X(k)|²` in
/// bins, and `L` the bins in the peak.
fn nbd(power: &[f64], lo: usize, hi: usize) -> f64 {
    let bins = &power[lo..=hi];
    let energy: f64 = bins.iter().sum();
    let k_bar = bins
        .iter()
        .enumerate()
        .map(|(k, &p)| k as f64 * p)
        .sum::<f64>()
        / energy;
    let bw_rms = (bins
        .iter()
        .enumerate()
        .map(|(k, &p)| (k as f64 - k_bar).powi(2) * p)
        .sum::<f64>()
        / energy)
        .sqrt();
    bw_rms / bins.len() as f64
}

/// Eqs. 5–8: the normalised duration of the peak on bins `lo..=hi` of an
/// `m`-sample window, `NDD = T_rms/M` with
/// `T_rms = √(Σ(A′(k)² + (g_d(k) + n̄)²)|X(k)|² / Σ|X(k)|²)` in samples, where
/// `g_d(k) = −Re(X_t X*/|X|²)` (Eq. 7), `A′(k) = −Im(X_t X*/|X|²)` (Eq. 8, whose
/// left side is printed `g_d`) and `n̄ = −Σg_d|X|² / Σ|X|²` (Eq. 6). With these
/// substituted, each term of the sum is `|X_t(k) − n̄·X(k)|²`, the form summed
/// here, which stays finite at a bin where `X(k)` vanishes.
fn ndd(x: &[Complex<f64>], xt: &[Complex<f64>], lo: usize, hi: usize, m: usize) -> f64 {
    let (x, xt) = (&x[lo..=hi], &xt[lo..=hi]);
    let energy: f64 = x.iter().map(|c| c.norm_sqr()).sum();
    let n_bar = x
        .iter()
        .zip(xt)
        .map(|(&a, &t)| (t * a.conj()).re)
        .sum::<f64>()
        / energy;
    let spread: f64 = x
        .iter()
        .zip(xt)
        .map(|(&a, &t)| (t - a * n_bar).norm_sqr())
        .sum();
    (spread / energy).sqrt() / m as f64
}

/// Eqs. 9–10: the frequency coherence of the peak on bins `lo..=hi` of an
/// `n`-point DFT, `FCD = (N/2π)·min_k |Δω(k)|` with `Δω(k) = Im(X_dt X*/|X|²)`
/// the offset in radians between bin `k`'s centre and its reassigned frequency;
/// in bins. A bin where `X(k)` vanishes has no reassigned frequency and is
/// passed over.
fn fcd(x: &[Complex<f64>], xdt: &[Complex<f64>], lo: usize, hi: usize, n: usize) -> f64 {
    let offset = x[lo..=hi]
        .iter()
        .zip(&xdt[lo..=hi])
        .filter_map(|(&a, &d)| {
            let p = a.norm_sqr();
            (p > 0.0).then(|| ((d * a.conj()).im / p).abs())
        })
        .fold(f64::INFINITY, f64::min);
    n as f64 / TAU * offset
}

/// Eq. 11 without noise, under the window: the worst-case sinusoid
/// `cos[2πF_o·n + A_FM·sin(2πF_FM·n + α)]·[1 + A_AM·cos(2πF_AM·n + β)]·w(n)`,
/// with `F_FM = 1/(4.2M)` and `F_AM = 2F_FM` in cycles per sample and `n` from 0
/// at the window's first sample.
fn worst_case(f_o: f64, alpha: f64, beta: f64, window: &[f64]) -> Vec<f64> {
    let f_fm = 1.0 / (FM_PERIOD_WINDOWS * window.len() as f64);
    let f_am = 2.0 * f_fm;
    window
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let n = i as f64;
            (TAU * f_o * n + A_FM * (TAU * f_fm * n + alpha).sin()).cos()
                * (1.0 + A_AM * (TAU * f_am * n + beta).cos())
                * w
        })
        .collect()
}

/// Eq. 13's signal pattern `x(n; β)·w(n)`, Eq. 12 without noise:
/// `cos(2πF_o·n)·[1 + A_AM·cos(2πF_AM·n + β)]·w(n)`.
fn signal_pattern(f_o: f64, beta: f64, window: &[f64]) -> Vec<f64> {
    let f_am = 2.0 / (FM_PERIOD_WINDOWS * window.len() as f64);
    window
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let n = i as f64;
            (TAU * f_o * n).cos() * (1.0 + A_AM * (TAU * f_am * n + beta).cos()) * w
        })
        .collect()
}

/// Eq. 14's `r_dmin` at unit amplitude, under the window:
/// `cos(2πF_o·n)·[1 + m_min·cos(4πn/M)]·w(n)`.
fn noise_pattern_min(f_o: f64, m_min: f64, window: &[f64]) -> Vec<f64> {
    let m = window.len() as f64;
    window
        .iter()
        .enumerate()
        .map(|(i, &w)| {
            let n = i as f64;
            (TAU * f_o * n).cos() * (1.0 + m_min * (2.0 * TAU * n / m).cos()) * w
        })
        .collect()
}

/// Eq. 14's `r_dmax` at unit amplitude, untapered over the window's `m`
/// samples: `−cos(2πF_o·n)·[1 − m_max·cos(2πn/M)]`.
fn noise_pattern_max(f_o: f64, m_max: f64, m: usize) -> Vec<f64> {
    (0..m)
        .map(|i| {
            let n = i as f64;
            -(TAU * f_o * n).cos() * (1.0 - m_max * (TAU * n / m as f64).cos())
        })
        .collect()
}

/// Table 3's `m_min` at `snr_p` dB.
fn m_min(snr_p: f64) -> f64 {
    A0 * snr_p * snr_p + A1 * snr_p + A2
}

/// Table 3's `m_max` at `snr_p` dB.
fn m_max(snr_p: f64) -> f64 {
    B0 * snr_p * snr_p + B1 * snr_p + B2
}

/// The NDD of the largest peak of `samples`, which already carry their window.
fn pattern_ndd(tf: &mut Transforms, samples: &[f64]) -> f64 {
    tf.windowed(samples);
    let (lo, _, hi) = peak_bounds(&tf.power, argmax(&tf.power));
    ndd(&tf.x, &tf.xt, lo, hi, tf.m)
}

/// The NDD of a signal pattern whose spectral peak is `peak` with the unit
/// noise pattern `noise` added at `snr_p` dB: the amplitude `A` puts the noise
/// pattern's spectral peak `snr_p` dB under the signal pattern's.
fn noisy_ndd(tf: &mut Transforms, signal: &[f64], peak: f64, noise: &[f64], snr_p: f64) -> f64 {
    let a = 10f64.powf(-snr_p / 20.0) * peak / tf.peak_magnitude(noise);
    let sum: Vec<f64> = signal.iter().zip(noise).map(|(&s, &r)| s + a * r).collect();
    pattern_ndd(tf, &sum)
}

/// Table 1's limits by the paper's model on the SNR_p grid from
/// [`SNR_P_FLOOR_DB`] to [`SNR_P_TOP_DB`], and without noise, on `tf`'s window
/// and padding.
fn tabulate(tf: &mut Transforms) -> (Vec<Limits>, Limits) {
    let window = tf.window.clone();
    // NBD_max: the worst case's spectrum, computed once.
    tf.windowed(&worst_case(F_O, ALPHA_MAX, BETA_MAX, &window));
    let lobe = tf.power.clone();
    let (lo, top, hi) = peak_bounds(&lobe, argmax(&lobe));
    // NDD_min and NDD_max: the two signal patterns.
    let x_dmin = signal_pattern(F_O, BETA_DMIN, &window);
    let x_dmax = signal_pattern(F_O, BETA_DMAX, &window);
    let peak_dmin = tf.peak_magnitude(&x_dmin);
    let peak_dmax = tf.peak_magnitude(&x_dmax);
    let noise_free = Limits {
        nbd_max: nbd(&lobe, lo, hi),
        ndd_min: pattern_ndd(tf, &x_dmin),
        ndd_max: pattern_ndd(tf, &x_dmax),
    };
    let steps = ((SNR_P_TOP_DB - SNR_P_FLOOR_DB) / SNR_P_STEP_DB).round() as usize;
    let limits = (0..=steps)
        .map(|i| {
            let snr_p = SNR_P_FLOOR_DB + i as f64 * SNR_P_STEP_DB;
            // The main lobe's bins above the floor SNR_p dB under its maximum.
            let floor = lobe[top] * 10f64.powf(-snr_p / 10.0);
            let (mut a, mut b) = (top, top);
            while a > lo && lobe[a - 1] >= floor {
                a -= 1;
            }
            while b < hi && lobe[b + 1] >= floor {
                b += 1;
            }
            let r_dmin = noise_pattern_min(F_O, m_min(snr_p), &window);
            let r_dmax = noise_pattern_max(F_O, m_max(snr_p), window.len());
            Limits {
                nbd_max: nbd(&lobe, a, b),
                ndd_min: noisy_ndd(tf, &x_dmin, peak_dmin, &r_dmin, snr_p),
                ndd_max: noisy_ndd(tf, &x_dmax, peak_dmax, &r_dmax, snr_p),
            }
        })
        .collect();
    (limits, noise_free)
}

/// A peak's SNR_p in dB: `peak_power`, its maximum `|X(k)|²` on the padded DFT,
/// over the noise's mean power per bin, taken as `median²/ln 2` of the picker's
/// magnitude `mag` over ±[`FLOOR_HALF_BINS`] bins of `bin` with ±[`FLOOR_GUARD_BINS`]
/// left out: the mean power of Rayleigh magnitudes with that median. Zero padding
/// leaves a bin's noise power unchanged, so the two spectra share units.
fn snr_p(peak_power: f64, mag: &[f32], bin: usize) -> f64 {
    let mut cells = [0.0f32; 2 * FLOOR_HALF_BINS];
    let mut count = 0;
    let lo = bin.saturating_sub(FLOOR_HALF_BINS).max(1);
    let hi = (bin + FLOOR_HALF_BINS).min(mag.len() - 1);
    for b in (lo..=hi).filter(|b| b.abs_diff(bin) > FLOOR_GUARD_BINS) {
        cells[count] = mag[b];
        count += 1;
    }
    if count == 0 {
        return f64::INFINITY;
    }
    let (_, median, _) = cells[..count].select_nth_unstable_by(count / 2, f32::total_cmp);
    let floor = (*median as f64).powi(2) / LN_2;
    if floor > 0.0 {
        10.0 * (peak_power / floor).log10()
    } else {
        f64::INFINITY
    }
}

/// Whether `bin` lies within the main lobe of one of the capture's measured
/// partials.
fn at_partial(hop: &HopCtx, bin: usize) -> bool {
    hop.capture.f_meas.iter().any(|&f| {
        let centre = (f / hop.hz_per_bin).round() as usize;
        bin.abs_diff(centre) <= LOBE_HALF_BINS
    })
}

impl PeakShape {
    /// The shipped scalar at each capture's recorded σ, then the classifier; its
    /// limits are tabulated here.
    pub(super) fn new() -> Self {
        let mut transforms = Transforms::new(M, N);
        let (limits, noise_free) = tabulate(&mut transforms);
        Self {
            first: Shipped::recorded(),
            transforms,
            limits,
            noise_free,
            held: None,
            preroll: Cell::new(Tally::default()),
            partial: Cell::new(Tally::default()),
            note: Cell::new(Tally::default()),
            transform_cost: Duration::ZERO,
            hops: 0,
            classify_cost: Cell::new(Duration::ZERO),
        }
    }

    /// The limits at `snr_p` dB: linear between grid points, held at the grid's
    /// ends.
    fn limits_at(&self, snr_p: f64) -> Limits {
        let last = self.limits.len() - 1;
        let pos = (snr_p.clamp(SNR_P_FLOOR_DB, SNR_P_TOP_DB) - SNR_P_FLOOR_DB) / SNR_P_STEP_DB;
        let i = (pos.floor() as usize).min(last);
        let j = (i + 1).min(last);
        let f = pos - i as f64;
        let (a, b) = (self.limits[i], self.limits[j]);
        Limits {
            nbd_max: a.nbd_max + f * (b.nbd_max - a.nbd_max),
            ndd_min: a.ndd_min + f * (b.ndd_min - a.ndd_min),
            ndd_max: a.ndd_max + f * (b.ndd_max - a.ndd_max),
        }
    }

    /// Table 1's tree on the local maximum at `bin` of the picker's magnitude
    /// `mag`, read on the held transforms.
    fn classify(&self, mag: &[f32], bin: usize) -> Verdict {
        let tf = &self.transforms;
        let (lo, top, hi) = peak_bounds(&tf.power, PAD * bin);
        let descriptors = Descriptors {
            nbd: nbd(&tf.power, lo, hi),
            ndd: ndd(&tf.x, &tf.xt, lo, hi, M),
            fcd: fcd(&tf.x, &tf.xdt, lo, hi, N),
        };
        let snr_p = snr_p(tf.power[top], mag, bin);
        let limits = self.limits_at(snr_p);
        let class = if descriptors.fcd >= PAD as f64 {
            Class::SideLobe
        } else if descriptors.nbd > limits.nbd_max {
            Class::Wide
        } else if !(limits.ndd_min..=limits.ndd_max).contains(&descriptors.ndd) {
            Class::OffDuration
        } else {
            Class::Sinusoid
        };
        Verdict {
            class,
            descriptors,
            snr_p,
        }
    }
}

impl Arm for PeakShape {
    fn name(&self) -> &str {
        "shape"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.first.prepare(set);
        self.held = None;
        self.preroll.set(Tally::default());
        self.partial.set(Tally::default());
        self.note.set(Tally::default());
        self.transform_cost = Duration::ZERO;
        self.hops = 0;
        self.classify_cost.set(Duration::ZERO);
        let at = |snr_p: f64| {
            let l = self.limits_at(snr_p);
            format!(
                "NBD ≤ {:.4}, {:.4} ≤ NDD ≤ {:.4} at {snr_p:.0} dB",
                l.nbd_max, l.ndd_min, l.ndd_max
            )
        };
        let free = self.noise_free;
        println!(
            "[{}] arm shape: the shipped scalar at {}, then Zivanovic, Röbel & Rodet's classifier on \
             the {N}-point DFT of each {M}-sample frame: a side lobe at FCD ≥ {PAD} bins; a sinusoid \
             within NBD ≤ {:.4}, {:.4} ≤ NDD ≤ {:.4} without noise, {}, {}, {}; SNR_p over the median \
             of ±{FLOOR_HALF_BINS} bins less ±{FLOOR_GUARD_BINS}, held at {SNR_P_FLOOR_DB:.0} dB below it",
            set.label,
            match set.sigma_override {
                Some(s) => format!("σ = {s:.2e}"),
                None => "each capture's recorded σ".to_string(),
            },
            free.nbd_max,
            free.ndd_min,
            free.ndd_max,
            at(10.0),
            at(20.0),
            at(30.0)
        );
    }

    fn begin_capture(&mut self, cap: &CaptureCtx) {
        self.first.begin_capture(cap);
        self.held = None;
    }

    fn observe(&mut self, hop: &HopCtx) {
        self.first.observe(hop);
    }

    fn finish(&mut self, set: &SetCtx) {
        self.first.finish(set);
        println!(
            "[{}] arm shape: Table 1's tree on the peaks the shipped scalar admitted",
            set.label
        );
        for (label, tally) in [
            ("pre-roll, scored", self.preroll.get()),
            ("note, at a partial", self.partial.get()),
            ("note, elsewhere", self.note.get()),
        ] {
            if tally.asked == 0 {
                println!("     {label}: no peak reached the classifier");
                continue;
            }
            let mean = |sum: f64| sum / tally.asked as f64;
            println!(
                "     {label}: {} peaks: {} side lobes, {} wider than NBD_max, {} outside the NDD \
                 limits, {} kept ({:.1} %); means SNR_p {:.1} dB, NBD {:.4}, NDD {:.4}; {} under \
                 {SNR_P_FLOOR_DB:.0} dB",
                tally.asked,
                tally.side_lobe,
                tally.wide,
                tally.off_duration,
                tally.kept(),
                100.0 * tally.kept() as f64 / tally.asked as f64,
                mean(tally.snr_p_sum),
                mean(tally.nbd_sum),
                mean(tally.ndd_sum),
                tally.under_floor
            );
        }
        if self.hops > 0 {
            let peaks = self.preroll.get().asked + self.partial.get().asked + self.note.get().asked;
            let hop_ms = HOP_SIZE as f64 / SAMPLE_RATE as f64 * 1.0e3;
            let per_hop = |d: Duration| d.as_secs_f64() * 1.0e6 / self.hops as f64;
            println!(
                "     cost: {:.0} µs per hop for the three {N}-point DFTs over {} hops, {:.1} µs per \
                 hop classifying ({:.1} µs per peak), against the {hop_ms:.1} ms hop",
                per_hop(self.transform_cost),
                self.hops,
                per_hop(self.classify_cost.get()),
                self.classify_cost.get().as_secs_f64() * 1.0e6 / peaks.max(1) as f64
            );
        }
    }

    fn decision_only(&self) -> bool {
        self.first.decision_only()
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        self.held = None;
        let first = self.first.g1_threshold(hop, out);
        if first == Availability::Available && hop.audio.len() == M {
            let start = Instant::now();
            self.transforms.frame(hop.audio);
            self.transform_cost += start.elapsed();
            self.hops += 1;
            self.held = Some(hop.hop);
        }
        first
    }

    fn g1_keep_peak(&self, hop: &HopCtx, bin: usize) -> bool {
        if self.held != Some(hop.hop) || bin == 0 || bin >= hop.mag.len() {
            return true;
        }
        let start = Instant::now();
        let verdict = self.classify(hop.mag, bin);
        let tally = match hop.block {
            Block::Note if at_partial(hop, bin) => &self.partial,
            Block::Note => &self.note,
            _ => &self.preroll,
        };
        let mut t = tally.get();
        t.add(&verdict);
        tally.set(t);
        self.classify_cost
            .set(self.classify_cost.get() + start.elapsed());
        verdict.class == Class::Sinusoid
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::stats;
    use crate::gates::truth::{Gaussian, tone_amplitude};
    use tuner_core::algorithms::spectral;

    /// The paper's geometry: a 40 ms window at 44 kS/s in a 4096-point DFT, and
    /// its 880 Hz carrier in cycles per sample.
    const PAPER_M: usize = 1760;
    const PAPER_N: usize = 4096;
    const PAPER_F_O: f64 = 880.0 / 44_000.0;

    const SIGMA: f32 = 1.0e-3;

    fn set(sigma: Option<f32>) -> SetCtx<'static> {
        SetCtx {
            label: "test",
            p_fa: 1.0e-3,
            sigma_override: sigma,
            nu_half_per_frame: None,
        }
    }

    fn capture() -> CaptureCtx<'static> {
        CaptureCtx {
            dir: "test",
            key: 48,
            register: 1,
            ns: &[1],
            f_meas: &[440.0],
            spacing_hz: 440.0,
            b: 0.0,
            sigma_rec: SIGMA,
        }
    }

    /// The picker's 8192-point frame of `audio`: its Hann spectrum and magnitudes.
    struct Frame {
        plan: Arc<dyn realfft::RealToComplex<f32>>,
        time: Vec<f32>,
        spectrum: Vec<Complex<f32>>,
        mag: Vec<f32>,
    }

    impl Frame {
        fn new() -> Self {
            Self {
                plan: RealFftPlanner::<f32>::new().plan_fft_forward(M),
                time: vec![0.0; M],
                spectrum: vec![Complex::new(0.0, 0.0); M / 2 + 1],
                mag: vec![0.0; M / 2],
            }
        }

        fn compute(&mut self, audio: &[f32]) {
            spectral::fft(audio, &mut self.time, &mut self.spectrum, &self.plan, M);
            spectral::magnitude_spectrum(&self.spectrum, M, &mut self.mag);
        }
    }

    fn hop<'a>(
        index: i64,
        block: Block,
        audio: &'a [f32],
        frame: &'a Frame,
        cap: &'a CaptureCtx<'a>,
    ) -> HopCtx<'a> {
        HopCtx {
            hop: index,
            t_since: 0.0,
            bucket: 0,
            is_silence: block != Block::Note,
            block,
            audio,
            spectrum: &frame.spectrum,
            mag: &frame.mag,
            hz_per_bin: SAMPLE_RATE as f32 / M as f32,
            sigma: SIGMA,
            truth_level: None,
            oracle: &[],
            capture: cap,
        }
    }

    /// A cosine at `bin` (fractional, of the `m`-point grid) and `phase`.
    fn tone(m: usize, bin: f64, phase: f64, amplitude: f32) -> Vec<f32> {
        (0..m)
            .map(|i| amplitude * (TAU * bin * i as f64 / m as f64 + phase).cos() as f32)
            .collect()
    }

    /// The descriptors of the peak around bin `k` of the held transforms.
    fn read(tf: &Transforms, k: usize, n: usize) -> (Descriptors, (usize, usize, usize)) {
        let (lo, top, hi) = peak_bounds(&tf.power, k);
        let d = Descriptors {
            nbd: nbd(&tf.power, lo, hi),
            ndd: ndd(&tf.x, &tf.xt, lo, hi, tf.m),
            fcd: fcd(&tf.x, &tf.xdt, lo, hi, n),
        };
        (d, (lo, top, hi))
    }

    /// The first side lobe above the main lobe that ends at `hi`: the peak the
    /// next ascent reaches.
    fn first_side_lobe(power: &[f64], hi: usize) -> usize {
        let mut k = hi;
        while power[k + 1] > power[k] {
            k += 1;
        }
        k
    }

    /// At the paper's own geometry the port gives its published limits: the NDD
    /// of Eq. 13's two signal patterns against Figure 5's legend (NDD_min
    /// 0.1345, NDD_max 0.1601), the worst case's NBD at Table 2's phases
    /// spanning the legend's NBD_max 0.1320 as the carrier moves across one bin
    /// (with the bounding minima left out of the peak it would reach past 0.15),
    /// and Table 1 on a stationary tone at the paper's carrier and on its first
    /// side lobe.
    #[test]
    fn the_port_gives_the_papers_limits_at_its_geometry() {
        let mut tf = Transforms::new(PAPER_M, PAPER_N);
        let window = tf.window.clone();
        for (beta, published) in [(BETA_DMIN, 0.1345), (BETA_DMAX, 0.1601)] {
            let got = pattern_ndd(&mut tf, &signal_pattern(PAPER_F_O, beta, &window));
            println!(
                "signal pattern β = {:+.1}π: NDD {got:.4}, published {published}",
                beta / PI
            );
            assert!(
                (got - published).abs() < 0.002,
                "β = {beta}: {got} vs {published}"
            );
        }
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        let mut inner_high = f64::NEG_INFINITY;
        let centre = (PAPER_F_O * PAPER_N as f64).round();
        for j in 0..20 {
            let f_o = (centre + j as f64 / 20.0) / PAPER_N as f64;
            tf.windowed(&worst_case(f_o, ALPHA_MAX, BETA_MAX, &window));
            let (lo, _, hi) = peak_bounds(&tf.power, argmax(&tf.power));
            let v = nbd(&tf.power, lo, hi);
            (low, high) = (low.min(v), high.max(v));
            inner_high = inner_high.max(nbd(&tf.power, lo + 1, hi - 1));
        }
        println!(
            "worst case at Table 2's phases: NBD {low:.4}–{high:.4} over the carrier's place, \
             up to {inner_high:.4} with the minima left out"
        );
        assert!(low < 0.1320 && 0.1320 < high, "{low}–{high}");
        assert!(inner_high > 0.15, "{inner_high}");

        tf.frame(&tone(PAPER_M, PAPER_F_O * PAPER_M as f64, 0.4, 1.0));
        let (d, (_, _, hi)) = read(&tf, argmax(&tf.power), PAPER_N);
        let side = first_side_lobe(&tf.power, hi);
        let (s, _) = read(&tf, side, PAPER_N);
        let n_over_m = PAPER_N as f64 / PAPER_M as f64;
        println!(
            "stationary tone: NBD {:.4}, NDD {:.4}, FCD {:.3}; its first side lobe FCD {:.2} (N/M = {n_over_m:.3})",
            d.nbd, d.ndd, d.fcd, s.fcd
        );
        assert!(d.nbd <= 0.13, "NBD {}", d.nbd);
        assert!((0.13..=0.16).contains(&d.ndd), "NDD {}", d.ndd);
        assert!(d.fcd < n_over_m, "FCD {}", d.fcd);
        assert!(s.fcd >= n_over_m, "side lobe FCD {}", s.fcd);
    }

    /// A search over Eq. 11's phases on a 0.05π grid at the paper's geometry
    /// finds the widest peak at Table 2's α_max = 0.75π and within one step of
    /// its β_max = 0.5π, and every phase pair's NDD inside Table 1's
    /// 0.13 ≤ NDD ≤ 0.16.
    #[test]
    fn table_2_phases_give_the_widest_peak() {
        let mut tf = Transforms::new(PAPER_M, PAPER_N);
        let window = tf.window.clone();
        let grid: Vec<f64> = (0..=40).map(|j| -PI + TAU * j as f64 / 40.0).collect();
        let mut widest = (f64::NEG_INFINITY, 0.0, 0.0);
        let (mut ndd_low, mut ndd_high) = (f64::INFINITY, f64::NEG_INFINITY);
        for &alpha in &grid {
            for &beta in &grid {
                tf.windowed(&worst_case(PAPER_F_O, alpha, beta, &window));
                let (lo, _, hi) = peak_bounds(&tf.power, argmax(&tf.power));
                let v = nbd(&tf.power, lo, hi);
                if v > widest.0 {
                    widest = (v, alpha, beta);
                }
                let d = ndd(&tf.x, &tf.xt, lo, hi, PAPER_M);
                (ndd_low, ndd_high) = (ndd_low.min(d), ndd_high.max(d));
            }
        }
        println!(
            "widest: NBD {:.4} at α = {:.2}π, β = {:.2}π; NDD over the grid {ndd_low:.4}–{ndd_high:.4}",
            widest.0,
            widest.1 / PI,
            widest.2 / PI
        );
        assert!((widest.1 - ALPHA_MAX).abs() < 1e-9, "α {}", widest.1);
        assert!(
            (widest.2 - BETA_MAX).abs() <= PI / 20.0 + 1e-9,
            "β {}",
            widest.2
        );
        assert!(0.13 <= ndd_low && ndd_high <= 0.16, "{ndd_low}–{ndd_high}");
    }

    /// Table 3 is read highest power first: Figure 3 (SNR_p = 10 dB, read off
    /// the published figure at 300 dpi to ±0.015 of the window) shows r_dmin
    /// changing sign at u ≈ 0.135 and 0.364 and r_dmax at u ≈ 0.158; the printed
    /// order's m_max = 84 would put r_dmax's at 0.248. Both indices stay above 1
    /// over the paper's fitted 10–45 dB, as the model requires.
    #[test]
    fn table_3_is_read_highest_power_first() {
        let snr_p = 10.0;
        let (lo_index, hi_index) = (m_min(snr_p), m_max(snr_p));
        // 1 + m_min·cos 4πu = 0 and 1 − m_max·cos 2πu = 0.
        let outer = (-1.0 / lo_index).acos() / (2.0 * TAU);
        let inner = 0.5 - outer;
        let edge = (1.0 / hi_index).acos() / TAU;
        println!(
            "SNR_p 10 dB: m_min {lo_index:.3}, m_max {hi_index:.3}; r_dmin crosses at {outer:.3} and {inner:.3}, r_dmax at {edge:.3}"
        );
        for (got, figure) in [(outer, 0.135), (inner, 0.364), (edge, 0.158)] {
            assert!((got - figure).abs() < 0.015, "{got} vs Figure 3's {figure}");
        }
        let printed = B0 + B1 * snr_p + B2 * snr_p * snr_p;
        assert!(((1.0 / printed).acos() / TAU - 0.158).abs() > 0.05);
        for s in 10..=45 {
            let s = s as f64;
            assert!(m_min(s) > 1.0 && m_max(s) > 1.0, "{s} dB");
        }
    }

    /// The tabulated limits at this geometry: without noise, at the floor and
    /// along the grid, held below the floor and above the top. Pinned from an
    /// independent numpy evaluation of the same model to 5·10⁻⁴.
    #[test]
    fn the_limits_at_this_geometry() {
        let arm = PeakShape::new();
        let free = arm.noise_free;
        println!(
            "without noise: NBD_max {:.4}, NDD {:.4}–{:.4}",
            free.nbd_max, free.ndd_min, free.ndd_max
        );
        let pinned = [
            (10.0, 0.2171, 0.0874, 0.2122),
            (20.0, 0.1743, 0.1170, 0.1724),
            (30.0, 0.1497, 0.1290, 0.1628),
            (45.0, 0.1398, 0.1338, 0.1591),
        ];
        for (snr_p, nbd_max, ndd_min, ndd_max) in pinned {
            let l = arm.limits_at(snr_p);
            println!(
                "SNR_p {snr_p:>4.0} dB: NBD_max {:.4}, NDD {:.4}–{:.4}",
                l.nbd_max, l.ndd_min, l.ndd_max
            );
            assert!((l.nbd_max - nbd_max).abs() < 5e-4, "{snr_p}: {}", l.nbd_max);
            assert!((l.ndd_min - ndd_min).abs() < 5e-4, "{snr_p}: {}", l.ndd_min);
            assert!((l.ndd_max - ndd_max).abs() < 5e-4, "{snr_p}: {}", l.ndd_max);
        }
        assert!((free.nbd_max - 0.1398).abs() < 5e-4);
        assert!((free.ndd_min - 0.1349).abs() < 5e-4);
        assert!((free.ndd_max - 0.1583).abs() < 5e-4);
        assert_eq!(arm.limits_at(3.0), arm.limits_at(SNR_P_FLOOR_DB));
        assert_eq!(arm.limits_at(200.0), arm.limits_at(SNR_P_TOP_DB));
        let top = arm.limits_at(SNR_P_TOP_DB);
        assert!((top.nbd_max - free.nbd_max).abs() < 2e-4);
        assert!((top.ndd_min - free.ndd_min).abs() < 2e-4);
        assert!((top.ndd_max - free.ndd_max).abs() < 2e-4);
    }

    /// The model brackets what it models: Eq. 12's patterns plus white noise
    /// whose mean power per bin sits SNR_p dB under the pattern's spectral
    /// peak, 600 draws each. The tabulated NDD_min and NDD_max lie within 0.015
    /// of the realised 2.5 % and 97.5 % points at 10, 20 and 30 dB, the size of
    /// error the paper's Figure 5 shows for its model near 10 dB; Table 3 read
    /// in its printed order would miss NDD_max at 10 dB by more.
    #[test]
    fn the_model_brackets_the_patterns_in_white_noise() {
        let arm = PeakShape::new();
        let mut tf = Transforms::new(M, N);
        let window = tf.window.clone();
        let energy: f64 = window.iter().map(|w| w * w).sum();
        let flat = vec![1.0; M];
        let mut rng = Gaussian::new(20261009);
        for snr_p in [10.0, 20.0, 30.0] {
            let model = arm.limits_at(snr_p);
            for (beta, lower) in [(BETA_DMIN, true), (BETA_DMAX, false)] {
                let clean = signal_pattern(F_O, beta, &flat);
                let peak = tf.peak_magnitude(&signal_pattern(F_O, beta, &window));
                let sigma = (peak * peak * 10f64.powf(-snr_p / 10.0) / energy).sqrt();
                let mut values: Vec<f64> = (0..600)
                    .map(|_| {
                        let noisy: Vec<f32> = clean
                            .iter()
                            .map(|&s| (s + sigma * rng.next() as f64) as f32)
                            .collect();
                        tf.frame(&noisy);
                        let (lo, _, hi) = peak_bounds(&tf.power, (F_O * N as f64) as usize);
                        ndd(&tf.x, &tf.xt, lo, hi, M)
                    })
                    .collect();
                values.sort_by(f64::total_cmp);
                let (p025, p975) = (values[15], values[584]);
                let (bound, realised) = if lower {
                    (model.ndd_min, p025)
                } else {
                    (model.ndd_max, p975)
                };
                println!(
                    "SNR_p {snr_p:.0} dB, β = {:+.1}π: NDD 2.5–97.5 % {p025:.4}–{p975:.4}, model {} {bound:.4}",
                    beta / PI,
                    if lower { "NDD_min" } else { "NDD_max" }
                );
                assert!(
                    (bound - realised).abs() < 0.015,
                    "{snr_p} dB: {bound} vs {realised}"
                );
                if snr_p == 10.0 && !lower {
                    let printed = B0 + B1 * snr_p + B2 * snr_p * snr_p;
                    let r_dmax = noise_pattern_max(F_O, printed, M);
                    let x_dmax = signal_pattern(F_O, BETA_DMAX, &window);
                    let other = noisy_ndd(&mut tf, &x_dmax, peak, &r_dmax, snr_p);
                    println!("     Table 3 in its printed order: NDD_max {other:.4}");
                    assert!((other - realised).abs() > 0.015, "{other} vs {realised}");
                }
            }
        }
    }

    /// The padding the peaks are read at: unpadded, a noise-free tone on a bin
    /// leaks monotonically and its peak runs across the spectrum; padded twice,
    /// a quarter bin off, its main lobe merges into its side lobes; padded four
    /// times, the worst case's peak is 15 bins wide at twenty places of the
    /// carrier across a bin, its NBD within 5·10⁻⁴.
    #[test]
    fn four_times_padding_resolves_every_peak() {
        let mut once = Transforms::new(M, M);
        once.frame(&tone(M, 2048.0, 0.4, 1.0));
        let (lo, _, hi) = peak_bounds(&once.power, argmax(&once.power));
        let mut twice = Transforms::new(M, 2 * M);
        twice.frame(&tone(M, 2048.25, 0.4, 1.0));
        let (lo2, _, hi2) = peak_bounds(&twice.power, argmax(&twice.power));
        println!(
            "tone, unpadded: L = {}; twice, a quarter bin off: L = {}",
            hi - lo + 1,
            hi2 - lo2 + 1
        );
        assert!(hi - lo > M / 4);
        assert!(hi2 - lo2 > 100);

        let mut tf = Transforms::new(M, N);
        let window = tf.window.clone();
        let (mut low, mut high) = (f64::INFINITY, f64::NEG_INFINITY);
        for j in 0..20 {
            let f_o = (F_O * N as f64 + j as f64 / 20.0) / N as f64;
            tf.windowed(&worst_case(f_o, ALPHA_MAX, BETA_MAX, &window));
            let (lo, _, hi) = peak_bounds(&tf.power, argmax(&tf.power));
            assert_eq!(hi - lo + 1, 15, "place {j}");
            let v = nbd(&tf.power, lo, hi);
            (low, high) = (low.min(v), high.max(v));
        }
        println!("worst case, four times: L = 15 at every place, NBD {low:.4}–{high:.4}");
        assert!(high - low < 5e-4, "{low}–{high}");
    }

    /// Table 1 at this geometry: a noise-free tone, on a bin and 0.37 bin off
    /// it, reads as a sinusoid against the noise-free limits and inside Table
    /// 1's 0.13 ≤ NDD ≤ 0.16, with FCD under half a bin; its first side lobe
    /// reads FCD ≥ N/M. The tree calls the tone's bin a sinusoid.
    #[test]
    fn a_tone_reads_as_a_sinusoid_and_its_side_lobe_as_a_side_lobe() {
        let mut arm = PeakShape::new();
        let mut frame = Frame::new();
        for offset in [0.0, 0.37] {
            let audio = tone(M, 2048.0 + offset, 0.4, 1.0);
            arm.transforms.frame(&audio);
            let (d, (_, _, hi)) = read(&arm.transforms, argmax(&arm.transforms.power), N);
            let side = first_side_lobe(&arm.transforms.power, hi);
            let (s, _) = read(&arm.transforms, side, N);
            println!(
                "tone at +{offset} bin: NBD {:.4}, NDD {:.4}, FCD {:.3}; first side lobe {:.2} bins out, FCD {:.2}",
                d.nbd,
                d.ndd,
                d.fcd,
                (side - argmax(&arm.transforms.power)) as f64 / PAD as f64,
                s.fcd
            );
            let free = arm.noise_free;
            assert!(d.nbd <= free.nbd_max, "NBD {}", d.nbd);
            assert!(
                (free.ndd_min..=free.ndd_max).contains(&d.ndd),
                "NDD {}",
                d.ndd
            );
            assert!((0.13..=0.16).contains(&d.ndd), "NDD {}", d.ndd);
            assert!(d.fcd <= 0.5, "FCD {}", d.fcd);
            assert!(s.fcd >= PAD as f64, "side lobe FCD {}", s.fcd);
            frame.compute(&audio);
            assert_eq!(arm.classify(&frame.mag, 2048).class, Class::Sinusoid);
        }
    }

    /// The floor a peak's SNR_p is read against is the mean power whose Rayleigh
    /// median is the local median.
    #[test]
    fn the_floor_is_the_rayleigh_mean_of_the_local_median() {
        let mag: Vec<f32> = (0..200)
            .map(|b| if b == 100 { 50.0 } else { 2.0 })
            .collect();
        let snr = snr_p(400.0, &mag, 100);
        let floor = 400.0 / 10f64.powf(snr / 10.0);
        assert!(
            (stats::rayleigh_median(floor) - 2.0).abs() < 1e-9,
            "{floor}"
        );
    }

    /// The control: 200 frames of white Gaussian noise, the local maxima of each
    /// frame over arm A's threshold at the true σ, and how many the classifier
    /// keeps. It may only lower the rate.
    #[test]
    fn white_noise_candidates_are_thinned() {
        let mut arm = PeakShape::new();
        arm.prepare(&set(Some(SIGMA)));
        let cap = capture();
        let mut frame = Frame::new();
        let mut out = vec![0.0f32; M / 2];
        let mut rng = Gaussian::new(7);
        let (mut bins, mut candidates, mut kept) = (0u64, 0u64, 0u64);
        for index in 0..200 {
            let audio: Vec<f32> = (0..M).map(|_| SIGMA * rng.next()).collect();
            frame.compute(&audio);
            let h = hop(index, Block::Holdout, &audio, &frame, &cap);
            out.fill(f32::INFINITY);
            assert_eq!(arm.g1_threshold(&h, &mut out), Availability::Available);
            let mag = &frame.mag;
            for b in 1..M / 2 - 1 {
                bins += 1;
                if mag[b] > mag[b - 1] && mag[b] > mag[b + 1] && mag[b] >= out[b] {
                    candidates += 1;
                    kept += u64::from(arm.g1_keep_peak(&h, b));
                }
            }
        }
        let tally = arm.preroll.get();
        for (label, k) in [("arm A", candidates), ("arm A then the classifier", kept)] {
            let (lo, hi) = stats::clopper_pearson(k, bins, 0.05);
            println!(
                "{label}: {k} / {bins} = {:.3e} per bin, 95 % CP [{lo:.2e}, {hi:.2e}]",
                k as f64 / bins as f64
            );
        }
        println!(
            "classifier: {} side lobes, {} wide, {} off duration, {} kept; mean SNR_p {:.1} dB, {} under {SNR_P_FLOOR_DB} dB",
            tally.side_lobe,
            tally.wide,
            tally.off_duration,
            tally.kept(),
            tally.snr_p_sum / tally.asked as f64,
            tally.under_floor
        );
        assert_eq!(tally.asked, candidates);
        assert_eq!(tally.kept(), kept);
        assert!(kept < candidates, "{kept} of {candidates}");
    }

    /// T1's partials: a tone 30 dB over the noise per bin, at a fractional bin
    /// and a random phase, is kept in at least 297 of 300 frames.
    #[test]
    fn tones_at_thirty_db_are_kept() {
        let mut arm = PeakShape::new();
        arm.prepare(&set(Some(SIGMA)));
        let cap = capture();
        let mut frame = Frame::new();
        let mut out = vec![0.0f32; M / 2];
        let mut rng = Gaussian::new(11);
        let amplitude = tone_amplitude(SIGMA, 30.0, M);
        let mut kept = 0;
        for index in 0..300i64 {
            let place = 200.0 + 11.3 * index as f64 + (index as f64 * 0.618_033_988_75) % 1.0;
            let phase = TAU * ((index as f64 * 0.414_213_562_37) % 1.0);
            let audio: Vec<f32> = tone(M, place, phase, amplitude)
                .iter()
                .map(|&s| s + SIGMA * rng.next())
                .collect();
            frame.compute(&audio);
            let h = hop(index, Block::Note, &audio, &frame, &cap);
            out.fill(f32::INFINITY);
            assert_eq!(arm.g1_threshold(&h, &mut out), Availability::Available);
            let centre = place.round() as usize;
            let bin = (centre - 2..=centre + 2)
                .max_by(|&a, &b| frame.mag[a].total_cmp(&frame.mag[b]))
                .unwrap();
            assert!(frame.mag[bin] >= out[bin]);
            kept += usize::from(arm.g1_keep_peak(&h, bin));
        }
        let tally = arm.note.get();
        println!(
            "30 dB tones kept {kept} / 300: {} wide, {} off duration, {} side lobes; mean SNR_p {:.1} dB",
            tally.wide,
            tally.off_duration,
            tally.side_lobe,
            tally.snr_p_sum / tally.asked as f64
        );
        assert!(kept >= 297, "{kept}");
    }

    /// Through the trait: the hop the threshold transformed is classified, and a
    /// peak on any other hop is left to the first stage; a peak within a
    /// measured partial's lobe is tallied apart.
    #[test]
    fn only_the_transformed_hop_is_classified() {
        let mut arm = PeakShape::new();
        arm.prepare(&set(Some(SIGMA)));
        let cap = capture();
        let mut frame = Frame::new();
        let mut rng = Gaussian::new(5);
        let audio: Vec<f32> = tone(M, 1000.25, 1.0, tone_amplitude(SIGMA, 40.0, M))
            .iter()
            .map(|&s| s + SIGMA * rng.next())
            .collect();
        frame.compute(&audio);
        let mut out = vec![f32::INFINITY; M / 2];
        let transformed = hop(7, Block::Note, &audio, &frame, &cap);
        assert_eq!(
            arm.g1_threshold(&transformed, &mut out),
            Availability::Available
        );
        assert!(out.iter().all(|&t| t == out[0] && t.is_finite()));
        assert!(arm.g1_keep_peak(&transformed, 1000));
        assert_eq!(arm.note.get().asked, 1);
        let other = hop(8, Block::Note, &audio, &frame, &cap);
        assert!(arm.g1_keep_peak(&other, 1000));
        assert_eq!(arm.note.get().asked, 1);
        // The capture's partial at 440 Hz sits at bin 81.7.
        arm.g1_keep_peak(&transformed, 82);
        assert_eq!(arm.partial.get().asked, 1);
        assert_eq!(arm.note.get().asked, 1);
        assert!(!arm.decision_only());
        assert_eq!(arm.consumers(), &[Consumer::G1]);
    }
}
