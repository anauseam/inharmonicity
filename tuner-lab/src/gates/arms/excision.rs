//! Arm 5: forward consecutive mean excision (FCME) on the whitened frame. Each
//! hop's power is divided by the room's per-bin shape, FCME finds the bins that
//! hold only noise and their mean, and that mean, corrected for the excision and
//! for the whitening, times the shape is the level each bin's threshold is read
//! from.

use std::time::{Duration, Instant};

use super::{
    Arm, Availability, Block, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx,
    analytic_nu_half, raw_to_gate,
};
use crate::gates::stats;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};

/// Fewest silent hops learned before the arm answers: arm 1's count, since the
/// shape is arm 1's running mean.
const MIN_HOPS: usize = 8;

/// The initial clean set's share of the samples: "ca. 10 % of samples"
/// (Vartiainen et al. 2010, Table I).
const INITIAL_SET_FRACTION: f64 = 0.1;

/// Most passes before the excision stops unconverged; the bound caps a hop's
/// time.
const MAX_PASSES: usize = 20;

/// Contiguous bins of a Hann-windowed white-noise spectrum worth one independent
/// bin in a mean, `1 + 2(4/9 + 1/36)`: the window's `w²` has Fourier weights 3/8,
/// −1/4 and 1/16 at lags 0, 1 and 2 and none beyond, so neighbouring complex bins
/// correlate by −2/3 and the next by 1/6, and their powers by the squares.
const HANN_BINS_PER_INDEPENDENT: f64 = 35.0 / 18.0;

/// Mean of a unit exponential truncated at `tau`, `c(τ) = 1 − τe^{−τ}/(1 − e^{−τ})
/// = 1 − τ/(e^τ − 1)`: what the mean of an exponential sample below `τμ` reads,
/// in units of `μ`. `τ/2` near 0, rising to 1.
fn truncated_mean(tau: f64) -> f64 {
    1.0 - tau / tau.exp_m1()
}

/// The exponential mean `μ` of which `zeta_bar` is the mean of the sample below
/// `th`: the root of `ζ̄ = μ c(th/μ)`, by bisection on `τ = th/μ` over
/// `(0, th/ζ̄)`, where `c(τ)/τ` falls from ½ towards 0 and stays below `1/τ`.
/// `None` unless `0 < ζ̄/th < ½`, which `c(τ)/τ` cannot reach otherwise.
fn untruncated_mean(zeta_bar: f64, th: f64) -> Option<f64> {
    let r = zeta_bar / th;
    if r.is_nan() || r <= 0.0 || r >= 0.5 {
        return None;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0 / r);
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        if truncated_mean(mid) / mid > r {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Some(th / (0.5 * (lo + hi)))
}

/// Where FCME settles on exponential noise, in units of its mean: the root of
/// `τ = T c(τ)`, the truncation point at which `T` times the clean set's own mean
/// admits nothing more. 6.858 at `T = −ln 10⁻³` and 4.341 at `−ln 10⁻²`; for
/// `T ≤ 2` the only root is 0.
fn settled_tau(t: f64) -> f64 {
    let mut tau = t;
    for _ in 0..1000 {
        let next = t * truncated_mean(tau);
        if (next - tau).abs() <= 1e-15 * tau {
            return next;
        }
        tau = next;
    }
    tau
}

/// Mean of an `F(2, 2M)` variable truncated at `u`: the ratio of a unit
/// exponential to an independent unit-mean Gamma variable of shape `M` (a mean of
/// `M` exponentials when `M` is whole), whose survival is `(1 + r/M)^(−M)`. So
/// `E[R; R < u] = ∫₀ᵘ (1 + r/M)^(−M) dr − u(1 + u/M)^(−M)`, the integral being
/// `M/(M − 1)·(1 − (1 + u/M)^(1−M))` (`ln(1 + u)` at `M = 1`), over
/// `P(R < u) = 1 − (1 + u/M)^(−M)`.
fn f_2_2m_truncated_mean(u: f64, m: f64) -> f64 {
    let log1p = (u / m).ln_1p();
    let survival = (-m * log1p).exp();
    let integral = if (m - 1.0).abs() < 1e-9 {
        log1p
    } else {
        m / (m - 1.0) * (1.0 - ((1.0 - m) * log1p).exp())
    };
    (integral - u * survival) / (1.0 - survival)
}

/// `κ`: the level FCME reports on whitened noise, over the truth, when the shape
/// divided by is a mean of `m_shape` periodograms. A noise bin over such a shape
/// is `F(2, 2M_s)`-distributed about the shape's normaliser; FCME's passes settle
/// where `ζ̄ = E[R | R < T ζ̄]`, and the truncation correction, made for an
/// exponential, divides that by `c(τ*)`. 1 for a known shape (`m_shape`
/// infinite); 1.253 at `M_s = 3.45` and 1.025 at 34.5, for `T = −ln 10⁻³`. `NAN`
/// for `T ≤ 2`, where FCME has no level to settle on.
fn whitening_bias(m_shape: f64, t: f64) -> f64 {
    if t <= 2.0 {
        return f64::NAN;
    }
    if m_shape.is_infinite() {
        return 1.0;
    }
    // From below, as the passes run: 1/T is under every root.
    let mut zeta_bar = 1.0 / t;
    for _ in 0..1000 {
        let next = f_2_2m_truncated_mean(t * zeta_bar, m_shape);
        let settled = (next - zeta_bar).abs() <= 1e-14 * next;
        zeta_bar = next;
        if settled {
            break;
        }
    }
    zeta_bar / truncated_mean(settled_tau(t))
}

/// Equivalent independent periodograms behind one bin's level: the shape's
/// `m_shape` and the clean mean's `m_clean`, combined as their relative variances
/// add.
fn combined_m(m_shape: f64, m_clean: f64) -> f64 {
    1.0 / (1.0 / m_shape + 1.0 / m_clean)
}

/// One hop's excision over its whitened samples.
#[derive(Clone, Copy, Debug)]
struct Excision {
    /// The clean set's mean corrected for its truncation: `μ`, in the samples'
    /// units.
    mean: f64,
    /// Samples in the clean set, `Q`.
    clean: usize,
    /// Samples excised over.
    samples: usize,
    /// Passes made; the last admitted nothing unless `capped`.
    passes: usize,
    /// Whether the last pass still admitted samples.
    capped: bool,
}

/// FCME over `sorted`, ascending and non-negative: the initial clean set is the
/// lowest `INITIAL_SET_FRACTION`, and each pass admits every sample below
/// `Th = T ζ̄` of the set so far, until a pass admits none or `MAX_PASSES` have
/// run. The set is then the sample below the last `Th`, and its mean is
/// corrected for that truncation. `None` for an empty input or a set the
/// correction cannot invert.
fn excise(sorted: &[f32], t: f64) -> Option<Excision> {
    let n = sorted.len();
    if n == 0 {
        return None;
    }
    let mut q = ((n as f64 * INITIAL_SET_FRACTION).ceil() as usize).clamp(1, n);
    let mut sum: f64 = sorted[..q].iter().map(|&x| f64::from(x)).sum();
    let mut passes = 0;
    let (th, capped) = loop {
        passes += 1;
        let th = t * sum / q as f64;
        let next = q + sorted[q..].partition_point(|&x| f64::from(x) < th);
        if next == q {
            break (th, false);
        }
        sum += sorted[q..next].iter().map(|&x| f64::from(x)).sum::<f64>();
        q = next;
        if passes == MAX_PASSES {
            break (th, true);
        }
    };
    Some(Excision {
        mean: untruncated_mean(sum / q as f64, th)?,
        clean: q,
        samples: n,
        passes,
        capped,
    })
}

/// One hop's noise level in whitened units, and what its threshold takes.
#[derive(Clone, Copy, Debug)]
struct Level {
    /// Mean noise power per unit of shape: a bin's mean power is its shape
    /// times this, in the raw frame's units.
    power: f64,
    /// Equivalent independent periodograms behind a bin's level, `M`.
    m: f64,
    /// The upper `P_fa` point of `F(2, 2M)`, the threshold over the level in
    /// power.
    multiplier: f64,
}

/// The level of one whitened frame, its samples sorted ascending, under a shape
/// of `m_shape` periodograms that FCME reads high by `bias`; with the excision
/// it came from. `None` where the excision finds no level.
fn level_of(
    sorted: &[f32],
    t: f64,
    p_fa: f64,
    m_shape: f64,
    bias: f64,
) -> Option<(Level, Excision)> {
    let ex = excise(sorted, t)?;
    let m = combined_m(m_shape, ex.clean as f64 / HANN_BINS_PER_INDEPENDENT);
    let level = Level {
        power: ex.mean / bias,
        m,
        multiplier: stats::f_2_2m_threshold(m, p_fa),
    };
    (level.power.is_finite() && level.multiplier.is_finite()).then_some((level, ex))
}

/// Excisions over one population of hops, as counts per value: the clean set's
/// size and the passes, and how many hops the pass cap stopped.
struct Tally {
    clean: Vec<u64>,
    passes: [u64; MAX_PASSES + 1],
    capped: u64,
    hops: u64,
    /// Most samples one excision ran over.
    samples: usize,
}

impl Tally {
    fn new(bins: usize) -> Self {
        Self {
            clean: vec![0; bins + 1],
            passes: [0; MAX_PASSES + 1],
            capped: 0,
            hops: 0,
            samples: 0,
        }
    }

    fn add(&mut self, ex: &Excision) {
        let last = self.clean.len() - 1;
        self.clean[ex.clean.min(last)] += 1;
        self.passes[ex.passes.min(MAX_PASSES)] += 1;
        self.capped += u64::from(ex.capped);
        self.hops += 1;
        self.samples = self.samples.max(ex.samples);
    }
}

/// The value at quantile `q` of a population kept as counts per value, the
/// upper one where `q` falls between two.
fn count_quantile(counts: &[u64], q: f64) -> usize {
    let total: u64 = counts.iter().sum();
    if total == 0 {
        return 0;
    }
    let rank = ((total - 1) as f64 * q).round() as u64;
    let mut seen = 0;
    for (value, &c) in counts.iter().enumerate() {
        seen += c;
        if seen > rank {
            return value;
        }
    }
    counts.len() - 1
}

/// Arm 5 (G1, G2, G3): FCME on the 8192-point frame's power over the room's
/// per-bin shape.
///
/// FCME separates energy samples into a clean set of noise and the outliers
/// above it without knowing the noise level. The samples are sorted and the
/// lowest tenth is the initial clean set; a pass admits every sample below
/// `Th = T ζ̄`, `ζ̄` the clean set's mean (Eq. 1), and passes repeat until one
/// admits nothing (Table I). `T` is fixed by the desired false-alarm probability
/// of one noise sample, `P_FA,DES = e^{−T}` for the exponential samples of a
/// Gaussian spectrum, so the paper's 2.3, 4.6 and 6.9 are 10⁻¹, 10⁻² and 10⁻³;
/// here `T = −ln P_fa`.
///
/// The samples are bins `1..len − 1` of the hop's power, each divided by the
/// shape: that bin's mean power over the Gatekeeper-silent hops the arm may learn
/// from, kept as arm 1 keeps its floor, over its mean across the same bins. One
/// level then serves a room whose noise is tilted. The level at bin `b`, its
/// mean noise power, is the shape there times the corrected clean mean, and
/// its threshold is the square root of that level times a multiplier. Three
/// corrections, each derived for Gaussian noise, set the level and the
/// multiplier:
///
/// - **Truncation.** The clean set is the sample below `Th`, so `ζ̄` reads the
///   exponential's mean `μ` times `c(Th/μ)`, `c(τ) = 1 − τe^{−τ}/(1 − e^{−τ})`,
///   and the level takes the `μ` that solves `ζ̄ = μ c(Th/μ)`. At convergence
///   `Th = T ζ̄`, so this is `ζ̄ / c(τ*)` with `τ* = T c(τ*)`: `c(τ*)` is 0.9928
///   at 10⁻³ and 0.9427 at 10⁻².
/// - **Whitening.** The shape at a bin is itself a mean of `M_s` periodograms,
///   so a whitened noise bin is `F(2, 2M_s)`-distributed rather than
///   exponential, and FCME reads it high by `κ(M_s)`, which the level is divided
///   by: at 10⁻³, 1.25 at the 3.45 periodograms of twelve contiguous hops and
///   1.03 at ten times that.
/// - **Finite N.** The multiplier is the upper `P_fa` point of `F(2, 2M)`
///   (Talbot & Thrane 2020), `M` the equivalent independent periodograms behind
///   the level: the shape's `M_s` (Percival & Walden's EDOF summed over its runs
///   of hops, or the measured ν/2 per hop where the set gives one) and the clean
///   set's own, its size over Hann's 35/18 bins per independent one, combined as
///   their relative variances add. The shape is modelled as Gamma with the
///   EDOF's variance, but a run of overlapping frames is a weighted sum of
///   exponentials with a lighter lower tail, so a shape from few short runs
///   puts the threshold high: from one run of twelve hops, noise crosses it at
///   about half of `P_fa` at 10⁻³.
///
/// A known shape, `M_s` infinite, leaves `κ = 1` and the clean set's count
/// alone. G2 and G3 read the same level at the probe's bin, converted to the
/// Goertzel's window. Until `MIN_HOPS` silent hops have been learned the arm
/// declines every hop and every probe.
///
/// # References
///
/// - Vartiainen, J., Lehtomäki, J., Saarnisaari, H. & Juntti, M. (2010). Limits
///   of detection for the consecutive mean excision algorithms. Proc. 5th
///   CROWNCOM. DOI: 10.4108/ICST.CROWNCOM2010.9177. Eq. 1, Table I, §IV.
///   Lineage, as cited there: Henttu, P. & Aromaa, S. (2002), Consecutive mean
///   excision algorithm, Proc. IEEE ISSSTA, pp. 450–454; Saarnisaari, H.,
///   Henttu, P. & Juntti, M. (2005), Iterative multidimensional impulse
///   detectors for communications based on the classical diagnostic methods,
///   IEEE Trans. Commun. 53(3), pp. 395–398.
/// - Talbot & Thrane (2020) for the `F(2, 2M)` law of a level averaged from `M`
///   periodograms; Percival & Walden (1993) for the equivalent degrees of
///   freedom of overlapped Hann frames.
pub(super) struct Fcme {
    /// Σ|X|² per bin over the hops learned from, in the raw frame's units.
    sum: Vec<f64>,
    hops: usize,
    /// Lengths of the contiguous runs of hops learned from, the open run last.
    runs: Vec<usize>,
    last_hop: Option<i64>,
    /// ν/2 per hop of averaging as Stage B measured it, or the analytic value.
    nu_half_per_hop: Option<f32>,
    /// `sum` over its mean across bins `1..len − 1`, refreshed when a hop is
    /// learned.
    shape: Vec<f32>,
    /// The shape's equivalent independent periodograms, `M_s`.
    m_shape: f64,
    /// `κ(M_s)` at the operating point.
    bias: f64,
    p_fa: f64,
    /// FCME's threshold parameter, `T = −ln P_fa`.
    t: f64,
    /// The hop's whitened samples, sorted in place.
    sorted: Vec<f32>,
    /// This hop's level once a consumer has asked for it, `None` before then.
    current: Option<Option<Level>>,
    /// Excisions of the scored pre-roll and of the note.
    preroll: Tally,
    note: Tally,
    /// Hops asked for before `MIN_HOPS` were learned, and hops the excision
    /// found no level on.
    declined: u64,
    failed: u64,
    /// Time spent whitening, sorting, excising and filling the threshold, and
    /// the hops it served.
    cost: Duration,
    timed_hops: u64,
}

impl Fcme {
    /// An arm with nothing learned, at 10⁻³ until `prepare` sets the operating
    /// point.
    pub(super) fn new() -> Self {
        let bins = BASS_WINDOW_SIZE / 2;
        let p_fa = 1.0e-3f64;
        Self {
            sum: vec![0.0; bins],
            hops: 0,
            runs: Vec::new(),
            last_hop: None,
            nu_half_per_hop: None,
            shape: vec![0.0; bins],
            m_shape: 0.0,
            bias: f64::NAN,
            p_fa,
            t: -p_fa.ln(),
            sorted: Vec::with_capacity(bins),
            current: None,
            preroll: Tally::new(bins),
            note: Tally::new(bins),
            declined: 0,
            failed: 0,
            cost: Duration::ZERO,
            timed_hops: 0,
        }
    }

    /// `M_s`, the shape and `κ(M_s)` from the hops learned so far.
    fn refresh(&mut self) {
        self.m_shape = match self.nu_half_per_hop {
            Some(per_hop) => f64::from(per_hop) * self.hops as f64,
            None => self
                .runs
                .iter()
                .map(|&k| f64::from(analytic_nu_half(k)))
                .sum(),
        };
        let len = self.sum.len();
        let mean = self.sum[1..len - 1].iter().sum::<f64>() / (len - 2) as f64;
        for (s, &p) in self.shape.iter_mut().zip(&self.sum) {
            *s = if mean > 0.0 { (p / mean) as f32 } else { 0.0 };
        }
        self.bias = whitening_bias(self.m_shape, self.t);
    }

    /// This hop's level: computed on the first consumer's request and kept for
    /// the others. `None` before `MIN_HOPS` hops have been learned.
    fn level(&mut self, hop: &HopCtx) -> Option<Level> {
        if let Some(known) = self.current {
            return known;
        }
        let level = if self.hops < MIN_HOPS {
            self.declined += 1;
            None
        } else {
            let start = Instant::now();
            let len = hop.mag.len().min(self.shape.len());
            let shape = &self.shape;
            self.sorted.clear();
            self.sorted.extend(
                (1..len - 1)
                    .filter(|&b| shape[b] > 0.0)
                    .map(|b| hop.mag[b] * hop.mag[b] / shape[b]),
            );
            self.sorted.sort_unstable_by(f32::total_cmp);
            let found = level_of(&self.sorted, self.t, self.p_fa, self.m_shape, self.bias);
            match &found {
                Some((_, ex)) if hop.block == Block::Note => self.note.add(ex),
                Some((_, ex)) => self.preroll.add(ex),
                None => self.failed += 1,
            }
            self.cost += start.elapsed();
            self.timed_hops += 1;
            found.map(|(level, _)| level)
        };
        self.current = Some(level);
        level
    }
}

impl Arm for Fcme {
    fn name(&self) -> &str {
        "fcme"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.sum.iter_mut().for_each(|s| *s = 0.0);
        self.shape.iter_mut().for_each(|s| *s = 0.0);
        self.hops = 0;
        self.runs.clear();
        self.last_hop = None;
        self.nu_half_per_hop = set.nu_half_per_frame;
        self.m_shape = 0.0;
        self.p_fa = f64::from(set.p_fa);
        self.t = -self.p_fa.ln();
        self.bias = f64::NAN;
        self.current = None;
        let bins = self.sum.len();
        self.preroll = Tally::new(bins);
        self.note = Tally::new(bins);
        self.declined = 0;
        self.failed = 0;
        self.cost = Duration::ZERO;
        self.timed_hops = 0;
        let tau = settled_tau(self.t);
        println!(
            "[{}] arm fcme: T = −ln P_fa = {:.3}, initial clean set the lowest {:.0} % of bins 1–{}, \
             at most {MAX_PASSES} passes; on exponential noise the excision settles at {:.3} × the \
             mean, where the clean mean reads {:.4} of it; the shape starts empty and learns from \
             the build half of each capture's silence; ν/2 per frame {}",
            set.label,
            self.t,
            INITIAL_SET_FRACTION * 100.0,
            bins - 2,
            tau,
            truncated_mean(tau),
            match set.nu_half_per_frame {
                Some(v) => format!("{v:.3} (Stage B)"),
                None => "analytic Hann, summed over runs".to_string(),
            }
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
            "[{}] arm fcme: {} hops excised; {} asked for before {MIN_HOPS} hops were learned, {} \
             where the excision found no level",
            set.label, self.timed_hops, self.declined, self.failed
        );
        for (label, tally) in [("scored pre-roll", &self.preroll), ("note", &self.note)] {
            if tally.hops == 0 {
                continue;
            }
            let most_passes = tally.passes.iter().rposition(|&c| c > 0).unwrap_or(0);
            println!(
                "     {label}: clean set median {} of {} bins [5 %: {}, 95 %: {}]; passes median {} \
                 (most {most_passes}); the {MAX_PASSES}-pass cap reached on {} of {} hops",
                count_quantile(&tally.clean, 0.5),
                tally.samples,
                count_quantile(&tally.clean, 0.05),
                count_quantile(&tally.clean, 0.95),
                count_quantile(&tally.passes, 0.5),
                tally.capped,
                tally.hops
            );
        }
        println!(
            "     the shape at the end: {} hops in {} runs, M_s = {:.1}, κ(M_s) = {:.4}",
            self.hops,
            self.runs.len(),
            self.m_shape,
            self.bias
        );
        if self.timed_hops > 0 {
            let hop_ms = HOP_SIZE as f64 / SAMPLE_RATE as f64 * 1.0e3;
            println!(
                "     whitening, sort, excision and the G1 threshold: {:.0} µs per hop over {} hops, \
                 against the {hop_ms:.1} ms hop",
                self.cost.as_secs_f64() * 1.0e6 / self.timed_hops as f64,
                self.timed_hops
            );
        }
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        let Some(level) = self.level(hop) else {
            return Availability::Unavailable;
        };
        let start = Instant::now();
        let scale = level.power * level.multiplier;
        let len = out.len().min(self.shape.len());
        for (o, &s) in out.iter_mut().zip(&self.shape).take(len - 1).skip(1) {
            if s > 0.0 {
                *o = (f64::from(s) * scale).sqrt() as f32;
            }
        }
        self.cost += start.elapsed();
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let Some(level) = self.level(hop) else {
            return GateVerdict::default();
        };
        let bin = (probe.target_hz / hop.hz_per_bin).round() as usize;
        if !(1..self.shape.len() - 1).contains(&bin) || self.shape[bin] <= 0.0 {
            return GateVerdict::default();
        }
        // The bin's mean noise power, in the raw frame's units.
        let power = f64::from(self.shape[bin]) * level.power;
        let threshold = raw_to_gate((power * level.multiplier).sqrt() as f32, probe.window);
        let median = stats::rayleigh_median(power);
        // The level is a mean of M periodograms: its 5–95 % band in power, by the
        // normal approximation at these M.
        let rel = 1.645 / level.m.sqrt();
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
    const FRAMES: usize = 200;
    const SEED: u64 = 20261009;

    fn set_at(p_fa: f32) -> SetCtx<'static> {
        SetCtx {
            label: "test",
            p_fa,
            sigma_override: None,
            nu_half_per_frame: None,
        }
    }

    /// The mean power of one bin of the Hann frame on white noise at `SIGMA`,
    /// `σ²·0.375·N`.
    fn bin_power() -> f64 {
        f64::from(SIGMA).powi(2) * 0.375 * N as f64
    }

    /// A unit exponential: half the sum of two squared standard normals.
    fn exponential(rng: &mut Gaussian) -> f64 {
        let (a, b) = (f64::from(rng.next()), f64::from(rng.next()));
        0.5 * (a * a + b * b)
    }

    /// `N` samples of white Gaussian noise at `SIGMA`.
    fn noise(rng: &mut Gaussian) -> Vec<f32> {
        (0..N).map(|_| SIGMA * rng.next()).collect()
    }

    /// Bins `1..len − 1` of `mag` as powers, ascending: the frame over a flat
    /// shape.
    fn sorted_powers(mag: &[f32], out: &mut Vec<f32>) {
        out.clear();
        out.extend(mag[1..mag.len() - 1].iter().map(|&m| m * m));
        out.sort_unstable_by(f32::total_cmp);
    }

    /// The upper median.
    fn median(v: &mut [f64]) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
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

    /// The truncation correction's closed form against a Monte Carlo at
    /// `Th = 4.6μ`: the mean of the values below 4.6 among 2·10⁶ unit
    /// exponentials within four standard errors of `c(4.6) = 0.95329`, and the
    /// inversion returning `μ = 1` from it.
    #[test]
    fn truncation_correction_matches_a_monte_carlo() {
        let tau = 4.6;
        let mut rng = Gaussian::new(SEED);
        let (mut n, mut sum, mut sum2) = (0u64, 0.0f64, 0.0f64);
        for _ in 0..2_000_000 {
            let x = exponential(&mut rng);
            if x < tau {
                n += 1;
                sum += x;
                sum2 += x * x;
            }
        }
        let mean = sum / n as f64;
        let se = ((sum2 / n as f64 - mean * mean) / n as f64).sqrt();
        let closed = truncated_mean(tau);
        println!(
            "c(4.6): closed form {closed:.6}, Monte Carlo {mean:.6} ± {se:.6} over the {n} values below 4.6"
        );
        assert!((closed - 0.953_292_055).abs() < 1e-9, "{closed}");
        assert!((mean - closed).abs() < 4.0 * se, "{mean} vs {closed}");
        let mu = untruncated_mean(closed, tau).expect("an exponential mean");
        assert!((mu - 1.0).abs() < 1e-12, "{mu}");
    }

    /// White noise through the 8192-point Hann FFT over a flat shape: the
    /// corrected clean mean within 1 % of `σ²·0.375·N` in median over 200 frames
    /// at 10⁻² and at 10⁻³, and at 10⁻² the share of bins at or above their
    /// threshold inside the 95 % Clopper–Pearson interval of its count. The bins
    /// are not independent trials: neighbouring Hann bins' powers correlate by
    /// 4/9, and every bin of a frame shares that frame's estimated level. So the
    /// interval is taken at the count's effective size, the bins over the ratio
    /// of the frames' measured count variance to the binomial's, with the frames
    /// as the independent units; the ratio and the interval a binomial count
    /// would have are printed beside it.
    #[test]
    fn the_level_reads_white_noise() {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED);
        let mut sorted = Vec::with_capacity(N / 2);
        let operating = [1.0e-2f64, 1.0e-3];
        let mut corrected: [Vec<f64>; 2] = Default::default();
        let mut uncorrected: [Vec<f64>; 2] = Default::default();
        let mut clean: [Vec<f64>; 2] = Default::default();
        let mut counts = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            frame.compute(&noise(&mut rng));
            sorted_powers(&frame.mag, &mut sorted);
            for (i, &p) in operating.iter().enumerate() {
                let (level, ex) =
                    level_of(&sorted, -p.ln(), p, f64::INFINITY, 1.0).expect("a level");
                let zeta_bar = sorted[..ex.clean]
                    .iter()
                    .map(|&x| f64::from(x))
                    .sum::<f64>()
                    / ex.clean as f64;
                corrected[i].push(level.power / bin_power());
                uncorrected[i].push(zeta_bar / bin_power());
                clean[i].push(ex.clean as f64);
                if i == 0 {
                    let out = (level.power * level.multiplier).sqrt() as f32;
                    let scored = &frame.mag[1..N / 2 - 1];
                    counts.push(scored.iter().filter(|&&m| m >= out).count() as f64);
                }
            }
        }
        let rows = operating
            .iter()
            .zip(corrected.iter_mut())
            .zip(uncorrected.iter_mut().zip(clean.iter_mut()));
        for ((p, c), (u, q)) in rows {
            let c = median(c);
            println!(
                "P_fa {p:.0e}: clean mean over σ²·0.375·N, median of {FRAMES} frames: corrected {c:.4}, \
                 uncorrected {:.4}; clean set median {:.0} of {} bins",
                median(u),
                median(q),
                N / 2 - 2
            );
            assert!((c - 1.0).abs() < 0.01, "P_fa {p:e}: {c}");
        }
        let p = 1.0e-2;
        let per_frame = (N / 2 - 2) as f64;
        let bins = per_frame * FRAMES as f64;
        let over: f64 = counts.iter().sum();
        let rate = over / bins;
        let mean = over / FRAMES as f64;
        let var = counts.iter().map(|c| (c - mean).powi(2)).sum::<f64>() / (FRAMES - 1) as f64;
        let design = (var / (per_frame * rate * (1.0 - rate))).max(1.0);
        let effective = bins / design;
        let (lo, hi) = stats::clopper_pearson(
            (rate * effective).round() as u64,
            effective.round() as u64,
            0.05,
        );
        let (naive_lo, naive_hi) = stats::clopper_pearson(over as u64, bins as u64, 0.05);
        println!(
            "P_fa 1e-2: {over} / {bins} bins = {rate:.4e}; a frame's count varies {design:.2} times as \
             much as a binomial's, so the count is worth {effective:.0} independent bins: 95 % CP \
             [{lo:.4e}, {hi:.4e}] (as a binomial count [{naive_lo:.4e}, {naive_hi:.4e}])"
        );
        assert!(lo <= p && p <= hi, "1e-2 outside [{lo:e}, {hi:e}]");
    }

    /// The same frames with 40 lines 40 dB above the noise, 5.11 bins apart from
    /// bin 100 to bin 299 (A0's partial spacing at this resolution): at 10⁻³
    /// every frame's corrected clean mean within 0.5 dB of the same frame's
    /// without the lines.
    #[test]
    fn dense_lines_leave_the_level() {
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED);
        let mut phases = Gaussian::new(SEED + 1);
        let amp = tone_amplitude(SIGMA, 40.0, N);
        let p = 1.0e-3f64;
        let t = -p.ln();
        let mut sorted = Vec::with_capacity(N / 2);
        let mut shift_db = Vec::with_capacity(FRAMES);
        let mut plain_db = Vec::with_capacity(FRAMES);
        let mut clean = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let quiet = noise(&mut rng);
            frame.compute(&quiet);
            sorted_powers(&frame.mag, &mut sorted);
            let (alone, _) = level_of(&sorted, t, p, f64::INFINITY, 1.0).expect("a level");
            let mut loud = quiet.clone();
            for j in 0..40 {
                let bin = 100.0 + 5.11 * j as f64;
                let phase = f64::from(phases.next()).atan2(f64::from(phases.next()));
                for (n, x) in loud.iter_mut().enumerate() {
                    let arg = std::f64::consts::TAU * bin * n as f64 / N as f64 + phase;
                    *x += amp * arg.cos() as f32;
                }
            }
            frame.compute(&loud);
            sorted_powers(&frame.mag, &mut sorted);
            let (with, ex) = level_of(&sorted, t, p, f64::INFINITY, 1.0).expect("a level");
            shift_db.push(10.0 * (with.power / alone.power).log10());
            let plain = sorted.iter().map(|&x| f64::from(x)).sum::<f64>() / sorted.len() as f64;
            plain_db.push(10.0 * (plain / alone.power).log10());
            clean.push(ex.clean as f64);
        }
        let worst = shift_db.iter().fold(0.0f64, |a, &d| a.max(d.abs()));
        println!(
            "40 lines at 40 dB over bins 100–299: the level moves {:+.3} dB in median, {worst:.3} dB at \
             worst over {FRAMES} frames; clean set median {:.0} of {} bins; the plain mean of every \
             bin would read {:+.1} dB",
            median(&mut shift_db),
            median(&mut clean),
            N / 2 - 2,
            median(&mut plain_db)
        );
        assert!(worst < 0.5, "the lines move the level by {worst} dB");
    }

    /// `κ` against the excision itself on `F(2, 2M)` samples, each an exponential
    /// over a mean of `M` exponentials, at whole `M` = 4 and 12 and
    /// `T = −ln 10⁻³`: the mean corrected level over 300 sets of 4094 within four
    /// standard errors of the closed form. The fixed points and the values the
    /// doc comments state are pinned beside it.
    #[test]
    fn whitening_bias_matches_the_excision() {
        let t = -(1.0e-3f64).ln();
        let t2 = -(1.0e-2f64).ln();
        assert!(
            (settled_tau(t) - 6.857_909).abs() < 1e-5,
            "{}",
            settled_tau(t)
        );
        assert!((truncated_mean(settled_tau(t)) - 0.992_784).abs() < 1e-6);
        assert!(
            (settled_tau(t2) - 4.341_481).abs() < 1e-5,
            "{}",
            settled_tau(t2)
        );
        assert!((truncated_mean(settled_tau(t2)) - 0.942_741).abs() < 1e-6);
        assert_eq!(whitening_bias(f64::INFINITY, t), 1.0);
        assert!((whitening_bias(1.0e9, t) - 1.0).abs() < 1e-6);
        assert!(whitening_bias(1.0e3, 2.0).is_nan());
        let one_run = f64::from(analytic_nu_half(12));
        let kappa_one = whitening_bias(one_run, t);
        let kappa_ten = whitening_bias(10.0 * one_run, t);
        println!("M_s = {one_run:.4}: κ = {kappa_one:.5}; ten runs: κ = {kappa_ten:.5}");
        assert!((kappa_one - 1.252_62).abs() < 1e-4, "{kappa_one}");
        assert!((kappa_ten - 1.025_16).abs() < 1e-4, "{kappa_ten}");
        let mut rng = Gaussian::new(SEED);
        let mut sorted = Vec::with_capacity(4094);
        for (m, pinned) in [(4usize, 1.218_389), (12, 1.072_701)] {
            let closed = whitening_bias(m as f64, t);
            assert!((closed - pinned).abs() < 1e-5, "κ({m}) = {closed}");
            let mut levels = Vec::with_capacity(300);
            for _ in 0..300 {
                sorted.clear();
                for _ in 0..4094 {
                    let g = (0..m).map(|_| exponential(&mut rng)).sum::<f64>() / m as f64;
                    sorted.push((exponential(&mut rng) / g) as f32);
                }
                sorted.sort_unstable_by(f32::total_cmp);
                levels.push(excise(&sorted, t).expect("a level").mean);
            }
            let k = levels.len() as f64;
            let mean = levels.iter().sum::<f64>() / k;
            let se =
                (levels.iter().map(|l| (l - mean).powi(2)).sum::<f64>() / (k - 1.0) / k).sqrt();
            println!(
                "κ({m}): closed form {closed:.4}, the excision on F(2, {}) samples {mean:.4} ± {se:.4}",
                2 * m
            );
            assert!(
                (mean - closed).abs() < 4.0 * se,
                "κ({m}): {mean} vs {closed}"
            );
        }
    }

    /// The arm through its trait on white noise. It declines until `MIN_HOPS`
    /// hops are learned. After ten captures' silence in runs of twelve
    /// contiguous hops (`M_s` = 34.5), the bins at or above the G1 threshold over
    /// 200 independent frames come within 10 % of 10⁻², and the G3 estimate at
    /// 64 frequencies from 100 Hz to 15 kHz has a median within 5 % of the
    /// noise's exact median amplitude at the 4096-sample window. The rate the
    /// same hops give with the shape's own count left out of the law is printed
    /// beside it.
    #[test]
    fn the_arm_learns_its_shape_and_holds_its_rate() {
        let p = 1.0e-2f32;
        let mut arm = Fcme::new();
        arm.prepare(&set_at(p));
        let (ns, f_meas) = ([1usize], [440.0f32]);
        let cap = CaptureCtx {
            dir: "white",
            key: 48,
            register: 1,
            ns: &ns,
            f_meas: &f_meas,
            spacing_hz: 440.0,
            b: 0.0,
            sigma_rec: SIGMA,
        };
        let mut planner = RealFftPlanner::<f32>::new();
        let mut frame = Frame::new(&mut planner);
        let mut rng = Gaussian::new(SEED + 2);
        let mut out = vec![f32::INFINITY; N / 2];
        for capture in 0..10 {
            arm.begin_capture(&cap);
            let stream: Vec<f32> = (0..N + 11 * HOP_SIZE).map(|_| SIGMA * rng.next()).collect();
            for h in 0..12 {
                let audio = &stream[h * HOP_SIZE..h * HOP_SIZE + N];
                frame.compute(audio);
                arm.observe(&hop(h as i64 - 40, Block::Build, audio, &frame, &cap));
                let learned = h + 1;
                if capture == 0 && (learned == MIN_HOPS - 1 || learned == MIN_HOPS) {
                    let scored = noise(&mut rng);
                    frame.compute(&scored);
                    let held = hop(0, Block::Holdout, &scored, &frame, &cap);
                    arm.observe(&held);
                    let answer = arm.g1_threshold(&held, &mut out);
                    assert_eq!(
                        answer == Availability::Available,
                        learned == MIN_HOPS,
                        "after {learned} hops"
                    );
                }
            }
        }
        assert_eq!(arm.runs, vec![12; 10]);
        let window = 4096;
        let hann = hann_vec(window);
        let truth = raw_to_gate(awgn_median_bin(SIGMA), window);
        let (mut bins, mut over, mut over_known) = (0u64, 0u64, 0u64);
        let (mut probes, mut admitted) = (0u64, 0u64);
        let mut ratios = Vec::new();
        for i in 0..FRAMES {
            let audio = noise(&mut rng);
            frame.compute(&audio);
            let held = hop(i as i64, Block::Holdout, &audio, &frame, &cap);
            arm.observe(&held);
            out.fill(f32::INFINITY);
            assert_eq!(arm.g1_threshold(&held, &mut out), Availability::Available);
            // The law with the shape taken as known: no κ, and M the clean set's.
            let ex = excise(&arm.sorted, arm.t).expect("a level");
            let known = ex.mean
                * stats::f_2_2m_threshold(ex.clean as f64 / HANN_BINS_PER_INDEPENDENT, arm.p_fa);
            let scored = frame.mag.iter().zip(&out).zip(&arm.shape);
            for ((&m, &o), &s) in scored.take(N / 2 - 1).skip(1) {
                bins += 1;
                over += u64::from(m >= o);
                over_known += u64::from(f64::from(m * m) >= f64::from(s) * known);
            }
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
                probes += 1;
                admitted += u64::from(v.admit == Some(true));
            }
        }
        let rate = over as f64 / bins as f64;
        let (lo, hi) = stats::clopper_pearson(admitted, probes, 0.05);
        let estimate = median(&mut ratios);
        println!(
            "M_s = {:.1}, κ = {:.4}: G1 {over} / {bins} = {rate:.4e} ({:.3} of 1e-2); with the shape \
             taken as known {:.4e} ({:.3}); G3 {admitted} / {probes} = {:.3e}, 95 % CP \
             [{lo:.2e}, {hi:.2e}]; estimate over the exact median, median {estimate:.4}",
            arm.m_shape,
            arm.bias,
            rate / 1.0e-2,
            over_known as f64 / bins as f64,
            over_known as f64 / bins as f64 / 1.0e-2,
            admitted as f64 / probes as f64
        );
        assert!((rate / 1.0e-2 - 1.0).abs() < 0.1, "G1 rate {rate:e}");
        assert!((estimate - 1.0).abs() < 0.05, "G3 estimate {estimate}");
    }
}
