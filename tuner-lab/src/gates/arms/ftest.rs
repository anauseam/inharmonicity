//! Thomson's harmonic F-test as an arm: a line at each tested frequency against
//! the residual of the same frame's Slepian tapers at that frequency, with the
//! discrete prolate spheroidal sequences it is built on.

use std::sync::Arc;
use std::time::{Duration, Instant};

use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use super::{
    Arm, Availability, Block, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx,
    rate_line,
};
use crate::gates::stats;
use crate::gates::truth::Gaussian;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};

/// The frame the test reads: the picker's 8192 samples.
const N: usize = BASS_WINDOW_SIZE;

/// Bins below Nyquist in the frame's one-sided spectrum.
const HALF: usize = N / 2;

/// Most tapers the arm carries: `K = 2NW − 1` up to NW = 8.
const MAX_K: usize = 15;

/// Inverse-iteration solves per taper. Bisection leaves each eigenvalue within
/// rounding of the true one, several units from its neighbours, so every solve
/// shrinks the other eigenvectors' share about 10⁹-fold: two reach rounding
/// from a random start, the third polishes.
const INVERSE_ITERATIONS: usize = 3;

/// Seed of the start vectors, fixed so the tapers are reproducible.
const START_SEED: u64 = 0x5EED_D955;

/// Hops between two frames that share no samples. Frames a hop apart share all
/// but one hop of theirs, so `F` at one bin is correlated from hop to hop and
/// only disjoint frames are independent trials for an interval.
const DISJOINT_HOPS: usize = N / HOP_SIZE;

/// Thomson's harmonic F-test. At a frequency `f` the line's complex amplitude is
/// regressed across the eigencoefficients `y_k(f)` of `K` Slepian tapers,
/// `μ̂(f) = Σ U_k(0) y_k(f) / Σ U_k(0)²` (Eq. 13.5), and tested against what the
/// regression leaves,
/// `F(f) = (K − 1) |μ̂(f)|² Σ U_k(0)² / Σ |y_k(f) − μ̂(f) U_k(0)|²` (Eq. 13.10),
/// which is `F(2, 2K − 2)` when the continuous spectrum is Gaussian and locally
/// white across ±W and no line is present. The threshold at `p` is therefore
/// exact, `(K − 1)(p^(−1/(K − 1)) − 1)`; the reference is the tested frequency's
/// own residual in the same frame, so nothing is learned and nothing carries
/// between hops. The tapers are real and the transforms plain DFTs, the form
/// Thomson gives beside Eq. 3.4, so `U_k(0) = Σ_n v_k[n]`, zero for odd `k`.
///
/// G1 tests every bin of the 8192-point frame and keeps a bin whose `F` reaches
/// the threshold and is a local maximum of `F`; the arm is decision-only. G2 and
/// G3 test the same 8192-sample frame at the probe's frequency, with the
/// eigencoefficients summed directly at that frequency, not on the gate's own
/// window. Within W of DC and Nyquist a real frame's eigencoefficients are not
/// circular, and the null there is only approximate.
///
/// A line δ from the tested frequency reaches the residual through the odd
/// tapers, so without noise `F` is `K − 1` times its fitted over its unfitted
/// energy, whatever its level. At NW = 3 the 10⁻³ threshold is out of reach
/// beyond 0.27 bin (1.47 Hz), so a scan at bin centres misses, at any SNR, a
/// line that falls that far between two bins.
///
/// # References
///
/// - Thomson, D. J. (1982). Spectrum estimation and harmonic analysis.
///   Proc. IEEE 70(9), 1055–1096. DOI: 10.1109/PROC.1982.12433. Eqs. 2.9, 3.4,
///   13.5, 13.10.
/// - Percival, D. B. & Walden, A. T. (1993). Spectral Analysis for Physical
///   Applications. Cambridge University Press. §8.3, the tridiagonal form of
///   the discrete prolate spheroidal sequences.
pub(super) struct FTest {
    name: String,
    nw: usize,
    /// The tapers `v_k`, unit energy, most concentrated first.
    tapers: Vec<Vec<f64>>,
    /// The same in the FFT's input type.
    tapers_f32: Vec<Vec<f32>>,
    /// `U_k(0) = Σ_n v_k[n]`.
    u0: Vec<f64>,
    /// `Σ_k U_k(0)²`.
    u0_energy: f64,
    /// Each taper's energy concentration in (−W, W), `λ_k`.
    concentration: Vec<f64>,
    p_fa: f32,
    /// The upper `p_fa` point of `F(2, 2K − 2)`.
    threshold: f64,
    plan: Arc<dyn RealToComplex<f32>>,
    time: Vec<f32>,
    /// The `K` eigenspectra of the last frame, `HALF + 1` bins each, taper-major.
    spectra: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    /// `F` at bins `0..=HALF` of the last frame.
    f: Vec<f64>,
    /// The G1 scan over the scored pre-roll, over those of its frames that
    /// share no samples, over the note's hops the Gatekeeper calls silent, and
    /// over the rest of the note.
    preroll: Tally,
    disjoint: Tally,
    note_silent: Tally,
    note: Tally,
    /// The last pre-roll hop counted into `disjoint`, this capture.
    last_disjoint: Option<i64>,
    /// Time spent in `g1_threshold`, and the hops it served.
    cost: Duration,
    timed_hops: u64,
}

/// The G1 scan's bins over one population of hops: tested, at or above the
/// threshold, and kept as a local maximum of `F`.
#[derive(Default)]
struct Tally {
    hops: u64,
    bins: u64,
    over: u64,
    kept: u64,
}

impl Tally {
    fn add(&mut self, bins: u64, over: u64, kept: u64) {
        self.hops += 1;
        self.bins += bins;
        self.over += over;
        self.kept += kept;
    }

    fn kept_per_hop(&self) -> f64 {
        self.kept as f64 / self.hops.max(1) as f64
    }
}

impl FTest {
    /// The test at time–bandwidth `nw`: `W = nw/N`, `K = 2nw − 1` tapers,
    /// computed here once. Named `ftest` at NW = 3, `ftest-nw<nw>` otherwise.
    ///
    /// # Panics
    ///
    /// If `nw` is below 2, which leaves no residual degree of freedom, or above 8.
    pub(super) fn new(nw: usize) -> Self {
        let most = MAX_K.div_ceil(2);
        assert!(
            (2..=most).contains(&nw),
            "NW must lie in 2..={most}, got {nw}"
        );
        let k = 2 * nw - 1;
        let tapers = dpss(N, nw, k);
        let w = nw as f64 / N as f64;
        let concentration = tapers.iter().map(|v| concentration(v, w)).collect();
        let u0: Vec<f64> = tapers.iter().map(|v| v.iter().sum()).collect();
        let u0_energy = u0.iter().map(|u| u * u).sum();
        let tapers_f32 = tapers
            .iter()
            .map(|v| v.iter().map(|&x| x as f32).collect())
            .collect();
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(N);
        let scratch = plan.make_scratch_vec();
        // NW = 3 is the first-pass representative, 2 and 4 its variants.
        // report 0023
        let name = if nw == 3 {
            "ftest".to_string()
        } else {
            format!("ftest-nw{nw}")
        };
        Self {
            name,
            nw,
            tapers,
            tapers_f32,
            u0,
            u0_energy,
            concentration,
            p_fa: 1.0e-3,
            threshold: threshold(k, 1.0e-3),
            plan,
            time: vec![0.0; N],
            spectra: vec![Complex::new(0.0, 0.0); k * (HALF + 1)],
            scratch,
            f: vec![0.0; HALF + 1],
            preroll: Tally::default(),
            disjoint: Tally::default(),
            note_silent: Tally::default(),
            note: Tally::default(),
            last_disjoint: None,
            cost: Duration::ZERO,
            timed_hops: 0,
        }
    }

    fn k(&self) -> usize {
        self.tapers.len()
    }

    /// `F` at bins `0..=HALF` of `audio`'s 8192-point frame into `self.f`: one
    /// FFT per tapered copy, then Eqs. 13.5 and 13.10 at each bin.
    fn statistic_per_bin(&mut self, audio: &[f32]) {
        let bins = HALF + 1;
        for (taper, out) in self
            .tapers_f32
            .iter()
            .zip(self.spectra.chunks_exact_mut(bins))
        {
            for ((t, &x), &v) in self.time.iter_mut().zip(audio).zip(taper) {
                *t = x * v;
            }
            self.plan
                .process_with_scratch(&mut self.time, out, &mut self.scratch)
                .expect("buffers are sized to the plan");
        }
        let k = self.k();
        let mut y = [Complex::new(0.0f64, 0.0); MAX_K];
        for (bin, f) in self.f.iter_mut().enumerate() {
            for (yk, spectrum) in y.iter_mut().zip(self.spectra.chunks_exact(bins)) {
                let c = spectrum[bin];
                *yk = Complex::new(c.re as f64, c.im as f64);
            }
            *f = statistic(&y[..k], &self.u0, self.u0_energy);
        }
    }

    /// `F` at `hz` on the 8192-sample `audio`, the eigencoefficients summed
    /// directly at that frequency: `y_k(f) = Σ_n v_k[n] x[n] e^(−j2πfn/fs)`.
    fn statistic_at(&self, audio: &[f32], hz: f64) -> f64 {
        let step = Complex::from_polar(1.0, -std::f64::consts::TAU * hz / SAMPLE_RATE as f64);
        let mut phasor = Complex::new(1.0f64, 0.0);
        let mut y = [Complex::new(0.0f64, 0.0); MAX_K];
        for (n, &x) in audio.iter().enumerate() {
            let xz = phasor * x as f64;
            for (yk, taper) in y.iter_mut().zip(&self.tapers) {
                *yk += xz * taper[n];
            }
            phasor *= step;
        }
        statistic(&y[..self.k()], &self.u0, self.u0_energy)
    }

    /// `F` of a noise-free complex line `offset_bins` from the tested
    /// frequency: `K − 1` times its fitted over its unfitted energy, the ceiling
    /// no level raises.
    fn line_ceiling(&self, offset_bins: f64) -> f64 {
        let step = Complex::from_polar(1.0, std::f64::consts::TAU * offset_bins / N as f64);
        let mut y = [Complex::new(0.0f64, 0.0); MAX_K];
        for (yk, taper) in y.iter_mut().zip(&self.tapers) {
            let mut phasor = Complex::new(1.0f64, 0.0);
            for &v in taper {
                *yk += phasor * v;
                phasor *= step;
            }
        }
        statistic(&y[..self.k()], &self.u0, self.u0_energy)
    }

    /// The offset in bins, within one bin, past which a noise-free line's `F`
    /// stays below the threshold; the ceiling falls monotonically over it.
    fn line_reach(&self) -> f64 {
        let (mut lo, mut hi) = (0.0f64, 1.0f64);
        if self.line_ceiling(hi) >= self.threshold {
            return hi;
        }
        for _ in 0..40 {
            let mid = 0.5 * (lo + hi);
            if self.line_ceiling(mid) >= self.threshold {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        0.5 * (lo + hi)
    }
}

impl Arm for FTest {
    fn name(&self) -> &str {
        &self.name
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G1, Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = set.p_fa;
        self.threshold = threshold(self.k(), set.p_fa as f64);
        self.preroll = Tally::default();
        self.disjoint = Tally::default();
        self.note_silent = Tally::default();
        self.note = Tally::default();
        self.last_disjoint = None;
        self.cost = Duration::ZERO;
        self.timed_hops = 0;
        let hz_per_bin = SAMPLE_RATE as f64 / N as f64;
        let reach = self.line_reach();
        let leakage: Vec<String> = self
            .concentration
            .iter()
            .map(|l| format!("{:.1e}", 1.0 - l))
            .collect();
        println!(
            "[{}] arm {}: K = {} Slepian tapers at NW = {} on the {N}-sample frame (W = {:.1} Hz), \
             1 − λ {}; F(2, {}) ≥ {:.2} at {:.1e}; a noise-free line more than {:.2} bin \
             ({:.2} Hz) from the tested frequency stays below it at any level",
            set.label,
            self.name,
            self.k(),
            self.nw,
            self.nw as f64 * hz_per_bin,
            leakage.join(" "),
            2 * self.k() - 2,
            self.threshold,
            self.p_fa,
            reach,
            reach * hz_per_bin
        );
    }

    fn begin_capture(&mut self, _cap: &CaptureCtx) {
        self.last_disjoint = None;
    }

    fn finish(&mut self, set: &SetCtx) {
        if self.timed_hops == 0 {
            println!("[{}] arm {}: no G1 scan ran", set.label, self.name);
            return;
        }
        let p = self.p_fa as f64;
        println!(
            "[{}] arm {}: P(F ≥ {:.2}) per bin of the G1 scan against {:.1e} (95 % Clopper–Pearson)",
            set.label, self.name, self.threshold, p
        );
        for (label, tally) in [
            ("pre-roll, disjoint frames", &self.disjoint),
            ("pre-roll, every hop", &self.preroll),
            ("note, Gatekeeper-silent", &self.note_silent),
            ("note", &self.note),
        ] {
            if tally.hops > 0 {
                println!("{}", rate_line(label, tally.over, tally.bins, Some(p)));
            }
        }
        println!(
            "     frames a hop apart share {}/{DISJOINT_HOPS} of their samples, so only the \
             disjoint frames are independent trials for the interval; the note's bins include \
             the partials', so its rates are no null. Kept (a local maximum of F at or above the \
             threshold) per hop: {:.2} pre-roll, {:.2} note, {:.2} note Gatekeeper-silent",
            DISJOINT_HOPS - 1,
            self.preroll.kept_per_hop(),
            self.note.kept_per_hop(),
            self.note_silent.kept_per_hop()
        );
        let hop_ms = HOP_SIZE as f64 / SAMPLE_RATE as f64 * 1.0e3;
        println!(
            "     g1_threshold: {:.0} µs per hop over {} hops ({} FFTs of {N} and the regression \
             per bin), against the {hop_ms:.1} ms hop",
            self.cost.as_secs_f64() * 1.0e6 / self.timed_hops as f64,
            self.timed_hops,
            self.k()
        );
    }

    fn decision_only(&self) -> bool {
        true
    }

    fn g1_threshold(&mut self, hop: &HopCtx, out: &mut [f32]) -> Availability {
        if hop.audio.len() != N {
            return Availability::Unavailable;
        }
        let start = Instant::now();
        self.statistic_per_bin(hop.audio);
        if let Some(dc) = out.first_mut() {
            *dc = f32::INFINITY;
        }
        let (mut bins, mut over, mut kept) = (0u64, 0u64, 0u64);
        for (bin, o) in out.iter_mut().enumerate().take(HALF).skip(1) {
            let f = self.f[bin];
            let is_over = f >= self.threshold;
            let is_kept = is_over && f > self.f[bin - 1] && f > self.f[bin + 1];
            *o = if is_kept { hop.mag[bin] } else { f32::INFINITY };
            bins += 1;
            over += u64::from(is_over);
            kept += u64::from(is_kept);
        }
        match (hop.block, hop.is_silence) {
            (Block::Note, true) => self.note_silent.add(bins, over, kept),
            (Block::Note, false) => self.note.add(bins, over, kept),
            _ => {
                self.preroll.add(bins, over, kept);
                if self
                    .last_disjoint
                    .is_none_or(|last| hop.hop >= last + DISJOINT_HOPS as i64)
                {
                    self.disjoint.add(bins, over, kept);
                    self.last_disjoint = Some(hop.hop);
                }
            }
        }
        self.cost += start.elapsed();
        self.timed_hops += 1;
        Availability::Available
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let nyquist = SAMPLE_RATE as f32 / 2.0;
        if hop.audio.len() != N || !(probe.target_hz > 0.0 && probe.target_hz < nyquist) {
            return GateVerdict::default();
        }
        let f = self.statistic_at(hop.audio, probe.target_hz as f64);
        GateVerdict {
            admit: Some(f >= self.threshold),
            ..GateVerdict::default()
        }
    }
}

/// The upper `p_fa` point of `F(2, 2K − 2)`, the null of Eq. 13.10.
fn threshold(k: usize, p_fa: f64) -> f64 {
    stats::f_2_2m_threshold((k - 1) as f64, p_fa)
}

/// Eqs. 13.5 and 13.10 at one frequency from its `K` eigencoefficients `y`:
/// `μ̂ = Σ U_k(0) y_k / Σ U_k(0)²` and
/// `F = (K − 1) |μ̂|² Σ U_k(0)² / Σ |y_k − μ̂ U_k(0)|²`. Infinite where a line
/// leaves no residual at all, zero for an all-zero frame.
fn statistic(y: &[Complex<f64>], u0: &[f64], u0_energy: f64) -> f64 {
    let mu = y
        .iter()
        .zip(u0)
        .map(|(&yk, &u)| yk * u)
        .sum::<Complex<f64>>()
        / u0_energy;
    let residual: f64 = y
        .iter()
        .zip(u0)
        .map(|(&yk, &u)| (yk - mu * u).norm_sqr())
        .sum();
    let line = mu.norm_sqr() * u0_energy;
    if residual > 0.0 {
        (y.len() - 1) as f64 * line / residual
    } else if line > 0.0 {
        f64::INFINITY
    } else {
        0.0
    }
}

/// The symmetric tridiagonal matrix that commutes with Thomson's Eq. 2.9 kernel
/// for length `n` and bandwidth `w`: diagonal `((n − 1 − 2t)/2)² cos 2πw`, and
/// `t(n − t)/2` coupling rows `t − 1` and `t` (entry 0 unused, zero).
fn prolate_matrix(n: usize, w: f64) -> (Vec<f64>, Vec<f64>) {
    let cos = (std::f64::consts::TAU * w).cos();
    let diag = (0..n)
        .map(|t| {
            let c = (n - 1) as f64 / 2.0 - t as f64;
            c * c * cos
        })
        .collect();
    let off = (0..n).map(|t| t as f64 * (n - t) as f64 / 2.0).collect();
    (diag, off)
}

/// The first `k` discrete prolate spheroidal sequences of length `n` at
/// time–bandwidth `nw` (`W = nw/n`), unit energy, most concentrated first: the
/// eigenvectors of the `k` largest eigenvalues of [`prolate_matrix`]
/// (Percival & Walden 1993, §8.3). Each eigenvalue comes from bisection on the
/// Sturm count inside the Gershgorin interval, each vector from inverse
/// iteration with a pivoted tridiagonal solve from a random start,
/// re-orthogonalised against the vectors before it. An even-order sequence has
/// a positive sum, an odd-order one a positive first moment about the centre.
fn dpss(n: usize, nw: usize, k: usize) -> Vec<Vec<f64>> {
    let (diag, off) = prolate_matrix(n, nw as f64 / n as f64);
    let off2: Vec<f64> = off.iter().map(|e| e * e).collect();
    let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
    for (t, &d) in diag.iter().enumerate() {
        let radius = off[t] + off.get(t + 1).copied().unwrap_or(0.0);
        lo = lo.min(d - radius);
        hi = hi.max(d + radius);
    }
    let margin = f64::EPSILON * n as f64 * (hi - lo);
    let (lo, hi) = (lo - margin, hi + margin);
    let pivmin = f64::MIN_POSITIVE * off2.iter().fold(1.0f64, |a, &b| a.max(b));
    let centre = (n - 1) as f64 / 2.0;
    let mut rng = Gaussian::new(START_SEED);
    let mut tapers: Vec<Vec<f64>> = Vec::with_capacity(k);
    for order in 0..k {
        let theta = bisect_eigenvalue(&diag, &off2, n - 1 - order, lo, hi, pivmin);
        let shifted: Vec<f64> = diag.iter().map(|&d| d - theta).collect();
        let lu = TridiagonalLu::factor(&shifted, &off);
        let mut v: Vec<f64> = (0..n).map(|_| rng.next() as f64).collect();
        for _ in 0..INVERSE_ITERATIONS {
            lu.solve(&mut v);
            for prev in &tapers {
                let c: f64 = v.iter().zip(prev).map(|(a, b)| a * b).sum();
                v.iter_mut().zip(prev).for_each(|(a, &b)| *a -= c * b);
            }
            let norm = v.iter().map(|x| x * x).sum::<f64>().sqrt();
            v.iter_mut().for_each(|x| *x /= norm);
        }
        let sign: f64 = if order % 2 == 0 {
            v.iter().sum()
        } else {
            v.iter()
                .enumerate()
                .map(|(t, &x)| (t as f64 - centre) * x)
                .sum()
        };
        if sign < 0.0 {
            v.iter_mut().for_each(|x| *x = -*x);
        }
        tapers.push(v);
    }
    tapers
}

/// The eigenvalue of rank `index` counted from the bottom, by bisection on
/// [`count_below`] between bounds that hold every eigenvalue.
fn bisect_eigenvalue(
    diag: &[f64],
    off2: &[f64],
    index: usize,
    mut lo: f64,
    mut hi: f64,
    pivmin: f64,
) -> f64 {
    loop {
        let mid = 0.5 * (lo + hi);
        if mid <= lo || mid >= hi {
            return mid;
        }
        if count_below(diag, off2, mid, pivmin) > index {
            hi = mid;
        } else {
            lo = mid;
        }
    }
}

/// Eigenvalues below `x` of the symmetric tridiagonal matrix with diagonal
/// `diag` and squared couplings `off2`: the negative pivots of the LDLᵀ
/// factorisation of the matrix less `x`, by Sylvester's law of inertia. A pivot
/// smaller than `pivmin` is taken as `−pivmin`.
fn count_below(diag: &[f64], off2: &[f64], x: f64, pivmin: f64) -> usize {
    let mut q = 1.0f64;
    let mut count = 0;
    for (&d, &e2) in diag.iter().zip(off2) {
        q = d - x - e2 / q;
        if q.abs() < pivmin {
            q = -pivmin;
        }
        if q < 0.0 {
            count += 1;
        }
    }
    count
}

/// LU factorisation with partial pivoting of a symmetric tridiagonal matrix:
/// `U` keeps its diagonal and two superdiagonals, the row operations their
/// multiplier and whether they swapped the two rows.
struct TridiagonalLu {
    u0: Vec<f64>,
    u1: Vec<f64>,
    u2: Vec<f64>,
    mult: Vec<f64>,
    swapped: Vec<bool>,
}

impl TridiagonalLu {
    /// Factors the matrix with diagonal `diag` and `off[t]` coupling rows
    /// `t − 1` and `t`. A zero pivot is replaced by the matrix's scale times
    /// machine epsilon, so a singular shift still yields a solve.
    fn factor(diag: &[f64], off: &[f64]) -> Self {
        let n = diag.len();
        let scale = diag.iter().chain(off).fold(0.0f64, |a, &b| a.max(b.abs()));
        let mut lu = Self {
            u0: vec![0.0; n],
            u1: vec![0.0; n],
            u2: vec![0.0; n],
            mult: vec![0.0; n],
            swapped: vec![false; n],
        };
        // The row still to be eliminated: its entries in columns i and i + 1.
        let (mut c0, mut c1) = (diag[0], off.get(1).copied().unwrap_or(0.0));
        for i in 0..n - 1 {
            // Row i + 1 in columns i, i + 1 and i + 2.
            let (n0, n1) = (off[i + 1], diag[i + 1]);
            let n2 = off.get(i + 2).copied().unwrap_or(0.0);
            if n0.abs() > c0.abs() {
                let m = c0 / n0;
                (lu.u0[i], lu.u1[i], lu.u2[i]) = (n0, n1, n2);
                (lu.mult[i], lu.swapped[i]) = (m, true);
                (c0, c1) = (c1 - m * n1, -m * n2);
            } else {
                let m = if c0 != 0.0 { n0 / c0 } else { 0.0 };
                (lu.u0[i], lu.u1[i]) = (c0, c1);
                lu.mult[i] = m;
                (c0, c1) = (n1 - m * c1, n2);
            }
        }
        lu.u0[n - 1] = c0;
        for p in lu.u0.iter_mut().filter(|p| **p == 0.0) {
            *p = f64::EPSILON * scale;
        }
        lu
    }

    /// Solves in place: `b` holds the right-hand side and receives the solution.
    fn solve(&self, b: &mut [f64]) {
        let n = b.len();
        let mut pending = b[0];
        for i in 0..n - 1 {
            let next = b[i + 1];
            if self.swapped[i] {
                b[i] = next;
                pending -= self.mult[i] * next;
            } else {
                b[i] = pending;
                pending = next - self.mult[i] * pending;
            }
        }
        b[n - 1] = pending;
        for i in (0..n).rev() {
            let mut s = b[i];
            if i + 1 < n {
                s -= self.u1[i] * b[i + 1];
            }
            if i + 2 < n {
                s -= self.u2[i] * b[i + 2];
            }
            b[i] = s / self.u0[i];
        }
    }
}

/// The energy of the unit-energy `v` in (−w, w), `λ = vᵀAv` with Thomson's
/// Eq. 2.9 kernel `A[n][m] = sin 2πw(n − m) / π(n − m)`, summed over the lags of
/// `v`'s autocorrelation, which an FFT of twice the length gives exactly.
fn concentration(v: &[f64], w: f64) -> f64 {
    let n = v.len();
    let mut planner = RealFftPlanner::<f64>::new();
    let forward = planner.plan_fft_forward(2 * n);
    let inverse = planner.plan_fft_inverse(2 * n);
    let mut buf = forward.make_input_vec();
    buf[..n].copy_from_slice(v);
    let mut spectrum = forward.make_output_vec();
    forward
        .process(&mut buf, &mut spectrum)
        .expect("buffers are sized to the plan");
    spectrum
        .iter_mut()
        .for_each(|c| *c = Complex::new(c.norm_sqr(), 0.0));
    let mut lag = inverse.make_output_vec();
    inverse
        .process(&mut spectrum, &mut lag)
        .expect("a power spectrum is real");
    // The inverse transform is unnormalised: every lag carries a factor 2n.
    let scale = 1.0 / (2 * n) as f64;
    let pi = std::f64::consts::PI;
    let tail: f64 = (1..n)
        .map(|tau| {
            let t = tau as f64;
            2.0 * lag[tau] * scale * (2.0 * pi * w * t).sin() / (pi * t)
        })
        .sum();
    2.0 * w * lag[0] * scale + tail
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::truth::tone_amplitude;
    use tuner_core::algorithms::spectral;

    const SIGMA: f32 = 1.0e-3;

    fn set_at(p_fa: f32) -> SetCtx<'static> {
        SetCtx {
            label: "test",
            p_fa,
            sigma_override: None,
            nu_half_per_frame: None,
        }
    }

    /// White Gaussian noise of standard deviation `SIGMA` plus, where `line`
    /// gives one, a cosine of that amplitude at that bin centre and phase.
    fn frame(rng: &mut Gaussian, line: Option<(f32, usize, f64)>) -> Vec<f32> {
        (0..N)
            .map(|n| {
                let tone = line.map_or(0.0, |(a, bin, phase)| {
                    a * (std::f64::consts::TAU * (bin * n) as f64 / N as f64 + phase).cos() as f32
                });
                SIGMA * rng.next() + tone
            })
            .collect()
    }

    /// The symmetric tridiagonal matrix times `v`.
    fn apply(diag: &[f64], off: &[f64], v: &[f64]) -> Vec<f64> {
        (0..v.len())
            .map(|t| {
                let mut s = diag[t] * v[t];
                if t > 0 {
                    s += off[t] * v[t - 1];
                }
                if t + 1 < v.len() {
                    s += off[t + 1] * v[t + 1];
                }
                s
            })
            .collect()
    }

    fn dot<A: Copy + Into<f64>, B: Copy + Into<f64>>(a: &[A], b: &[B]) -> f64 {
        a.iter().zip(b).map(|(&x, &y)| x.into() * y.into()).sum()
    }

    /// The pivoted solve inverts a symmetric indefinite tridiagonal matrix.
    #[test]
    fn tridiagonal_solve_inverts_the_matrix() {
        let mut rng = Gaussian::new(11);
        let n = 64;
        let diag: Vec<f64> = (0..n).map(|_| rng.next() as f64).collect();
        let off: Vec<f64> = (0..n)
            .map(|t| if t == 0 { 0.0 } else { 2.0 * rng.next() as f64 })
            .collect();
        let x: Vec<f64> = (0..n).map(|_| rng.next() as f64).collect();
        let mut b = apply(&diag, &off, &x);
        TridiagonalLu::factor(&diag, &off).solve(&mut b);
        let err = b
            .iter()
            .zip(&x)
            .map(|(a, e)| (a - e).abs())
            .fold(0.0, f64::max);
        assert!(err < 1e-9, "solve error {err:e}");
    }

    /// At NW = 3 the tapers are the five top eigenvectors of the prolate matrix,
    /// orthonormal, of alternating parity, and signed by the convention. Their
    /// concentrations: the first four exceed 0.99 and the fifth is 0.946, which
    /// the dense eigensolver below reproduces at N = 1024.
    #[test]
    fn tapers_at_nw3_are_the_top_eigenvectors() {
        let arm = FTest::new(3);
        let (diag, off) = prolate_matrix(N, 3.0 / N as f64);
        let off2: Vec<f64> = off.iter().map(|e| e * e).collect();
        let scale = diag.iter().chain(&off).fold(0.0f64, |a, &b| a.max(b.abs()));
        println!("NW = 3, K = 5, N = {N}");
        for (k, v) in arm.tapers.iter().enumerate() {
            let tv = apply(&diag, &off, v);
            let theta = dot(v.as_slice(), tv.as_slice());
            let residual = tv
                .iter()
                .zip(v)
                .map(|(a, b)| (a - theta * b).abs())
                .fold(0.0, f64::max);
            let above = N - count_below(&diag, &off2, theta - 1.0e-3, f64::MIN_POSITIVE);
            let parity = if k % 2 == 0 { 1.0 } else { -1.0 };
            let asym = (0..N)
                .map(|t| (v[N - 1 - t] - parity * v[t]).abs())
                .fold(0.0, f64::max);
            println!(
                "  k = {k}: θ = {theta:.6}, |Tv − θv|∞ = {residual:.1e}, λ = {:.12}, U(0) = {:+.9}, parity error {asym:.1e}",
                arm.concentration[k], arm.u0[k]
            );
            assert!(
                residual < 1e-9 * scale,
                "taper {k} is not an eigenvector: {residual:e}"
            );
            assert_eq!(above, k + 1, "taper {k} is not the {}-th largest", k + 1);
            assert!(
                asym < 1e-9,
                "taper {k} breaks the matrix's persymmetry: {asym:e}"
            );
        }
        for k in 0..4 {
            assert!(
                arm.concentration[k] > 0.99,
                "λ{k} = {}",
                arm.concentration[k]
            );
        }
        assert!(
            (0.94..0.95).contains(&arm.concentration[4]),
            "λ4 = {}",
            arm.concentration[4]
        );
        for (j, (a, a32)) in arm.tapers.iter().zip(&arm.tapers_f32).enumerate() {
            for (k, (b, b32)) in arm.tapers.iter().zip(&arm.tapers_f32).enumerate() {
                let want = if j == k { 1.0 } else { 0.0 };
                let d = dot(a.as_slice(), b.as_slice());
                let d32 = dot(a32.as_slice(), b32.as_slice());
                assert!((d - want).abs() < 1e-6, "⟨v{j}, v{k}⟩ = {d}");
                assert!((d32 - want).abs() < 1e-6, "f32 ⟨v{j}, v{k}⟩ = {d32}");
            }
        }
        for (k, &u) in arm.u0.iter().enumerate() {
            if k % 2 == 1 {
                assert!(u.abs() < 1e-6, "U{k}(0) = {u:e}");
            } else {
                assert!(u > 0.0, "U{k}(0) = {u}");
            }
        }
    }

    /// The same matrix at N = 1024 through LAPACK's dense symmetric eigensolver
    /// (`numpy.linalg.eigh`, the sign convention above, `λ` from the direct
    /// autocorrelation) gives these concentrations, sums and samples.
    #[test]
    fn dpss_agrees_with_a_dense_eigensolver() {
        let n = 1024;
        let tapers = dpss(n, 3, 5);
        let lambda = [
            0.999999865220,
            0.999990756143,
            0.999715017900,
            0.994914738827,
            0.946139629018,
        ];
        let u0 = [24.0469152074, 0.0, 16.1016255003, 0.0, 12.2433897905];
        let at_100 = [
            0.001865230555,
            -0.007871434496,
            0.021129424532,
            -0.040014851985,
            0.052087963093,
        ];
        let at_511 = [
            0.057521619931,
            -0.000232790427,
            -0.038521196482,
            0.000256121057,
            0.030108509343,
        ];
        for (k, v) in tapers.iter().enumerate() {
            let l = concentration(v, 3.0 / n as f64);
            let s: f64 = v.iter().sum();
            assert!((l - lambda[k]).abs() < 1e-10, "λ{k}: {l} vs {}", lambda[k]);
            assert!((s - u0[k]).abs() < 1e-8, "U{k}(0): {s} vs {}", u0[k]);
            assert!(
                (v[100] - at_100[k]).abs() < 1e-10,
                "v{k}[100]: {} vs {}",
                v[100],
                at_100[k]
            );
            assert!(
                (v[511] - at_511[k]).abs() < 1e-10,
                "v{k}[511]: {} vs {}",
                v[511],
                at_511[k]
            );
        }
    }

    /// The threshold is the upper point of F(2, 2K − 2): 61.25, 18.49 and 12.97
    /// at 10⁻³ for K = 3, 5, 7.
    #[test]
    fn thresholds_are_the_f_law() {
        for (nw, pinned) in [(2, 61.25), (3, 18.49), (4, 12.97)] {
            let mut arm = FTest::new(nw);
            arm.prepare(&set_at(1.0e-3));
            let m = (2 * nw - 2) as f64;
            assert_eq!(arm.threshold, stats::f_2_2m_threshold(m, 1.0e-3f32 as f64));
            assert!(
                (arm.threshold - pinned).abs() < 0.01,
                "NW = {nw}: {}",
                arm.threshold
            );
            let tail = (1.0 + arm.threshold / m).powf(-m);
            assert!((tail - 1.0e-3f32 as f64).abs() < 1e-12, "tail {tail}");
        }
    }

    /// The control: 200 frames of white Gaussian noise, `F` at every bin
    /// `1..N/2`, the realised exceedance against nominal at 10⁻² and 10⁻³. The
    /// assertion is the 10⁻² point inside the Clopper–Pearson interval of the
    /// realised count; the per-frame count's variance against the binomial's
    /// shows how far the bins depart from independent trials.
    #[test]
    fn white_noise_exceeds_at_the_nominal_rate() {
        let mut arm = FTest::new(3);
        let mut rng = Gaussian::new(20261009);
        let (t2, t3) = (threshold(5, 1.0e-2), threshold(5, 1.0e-3));
        let (mut bins, mut over2, mut over3) = (0u64, 0u64, 0u64);
        let mut per_frame = Vec::new();
        for _ in 0..200 {
            arm.statistic_per_bin(&frame(&mut rng, None));
            let f = &arm.f[1..HALF];
            let c2 = f.iter().filter(|&&x| x >= t2).count() as u64;
            bins += f.len() as u64;
            over2 += c2;
            over3 += f.iter().filter(|&&x| x >= t3).count() as u64;
            per_frame.push(c2 as f64);
        }
        for (p, over) in [(1.0e-2, over2), (1.0e-3, over3)] {
            let (lo, hi) = stats::clopper_pearson(over, bins, 0.05);
            println!(
                "NW = 3 on white noise at {p:.0e}: {over} / {bins} = {:.4e}, 95 % CP [{lo:.4e}, {hi:.4e}]",
                over as f64 / bins as f64
            );
        }
        let mean = per_frame.iter().sum::<f64>() / per_frame.len() as f64;
        let var = per_frame.iter().map(|c| (c - mean).powi(2)).sum::<f64>()
            / (per_frame.len() - 1) as f64;
        let p = 1.0e-2;
        println!(
            "per-frame count at 1e-2: variance {var:.1} against the binomial's {:.1}",
            (HALF - 1) as f64 * p * (1.0 - p)
        );
        let (lo, hi) = stats::clopper_pearson(over2, bins, 0.05);
        assert!(lo <= p && p <= hi, "1e-2 outside [{lo:e}, {hi:e}]");
    }

    /// A line 40 dB above the per-bin noise of the Hann frame, on a bin centre,
    /// clears the 10⁻³ threshold at its bin in at least 95 of 100 frames.
    #[test]
    fn a_40_db_line_on_a_bin_centre_is_found() {
        let arm_threshold = threshold(5, 1.0e-3);
        let mut arm = FTest::new(3);
        let mut rng = Gaussian::new(7);
        let amp = tone_amplitude(SIGMA, 40.0, N);
        let mut found = 0;
        for trial in 0..100usize {
            let bin = 12 + 40 * trial;
            let phase = std::f64::consts::TAU * ((trial as f64 * 0.618_033_988_75) % 1.0);
            arm.statistic_per_bin(&frame(&mut rng, Some((amp, bin, phase))));
            if arm.f[bin] >= arm_threshold {
                found += 1;
            }
        }
        println!("40 dB line on a bin centre: found in {found} of 100 frames");
        assert!(found >= 95, "found in {found} of 100");
    }

    /// The probe form, summed at the bin-centre frequency, agrees with the FFT
    /// form at the bin.
    #[test]
    fn the_probe_form_equals_the_scan_at_bin_centres() {
        let mut arm = FTest::new(3);
        let mut rng = Gaussian::new(3);
        let amp = tone_amplitude(SIGMA, 10.0, N);
        let audio = frame(&mut rng, Some((amp, 700, 0.3)));
        arm.statistic_per_bin(&audio);
        for bin in [9usize, 300, 699, 700, 701, 2048, 4000] {
            let hz = bin as f64 * SAMPLE_RATE as f64 / N as f64;
            let direct = arm.statistic_at(&audio, hz);
            let rel = (direct / arm.f[bin] - 1.0).abs();
            assert!(rel < 1e-3, "bin {bin}: {direct} vs {}", arm.f[bin]);
        }
    }

    /// Without noise, `F` of a line off the tested frequency is capped by the
    /// tapers alone. Half a bin off, no NW reaches its 10⁻³ threshold; at
    /// NW = 3 the reach lies between a quarter bin (F = 22.6) and 0.3 bin
    /// (F = 14.7).
    #[test]
    fn a_line_between_bins_is_out_of_reach() {
        for nw in [2, 3, 4] {
            let mut arm = FTest::new(nw);
            arm.prepare(&set_at(1.0e-3));
            let table: Vec<String> = [0.05, 0.1, 0.25, 0.3, 0.5]
                .iter()
                .map(|&d| format!("{d}: {:.3}", arm.line_ceiling(d)))
                .collect();
            println!(
                "NW = {nw}, threshold {:.2}: noise-free F by offset in bins {}; reach {:.3} bin",
                arm.threshold,
                table.join(", "),
                arm.line_reach()
            );
            assert!(arm.line_ceiling(0.5) < arm.threshold, "NW = {nw}");
        }
        let mut arm = FTest::new(3);
        arm.prepare(&set_at(1.0e-3));
        assert!(arm.line_ceiling(0.25) > arm.threshold);
        assert!(arm.line_ceiling(0.3) < arm.threshold);
        assert!((0.25..0.3).contains(&arm.line_reach()));
    }

    /// Through the trait: G1 keeps the line's bin at its magnitude and declines
    /// DC; the gate admits the line's frequency and declines one past Nyquist.
    #[test]
    fn the_arm_keeps_the_line_and_declines_the_rest() {
        let mut arm = FTest::new(3);
        arm.prepare(&set_at(1.0e-3));
        let mut rng = Gaussian::new(5);
        let bin = 1000;
        let audio = frame(&mut rng, Some((tone_amplitude(SIGMA, 40.0, N), bin, 1.0)));
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(N);
        let mut time = vec![0.0f32; N];
        let mut spectrum = vec![Complex::new(0.0f32, 0.0); HALF + 1];
        let mut mag = vec![0.0f32; HALF];
        spectral::fft(&audio, &mut time, &mut spectrum, &plan, N);
        spectral::magnitude_spectrum(&spectrum, N, &mut mag);
        let hz_per_bin = SAMPLE_RATE as f32 / N as f32;
        let cap = CaptureCtx {
            dir: "test",
            key: 48,
            register: 1,
            ns: &[1],
            f_meas: &[bin as f32 * hz_per_bin],
            spacing_hz: bin as f32 * hz_per_bin,
            b: 0.0,
            sigma_rec: SIGMA,
        };
        let hop = HopCtx {
            hop: 0,
            t_since: 0.0,
            bucket: 0,
            is_silence: false,
            block: Block::Note,
            audio: &audio,
            spectrum: &spectrum,
            mag: &mag,
            hz_per_bin,
            sigma: SIGMA,
            truth_level: None,
            oracle: &[],
            capture: &cap,
        };
        let mut out = vec![0.0f32; HALF];
        assert_eq!(arm.g1_threshold(&hop, &mut out), Availability::Available);
        assert_eq!(out[bin], mag[bin]);
        assert!(out[0].is_infinite());
        let kept: Vec<usize> = (0..HALF).filter(|&b| out[b].is_finite()).collect();
        assert!(kept.iter().all(|&b| out[b] == mag[b]));
        println!("kept bins: {kept:?}");
        let probe = |hz: f32| GateProbe {
            index: 0,
            target_hz: hz,
            window: 4096,
            amplitude: 0.0,
            f_inst: None,
            fixed: true,
            truth_level: None,
        };
        let at_line = arm.gate(&hop, &probe(bin as f32 * hz_per_bin));
        assert_eq!(at_line.admit, Some(true));
        assert!(at_line.threshold.is_none() && at_line.estimate.is_none());
        assert_eq!(arm.gate(&hop, &probe(SAMPLE_RATE as f32)).admit, None);
    }
}
