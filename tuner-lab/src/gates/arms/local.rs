//! The frequency-local family: the noise at a frequency read from the spectrum
//! around it, in the same window. A Goertzel probe against the same Goertzel at
//! neighbouring frequencies (G2, G3); Laurenti, De Poli & Montagner's
//! reciprocal-mean floor along a frame's own spectrum (G1, BG); and the median
//! over the inter-partial gaps flanking each partial on the 2¹⁶-point frame,
//! with the acceptance margins it may be read against (BG).

use std::collections::HashMap;
use std::f32::consts::{FRAC_PI_4, LN_2};
use std::time::{Duration, Instant};

use super::{
    Arm, Availability, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, LOBE_HALF_BINS,
    SetCtx, median_to_threshold,
};
use crate::gates::ambient;
use crate::gates::truth::Gaussian;
use crate::truth::{self, SAMPLE_RATE};
use tuner_core::algorithms::spectral;

// ── the neighbour bins ───────────────────────────────────────────────────────

/// Reference cells per probe.
const CELLS: usize = 10;

/// Offsets of the reference cells from the target, in bins of the probe's own
/// window, ascending; ±1 lies inside the target's Hann main lobe.
const OFFSETS: [i32; CELLS] = [-6, -5, -4, -3, -2, 2, 3, 4, 5, 6];

/// Every cell kept, bit `i` standing for `OFFSETS[i]`: the geometry the null is
/// first stated for.
const ALL_CELLS: u16 = (1 << CELLS) - 1;

/// Fewest cells a reference may keep; with fewer the probe is declined.
const MIN_CELLS: usize = 4;

/// The Goertzel windows the register rule chooses between.
const WINDOWS: [usize; 2] = [1024, 4096];

/// Trials of the numerical null per window.
const NULL_TRIALS: usize = 40_000;

/// Seed of the null's noise; the window length is added to it.
const NULL_SEED: u64 = 0x5EED_5B09;

/// The frequency the null's target sits at. White noise has the same statistics
/// wherever the cells' main lobes clear DC and Nyquist, as they do around 1 kHz
/// at both windows.
const NULL_TARGET_HZ: f32 = 1000.0;

/// Highest partial number the exclusion looks at: above A0's last partial below
/// Nyquist.
const MAX_PARTIAL: usize = 1024;

/// White noise through one window's cell geometry: per trial, the target's
/// Goertzel amplitude and the ten cells' in `OFFSETS` order, in the `4/n` units
/// of unit-variance noise.
struct NeighbourNull {
    target: Vec<f32>,
    cells: Vec<[f32; CELLS]>,
    /// The noise's median Goertzel amplitude, `(4/n)·√(Σw²·ln 2)`: the Rayleigh
    /// median of a bin whose mean power is `Σw²`.
    median: f32,
}

/// What the null gives one subset of cells.
#[derive(Clone, Copy, Debug)]
struct Calibration {
    /// The multiplier on the cells' median at the operating point: the
    /// `1 − p_fa` quantile of `amplitude(target) / median(cells)`.
    k: f32,
    /// The 5 % and 95 % points of the cells' median over the noise's median
    /// amplitude.
    band: (f32, f32),
}

/// `trials` windows of `window` samples of unit-variance white noise, each
/// through the probe's Hann Goertzel at `NULL_TARGET_HZ` and at every cell.
fn neighbour_null(window: usize, trials: usize, seed: u64) -> NeighbourNull {
    let hann = truth::hann_vec(window);
    let hz = SAMPLE_RATE as f32 / window as f32;
    let mut rng = Gaussian::new(seed);
    let mut noise = vec![0.0f32; window];
    let mut target = Vec::with_capacity(trials);
    let mut cells = Vec::with_capacity(trials);
    for _ in 0..trials {
        noise.iter_mut().for_each(|x| *x = rng.next());
        target.push(spectral::goertzel_windowed(&noise, SAMPLE_RATE, NULL_TARGET_HZ, &hann).0);
        let mut c = [0.0f32; CELLS];
        for (slot, &d) in c.iter_mut().zip(&OFFSETS) {
            let f = NULL_TARGET_HZ + d as f32 * hz;
            *slot = spectral::goertzel_windowed(&noise, SAMPLE_RATE, f, &hann).0;
        }
        cells.push(c);
    }
    let energy: f64 = hann.iter().map(|&w| w as f64 * w as f64).sum();
    NeighbourNull {
        target,
        cells,
        median: (4.0 / window as f64 * (energy * std::f64::consts::LN_2).sqrt()) as f32,
    }
}

/// The upper median of the cells `mask` keeps (bit `i` for `OFFSETS[i]`): of `m`
/// cells, the order statistic of rank `⌊m/2⌋ + 1`. `NAN` for an empty mask.
fn masked_median(cells: &[f32; CELLS], mask: u16) -> f32 {
    let mut kept = [0.0f32; CELLS];
    let mut m = 0;
    for (i, &c) in cells.iter().enumerate() {
        if mask & (1 << i) != 0 {
            kept[m] = c;
            m += 1;
        }
    }
    if m == 0 {
        return f32::NAN;
    }
    *kept[..m].select_nth_unstable_by(m / 2, f32::total_cmp).1
}

/// The value at quantile `q` of the ascending `sorted`.
fn quantile(sorted: &[f32], q: f64) -> f32 {
    let i = (q * (sorted.len() - 1) as f64).round() as usize;
    sorted[i.min(sorted.len() - 1)]
}

/// The multiplier and the median's band for the cells `mask` keeps, at `p_fa`.
fn calibrate(null: &NeighbourNull, mask: u16, p_fa: f64) -> Calibration {
    let mut ratio = Vec::with_capacity(null.target.len());
    let mut spread = Vec::with_capacity(null.target.len());
    for (&t, c) in null.target.iter().zip(&null.cells) {
        let m = masked_median(c, mask);
        ratio.push(t / m);
        spread.push(m / null.median);
    }
    ratio.sort_by(f32::total_cmp);
    spread.sort_by(f32::total_cmp);
    Calibration {
        k: quantile(&ratio, 1.0 - p_fa),
        band: (quantile(&spread, 0.05), quantile(&spread, 0.95)),
    }
}

/// The cells around `target_hz` at `window` that stand clear of the key's other
/// partials: bit `i` keeps `OFFSETS[i]`. A cell is dropped inside the main lobe
/// (closer than two bins) of a predicted partial other than `own`, and within
/// two bins of DC or Nyquist, where its lobe meets its own mirror image.
/// `partials` is ascending in frequency.
fn cell_mask(target_hz: f32, window: usize, partials: &[(usize, f32)], own: usize) -> u16 {
    let hz = SAMPLE_RATE as f32 / window as f32;
    let lobe = LOBE_HALF_BINS as f32 * hz;
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let mut mask = 0u16;
    for (i, &d) in OFFSETS.iter().enumerate() {
        let f = target_hz + d as f32 * hz;
        if f < lobe || f > nyquist - lobe {
            continue;
        }
        let first = partials.partition_point(|&(_, p)| p <= f - lobe);
        let inside = partials[first..]
            .iter()
            .take_while(|&&(_, p)| p < f + lobe)
            .any(|&(n, _)| n != own);
        if !inside {
            mask |= 1 << i;
        }
    }
    mask
}

/// The key's partials `n·f₀·√(1 + Bn²)` below Nyquist as (number, Hz),
/// ascending in frequency.
fn predicted_partials(spacing_hz: f32, b: f32) -> Vec<(usize, f32)> {
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let mut partials: Vec<(usize, f32)> = (1..=MAX_PARTIAL)
        .map(|n| (n, ambient::stiff_partial(spacing_hz, b, n)))
        .take_while(|&(_, f)| f.is_finite() && f < nyquist)
        .collect();
    partials.sort_by(|a, b| a.1.total_cmp(&b.1));
    partials
}

/// The offsets `mask` keeps.
fn kept_offsets(mask: u16) -> Vec<i32> {
    OFFSETS
        .iter()
        .enumerate()
        .filter(|&(i, _)| mask & (1 << i) != 0)
        .map(|(_, &d)| d)
        .collect()
}

/// Wall time the cells' Goertzels and their median took at one window, over
/// the probes the gate answered.
#[derive(Default)]
struct Cost {
    probes: u64,
    goertzels: u64,
    time: Duration,
}

/// The noise at a probe read from the same Goertzel at neighbouring
/// frequencies. At the probe's own window `n` (1024 or 4096) and resolution
/// `fs/n`, Hann Goertzels on the same samples at ±2 … ±6 bins from the target
/// are the reference cells; ±1 lies inside the target's main lobe. A cell is
/// dropped inside the main lobe (closer than two bins) of a predicted partial
/// `n·f₀·√(1 + Bn²)` of the key other than the probe's own, or within two bins
/// of DC or Nyquist; with fewer than four cells left the probe is declined. The
/// level is the cells' median, the upper one for an even count, in the probe's
/// `4/n` units; the threshold is `k` times it.
///
/// The cells are not independent: Hann bins one apart carry a power correlation
/// of 4/9, two apart 1/36. So `k` is numerical: the `1 − p_fa` quantile of
/// `amplitude(target) / median(cells)` over white noise run through the same
/// geometry, simulated once per window and read again for each subset of cells
/// a probe keeps. The same trials give the 5 % and 95 % points of the median
/// over the noise's own median, and the estimate's interval is the median
/// divided by them. With all ten
/// cells `k` sits about 8 % above Rohling's value for ten independent cells at
/// the same rank; on the cells two bins apart the two agree.
///
/// Reading the noise at a test frequency from the bins on either side of it is
/// the otoacoustic-emission practice of Johnson et al. and Burke et al., who
/// average eleven bins of a two-buffer difference spectrum; the cells'
/// geometry, the median and the numerical null are ours.
///
/// # References
///
/// - Johnson, T. A. et al. (2010). Clinical test performance of
///   distortion-product otoacoustic emissions using new stimulus conditions.
///   Ear & Hearing 31(1), 74.
/// - Burke, S. E. et al. (2010). Influence of calibration method on
///   distortion-product otoacoustic emission measurements: I. Test performance.
///   Ear & Hearing 31(4), 533.
/// - Rohling, H. (1983). Radar CFAR thresholding in clutter and multiple target
///   situations. IEEE Trans. Aerospace and Electronic Systems, AES-19(4),
///   608–621. DOI: 10.1109/TAES.1983.309350. Eqs. 14, 17.
pub(super) struct Neighbours {
    p_fa: f32,
    /// The Hann window per Goertzel window length.
    hann: HashMap<usize, Vec<f32>>,
    nulls: HashMap<usize, NeighbourNull>,
    /// Every cell subset met so far, by (window, mask): its calibration and the
    /// probes it served.
    calibrations: HashMap<(usize, u16), (Calibration, u64)>,
    /// The capture's predicted partials, ascending in frequency.
    partials: Vec<(usize, f32)>,
    /// The gate's cost by window.
    cost: HashMap<usize, Cost>,
    /// Hops on which the gate answered a probe, and the last of them.
    hops: u64,
    last_hop: Option<i64>,
}

impl Neighbours {
    pub(super) fn new() -> Self {
        Self {
            p_fa: 1.0e-3,
            hann: WINDOWS.iter().map(|&w| (w, truth::hann_vec(w))).collect(),
            nulls: HashMap::new(),
            calibrations: HashMap::new(),
            partials: Vec::new(),
            cost: HashMap::new(),
            hops: 0,
            last_hop: None,
        }
    }

    /// The verdict on one probe, the Goertzels it took and their time.
    fn verdict(&mut self, hop: &HopCtx, probe: &GateProbe) -> (GateVerdict, u64, Duration) {
        let (Some(null), Some(hann)) =
            (self.nulls.get(&probe.window), self.hann.get(&probe.window))
        else {
            return (GateVerdict::default(), 0, Duration::ZERO);
        };
        let own = hop.capture.ns.get(probe.index).copied().unwrap_or(0);
        let mask = cell_mask(probe.target_hz, probe.window, &self.partials, own);
        let cells = mask.count_ones() as u64;
        if (cells as usize) < MIN_CELLS {
            return (GateVerdict::default(), 0, Duration::ZERO);
        }
        let start = Instant::now();
        let hz = SAMPLE_RATE as f32 / probe.window as f32;
        let mut amplitudes = [0.0f32; CELLS];
        for (i, (a, &d)) in amplitudes.iter_mut().zip(&OFFSETS).enumerate() {
            if mask & (1 << i) != 0 {
                let f = probe.target_hz + d as f32 * hz;
                *a = spectral::goertzel_windowed(hop.audio, SAMPLE_RATE, f, hann).0;
            }
        }
        let median = masked_median(&amplitudes, mask);
        let elapsed = start.elapsed();
        // Digital silence leaves no level to read against.
        if !(median > 0.0 && median.is_finite()) {
            return (GateVerdict::default(), cells, elapsed);
        }
        let p_fa = self.p_fa as f64;
        let entry = self
            .calibrations
            .entry((probe.window, mask))
            .or_insert_with(|| (calibrate(null, mask, p_fa), 0));
        entry.1 += 1;
        let Calibration { k, band: (lo, hi) } = entry.0;
        let threshold = k * median;
        let verdict = GateVerdict {
            threshold: Some(threshold),
            estimate: Some(median),
            interval: Some((median / hi, median / lo)),
            admit: Some(probe.amplitude >= threshold),
        };
        (verdict, cells, elapsed)
    }
}

impl Arm for Neighbours {
    fn name(&self) -> &str {
        "sb9"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = set.p_fa;
        self.calibrations.clear();
        self.cost.clear();
        self.hops = 0;
        self.last_hop = None;
        let rank = CELLS / 2 + 1;
        let rohling = truth::cfar_multiplier_finite(CELLS, rank, set.p_fa);
        for window in WINDOWS {
            let null = self
                .nulls
                .entry(window)
                .or_insert_with(|| neighbour_null(window, NULL_TRIALS, NULL_SEED + window as u64));
            let cal = calibrate(null, ALL_CELLS, set.p_fa as f64);
            println!(
                "[{}] arm sb9: numerical null at the {window}-sample window, k = {:.3} at {:.1e} over \
                 all {CELLS} cells ({NULL_TRIALS} trials; Rohling's value for {CELLS} independent \
                 cells at rank {rank}: {rohling:.3}); the median's 5–95 % band {:.3}–{:.3} of the \
                 noise's median",
                set.label, cal.k, set.p_fa, cal.band.0, cal.band.1
            );
            self.calibrations.insert((window, ALL_CELLS), (cal, 0));
        }
    }

    fn begin_capture(&mut self, cap: &CaptureCtx) {
        self.partials = predicted_partials(cap.spacing_hz, cap.b);
        self.last_hop = None;
    }

    fn finish(&mut self, set: &SetCtx) {
        let mut used: Vec<(usize, u16, Calibration, u64)> = self
            .calibrations
            .iter()
            .filter(|(_, (_, probes))| *probes > 0)
            .map(|(&(window, mask), &(cal, probes))| (window, mask, cal, probes))
            .collect();
        used.sort_by_key(|&(window, mask, _, _)| {
            (window, std::cmp::Reverse(mask.count_ones()), mask)
        });
        for (window, mask, cal, probes) in used {
            let cells = kept_offsets(mask);
            let m = cells.len();
            println!(
                "[{}] arm sb9: {window}-sample window, cells at {cells:?}: k = {:.3} (independent \
                 cells: {:.3}), the median's band {:.3}–{:.3}; {probes} probes",
                set.label,
                cal.k,
                truth::cfar_multiplier_finite(m, m / 2 + 1, self.p_fa),
                cal.band.0,
                cal.band.1
            );
        }
        let mut windows: Vec<(&usize, &Cost)> = self.cost.iter().collect();
        windows.sort_by_key(|&(&window, _)| window);
        let mut total = Duration::ZERO;
        for (window, c) in windows {
            total += c.time;
            println!(
                "[{}] arm sb9: {window}-sample window, {:.1} µs per probe over {} probes, {:.1} \
                 Goertzels each",
                set.label,
                c.time.as_secs_f64() * 1.0e6 / c.probes as f64,
                c.probes,
                c.goertzels as f64 / c.probes as f64
            );
        }
        if self.hops > 0 {
            println!(
                "[{}] arm sb9: {:.0} µs per hop over {} hops, both gates and every partial",
                set.label,
                total.as_secs_f64() * 1.0e6 / self.hops as f64,
                self.hops
            );
        }
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let (verdict, goertzels, elapsed) = self.verdict(hop, probe);
        if goertzels > 0 {
            let cost = self.cost.entry(probe.window).or_default();
            cost.probes += 1;
            cost.goertzels += goertzels;
            cost.time += elapsed;
            if self.last_hop != Some(hop.hop) {
                self.hops += 1;
                self.last_hop = Some(hop.hop);
            }
        }
        verdict
    }
}

// ── Laurenti's reciprocal mean ───────────────────────────────────────────────

/// Laurenti et al.'s `N_f`, the smoothing length in bins at their 1024-point
/// frame and 44.1 kHz (the captions of Figs. 6–10).
const N_F: usize = 21;

/// The frame length `N_F` is stated for.
const N_F_FRAME: usize = 1024;

/// Eq. 10's bias factor `k₁`: the estimate's level on noise alone over the mean
/// magnitude, for three independent bins averaged before the reciprocal.
const K1: f32 = 0.90;

/// The validity bound of §IV-C.3, `f_pitch > 8/T_w`: a partial spacing of eight
/// bins.
const MIN_SPACING_BINS: f32 = 8.0;

/// `N_f` at the same width in hertz on a frame of `frame` points: 168 bins at
/// 8192, 1344 at 65 536.
fn n_f(frame: usize) -> usize {
    N_F * frame / N_F_FRAME
}

/// Step 3 of Laurenti et al.: the frame's two-sided magnitude spectrum, periodic
/// in `N = 2·mag.len()`, each bin replaced by the mean of itself and its two
/// neighbours to remove the spectrum's incidental zeros (Eq. 2). `mag` holds
/// bins `0 … N/2 − 1`; the Nyquist bin it lacks takes its neighbour's value.
fn pre_average(mag: &[f32], out: &mut Vec<f64>) {
    let half = mag.len();
    let n = 2 * half;
    let two_sided = |i: usize| -> f64 {
        let b = if i < half {
            i
        } else if i == half {
            half - 1
        } else {
            n - i
        };
        mag[b] as f64
    };
    out.clear();
    out.extend(
        (0..n).map(|i| (two_sided((i + n - 1) % n) + two_sided(i) + two_sided((i + 1) % n)) / 3.0),
    );
}

/// Steps 4–6: the reciprocal of the periodic spectrum `s` (Eq. 3), its cyclic
/// mean over `n_f` bins (Eq. 4), and the reciprocal of that (Eq. 5), written for
/// the first `out.len()` bins. For an even `n_f` the window takes one bin more
/// below its centre than above, a half-bin shift against a window of 168 or
/// more.
fn reciprocal_mean(s: &[f64], n_f: usize, prefix: &mut Vec<f64>, out: &mut [f32]) {
    let n = s.len();
    debug_assert!(n_f <= n && out.len() <= n);
    prefix.clear();
    prefix.push(0.0);
    let mut acc = 0.0;
    for &v in s {
        acc += 1.0 / v;
        prefix.push(acc);
    }
    for (b, o) in out.iter_mut().enumerate() {
        let start = (b + n - n_f / 2) % n;
        let end = start + n_f;
        let sum = if end <= n {
            prefix[end] - prefix[start]
        } else {
            prefix[n] - prefix[start] + prefix[end - n]
        };
        *o = (n_f as f64 / sum) as f32;
    }
}

/// Whether a key's partial spacing clears the paper's bound at `hz_per_bin`.
fn spaced(spacing_hz: f32, hz_per_bin: f32) -> bool {
    spacing_hz / hz_per_bin >= MIN_SPACING_BINS
}

/// Laurenti, De Poli & Montagner's stochastic spectrum: the noise's amplitude
/// spectrum from one frame's own magnitudes, with no model of the partials.
/// Each bin of the two-sided magnitude spectrum, periodic in `N`, is averaged
/// with its two neighbours to remove incidental zeros (step 3, Eq. 2); the
/// reciprocal (Eq. 3), in which the partials are deep narrow notches, is
/// smoothed by a cyclic mean over `N_f` bins (Eq. 4); the reciprocal of that
/// (Eq. 5), divided by the bias factor `k₁ = 0.90` (Eq. 10), estimates the
/// noise's mean magnitude `m̄`. `N_f` is the paper's 21 bins at 1024 points and
/// 44.1 kHz at the same width in hertz: 168 bins on the 8192-point frame, 1344
/// on the 2¹⁶-point one.
///
/// A Rayleigh bin of mean magnitude `m̄` has mean power `σ_b² = m̄²/(π/4)` and
/// tail `exp(−T²/σ_b²)`, so G1's threshold at `p_fa` is
/// `m̄·√(ln(1/p_fa))/√(π/4)` and the level it reads, the median,
/// `m̄·√(ln 2)/√(π/4)`; BG's floor is that median at the partial's bin of the
/// 2¹⁶-point frame. No finite-N correction is applied: `N_f` is 168 cells or
/// more.
///
/// The paper's validity bound is `f_pitch > 8/T_w`, a partial spacing of eight
/// bins: G1 declines below it, A0 to E1 on the 8192-point frame, while on the
/// 2¹⁶-point frame every key clears it. Within the bound partials still lift
/// the floor, by `1/(1 − p)`
/// with `p` the share of the window's bins the estimator sees as line, a Hann
/// main lobe of four bins widened to six by the three-bin mean: +1.4 dB at A3,
/// +3.0 dB at A2 and +7.7 dB at A1 on the 8192-point frame, +1.4 dB at A0 on
/// the 2¹⁶-point one.
///
/// `k₁` is the paper's value for independent bins: three-bin means of
/// unit-mean Rayleigh magnitudes read 0.901. Hann bins one apart are
/// correlated, and on Hann frames of white noise the estimator reads 0.85 of
/// the mean magnitude before `k₁`, so the threshold sits about 5 % low: a
/// per-bin rate near 2 × 10⁻³ at a nominal 10⁻³.
///
/// # References
///
/// Laurenti, N., De Poli, G. & Montagner, D. (2007). A nonlinear method for
/// stochastic spectrum estimation in the modeling of musical sounds. IEEE Trans.
/// Audio, Speech, and Language Processing 15(2), 531–541.
/// DOI: 10.1109/TASL.2006.881685. §II steps 1–6, Eqs. 2–5, 10; §IV-C.3.
pub(super) struct Laurenti {
    p_fa: f32,
    /// The pre-averaged two-sided spectrum.
    two_sided: Vec<f64>,
    /// Prefix sums of its reciprocal.
    prefix: Vec<f64>,
    /// The mean magnitude per one-sided bin, `B/k₁`.
    mean_magnitude: Vec<f32>,
}

impl Laurenti {
    pub(super) fn new() -> Self {
        Self {
            p_fa: 1.0e-3,
            two_sided: Vec::new(),
            prefix: Vec::new(),
            mean_magnitude: Vec::new(),
        }
    }

    /// The noise's mean magnitude in every bin of the one-sided `mag`, into
    /// `self.mean_magnitude`.
    fn estimate(&mut self, mag: &[f32]) {
        pre_average(mag, &mut self.two_sided);
        self.mean_magnitude.resize(mag.len(), 0.0);
        reciprocal_mean(
            &self.two_sided,
            n_f(2 * mag.len()),
            &mut self.prefix,
            &mut self.mean_magnitude,
        );
        self.mean_magnitude.iter_mut().for_each(|m| *m /= K1);
    }
}

impl Arm for Laurenti {
    fn name(&self) -> &str {
        "laurenti"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::Bg]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = set.p_fa;
        println!(
            "[{}] arm laurenti: N_f = {} bins on the 8192-point frame, {} on the 2¹⁶-point one; \
             k₁ = {K1:.2}; G1's threshold {:.3} × the mean magnitude at {:.1e}; G1 declines below a \
             partial spacing of {MIN_SPACING_BINS:.0} bins",
            set.label,
            n_f(8192),
            n_f(65_536),
            (1.0 / set.p_fa).ln().sqrt() / FRAC_PI_4.sqrt(),
            set.p_fa
        );
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        if !spaced(hop.capture.spacing_hz, hop.hz_per_bin) {
            return Availability::Unavailable;
        }
        self.estimate(hop.mag);
        let scale = (1.0 / self.p_fa).ln().sqrt() / FRAC_PI_4.sqrt();
        for (o, &m) in out.iter_mut().zip(&self.mean_magnitude) {
            *o = m * scale;
        }
        Availability::Available
    }

    fn bg_floor(
        &mut self,
        cap: &CaptureCtx,
        spectrum: &[f32],
        hz_per_bin: f32,
        partials: &[f32],
        out: &mut [f32],
    ) -> Availability {
        if !spaced(cap.spacing_hz, hz_per_bin) || spectrum.is_empty() {
            return Availability::Unavailable;
        }
        self.estimate(spectrum);
        let to_median = LN_2.sqrt() / FRAC_PI_4.sqrt();
        let last = self.mean_magnitude.len() - 1;
        for (o, &f) in out.iter_mut().zip(partials) {
            let bin = ((f / hz_per_bin).round() as usize).min(last);
            *o = self.mean_magnitude[bin] * to_median;
        }
        Availability::Available
    }
}

// ── the BG median and its margins ────────────────────────────────────────────

/// Where the lowest partial's lower gap stops: below `f₁` there is no partial,
/// only rumble, which the DC blocker's corner shapes.
// The oracle's own clamp; report 0015.
const GAP_FLOOR_HZ: f32 = 20.0;

/// ISO 3744 §4.2.1.1's minimum margin, the measured level at least 6 dB above
/// the background, as a partial-to-noise power ratio: `10^0.6 − 1`.
const ISO_6_DB: f32 = 2.98;

/// The same clause's preferred margin, more than 15 dB: `10^1.5 − 1`.
const ISO_15_DB: f32 = 30.6;

/// Oppenheimer's `z_α` at α = 95 %: `Φ⁻¹(0.95)`.
const Z_ALPHA: f32 = 1.644_853_6;

/// Oppenheimer's steadiness `M` (his Eq. 6) of one bin of one frame: a single
/// periodogram.
const M: f32 = 1.0;

/// The rule a BG floor is read against. The three stated margins are validity
/// margins, not detection thresholds: each says how far a measured level must
/// stand above its background before the level may be trusted, not whether a
/// line is present. A margin's power ratio `R` over the noise's mean power is an
/// amplitude `√(R / ln 2)` times the noise's median magnitude, against the
/// partial's measured amplitude, partial and noise together. ISO's `R` is the
/// partial-to-noise ratio its level difference implies, `10^(ΔL/10) − 1`;
/// Oppenheimer's is the measured-to-background ratio itself, `10^(ΔL₀/10)`.
///
/// # References
///
/// - ISO 3744:2010, sound power levels of noise sources from sound pressure,
///   engineering method over a reflecting plane. §4.2.1.1.
/// - Oppenheimer, C. H. (2016). A statistical background noise correction
///   sensitive to the steadiness of background noise. J. Acoust. Soc. Am.
///   140(4), 2888–2901. DOI: 10.1121/1.4963091. Eqs. 6, 16, 18.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Margin {
    /// The operating point's Rayleigh threshold, the driver's own:
    /// `√(ln(1/p_fa) / ln 2)`, 3.157 at 10⁻³.
    Rayleigh,
    /// ISO 3744 §4.2.1.1's 6 dB, `R = 2.98`: 2.07 × the median.
    Iso6,
    /// ISO 3744 §4.2.1.1's 15 dB, `R = 30.6`: 6.64 × the median.
    Iso15,
    /// Oppenheimer's ΔL₀ = 10 lg(1 + z_α√(2/M)) (Eq. 16) at α = 95 % and one
    /// frame's steadiness `M = 1`: 5.22 dB, `R = 3.33`, 2.19 × the median. His
    /// ΔL_min (Eq. 18) at |θ| = 5 % and δK_max = 0.2 dB is 7.67 dB, `R = 5.85`,
    /// 2.91 × the median.
    Oppenheimer,
}

impl Margin {
    /// The power ratio `R` over the noise's mean power the margin accepts from;
    /// `None` for the Rayleigh threshold, which the operating point sets.
    fn power_ratio(self) -> Option<f32> {
        match self {
            Margin::Rayleigh => None,
            Margin::Iso6 => Some(ISO_6_DB),
            Margin::Iso15 => Some(ISO_15_DB),
            Margin::Oppenheimer => Some(1.0 + Z_ALPHA * (2.0 / M).sqrt()),
        }
    }

    /// The multiplier on the noise's median magnitude, `√(R / ln 2)`.
    fn multiplier(self) -> Option<f32> {
        self.power_ratio().map(|r| (r / LN_2).sqrt())
    }
}

/// The partials bounding partial `n`'s two gaps, at `f_hz`: its predicted
/// neighbours, and below `f₁` the lower one reflected from the `f₁`–`f₂`
/// spacing.
fn gap_bounds(cap: &CaptureCtx, f_hz: f32, n: usize) -> (f32, f32) {
    let partial = |m: usize| ambient::stiff_partial(cap.spacing_hz, cap.b, m);
    let f_hi = partial(n + 1);
    let f_lo = if n >= 2 {
        partial(n - 1)
    } else {
        f_hz - (partial(2) - partial(1))
    };
    (f_lo, f_hi)
}

/// Appends the bins of `spectrum` whose frequencies lie in `[lo_hz, hi_hz]`.
fn collect_band(spectrum: &[f32], hz_per_bin: f32, lo_hz: f32, hi_hz: f32, out: &mut Vec<f32>) {
    let lo = (lo_hz / hz_per_bin).ceil().max(1.0) as usize;
    let hi = ((hi_hz / hz_per_bin).floor() as usize).min(spectrum.len() - 1);
    if lo <= hi {
        out.extend_from_slice(&spectrum[lo..=hi]);
    }
}

/// The noise's median magnitude at each partial on the 2¹⁶-point frame, over the
/// two inter-partial gaps flanking it: from `f_{n−1} + L` to `f_n − L` and from
/// `f_n + L` to `f_{n+1} − L`, with `L = 2·fs/N` the Hann main lobe's
/// half-width, the neighbours predicted from the key's `f₀` and `B`, and below
/// `f₁` the lower bound reflected from the `f₁`–`f₂` spacing and kept above
/// 20 Hz. Both gaps are pooled before the one median is taken (the upper one
/// for an even count); a partial whose gaps together hold fewer than three bins
/// is declined. It is the oracle's construction (`ambient::partial_probe_at`)
/// read on the 2¹⁶-point frame at MAT's resolution; the floor is read against
/// `margin`.
pub(super) struct BgMedian {
    margin: Margin,
    scratch: Vec<f32>,
}

impl BgMedian {
    pub(super) fn new(margin: Margin) -> Self {
        Self {
            margin,
            scratch: Vec::new(),
        }
    }
}

impl Arm for BgMedian {
    fn name(&self) -> &str {
        match self.margin {
            Margin::Rayleigh => "bgmed",
            Margin::Iso6 => "bgmed-iso6",
            Margin::Iso15 => "bgmed-iso15",
            Margin::Oppenheimer => "bgmed-opp",
        }
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::Bg]
    }

    fn prepare(&mut self, set: &SetCtx) {
        let rule = match (self.margin.power_ratio(), self.margin.multiplier()) {
            (Some(r), Some(k)) => format!(
                "{k:.3} × the median, a validity margin at a power ratio of {r:.3} over the noise's mean"
            ),
            _ => format!(
                "{:.3} × the median, the Rayleigh threshold at {:.1e}",
                median_to_threshold(set.p_fa),
                set.p_fa
            ),
        };
        println!(
            "[{}] arm {}: the median over each partial's flanking gaps on the 2¹⁶-point frame, read \
             against {rule}",
            set.label,
            self.name()
        );
    }

    fn bg_multiplier(&self) -> Option<f32> {
        self.margin.multiplier()
    }

    fn bg_floor(
        &mut self,
        cap: &CaptureCtx,
        spectrum: &[f32],
        hz_per_bin: f32,
        partials: &[f32],
        out: &mut [f32],
    ) -> Availability {
        if spectrum.is_empty() {
            return Availability::Unavailable;
        }
        let lobe = LOBE_HALF_BINS as f32 * hz_per_bin;
        for ((o, &f), &n) in out.iter_mut().zip(partials).zip(cap.ns) {
            let (f_lo, f_hi) = gap_bounds(cap, f, n);
            self.scratch.clear();
            let lo_start = (f_lo + lobe).max(GAP_FLOOR_HZ);
            if f - lobe > lo_start {
                collect_band(spectrum, hz_per_bin, lo_start, f - lobe, &mut self.scratch);
            }
            if f_hi - lobe > f + lobe {
                collect_band(
                    spectrum,
                    hz_per_bin,
                    f + lobe,
                    f_hi - lobe,
                    &mut self.scratch,
                );
            }
            *o = if self.scratch.len() < ambient::MIN_GAP_BINS {
                f32::NAN
            } else {
                let mid = self.scratch.len() / 2;
                *self.scratch.select_nth_unstable_by(mid, f32::total_cmp).1
            };
        }
        Availability::Available
    }
}

#[cfg(test)]
mod tests {
    use std::sync::OnceLock;

    use realfft::RealFftPlanner;
    use rustfft::num_complex::Complex;

    use super::*;
    use crate::gates::arms::Block;
    use crate::gates::stats;
    use crate::gates::truth::tone_amplitude;
    use tuner_core::models::{NOTES, get_expected_beta};

    /// Time-domain σ of the synthetic noise.
    const SIGMA: f32 = 1.0e-3;

    /// Trials of the 1024-sample null the tests share: five times the arm's.
    /// The 10⁻³ point of 40 000 trials scatters by about 2 % from seed to seed,
    /// of 200 000 by about 1 %.
    const TEST_TRIALS: usize = 200_000;

    /// The arm's own 1024-sample noise stream, read further than the arm reads it.
    fn null_1024() -> &'static NeighbourNull {
        static NULL: OnceLock<NeighbourNull> = OnceLock::new();
        NULL.get_or_init(|| neighbour_null(1024, TEST_TRIALS, NULL_SEED + 1024))
    }

    /// The mask that keeps exactly `offsets`.
    fn mask_of(offsets: &[i32]) -> u16 {
        OFFSETS
            .iter()
            .enumerate()
            .filter(|(_, d)| offsets.contains(d))
            .fold(0, |m, (i, _)| m | (1 << i))
    }

    /// A set at the operating point `p_fa`.
    fn set_at(p_fa: f32) -> SetCtx<'static> {
        SetCtx {
            label: "test",
            p_fa,
            sigma_override: None,
            nu_half_per_frame: None,
        }
    }

    /// A Rayleigh magnitude of unit mean.
    fn rayleigh(rng: &mut Gaussian) -> f32 {
        let (x, y) = (rng.next(), rng.next());
        (x * x + y * y).sqrt() / (std::f32::consts::PI / 2.0).sqrt()
    }

    /// A Fisher–Yates permutation of `0 … n − 1` drawn with xorshift64.
    fn shuffled(n: usize, seed: u64) -> Vec<usize> {
        let mut state = seed.max(1);
        let mut v: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            v.swap(i, (state % (i as u64 + 1)) as usize);
        }
        v
    }

    /// The target's and every cell's Goertzel amplitude on `samples`.
    fn probe(samples: &[f32], hann: &[f32], target_hz: f32) -> (f32, [f32; CELLS]) {
        let hz = SAMPLE_RATE as f32 / hann.len() as f32;
        let amp = spectral::goertzel_windowed(samples, SAMPLE_RATE, target_hz, hann).0;
        let mut cells = [0.0f32; CELLS];
        for (c, &d) in cells.iter_mut().zip(&OFFSETS) {
            *c = spectral::goertzel_windowed(samples, SAMPLE_RATE, target_hz + d as f32 * hz, hann)
                .0;
        }
        (amp, cells)
    }

    /// Hann magnitudes of `frames` frames of `n` samples of white noise of
    /// standard deviation `SIGMA`, as the driver computes a frame's.
    fn noise_frames(n: usize, frames: usize, seed: u64) -> Vec<Vec<f32>> {
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let mut rng = Gaussian::new(seed);
        let mut time = vec![0.0f32; n];
        let mut spec = vec![Complex::new(0.0, 0.0); n / 2 + 1];
        (0..frames)
            .map(|_| {
                let x: Vec<f32> = (0..n).map(|_| SIGMA * rng.next()).collect();
                spectral::fft(&x, &mut time, &mut spec, &plan, n);
                let mut mag = vec![0.0f32; n / 2];
                spectral::magnitude_spectrum(&spec, n, &mut mag);
                mag
            })
            .collect()
    }

    /// `Σw²` of the symmetric Hann window of `n` points.
    fn hann_energy(n: usize) -> f64 {
        truth::hann_vec(n)
            .iter()
            .map(|&w| w as f64 * w as f64)
            .sum()
    }

    /// With all ten cells the null's 10⁻³ point lies within 10 % of Rohling's
    /// value for ten independent cells at the median rank, and above it: cells
    /// one bin apart carry the Hann window's power correlation of 4/9. The cells
    /// two bins apart (±2, ±4, ±6) are nearly independent, and there the null
    /// meets Rohling's six-cell value within 5 %.
    #[test]
    fn the_null_against_rohling() {
        let null = null_1024();
        let p = 1.0e-3;
        let all = calibrate(null, ALL_CELLS, p).k;
        let rohling_10 = truth::cfar_multiplier_finite(10, 6, p as f32);
        let even = calibrate(null, mask_of(&[-6, -4, -2, 2, 4, 6]), p).k;
        let rohling_6 = truth::cfar_multiplier_finite(6, 4, p as f32);
        println!(
            "1024 at 1e-3, {TEST_TRIALS} trials: ten cells k = {all:.4}, Rohling (10, 6) {rohling_10:.4}, \
             ratio {:.4}; cells two bins apart k = {even:.4}, Rohling (6, 4) {rohling_6:.4}, ratio {:.4}",
            all / rohling_10,
            even / rohling_6
        );
        assert!(
            (all / rohling_10 - 1.0).abs() < 0.10,
            "{all} vs {rohling_10}"
        );
        assert!(all > rohling_10, "{all} vs {rohling_10}");
        assert!(
            (even / rohling_6 - 1.0).abs() < 0.05,
            "{even} vs {rohling_6}"
        );
    }

    /// Cells inside another partial's main lobe, or within two bins of DC, are
    /// dropped; the probe's own partial is left to the geometry.
    #[test]
    fn cells_clear_the_other_partials() {
        let hz = SAMPLE_RATE as f32 / 1024.0;
        let own = 10;
        // Partials 6.07 bins apart, as C4's are at 1024 samples: n ± 1 take the
        // cells at ±5 and ±6.
        let series = |spacing_bins: f32, centre: f32| -> Vec<(usize, f32)> {
            (1..=20)
                .map(|n| (n, centre + (n as f32 - own as f32) * spacing_bins * hz))
                .collect()
        };
        let target = 200.0 * hz;
        assert_eq!(
            kept_offsets(cell_mask(target, 1024, &series(6.07, target), own)),
            vec![-4, -3, -2, 2, 3, 4]
        );
        // 3.04 bins apart, as C3's are: nothing survives.
        assert_eq!(cell_mask(target, 1024, &series(3.04, target), own), 0);
        // Twelve bins apart: all ten, with the own partial half a bin off the target.
        assert_eq!(
            cell_mask(target, 1024, &series(12.0, target - 0.5 * hz), own),
            ALL_CELLS
        );
        // Three bins above DC with nothing near: only the upper five.
        assert_eq!(
            kept_offsets(cell_mask(3.0 * hz, 1024, &[], own)),
            vec![2, 3, 4, 5, 6]
        );
    }

    /// On fresh white noise the 10⁻² multiplier admits inside the Clopper–Pearson
    /// interval of 10⁻², and the median's band holds the noise's own median in
    /// 90 ± 2 % of the frames.
    #[test]
    fn white_noise_admits_at_the_nominal_rate_and_the_band_covers() {
        let null = null_1024();
        let p = 1.0e-2;
        let cal = calibrate(null, ALL_CELLS, p);
        let hann = truth::hann_vec(1024);
        let mut rng = Gaussian::new(99);
        let mut noise = vec![0.0f32; 1024];
        let truth_median = SIGMA * null.median;
        let trials = 20_000u64;
        let (mut over, mut covered) = (0u64, 0u64);
        for _ in 0..trials {
            noise.iter_mut().for_each(|x| *x = SIGMA * rng.next());
            let (amp, cells) = probe(&noise, &hann, 2000.0);
            let median = masked_median(&cells, ALL_CELLS);
            over += u64::from(amp >= cal.k * median);
            covered += u64::from(
                median / cal.band.1 <= truth_median && truth_median <= median / cal.band.0,
            );
        }
        let (lo, hi) = stats::clopper_pearson(over, trials, 0.05);
        let coverage = covered as f64 / trials as f64;
        println!(
            "1024 at 1e-2: k = {:.4}, {over} / {trials} admitted, 95 % CP [{lo:.4e}, {hi:.4e}]; \
             band {:.4}–{:.4}, coverage {coverage:.4}",
            cal.k, cal.band.0, cal.band.1
        );
        assert!(lo <= p && p <= hi, "{over} / {trials}");
        assert!((coverage - 0.90).abs() < 0.02, "coverage {coverage}");
    }

    /// A tone 30 dB above the noise in the probe's own window, at the target, is
    /// admitted at 10⁻³ in at least 95 of 100 frames.
    #[test]
    fn a_30_db_tone_is_admitted() {
        let cal = calibrate(null_1024(), ALL_CELLS, 1.0e-3);
        let hann = truth::hann_vec(1024);
        let amp = tone_amplitude(SIGMA, 30.0, 1024);
        let mut rng = Gaussian::new(30);
        let f = 1234.5f32;
        let mut admitted = 0;
        for _ in 0..100 {
            let phase = rng.next().atan2(rng.next());
            let x: Vec<f32> = (0..1024)
                .map(|t| {
                    let arg = std::f32::consts::TAU * f * t as f32 / SAMPLE_RATE as f32 + phase;
                    SIGMA * rng.next() + amp * arg.cos()
                })
                .collect();
            let (a, cells) = probe(&x, &hann, f);
            admitted += usize::from(a >= cal.k * masked_median(&cells, ALL_CELLS));
        }
        println!("30 dB tone at the target: admitted in {admitted} of 100 frames");
        assert!(admitted >= 95, "{admitted}");
    }

    /// On unit-mean Rayleigh magnitudes, independent from bin to bin, the
    /// estimate before `k₁` reads 0.901 ± 0.01 of the mean magnitude: Eq. 10.
    #[test]
    fn independent_bins_read_k1() {
        let mut rng = Gaussian::new(10);
        let (mut two_sided, mut prefix) = (Vec::new(), Vec::new());
        let mut out = vec![0.0f32; 4096];
        let mut sum = 0.0f64;
        let frames = 40;
        for _ in 0..frames {
            let mag: Vec<f32> = (0..4096).map(|_| rayleigh(&mut rng)).collect();
            pre_average(&mag, &mut two_sided);
            reciprocal_mean(&two_sided, n_f(8192), &mut prefix, &mut out);
            sum += out.iter().map(|&v| v as f64).sum::<f64>();
        }
        let level = sum / (frames * out.len()) as f64;
        println!("independent unit-mean Rayleigh bins: {level:.4} of the mean magnitude");
        assert!((level - 0.901).abs() < 0.01, "{level}");
    }

    /// Lines 40 dB up in a share `p = 0.25` of the bins lift the floor by
    /// `1/(1 − p)`, +2.50 dB, within 0.3 dB: in the reciprocal mean alone with the
    /// lines at random bins, and through the three-bin mean with four-bin lobes
    /// every 24 bins, each lobe seen as six.
    #[test]
    fn lines_lift_the_floor_by_the_closed_form() {
        let mut rng = Gaussian::new(25);
        let n = 8192;
        let closed = 20.0 * (1.0f64 / 0.75).log10();
        let (mut prefix, mut out) = (Vec::new(), vec![0.0f32; n]);
        let floor = |s: &[f64], prefix: &mut Vec<f64>, out: &mut [f32]| -> f64 {
            reciprocal_mean(s, n_f(n), prefix, out);
            out.iter().map(|&v| v as f64).sum::<f64>()
        };
        // The reciprocal mean on its own, lines at random bins.
        let noise: Vec<f64> = (0..n).map(|_| rayleigh(&mut rng) as f64).collect();
        let mut lined = noise.clone();
        for &b in shuffled(n, 7).iter().take(n / 4) {
            lined[b] = 100.0 * rayleigh(&mut rng) as f64;
        }
        let quiet = floor(&noise, &mut prefix, &mut out);
        let loud = floor(&lined, &mut prefix, &mut out);
        let random_db = 20.0 * (loud / quiet).log10();
        // The full estimator, lobes of four bins every 24.
        let half = n / 2;
        let mag: Vec<f32> = (0..half).map(|_| rayleigh(&mut rng)).collect();
        let mut lobed = mag.clone();
        for (b, m) in lobed.iter_mut().enumerate() {
            if b % 24 < 4 {
                *m *= 100.0;
            }
        }
        let mut two_sided = Vec::new();
        let edge = n_f(n);
        let mut through = |m: &[f32]| -> f64 {
            pre_average(m, &mut two_sided);
            reciprocal_mean(&two_sided, n_f(n), &mut prefix, &mut out[..half]);
            out[edge..half - edge]
                .iter()
                .map(|&v| v as f64)
                .sum::<f64>()
        };
        let lobe_db = 20.0 * (through(&lobed) / through(&mag)).log10();
        println!(
            "p = 0.25: random lines {random_db:+.2} dB, four-bin lobes through the three-bin mean \
             {lobe_db:+.2} dB; closed form {closed:+.2} dB"
        );
        assert!((random_db - closed).abs() < 0.3, "{random_db}");
        assert!((lobe_db - closed).abs() < 0.3, "{lobe_db}");
    }

    /// On the 8192-point Hann frame of white noise the estimate before `k₁`
    /// reads 0.852 ± 0.01 of the mean magnitude, not Eq. 10's 0.90: bins one
    /// apart are correlated, so the three-bin mean removes fewer zeros.
    #[test]
    fn hann_frames_read_below_k1() {
        let n = 8192;
        let mean_magnitude = (SIGMA as f64).powi(2) * hann_energy(n) * std::f64::consts::FRAC_PI_4;
        let mean_magnitude = mean_magnitude.sqrt();
        let (mut two_sided, mut prefix) = (Vec::new(), Vec::new());
        let mut out = vec![0.0f32; n / 2];
        let (mut sum, mut count) = (0.0f64, 0usize);
        for mag in noise_frames(n, 30, 852) {
            pre_average(&mag, &mut two_sided);
            reciprocal_mean(&two_sided, n_f(n), &mut prefix, &mut out);
            sum += out[200..3900].iter().map(|&v| v as f64).sum::<f64>();
            count += 3700;
        }
        let level = sum / count as f64 / mean_magnitude;
        let low = level / K1 as f64;
        println!(
            "Hann frames of white noise: {level:.4} of the mean magnitude; the threshold at k₁ sits \
             at {low:.4}, a per-bin rate of {:.2e} at 1e-3",
            1.0e-3f64.powf(low * low)
        );
        assert!((level - 0.852).abs() < 0.01, "{level}");
    }

    /// G1 declines below a partial spacing of eight bins; above it a flat
    /// spectrum of unit magnitudes reads `1/k₁` as its mean magnitude, so the
    /// threshold at 10⁻³ is `√(ln 1000)/(k₁√(π/4))`.
    #[test]
    fn laurenti_declines_below_eight_bins() {
        let mut arm = Laurenti::new();
        arm.prepare(&set_at(1.0e-3));
        let hz = SAMPLE_RATE as f32 / 8192.0;
        let mag = vec![1.0f32; 4096];
        for (bins, available) in [(7.9f32, false), (8.1, true)] {
            let f_meas = [bins * hz];
            let cap = CaptureCtx {
                dir: "test",
                key: 8,
                register: 0,
                ns: &[1],
                f_meas: &f_meas,
                spacing_hz: bins * hz,
                b: 0.0,
                sigma_rec: SIGMA,
            };
            let hop = HopCtx {
                hop: 0,
                t_since: 0.0,
                bucket: 0,
                is_silence: true,
                block: Block::Holdout,
                audio: &[],
                spectrum: &[],
                mag: &mag,
                hz_per_bin: hz,
                sigma: SIGMA,
                truth_level: None,
                oracle: &[],
                capture: &cap,
            };
            let mut out = vec![f32::INFINITY; 4096];
            let got = arm.g1_threshold(&hop, &mut out) == Availability::Available;
            assert_eq!(got, available, "{bins} bins");
            if available {
                let expected = 1000.0f32.ln().sqrt() / (K1 * FRAC_PI_4.sqrt());
                assert!(
                    (out[1000] / expected - 1.0).abs() < 1e-5,
                    "{} vs {expected}",
                    out[1000]
                );
            }
        }
    }

    /// On white noise at 2¹⁶ points the median over the flanking gaps reads the
    /// Rayleigh median within 3 %, in the median over 50 frames, at A0's series
    /// (its lowest gap reflected and clamped) and C4's.
    #[test]
    fn bg_median_reads_the_rayleigh_median() {
        let n = 65_536;
        let hz = SAMPLE_RATE as f32 / n as f32;
        let rayleigh_median =
            (SIGMA as f64 * (hann_energy(n) * std::f64::consts::LN_2).sqrt()) as f32;
        let ns: Vec<usize> = (1..=8).collect();
        let series: Vec<(f32, f32, Vec<f32>)> = [0usize, 39]
            .iter()
            .map(|&key| {
                let (f0, b) = (NOTES[key].frequency, get_expected_beta(key as u8));
                (
                    f0,
                    b,
                    ns.iter()
                        .map(|&m| ambient::stiff_partial(f0, b, m))
                        .collect(),
                )
            })
            .collect();
        let mut arm = BgMedian::new(Margin::Rayleigh);
        let mut ratios = Vec::new();
        let mut out = vec![0.0f32; ns.len()];
        for mag in noise_frames(n, 50, 65) {
            for (f0, b, f_meas) in &series {
                let cap = CaptureCtx {
                    dir: "test",
                    key: 0,
                    register: 0,
                    ns: &ns,
                    f_meas,
                    spacing_hz: *f0,
                    b: *b,
                    sigma_rec: SIGMA,
                };
                assert_eq!(
                    arm.bg_floor(&cap, &mag, hz, f_meas, &mut out),
                    Availability::Available
                );
                ratios.extend(out.iter().map(|&v| v / rayleigh_median));
            }
        }
        ratios.sort_by(f32::total_cmp);
        let median = ratios[ratios.len() / 2];
        println!("BG median over the gaps on white noise: {median:.4} of the Rayleigh median");
        assert!(ratios.iter().all(|v| v.is_finite()));
        assert!((median - 1.0).abs() < 0.03, "{median}");
    }

    /// The margins' multipliers on the noise's median: ISO's 6 dB and 15 dB as
    /// power ratios 2.98 and 30.6, Oppenheimer's ΔL₀ = 5.22 dB at M = 1; the
    /// arm hands the driver its margin's.
    #[test]
    fn the_margins_are_the_sources_numbers() {
        let k = |m: Margin| m.multiplier().expect("a stated margin");
        assert!(
            (k(Margin::Iso6) - 2.07).abs() < 0.005,
            "{}",
            k(Margin::Iso6)
        );
        assert!(
            (k(Margin::Iso15) - 6.64).abs() < 0.005,
            "{}",
            k(Margin::Iso15)
        );
        assert!(
            (k(Margin::Oppenheimer) - 2.19).abs() < 0.005,
            "{}",
            k(Margin::Oppenheimer)
        );
        let dl0 = 10.0 * Margin::Oppenheimer.power_ratio().unwrap_or(0.0).log10();
        assert!((dl0 - 5.22).abs() < 0.005, "{dl0}");
        assert!(Margin::Rayleigh.multiplier().is_none());
        for m in [
            Margin::Rayleigh,
            Margin::Iso6,
            Margin::Iso15,
            Margin::Oppenheimer,
        ] {
            assert_eq!(BgMedian::new(m).bg_multiplier(), m.multiplier());
        }
    }
}
