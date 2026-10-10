//! Arm 7: the running floor moved per hop by one scalar, the location of the
//! hop's standardised log-power read from the left half of its bulk (an
//! empirical null in Efron's sense).

use std::time::{Duration, Instant};

use super::{
    Arm, Availability, Block, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx,
    analytic_nu_half, quantiles, raw_to_gate,
};
use crate::gates::ambient::T_LABELS;
use crate::gates::stats;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};

/// Fewest silent hops learned before the arm answers: arm 1's count, since the
/// floor is arm 1's.
const MIN_HOPS: usize = 8;

/// Bins the location is read over when the floor is positive everywhere: `1` to
/// `len − 2`, the bins the picker scans. Bin 0 of a real frame is real-valued,
/// so its power is `χ²₁` rather than exponential.
const LOCATION_BINS: usize = BASS_WINDOW_SIZE / 2 - 2;

/// The unit exponential's first quartile, `ln(4/3)`: `1 − e^{−a} = ¼`.
const EXP_Q1: f64 = 0.287_682_072_451_780_9;

/// The log-exponential's first quartile, `ln ln(4/3)`: where the first quartile
/// of `z = ln(P/μ)` sits for an exponential power `P` of known mean `μ`.
const Z_Q1: f64 = -1.245_899_323_707_238_4;

/// Power correlation `|ρ|²` of two bins of a Hann frame of white noise one and
/// two bins apart: the window's `w²` has Fourier weights 3/8, −1/4 and 1/16 at
/// lags 0, 1 and 2 and none beyond, so the complex bins correlate by −2/3 and
/// 1/6, and bins further apart not at all.
const HANN_POWER_CORRELATION: [f64; 2] = [4.0 / 9.0, 1.0 / 36.0];

/// A natural log of a power ratio in dB, `10 lg e^x`.
fn to_db(ln_power: f64) -> f32 {
    (ln_power * 10.0 / std::f64::consts::LN_10) as f32
}

/// The first quartile of `z = ln(P/μ̂)` when `μ̂` is the mean of `m` periodograms:
/// `P/μ̂` is then `F(2, 2m)`, whose survival is `(1 + r/m)^{−m}`, so the quartile
/// is `ln(m((4/3)^{1/m} − 1))`. [`Z_Q1`] as `m → ∞`; 0.042 above it at the 3.45
/// periodograms of one twelve-hop run.
fn null_quartile(m: f64) -> f64 {
    if m.is_infinite() {
        return Z_Q1;
    }
    (m * (EXP_Q1 / m).exp_m1()).ln()
}

/// Covariance of `1{E₁ ≤ a}` and `1{E₂ ≤ a}` for two unit exponentials, the
/// squared moduli of two complex Gaussian variables whose powers correlate by
/// `r`: Kibble's (1941) bivariate exponential expanded by the Hille–Hardy
/// formula, `e^{−x−y} Σ_k r^k L_k(x) L_k(y)` with `L_k` the Laguerre
/// polynomials. Each term integrates over `[0, a]²` to the square of
/// `∫₀^a e^{−x} L_k(x) dx = e^{−a}(L_{k−1}(a) − L_k(a))`, and the `k = 0` term
/// is the product of the marginals, so the covariance is
/// `e^{−2a} Σ_{k≥1} r^k (L_{k−1}(a) − L_k(a))²`.
fn indicator_covariance(a: f64, r: f64) -> f64 {
    // L₀ and L₁, then the three-term recurrence
    // (k + 1) L_{k+1} = (2k + 1 − a) L_k − k L_{k−1}. |L_k(a)| ≤ e^{a/2} for
    // a ≥ 0, so the terms fall as r^k.
    let (mut below, mut at) = (1.0f64, 1.0 - a);
    let mut r_k = r;
    let mut sum = 0.0;
    for k in 1..=400 {
        let step = below - at;
        sum += r_k * step * step;
        let k = k as f64;
        let next = ((2.0 * k + 1.0 - a) * at - k * below) / (k + 1.0);
        below = at;
        at = next;
        r_k *= r;
        if r_k < 1e-20 {
            break;
        }
    }
    (-2.0 * a).exp() * sum
}

/// `δ̂`'s variance on white noise over a known floor, in (ln power)², for a
/// first quartile over `n` bins of a Hann frame. By Bahadur's (1966)
/// representation a sample quantile's error is `p` less the share of the sample
/// below the true quantile, over the density there, so its variance is the
/// share's over the squared density: `[p(1 − p) + 2 Σ_k c_k] / (n f(q)²)`, with
/// `p = ¼`, `f(q) = ¾ ln(4/3)` the log-exponential's density at its quartile,
/// and `c_k` the covariance of two bins' indicators `k` bins apart
/// ([`indicator_covariance`] at the Hann power correlations). 1.318 × 10⁻³ at
/// 4094 bins: an SD of 0.158 dB, 1.34 times the variance of independent bins.
fn delta_variance(n: usize) -> f64 {
    let p = 0.25;
    let density = 0.75 * EXP_Q1;
    let lagged: f64 = HANN_POWER_CORRELATION
        .iter()
        .map(|&r| indicator_covariance(EXP_Q1, r))
        .sum();
    (p * (1.0 - p) + 2.0 * lagged) / (n as f64 * density * density)
}

/// The location of the standardised powers `ratios` (`P/μ̂`, in any order): the
/// log of their first quartile, less the null's `null_q1`. The quartile is the
/// order statistic of rank `(n + 1)/4`, rounded, selected in place; `ln` is
/// monotone, so this is the first quartile of `z = ln(P/μ̂)` itself. `None` for
/// an empty input or a quartile that is not positive.
fn location(ratios: &mut [f32], null_q1: f64) -> Option<f64> {
    let n = ratios.len();
    if n == 0 {
        return None;
    }
    let rank = (((n + 1) as f64 / 4.0).round() as usize).clamp(1, n);
    let (_, q, _) = ratios.select_nth_unstable_by(rank - 1, f32::total_cmp);
    let q = f64::from(*q);
    (q > 0.0 && q.is_finite()).then(|| q.ln() - null_q1)
}

/// Arm 7 (G1, G2, G3): the running floor moved per hop by the location of the
/// hop's standardised log-power.
///
/// Efron's (2004) empirical null takes the null distribution of many
/// simultaneous test statistics from the central bulk of the statistics
/// themselves rather than from theory, on the premise that nearly all of them
/// are null. Here the statistics are the hop's bins `1` to `len − 2`, each
/// standardised by the floor arm 1 learns: `z_b = ln P_b − ln μ̂_b`, with
/// `P_b = |X_b|²` and `μ̂_b` the bin's mean power over the Gatekeeper-silent hops
/// the arm may learn from. For Gaussian noise at the floor's level `P_b/μ_b` is a
/// unit exponential, so `z` has the log-exponential density `e^{z − e^z}`: mode
/// 0, first quartile `ln ln(4/3) = −1.2459` (from `1 − e^{−e^z} = ¼`), median
/// `ln ln 2 = −0.3665`, mean `−γ = −0.5772` and SD `π/√6 = 1.2825`. Only its
/// location `δ` is unknown, the natural log of how much more power the room has
/// now than when the floor was learned, so the fit has one parameter, read from
/// the left half of the bulk where partials do not reach: `δ̂ = Q₁(z) − q₁`, `Q₁`
/// the sample first quartile and `q₁` the null's. A partial moves `δ̂` only by
/// the bins it takes from the null: a fraction `ε` of bins far above the quartile
/// raises it by `ε/(4(1 − ε)f(q₁))`, `f(q₁) = ¾ ln(4/3)` the density there, which
/// is 0.27 dB at 5 % of the bins. The threshold on bin `b` is the floor's, moved
/// by the scalar: `√(μ̂_b e^{δ̂ − Var δ̂/2} F_c)`. Two corrections, each derived
/// for Gaussian noise, make the null exact for a floor that is itself an
/// estimate and a scalar that scatters:
///
/// - **Finite floor.** `μ̂_b` is a mean of `M` periodograms, so `P_b/μ̂_b` is
///   `F(2, 2M)` rather than exponential, and `q₁` is the log of that law's
///   first quartile, `ln(M((4/3)^{1/M} − 1))`, which tends to `ln ln(4/3)` as
///   `M → ∞` and lies 0.18 dB above it at the 3.45 periodograms of one
///   twelve-hop run.
/// - **The scalar's scatter.** By Bahadur's (1966) representation the sample
///   quartile's error is ¼ less the share of bins below the true quartile, over
///   the density there, so `Var δ̂ = [p(1 − p) + 2 Σ_k c_k]/(n f(q₁)²)` with
///   `p = ¼` and `c_k` the covariance of two bins' indicators `k` bins apart,
///   from Kibble's (1941) bivariate exponential at the Hann bins' power
///   correlations 4/9 and 1/36 (none beyond two bins): 1.318 × 10⁻³ at 4094
///   bins, an SD of 0.158 dB. `e^{δ̂}` reads high by `e^{Var δ̂/2}` in power,
///   which the level `μ̂_b e^{δ̂ − Var δ̂/2}` takes out; that level then carries
///   both relative variances, `1/M_c = 1/M + Var δ̂`, and `F_c` is the upper
///   `P_fa` point of `F(2, 2M_c)`, the Student-Rayleigh law of a level
///   averaged from `M_c` periodograms (Talbot & Thrane 2020). `M` is Percival
///   & Walden's equivalent degrees of freedom of overlapped Hann frames summed
///   over the floor's runs of hops, or the measured ν/2 per hop where the set
///   gives one.
///
/// The form is derived here for this signal: the paper is cited for the idea of
/// fitting the null to the bulk, not for the one-parameter fit or its
/// constants. G2 and G3 read the same level at the probe's bin, converted to the
/// Goertzel's window, with this hop's `δ̂` from the 8192-point frame. Until
/// `MIN_HOPS` silent hops have been learned the arm declines every hop and every
/// probe.
///
/// # References
///
/// - Efron, B. (2004). Large-scale simultaneous hypothesis testing: the choice of
///   a null hypothesis. Journal of the American Statistical Association
///   99(465), 96–104. DOI: 10.1198/016214504000000089.
/// - Kibble (1941) for the bivariate exponential of two correlated bins' powers;
///   Bahadur (1966) for the sample quantile's representation; Talbot & Thrane
///   (2020) for the `F(2, 2M)` law and Percival & Walden (1993) for the
///   equivalent degrees of freedom, as arm 1 takes them.
pub(super) struct EmpiricalNull {
    /// Σ|X|² per bin over the hops learned from, in the raw frame's units.
    sum: Vec<f64>,
    hops: usize,
    /// Lengths of the contiguous runs of hops learned from, the open run last.
    runs: Vec<usize>,
    last_hop: Option<i64>,
    /// ν/2 per hop of averaging as Stage B measured it, or the analytic value.
    nu_half_per_hop: Option<f32>,
    /// `μ̂`: `sum` over `hops`, the floor's mean power per bin, refreshed when a
    /// hop is learned.
    mean: Vec<f32>,
    /// `M` for the hops learned so far, refreshed when one is added.
    m_eff: f64,
    /// Bins the location is read over: `1` to `len − 2` where `μ̂ > 0`.
    bins: usize,
    /// The null's first quartile of `z` at `M`.
    null_q1: f64,
    /// `Var δ̂` on white noise over `bins` bins, in (ln power)².
    delta_var: f64,
    /// `M_c`, the periodograms the moved level is worth.
    m_level: f64,
    /// `F_c`, the upper `P_fa` point of `F(2, 2M_c)`: the threshold over the
    /// moved level, in power.
    multiplier: f64,
    p_fa: f64,
    /// The hop's standardised powers, selected in place.
    ratios: Vec<f32>,
    /// This hop's `δ̂` once a consumer has asked for it, `None` before then.
    current: Option<Option<f64>>,
    /// `δ̂` in dB on the silent hops asked for, and on the note by time bucket.
    silent_db: Vec<f32>,
    note_db: Vec<Vec<f32>>,
    /// Hops asked for before `MIN_HOPS` were learned, and hops whose quartile
    /// was not positive.
    declined: u64,
    failed: u64,
    /// Time spent standardising, selecting the quartile and filling the
    /// threshold, and the hops it served.
    cost: Duration,
    timed_hops: u64,
}

impl EmpiricalNull {
    /// An arm with nothing learned, at 10⁻³ until `prepare` sets the operating
    /// point.
    pub(super) fn new() -> Self {
        let bins = BASS_WINDOW_SIZE / 2;
        Self {
            sum: vec![0.0; bins],
            hops: 0,
            runs: Vec::new(),
            last_hop: None,
            nu_half_per_hop: None,
            mean: vec![0.0; bins],
            m_eff: 0.0,
            bins: 0,
            null_q1: Z_Q1,
            delta_var: f64::NAN,
            m_level: 0.0,
            multiplier: f64::NAN,
            p_fa: 1.0e-3,
            ratios: Vec::with_capacity(bins),
            current: None,
            silent_db: Vec::new(),
            note_db: vec![Vec::new(); T_LABELS.len()],
            declined: 0,
            failed: 0,
            cost: Duration::ZERO,
            timed_hops: 0,
        }
    }

    /// `μ̂`, `M`, the null's quartile and the multiplier from the hops learned so
    /// far.
    fn refresh(&mut self) {
        self.m_eff = match self.nu_half_per_hop {
            Some(per_hop) => f64::from(per_hop) * self.hops as f64,
            None => self
                .runs
                .iter()
                .map(|&k| f64::from(analytic_nu_half(k)))
                .sum(),
        };
        let hops = self.hops as f64;
        for (m, &s) in self.mean.iter_mut().zip(&self.sum) {
            *m = (s / hops) as f32;
        }
        let len = self.mean.len();
        self.bins = self.mean[1..len - 1].iter().filter(|&&m| m > 0.0).count();
        self.null_q1 = null_quartile(self.m_eff);
        self.delta_var = delta_variance(self.bins);
        self.m_level = 1.0 / (1.0 / self.m_eff + self.delta_var);
        self.multiplier = stats::f_2_2m_threshold(self.m_level, self.p_fa);
    }

    /// The factor on `μ̂` for a hop's `δ̂`: `e^{δ̂}` with its bias in power,
    /// `e^{Var δ̂/2}`, taken out, so the moved level is the mean-one product the
    /// `F(2, 2M_c)` law assumes.
    fn shift(&self, delta: f64) -> f64 {
        (delta - 0.5 * self.delta_var).exp()
    }

    /// This hop's `δ̂`, in ln power: computed on the first consumer's request and
    /// kept for the others. `None` before `MIN_HOPS` hops have been learned.
    fn delta(&mut self, hop: &HopCtx) -> Option<f64> {
        if let Some(known) = self.current {
            return known;
        }
        let delta = if self.hops < MIN_HOPS {
            self.declined += 1;
            None
        } else {
            let start = Instant::now();
            let len = hop.mag.len().min(self.mean.len());
            let mean = &self.mean;
            self.ratios.clear();
            self.ratios.extend(
                (1..len - 1)
                    .filter(|&b| mean[b] > 0.0)
                    .map(|b| hop.mag[b] * hop.mag[b] / mean[b]),
            );
            let found = location(&mut self.ratios, self.null_q1);
            match found {
                Some(d) if hop.block == Block::Note => {
                    self.note_db[hop.bucket.min(T_LABELS.len() - 1)].push(to_db(d))
                }
                Some(d) => self.silent_db.push(to_db(d)),
                None => self.failed += 1,
            }
            self.cost += start.elapsed();
            self.timed_hops += 1;
            found
        };
        self.current = Some(delta);
        delta
    }
}

impl Arm for EmpiricalNull {
    fn name(&self) -> &str {
        "7"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.sum.iter_mut().for_each(|s| *s = 0.0);
        self.mean.iter_mut().for_each(|m| *m = 0.0);
        self.hops = 0;
        self.runs.clear();
        self.last_hop = None;
        self.nu_half_per_hop = set.nu_half_per_frame;
        self.m_eff = 0.0;
        self.bins = 0;
        self.null_q1 = Z_Q1;
        self.delta_var = f64::NAN;
        self.m_level = 0.0;
        self.p_fa = f64::from(set.p_fa);
        self.multiplier = f64::NAN;
        self.current = None;
        self.silent_db.clear();
        self.note_db.iter_mut().for_each(Vec::clear);
        self.declined = 0;
        self.failed = 0;
        self.cost = Duration::ZERO;
        self.timed_hops = 0;
        println!(
            "[{}] arm 7: the floor starts empty and learns from the build half of each capture's \
             silence; ν/2 per frame {}; each hop's δ̂ is the log of the first quartile of |X|²/μ̂ \
             over bins 1–{LOCATION_BINS} less the null's ln(M((4/3)^(1/M) − 1)) (ln ln(4/3) = \
             {Z_Q1:.4} for a known floor); on white noise δ̂ scatters {:.3} dB SD, carried into the \
             multiplier as F(2, 2M_c), 1/M_c = 1/M + Var δ̂",
            set.label,
            match set.nu_half_per_frame {
                Some(v) => format!("{v:.3} (Stage B)"),
                None => "analytic Hann, summed over runs".to_string(),
            },
            to_db(delta_variance(LOCATION_BINS).sqrt())
        );
    }

    fn begin_capture(&mut self, _cap: &CaptureCtx) {
        self.last_hop = None;
    }

    fn observe(&mut self, hop: &HopCtx) {
        self.current = None;
        // Learn from silence only, and only from the half of it that is not
        // scored; the note's hops never enter.
        if hop.is_silence && hop.block == Block::Build {
            for (s, &m) in self.sum.iter_mut().zip(hop.mag) {
                *s += f64::from(m) * f64::from(m);
            }
            self.hops += 1;
            match self.last_hop {
                Some(prev) if hop.hop == prev + 1 => {
                    *self.runs.last_mut().expect("an open run") += 1
                }
                _ => self.runs.push(1),
            }
            self.last_hop = Some(hop.hop);
            self.refresh();
        }
    }

    fn finish(&mut self, set: &SetCtx) {
        println!(
            "[{}] arm 7: δ̂ in dB (10 lg e^δ̂) on the hops a consumer asked for; {} asked for \
             before {MIN_HOPS} hops were learned, {} with no positive quartile",
            set.label, self.declined, self.failed
        );
        let mut note: Vec<f32> = self.note_db.iter().flatten().copied().collect();
        for (label, v) in [
            ("silent hops (the scored pre-roll)", &mut self.silent_db),
            ("note hops", &mut note),
        ] {
            if v.is_empty() {
                continue;
            }
            let (median, lo, hi) = quantiles(v);
            println!(
                "     {label}: {} hops, median {median:+.3} dB [5 %: {lo:+.3}, 95 %: {hi:+.3}]",
                v.len()
            );
        }
        if !note.is_empty() {
            let by_bucket: Vec<String> = T_LABELS
                .iter()
                .zip(self.note_db.iter_mut())
                .filter(|(_, v)| !v.is_empty())
                .map(|(l, v)| {
                    let n = v.len();
                    format!("{l} s {:+.2} ({n})", quantiles(v).0)
                })
                .collect();
            println!(
                "     note hops by time since the onset, median dB (hops): {}",
                by_bucket.join(" · ")
            );
        }
        println!(
            "     the floor at the end: {} hops in {} runs, M = {:.1}, null quartile {:.4} \
             ({:+.3} dB from ln ln(4/3)); Var δ̂ {:.3e} over {} bins (SD {:.3} dB), M_c = {:.1}, \
             multiplier {:.4} (F(2, 2M) alone {:.4})",
            self.hops,
            self.runs.len(),
            self.m_eff,
            self.null_q1,
            to_db(self.null_q1 - Z_Q1),
            self.delta_var,
            self.bins,
            to_db(self.delta_var.sqrt()),
            self.m_level,
            self.multiplier,
            stats::f_2_2m_threshold(self.m_eff, self.p_fa)
        );
        if self.timed_hops > 0 {
            let hop_ms = HOP_SIZE as f64 / SAMPLE_RATE as f64 * 1.0e3;
            println!(
                "     standardising, the quartile and the G1 threshold: {:.1} µs per hop over {} \
                 hops, against the {hop_ms:.1} ms hop",
                self.cost.as_secs_f64() * 1.0e6 / self.timed_hops as f64,
                self.timed_hops
            );
        }
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        let Some(delta) = self.delta(hop) else {
            return Availability::Unavailable;
        };
        let start = Instant::now();
        let scale = (self.shift(delta) * self.multiplier) as f32;
        for (o, &m) in out.iter_mut().zip(&self.mean) {
            *o = (m * scale).sqrt();
        }
        self.cost += start.elapsed();
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let bin = (probe.target_hz / hop.hz_per_bin).round() as usize;
        if bin >= self.mean.len() {
            return GateVerdict::default();
        }
        let Some(delta) = self.delta(hop) else {
            return GateVerdict::default();
        };
        // The bin's mean noise power this hop, in the raw frame's units.
        let power = f64::from(self.mean[bin]) * self.shift(delta);
        let threshold = raw_to_gate((power * self.multiplier).sqrt() as f32, probe.window);
        let median = stats::rayleigh_median(power);
        // The moved level is worth M_c periodograms: its 5–95 % band in power,
        // by the normal approximation at these M_c.
        let rel = 1.645 / self.m_level.sqrt();
        let lo = median * (1.0 - rel).max(0.05).sqrt();
        let hi = median * (1.0 + rel).sqrt();
        GateVerdict {
            threshold: Some(threshold),
            estimate: Some(raw_to_gate(median as f32, probe.window)),
            interval: Some((
                raw_to_gate(lo as f32, probe.window),
                raw_to_gate(hi as f32, probe.window),
            )),
            admit: Some(probe.amplitude >= threshold),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::arms::Frame;
    use crate::gates::truth::{Gaussian, awgn_median_bin, tone_amplitude};
    use crate::truth::hann_vec;
    use realfft::RealFftPlanner;
    use tuner_core::algorithms::spectral::goertzel_windowed;

    const SIGMA: f32 = 1.0e-3;
    const N: usize = BASS_WINDOW_SIZE;
    const SEED: u64 = 20261009;
    /// Hops per capture's build run, as the T1 control's within-capture split
    /// gives them.
    const RUN: usize = 12;

    /// The log-exponential's median `ln ln 2`, from `1 − e^{−e^z} = ½`.
    const Z_MEDIAN: f64 = -0.366_512_920_581_664_35;
    /// Its mean, `−γ`: `E[ln E] = Γ′(1) = −γ` (Euler–Mascheroni).
    const Z_MEAN: f64 = -0.577_215_664_901_532_9;
    /// Its SD, `π/√6`: `Var ln E = ψ′(1) = π²/6`.
    const Z_SD: f64 = 1.282_549_830_161_864;

    /// The test capture's one partial.
    static NS: [usize; 1] = [1];
    static F_MEAS: [f32; 1] = [440.0];

    /// A set at the operating point `p_fa`, with nothing fixed.
    fn set_at(p_fa: f32) -> SetCtx<'static> {
        SetCtx {
            label: "test",
            p_fa,
            sigma_override: None,
            nu_half_per_frame: None,
        }
    }

    /// A capture of white noise at `SIGMA` with one partial at 440 Hz.
    fn white() -> CaptureCtx<'static> {
        CaptureCtx {
            dir: "white",
            key: 48,
            register: 1,
            ns: &NS,
            f_meas: &F_MEAS,
            spacing_hz: 440.0,
            b: 0.0,
            sigma_rec: SIGMA,
        }
    }

    /// A unit exponential: half the sum of two squared standard normals.
    fn exponential(rng: &mut Gaussian) -> f64 {
        let (a, b) = (f64::from(rng.next()), f64::from(rng.next()));
        0.5 * (a * a + b * b)
    }

    /// `N` samples of white Gaussian noise at `sigma`.
    fn noise(rng: &mut Gaussian, sigma: f32) -> Vec<f32> {
        (0..N).map(|_| sigma * rng.next()).collect()
    }

    /// The value at quantile `q` of `v`, sorted in place, by nearest rank.
    fn quantile(v: &mut [f64], q: f64) -> f64 {
        v.sort_by(f64::total_cmp);
        v[((v.len() - 1) as f64 * q).round() as usize]
    }

    /// A Gatekeeper-silent hop of `frame`, as the driver hands one over.
    fn hop<'a>(
        index: i64,
        block: Block,
        audio: &'a [f32],
        frame: &'a Frame,
        cap: &'a CaptureCtx<'a>,
    ) -> HopCtx<'a> {
        HopCtx {
            hop: index,
            t_since: -1.0,
            bucket: 0,
            is_silence: true,
            block,
            audio,
            spectrum: &frame.spectrum,
            mag: &frame.mag,
            hz_per_bin: SAMPLE_RATE as f32 / N as f32,
            sigma: SIGMA,
            truth_level: None,
            oracle: &[],
            capture: cap,
        }
    }

    /// An arm at `p_fa` that has learned `captures` runs of `RUN` contiguous hops
    /// of white noise at `SIGMA`, one run per capture, as the driver hands a
    /// build block over.
    fn learned(
        p_fa: f32,
        captures: usize,
        rng: &mut Gaussian,
        frame: &mut Frame,
        cap: &CaptureCtx,
    ) -> EmpiricalNull {
        let mut arm = EmpiricalNull::new();
        arm.prepare(&set_at(p_fa));
        for _ in 0..captures {
            arm.begin_capture(cap);
            let stream: Vec<f32> = (0..N + (RUN - 1) * HOP_SIZE)
                .map(|_| SIGMA * rng.next())
                .collect();
            for h in 0..RUN {
                let audio = &stream[h * HOP_SIZE..h * HOP_SIZE + N];
                frame.compute(audio);
                arm.observe(&hop(h as i64 - 40, Block::Build, audio, frame, cap));
            }
        }
        arm
    }

    /// The constants against their closed forms, and the log-exponential's first
    /// quartile, median, mean and SD against 10⁶ draws of `ln E`, `E` a unit
    /// exponential: each within four standard errors.
    #[test]
    fn the_log_exponential_matches_a_monte_carlo() {
        assert!((EXP_Q1 - (4.0f64 / 3.0).ln()).abs() < 1e-15);
        assert!((Z_Q1 - (4.0f64 / 3.0).ln().ln()).abs() < 1e-15);
        assert!((Z_MEDIAN - std::f64::consts::LN_2.ln()).abs() < 1e-15);
        assert!((Z_SD - std::f64::consts::PI / 6.0f64.sqrt()).abs() < 1e-15);
        assert_eq!(null_quartile(f64::INFINITY), Z_Q1);
        let draws = 1_000_000usize;
        let mut rng = Gaussian::new(SEED);
        let mut z: Vec<f64> = (0..draws).map(|_| exponential(&mut rng).ln()).collect();
        let n = draws as f64;
        let mean = z.iter().sum::<f64>() / n;
        let sd = (z.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
        let q1 = quantile(&mut z, 0.25);
        let median = quantile(&mut z, 0.5);
        // Standard errors: a quantile's √(p(1 − p)/n)/f, the density
        // f(z) = e^{z − e^z}; the mean's SD/√n; the SD's SD·√((κ − 1)/4n), the
        // log-exponential's kurtosis κ being 5.4.
        let density = |z: f64| (z - z.exp()).exp();
        let se_q1 = (0.1875 / n).sqrt() / density(Z_Q1);
        let se_median = (0.25 / n).sqrt() / density(Z_MEDIAN);
        let se_mean = Z_SD / n.sqrt();
        let se_sd = Z_SD * (4.4 / (4.0 * n)).sqrt();
        println!(
            "ln Exp(1) over {draws} draws: Q₁ {q1:.5} (−1.24590 ± {se_q1:.5}), median {median:.5} \
             (−0.36651 ± {se_median:.5}), mean {mean:.5} (−0.57722 ± {se_mean:.5}), SD {sd:.5} \
             (1.28255 ± {se_sd:.5})"
        );
        assert!((q1 - Z_Q1).abs() < 4.0 * se_q1, "Q₁ {q1}");
        assert!(
            (median - Z_MEDIAN).abs() < 4.0 * se_median,
            "median {median}"
        );
        assert!((mean - Z_MEAN).abs() < 4.0 * se_mean, "mean {mean}");
        assert!((sd - Z_SD).abs() < 4.0 * se_sd, "SD {sd}");
    }

    /// The finite floor's null quartile against 4·10⁵ draws of
    /// `ln(E / Ḡ)`, `Ḡ` the mean of `m` further unit exponentials, at whole
    /// `m` = 1 and 4: within four standard errors, the density of `ln F(2, 2m)`
    /// at its quartile being `r(1 + r/m)^{−m−1}` at `r = e^q`. The value at one
    /// twelve-hop run and the limit are pinned beside it.
    #[test]
    fn the_finite_floor_quartile_matches_f_2_2m() {
        let one_run = f64::from(analytic_nu_half(RUN));
        let at_run = null_quartile(one_run);
        println!(
            "M = {one_run:.4}: null quartile {at_run:.5}, {:+.4} dB from ln ln(4/3)",
            to_db(at_run - Z_Q1)
        );
        assert!((one_run - 3.453_82).abs() < 1e-4, "{one_run}");
        assert!((at_run - Z_Q1 - 0.041_936).abs() < 1e-5, "{at_run}");
        assert!((null_quartile(1.0e9) - Z_Q1).abs() < 1e-8);
        let draws = 400_000usize;
        let mut rng = Gaussian::new(SEED + 1);
        for m in [1usize, 4] {
            let mut z: Vec<f64> = (0..draws)
                .map(|_| {
                    let g = (0..m).map(|_| exponential(&mut rng)).sum::<f64>() / m as f64;
                    (exponential(&mut rng) / g).ln()
                })
                .collect();
            let closed = null_quartile(m as f64);
            let r = closed.exp();
            let mf = m as f64;
            let density = r * (1.0 + r / mf).powf(-mf - 1.0);
            let se = (0.1875 / draws as f64).sqrt() / density;
            let q1 = quantile(&mut z, 0.25);
            println!(
                "M = {m}: closed form {closed:.5}, Monte Carlo {q1:.5} ± {se:.5} over {draws} draws"
            );
            assert!((q1 - closed).abs() < 4.0 * se, "M = {m}: {q1} vs {closed}");
        }
    }

    /// The indicator covariance against 2·10⁶ pairs of complex Gaussian bins with
    /// power correlation 4/9 and 1/36, within four standard errors; the series
    /// values are pinned beside it (a two-dimensional quadrature of Kibble's
    /// density agrees to 10⁻⁹).
    #[test]
    fn the_indicator_covariance_matches_a_monte_carlo() {
        let c1 = indicator_covariance(EXP_Q1, 4.0 / 9.0);
        let c2 = indicator_covariance(EXP_Q1, 1.0 / 36.0);
        assert!((c1 - 0.030_546_202).abs() < 1e-8, "{c1}");
        assert!((c2 - 0.001_320_007).abs() < 1e-8, "{c2}");
        assert_eq!(indicator_covariance(EXP_Q1, 0.0), 0.0);
        let pairs = 2_000_000usize;
        let mut rng = Gaussian::new(SEED + 2);
        for (r, series) in [(4.0f64 / 9.0, c1), (1.0 / 36.0, c2)] {
            let (rho, rest) = (r.sqrt(), (1.0 - r).sqrt());
            let mut both = 0u64;
            for _ in 0..pairs {
                let (a_re, a_im) = (f64::from(rng.next()), f64::from(rng.next()));
                let (w_re, w_im) = (f64::from(rng.next()), f64::from(rng.next()));
                let (b_re, b_im) = (rho * a_re + rest * w_re, rho * a_im + rest * w_im);
                let e1 = 0.5 * (a_re * a_re + a_im * a_im);
                let e2 = 0.5 * (b_re * b_re + b_im * b_im);
                both += u64::from(e1 <= EXP_Q1 && e2 <= EXP_Q1);
            }
            let p_both = both as f64 / pairs as f64;
            let se = (p_both * (1.0 - p_both) / pairs as f64).sqrt();
            let measured = p_both - 0.0625;
            println!("r = {r:.4}: series {series:.6}, Monte Carlo {measured:.6} ± {se:.6}");
            assert!((measured - series).abs() < 4.0 * se, "r = {r}: {measured}");
        }
    }

    /// `δ̂` over a known floor, `σ²·0.375·N` at every bin, on 400 independent
    /// white frames: its SD within 15 % of the derived `√Var δ̂` (the SD of an SD
    /// from 400 normal draws is 3.5 %), and its mean within four standard
    /// errors of 0.
    #[test]
    fn delta_scatters_as_derived() {
        let frames = 400usize;
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED + 3);
        let mu = (f64::from(SIGMA).powi(2) * 0.375 * N as f64) as f32;
        let mut ratios = Vec::with_capacity(N / 2);
        let mut deltas = Vec::with_capacity(frames);
        for _ in 0..frames {
            frame.compute(&noise(&mut rng, SIGMA));
            ratios.clear();
            ratios.extend(frame.mag[1..N / 2 - 1].iter().map(|&m| m * m / mu));
            deltas.push(location(&mut ratios, Z_Q1).expect("a location"));
        }
        let k = frames as f64;
        let mean = deltas.iter().sum::<f64>() / k;
        let sd = (deltas.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / (k - 1.0)).sqrt();
        let derived = delta_variance(LOCATION_BINS).sqrt();
        let independent = (0.1875 / LOCATION_BINS as f64).sqrt() / (0.75 * EXP_Q1);
        println!(
            "δ̂ over a known floor, {frames} frames: mean {mean:+.5} ({:+.4} dB), SD {sd:.5} \
             ({:.4} dB); derived {derived:.5} ({:.4} dB); independent bins would give \
             {independent:.5}",
            to_db(mean),
            to_db(sd),
            to_db(derived)
        );
        assert!((derived - 0.036_307).abs() < 1e-5, "{derived}");
        assert!((sd / derived - 1.0).abs() < 0.15, "SD {sd} vs {derived}");
        assert!(mean.abs() < 4.0 * sd / k.sqrt(), "mean {mean}");
    }

    /// Through the trait, on a floor learned from ten captures' runs of twelve
    /// hops (`M` = 34.5): white frames 6 dB above the learned level read `δ̂`
    /// within ±0.3 dB of +6 dB in median over 100 frames; the same frames at the
    /// learned level with 40 lines 40 dB above the noise, 5.11 bins apart from
    /// bin 100 to bin 299 (A0's partial spacing at this resolution), move `δ̂`
    /// by less than 0.3 dB in median against the same noise without them. The
    /// comb lifts a share `ε` of about 5 % of the bins clear of the quartile,
    /// which moves it by `ε/(4(1 − ε)f(q₁))`, about a quarter of a dB whatever
    /// the lines' level.
    #[test]
    fn delta_reads_a_rise_and_a_comb_barely() {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED + 4);
        let mut phases = Gaussian::new(SEED + 5);
        let cap = white();
        let mut arm = learned(1.0e-3, 10, &mut rng, &mut frame, &cap);
        assert_eq!(arm.runs, vec![RUN; 10]);
        let rise = 10f32.powf(6.0 / 20.0);
        let amp = tone_amplitude(SIGMA, 40.0, N);
        let (mut at_six, mut moved) = (Vec::new(), Vec::new());
        for i in 0..100 {
            let quiet = noise(&mut rng, SIGMA);
            let delta_of = |audio: &[f32], frame: &mut Frame, arm: &mut EmpiricalNull| {
                frame.compute(audio);
                let held = hop(i, Block::Holdout, audio, frame, &cap);
                arm.observe(&held);
                arm.delta(&held).expect("a location")
            };
            let plain = delta_of(&quiet, &mut frame, &mut arm);
            let louder: Vec<f32> = quiet.iter().map(|&x| x * rise).collect();
            at_six.push(f64::from(to_db(delta_of(&louder, &mut frame, &mut arm))));
            let mut comb = quiet;
            for j in 0..40 {
                let bin = 100.0 + 5.11 * j as f64;
                let phase = f64::from(phases.next()).atan2(f64::from(phases.next()));
                for (n, x) in comb.iter_mut().enumerate() {
                    let arg = std::f64::consts::TAU * bin * n as f64 / N as f64 + phase;
                    *x += amp * arg.cos() as f32;
                }
            }
            let with = delta_of(&comb, &mut frame, &mut arm);
            moved.push(f64::from(to_db(with - plain)));
        }
        let worst = moved.iter().fold(0.0f64, |a, &d| a.max(d.abs()));
        let (six, six_lo, six_hi) = (
            quantile(&mut at_six, 0.5),
            quantile(&mut at_six, 0.05),
            quantile(&mut at_six, 0.95),
        );
        let comb = quantile(&mut moved, 0.5);
        println!(
            "M = {:.1}: 6 dB up, δ̂ median {six:+.3} dB [5 %: {six_lo:+.3}, 95 %: {six_hi:+.3}] over \
             100 frames; 40 lines at 40 dB over bins 100–299 move it {comb:+.3} dB in median, \
             {worst:.3} dB at worst",
            arm.m_eff
        );
        assert!((six - 6.0).abs() < 0.3, "6 dB read as {six}");
        assert!(comb.abs() < 0.3, "the comb moves δ̂ by {comb} dB");
    }

    /// Through the trait at 10⁻², on a floor learned from a hundred captures'
    /// runs of twelve hops (`M` = 345). It declines until `MIN_HOPS` hops are
    /// learned. On 200 independent white frames at the learned level the share
    /// of bins at or above the G1 threshold is inside the 95 % Clopper–Pearson
    /// interval of its count, taken at the count's effective size: the bins
    /// over the ratio of the frames' measured count variance to the binomial's,
    /// with the frames as the independent units, since every bin of a frame
    /// shares that frame's `δ̂` and neighbouring Hann bins correlate. The rate
    /// the same frames give without the two corrections (the quartile against
    /// `ln ln(4/3)` and the multiplier at `M`) is printed beside it, and the G3
    /// estimate at 64 frequencies from 100 Hz to 15 kHz has a median within 5 %
    /// of the noise's exact median amplitude at the 4096-sample window.
    #[test]
    fn the_arm_holds_its_rate() {
        let p = 1.0e-2f32;
        let frames = 200usize;
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED + 6);
        let cap = white();
        let mut out = vec![f32::INFINITY; N / 2];

        // Declines through the first capture's seventh hop, answers at its eighth.
        let mut arm = EmpiricalNull::new();
        arm.prepare(&set_at(p));
        let stream: Vec<f32> = (0..N + (RUN - 1) * HOP_SIZE)
            .map(|_| SIGMA * rng.next())
            .collect();
        for h in 0..MIN_HOPS {
            let audio = &stream[h * HOP_SIZE..h * HOP_SIZE + N];
            frame.compute(audio);
            arm.observe(&hop(h as i64, Block::Build, audio, &frame, &cap));
            let scored = noise(&mut rng, SIGMA);
            frame.compute(&scored);
            let held = hop(100, Block::Holdout, &scored, &frame, &cap);
            arm.observe(&held);
            let answer = arm.g1_threshold(&held, &mut out);
            assert_eq!(
                answer == Availability::Available,
                h + 1 == MIN_HOPS,
                "after {} hops",
                h + 1
            );
        }

        let mut arm = learned(p, 100, &mut rng, &mut frame, &cap);
        let window = 4096;
        let hann = hann_vec(window);
        let truth = raw_to_gate(awgn_median_bin(SIGMA), window);
        let literal_multiplier = stats::f_2_2m_threshold(arm.m_eff, f64::from(p));
        let per_frame = (N / 2 - 2) as f64;
        let mut counts = Vec::with_capacity(frames);
        let mut literal_over = 0u64;
        let mut ratios = Vec::new();
        for i in 0..frames {
            let audio = noise(&mut rng, SIGMA);
            frame.compute(&audio);
            let held = hop(i as i64, Block::Holdout, &audio, &frame, &cap);
            arm.observe(&held);
            out.fill(f32::INFINITY);
            assert_eq!(arm.g1_threshold(&held, &mut out), Availability::Available);
            let delta = arm.current.flatten().expect("this hop's δ̂");
            let literal_delta = delta + arm.null_q1 - Z_Q1;
            let literal_scale = literal_delta.exp() * literal_multiplier;
            let mut over = 0u64;
            let scored = frame.mag.iter().zip(&out).zip(&arm.mean);
            for ((&m, &o), &mu) in scored.take(N / 2 - 1).skip(1) {
                over += u64::from(m >= o);
                literal_over += u64::from(f64::from(m * m) >= f64::from(mu) * literal_scale);
            }
            counts.push(over as f64);
            for k in 0..64 {
                let f = 100.0 * 150.0f32.powf(k as f32 / 63.0);
                let (amplitude, _) = goertzel_windowed(&audio, SAMPLE_RATE, f, &hann);
                let probe = GateProbe {
                    index: k,
                    target_hz: f,
                    window,
                    amplitude,
                    f_inst: None,
                    fixed: true,
                    truth_level: None,
                };
                let v = arm.gate(&held, &probe);
                ratios.push(f64::from(v.estimate.expect("an estimate") / truth));
            }
        }
        let bins = per_frame * frames as f64;
        let over: f64 = counts.iter().sum();
        let rate = over / bins;
        let mean = over / frames as f64;
        let var = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (frames - 1) as f64;
        let design = (var / (per_frame * rate * (1.0 - rate))).max(1.0);
        let effective = bins / design;
        let (lo, hi) = stats::clopper_pearson(
            (rate * effective).round() as u64,
            effective.round() as u64,
            0.05,
        );
        let literal = literal_over as f64 / bins;
        let estimate = quantile(&mut ratios, 0.5);
        println!(
            "M = {:.1}, M_c = {:.1}: G1 {over} / {bins} = {rate:.4e} ({:.3} of 1e-2); a frame's \
             count varies {design:.2} times as much as a binomial's, so 95 % CP [{lo:.4e}, \
             {hi:.4e}] at {effective:.0} bins; without the corrections {literal:.4e} \
             ({:.3}); G3 estimate over the exact median, median {estimate:.4}",
            arm.m_eff,
            arm.m_level,
            rate / 1.0e-2,
            literal / 1.0e-2
        );
        assert!(
            lo <= 1.0e-2 && 1.0e-2 <= hi,
            "1e-2 outside [{lo:e}, {hi:e}]"
        );
        assert!((estimate - 1.0).abs() < 0.05, "G3 estimate {estimate}");
    }
}
