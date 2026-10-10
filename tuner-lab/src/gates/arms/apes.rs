//! The APES residual at a known frequency: the noise at a gate's own frequency,
//! read from what a fit of the named key's partials leaves of the data's
//! covariance (Nørholm, Jensen & Christensen 2016). A G2/G3 noise estimate.

use std::collections::HashMap;
use std::f64::consts::{FRAC_1_SQRT_2, LN_2, PI, TAU};
use std::time::{Duration, Instant};

use rustfft::num_complex::Complex;

use super::{
    Arm, Block, CaptureCtx, Consumer, GateProbe, GateVerdict, HopCtx, SetCtx, median_to_threshold,
    raw_to_gate,
};
use crate::gates::ambient;
use crate::gates::truth::{Gaussian, awgn_median_bin, tone_amplitude};
use crate::truth::{SAMPLE_RATE, hann_vec};
use tuner_core::algorithms::spectral::goertzel_windowed;
use tuner_core::audio::BASS_WINDOW_SIZE;

type C64 = Complex<f64>;

/// The moving average's length and the decimation factor, `D`: the hop's 8192
/// samples become 128 over ±344.5 Hz around the probe. Ours, with
/// [`FILTER_LEN`]: their resolution `fs/(D·M)` = 21.5 Hz separates A0's 27.5 Hz
/// partials.
const DECIMATION: usize = 64;

/// The APES filter's length, the paper's `M`. Ours: 32 of the 128 decimated
/// samples leave 97 snapshots, inside the paper's `2M + 1 ≤ N`.
const FILTER_LEN: usize = 32;

/// Decimated samples per hop, the paper's `N`.
const DECIMATED_LEN: usize = BASS_WINDOW_SIZE / DECIMATION;

/// Snapshots, `L = N − M + 1`.
const SNAPSHOTS: usize = DECIMATED_LEN - FILTER_LEN + 1;

/// Most lines a model may hold: `Q̂` has rank at most `L − K`, which must exceed
/// `M` for the filter to exist.
const MAX_LINES: usize = SNAPSHOTS - FILTER_LEN - 1;

/// Bound on the walk along a key's series: A0's partials reach Nyquist near
/// `n` = 800.
const MAX_PARTIAL: usize = 2048;

/// White-noise trials behind each model size's correction and interval: the
/// median to about 1.5 % (one standard error).
const CALIBRATION_TRIALS: usize = 2000;

/// Trials of the threshold's null, as the other arms' simulated nulls take:
/// about forty exceedances at 10⁻³.
const NULL_TRIALS: usize = 40_000;

/// The null's probe frequency: mid-band, where the white-noise law does not
/// depend on it.
const NULL_HZ: f32 = 1000.0;

/// The windows the register rule gives a probe.
const WINDOWS: [usize; 2] = [1024, 4096];

/// Frames in the frequency-error sweep.
const SWEEP_FRAMES: usize = 100;

/// The sweep's line levels per 8192-point bin: 40 dB, and the control's 30.
const SWEEP_SNR_DB: [f32; 2] = [40.0, 30.0];

/// The sweep's line frequency and noise σ, and the window it is read at, the
/// register rule's at that frequency.
const SWEEP_HZ: f32 = 1000.0;
const SWEEP_SIGMA: f32 = 1.0e-3;
const SWEEP_WINDOW: usize = 1024;

/// The model's displacements in the sweep, in 8192-point bins.
const SWEEP_OFFSETS_BINS: [f64; 4] = [0.0, 0.1, 0.25, 0.5];

/// Seed of the arm's simulations.
const SEED: u64 = 20_161_004;

/// Cholesky factor of the Hermitian positive-definite `n × n` matrix whose lower
/// triangle the row-major `a` holds, in place: `A = L Lᴴ` with `L`
/// lower-triangular and real on its diagonal. Only the lower triangle is read or
/// written. `false` at a pivot that is not positive and finite.
fn cholesky(a: &mut [C64], n: usize) -> bool {
    for j in 0..n {
        let pivot = a[j * n + j].re
            - a[j * n..j * n + j]
                .iter()
                .map(|c| c.norm_sqr())
                .sum::<f64>();
        if !(pivot > 0.0 && pivot.is_finite()) {
            return false;
        }
        let pivot = pivot.sqrt();
        a[j * n + j] = C64::new(pivot, 0.0);
        for i in j + 1..n {
            let dot: C64 = (0..j).map(|k| a[i * n + k] * a[j * n + k].conj()).sum();
            a[i * n + j] = (a[i * n + j] - dot) / pivot;
        }
    }
    true
}

/// `L y = b` in place, for the factor [`cholesky`] leaves in `l`.
fn solve_lower(l: &[C64], n: usize, b: &mut [C64]) {
    for i in 0..n {
        let dot: C64 = l[i * n..i * n + i]
            .iter()
            .zip(&b[..i])
            .map(|(x, y)| x * y)
            .sum();
        b[i] = (b[i] - dot) / l[i * n + i].re;
    }
}

/// `Lᴴ x = y` in place, for the factor [`cholesky`] leaves in `l`.
fn solve_upper(l: &[C64], n: usize, b: &mut [C64]) {
    for i in (0..n).rev() {
        let dot: C64 = (i + 1..n).map(|k| l[k * n + i].conj() * b[k]).sum();
        b[i] = (b[i] - dot) / l[i * n + i].re;
    }
}

/// The last `N·D` samples of `audio` moved to baseband at `f_t` Hz and
/// block-averaged over `D` samples: `z(m) = (1/D) Σ_{k<D} x(mD + k)
/// e^{−j2πf_t(mD + k)/fs}`, `m < N`. A component at `f_t + ν` becomes an
/// exponential at `2πνD/fs` radians per sample, folded into one turn, with the
/// block average's gain `sin(πνD/fs) / (D sin(πν/fs))`: one at the centre and
/// nil at every other multiple of `fs/D`. White noise of variance σ² becomes
/// white noise of power σ²/D per sample.
fn decimate(audio: &[f32], f_t: f64, rotor: &mut [C64], z: &mut [C64]) {
    let x = &audio[audio.len() - DECIMATED_LEN * DECIMATION..];
    let omega = -TAU * f_t / f64::from(SAMPLE_RATE);
    for (k, r) in rotor.iter_mut().enumerate() {
        *r = C64::from_polar(1.0, omega * k as f64);
    }
    for (m, (out, block)) in z.iter_mut().zip(x.as_chunks::<DECIMATION>().0).enumerate() {
        let sum: C64 = block
            .iter()
            .zip(rotor.iter())
            .map(|(&s, r)| r * f64::from(s))
            .sum();
        *out = C64::from_polar(1.0 / DECIMATION as f64, omega * (m * DECIMATION) as f64) * sum;
    }
}

/// The model's frequencies, in radians per decimated sample, into `psi`: the
/// target's line at `f_line` first, then every other partial of the key whose
/// baseband frequency falls in the band — its measured frequency where the
/// capture located it, else `stiff_partial`'s prediction — and of each line the
/// image at `−f − f_t` a real signal carries (the paper's `z(−l)`, Eq. 8) where
/// that falls in the band. `false` where the key has no series to predict from.
/// `f_line` must lie within the band of `f_t`.
fn model_lines(cap: &CaptureCtx, index: usize, f_t: f64, f_line: f64, psi: &mut Vec<f64>) -> bool {
    psi.clear();
    let Some(&target) = cap.ns.get(index) else {
        return false;
    };
    if !(cap.spacing_hz > 0.0 && cap.b.is_finite()) {
        return false;
    }
    let fs = f64::from(SAMPLE_RATE);
    let half = fs / DECIMATION as f64 / 2.0;
    let mut add = |f: f64| {
        for nu in [f - f_t, -f - f_t] {
            // The sampled spectrum repeats every fs.
            let nu = (nu + fs / 2.0).rem_euclid(fs) - fs / 2.0;
            if (-half..half).contains(&nu) {
                psi.push(TAU * nu * DECIMATION as f64 / fs);
            }
        }
    };
    add(f_line);
    // A measured partial sits within a fraction of a spacing of its prediction.
    let top = f_t + half + f64::from(cap.spacing_hz);
    for n in (1..=MAX_PARTIAL).filter(|&n| n != target) {
        let predicted = f64::from(ambient::stiff_partial(cap.spacing_hz, cap.b, n));
        if !(predicted.is_finite() && predicted <= top && predicted < fs / 2.0) {
            break;
        }
        let measured = cap
            .ns
            .iter()
            .position(|&m| m == n)
            .map(|j| f64::from(cap.f_meas[j]));
        add(measured.unwrap_or(predicted));
    }
    true
}

/// `M / (aᴴ A⁻¹ a)` at the band's centre, `a = [1 … 1]ᵀ`, for the Hermitian
/// `M × M` matrix whose lower triangle `mat` holds. For `Q̂` it is the APES
/// filter's output noise power `hᴴ Q̂ h` under the one constraint `hᴴ a = 1`
/// (Eqs. 53, 57), times `M`: the noise power per decimated sample. Factors `mat`
/// in place; `None` where it is not positive definite.
fn capon(mat: &mut [C64], steer: &mut [C64]) -> Option<f64> {
    if !cholesky(mat, FILTER_LEN) {
        return None;
    }
    steer.fill(C64::new(1.0, 0.0));
    solve_lower(mat, FILTER_LEN, steer);
    let norm: f64 = steer.iter().map(|c| c.norm_sqr()).sum();
    (norm.is_finite() && norm > 0.0).then(|| FILTER_LEN as f64 / norm)
}

/// The noise's median Goertzel amplitude, in `4/n` units at a window of `n`
/// samples, where its power per decimated sample is `p` at the probe: the block
/// average passes `S·fs/D` per sample of a density `S` flat across its unit-gain
/// centre, the Hann Goertzel's mean power is `(4/n)²·Σw²·S·fs` with
/// `Σw² = 0.375n`, and a Rayleigh magnitude's median is `√(ln 2)` times its root
/// mean power: `√(ln 2 · 6D·p / n)`. White noise of σ has `p = σ²/D`, and this
/// is `raw_to_gate` of the 8192-point level `σ√(0.375 · 8192 · ln 2)`.
fn gate_level(p: f64, window: usize) -> f64 {
    (LN_2 * 6.0 * DECIMATION as f64 * p / window as f64).sqrt()
}

/// Why a probe is declined.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Decline {
    /// No series to model, a frequency outside the band, or a window the null
    /// was not simulated at.
    Series,
    /// More lines in the band than [`MAX_LINES`].
    Lines,
    /// A matrix that is not positive definite: coincident lines, or no signal.
    Solve,
}

/// Buffers one evaluation reuses.
struct Workspace {
    /// `e^{−jωk}` over one block.
    rotor: Vec<C64>,
    /// The decimated samples, `N`.
    z: Vec<C64>,
    /// The model's frequencies, radians per decimated sample, the target's first.
    psi: Vec<f64>,
    /// `e^{jψ_k n}` over the snapshots, `L × K` row-major: `w(n)ᵀ` row by row.
    e: Vec<C64>,
    /// The Cholesky factor of `Γ = Σ_n w̄(n) w(n)ᵀ`, the exponentials' Gram
    /// matrix, `L·Wᵀ` of Eq. 55.
    gram: Vec<C64>,
    /// One right-hand side for `Γ`.
    rhs: Vec<C64>,
    /// Each sub-vector element's residual along the snapshots, `M × L`.
    resid: Vec<C64>,
    /// `Q̂`'s lower triangle, then its factor.
    q: Vec<C64>,
    /// `T`'s lower triangle, then its factor.
    t: Vec<C64>,
    /// Running sums over the snapshots of `e^{jψ_k n} (Γ⁻¹ w̄(n))_k`,
    /// `(L + 1) × K`.
    prefix: Vec<C64>,
    /// The steering vector, then its forward solve.
    steer: Vec<C64>,
}

impl Workspace {
    /// Buffers sized for the decimated hop and the filter; the model's grow on
    /// first use.
    fn new() -> Self {
        let zero = C64::new(0.0, 0.0);
        Self {
            rotor: vec![zero; DECIMATION],
            z: vec![zero; DECIMATED_LEN],
            psi: Vec::with_capacity(MAX_LINES),
            e: Vec::new(),
            gram: Vec::new(),
            rhs: Vec::new(),
            resid: vec![zero; FILTER_LEN * SNAPSHOTS],
            q: vec![zero; FILTER_LEN * FILTER_LEN],
            t: vec![zero; FILTER_LEN * FILTER_LEN],
            prefix: Vec::new(),
            steer: vec![zero; FILTER_LEN],
        }
    }

    /// The model's exponentials over the snapshots and the factor of their Gram
    /// matrix; `false` where two lines are too close to separate in `L`
    /// snapshots.
    fn fit_lines(&mut self) -> bool {
        let k = self.psi.len();
        let Workspace { psi, e, gram, .. } = self;
        e.clear();
        for n in 0..SNAPSHOTS {
            e.extend(psi.iter().map(|&p| C64::from_polar(1.0, p * n as f64)));
        }
        gram.clear();
        gram.resize(k * k, C64::new(0.0, 0.0));
        for a in 0..k {
            for b in 0..=a {
                gram[a * k + b] = (0..SNAPSHOTS)
                    .map(|n| e[n * k + a].conj() * e[n * k + b])
                    .sum();
            }
        }
        cholesky(gram, k)
    }

    /// `Q̂ = R̂ − Gᴴ W⁻¹ G` of `z` (Eqs. 54–56, 58) into `q`, after
    /// [`Workspace::fit_lines`], over the forward sub-vectors
    /// `x(n) = [z(n) … z(n + M − 1)]ᵀ`, `n < L`. Each element's sequence along
    /// the snapshots, `z(m + n)`, is fitted by the lines' exponentials in least
    /// squares, and `Q̂ = (1/L) Σ_n r(n) r(n)ᴴ` over the residual sub-vectors: the
    /// same matrix as Eq. 56, positive semi-definite by construction.
    fn residual_covariance(&mut self) {
        let k = self.psi.len();
        let (l, m) = (SNAPSHOTS, FILTER_LEN);
        let Workspace {
            z,
            e,
            gram,
            rhs,
            resid,
            q,
            ..
        } = self;
        for row in 0..m {
            let x = &z[row..row + l];
            rhs.clear();
            rhs.extend((0..k).map(|a| (0..l).map(|n| e[n * k + a].conj() * x[n]).sum::<C64>()));
            solve_lower(gram, k, rhs);
            solve_upper(gram, k, rhs);
            for (n, r) in resid[row * l..(row + 1) * l].iter_mut().enumerate() {
                let fit: C64 = e[n * k..(n + 1) * k]
                    .iter()
                    .zip(rhs.iter())
                    .map(|(w, c)| w * c)
                    .sum();
                *r = x[n] - fit;
            }
        }
        let scale = 1.0 / l as f64;
        for i in 0..m {
            for j in 0..=i {
                let dot: C64 = resid[i * l..(i + 1) * l]
                    .iter()
                    .zip(&resid[j * l..(j + 1) * l])
                    .map(|(a, b)| a * b.conj())
                    .sum();
                q[i * m + j] = dot * scale;
            }
        }
    }

    /// `T = E[Q̂]/σ²` for white noise of power σ² per decimated sample, into `t`,
    /// after [`Workspace::fit_lines`]. With `P` the projection on the lines'
    /// exponentials along the snapshots, `E[Q̂]` is Toeplitz, `T[i][j] = t(i − j)`
    /// with `t(d) = δ_d − (1/L) Σ_n P[n + d, n]` over `0 ≤ n < L − d`: the fit
    /// takes the noise at every modelled frequency with it, a notch `2π/L` wide at
    /// each line, which a filter of `M` taps centred on a line reads as a loss of
    /// about `M/L` of the noise.
    fn expected_covariance(&mut self) {
        let k = self.psi.len();
        let (l, m) = (SNAPSHOTS, FILTER_LEN);
        let Workspace {
            psi,
            e,
            gram,
            rhs,
            prefix,
            t,
            ..
        } = self;
        // P[n + d, n] = Σ_k e^{jψ_k d} · e^{jψ_k n} (Γ⁻¹ w̄(n))_k.
        prefix.clear();
        prefix.resize((l + 1) * k, C64::new(0.0, 0.0));
        for n in 0..l {
            rhs.clear();
            rhs.extend(e[n * k..(n + 1) * k].iter().map(|w| w.conj()));
            solve_lower(gram, k, rhs);
            solve_upper(gram, k, rhs);
            let (done, next) = prefix.split_at_mut((n + 1) * k);
            let terms = e[n * k..(n + 1) * k].iter().zip(rhs.iter());
            for ((sum, before), (w, u)) in next[..k].iter_mut().zip(&done[n * k..]).zip(terms) {
                *sum = before + w * u;
            }
        }
        for d in 0..m {
            let along: C64 = psi
                .iter()
                .zip(&prefix[(l - d) * k..(l - d + 1) * k])
                .map(|(&p, s)| C64::from_polar(1.0, p * d as f64) * s)
                .sum();
            let diagonal = if d == 0 { 1.0 } else { 0.0 };
            let t_d = C64::new(diagonal, 0.0) - along / l as f64;
            for j in 0..m - d {
                t[(j + d) * m + j] = t_d;
            }
        }
    }

    /// The noise power per decimated sample at the band's centre over its
    /// white-noise expectation, `M/(aᴴ Q̂⁻¹ a)` over `M/(aᴴ T⁻¹ a)`, for the
    /// probe at `f_t` with the target's line at `f_line`; with the model's size.
    fn relative_power(
        &mut self,
        audio: &[f32],
        cap: &CaptureCtx,
        index: usize,
        f_t: f64,
        f_line: f64,
    ) -> Result<(f64, usize), Decline> {
        if audio.len() < DECIMATED_LEN * DECIMATION
            || !(f_t > 0.0 && f_t < f64::from(SAMPLE_RATE) / 2.0)
        {
            return Err(Decline::Series);
        }
        if !model_lines(cap, index, f_t, f_line, &mut self.psi) {
            return Err(Decline::Series);
        }
        let k = self.psi.len();
        if k > MAX_LINES {
            return Err(Decline::Lines);
        }
        if !self.fit_lines() {
            return Err(Decline::Solve);
        }
        decimate(audio, f_t, &mut self.rotor, &mut self.z);
        self.residual_covariance();
        self.expected_covariance();
        let measured = capon(&mut self.q, &mut self.steer).ok_or(Decline::Solve)?;
        let expected = capon(&mut self.t, &mut self.steer).ok_or(Decline::Solve)?;
        Ok((measured / expected, k))
    }
}

/// The estimate's law on white noise at one model size: the ratio
/// `M/(aᴴ Q̂⁻¹ a)` over `M/(aᴴ T⁻¹ a)` on unit complex white noise, at `K` lines
/// spaced `2π/K` around the band with one at its centre (the spacing a key's
/// partials take when `K` of them fill it).
#[derive(Clone, Copy, Debug)]
struct Calibration {
    /// The ratio's median: what the filter, fitted to the same data, reads
    /// under the expectation.
    median: f64,
    /// The ratio's 5 % and 95 % points over its median.
    lo: f64,
    hi: f64,
}

/// [`Calibration`] at `k` lines, by `CALIBRATION_TRIALS` trials on `ws`; `None`
/// where the comb cannot be fitted.
fn calibrate(k: usize, ws: &mut Workspace) -> Option<Calibration> {
    ws.psi.clear();
    ws.psi.extend((0..k).map(|j| {
        let p = TAU * j as f64 / k as f64;
        if p > PI { p - TAU } else { p }
    }));
    if !ws.fit_lines() {
        return None;
    }
    ws.expected_covariance();
    let expected = capon(&mut ws.t, &mut ws.steer)?;
    let mut rng = Gaussian::new(SEED.wrapping_add(k as u64));
    let mut ratios = Vec::with_capacity(CALIBRATION_TRIALS);
    for _ in 0..CALIBRATION_TRIALS {
        for s in ws.z.iter_mut() {
            *s = C64::new(f64::from(rng.next()), f64::from(rng.next())) * FRAC_1_SQRT_2;
        }
        ws.residual_covariance();
        if let Some(p) = capon(&mut ws.q, &mut ws.steer) {
            ratios.push(p / expected);
        }
    }
    if ratios.len() < CALIBRATION_TRIALS / 2 {
        return None;
    }
    ratios.sort_by(f64::total_cmp);
    let at = |q: f64| ratios[((ratios.len() - 1) as f64 * q).round() as usize];
    let median = at(0.5);
    Some(Calibration {
        median,
        lo: at(0.05) / median,
        hi: at(0.95) / median,
    })
}

/// One probe's estimate.
#[derive(Clone, Copy, Debug)]
struct Estimate {
    /// The noise power per decimated sample at the probe's frequency.
    power: f64,
    /// The estimate's law on white noise at the model's size.
    cal: Calibration,
    /// Lines in the model, `K`.
    lines: usize,
}

impl Estimate {
    /// The noise's median magnitude at a window of `n` samples, in `4/n` units.
    fn level(&self, window: usize) -> f64 {
        gate_level(self.power, window)
    }

    /// The level's 5–95 % interval: the estimate's own scatter on white noise at
    /// its model size.
    fn interval(&self, window: usize) -> (f64, f64) {
        let level = self.level(window);
        (level / self.cal.hi.sqrt(), level / self.cal.lo.sqrt())
    }
}

/// What the arm counts over a set.
#[derive(Default)]
struct Tally {
    /// Probes the arm answered.
    answered: u64,
    /// Probes declined, by reason.
    declined: HashMap<Decline, u64>,
    /// Probes whose target line took the tracker's instantaneous frequency.
    from_f_inst: u64,
    /// Probes per model size.
    sizes: HashMap<usize, u64>,
    /// Scored pre-roll probes per window: probes, admitted, and admitted at the
    /// known-σ multiplier on the same estimate.
    silence: HashMap<usize, (u64, u64, u64)>,
    /// Time in the estimate and the decision, white-noise calibrations excluded.
    cost: Duration,
    /// Time spent calibrating model sizes.
    calibration: Duration,
}

/// The APES residual (G2, G3).
///
/// APES's noise covariance is `Q̂ = R̂ − Gᴴ W⁻¹ G` (Eq. 56): `R̂` the sample
/// covariance of the forward sub-vectors `x(n) = [z(n) … z(n + M − 1)]ᵀ`
/// (Eq. 58), `G` their projections on the model's exponentials
/// `w(n) = [e^{jψ₁n} … e^{jψ_K n}]ᵀ` and `W` those exponentials' own Gram
/// matrix (Eqs. 54, 55; the 2013 paper's Eqs. 13–15 give `w(n)` at arbitrary
/// frequencies). In large samples `Gᴴ W⁻¹ G` tends to `Σ_l |α_l|² a_l a_lᴴ`, the
/// fitted lines' own covariance (§IV). The APES filter under the one constraint
/// `hᴴ a = 1` at the probe's frequency (Eq. 57) leaves output noise power
/// `hᴴ Q̂ h = 1/(aᴴ Q̂⁻¹ a)` (Eq. 53), and `M` times that is the noise power per
/// decimated sample.
///
/// Per probe, the hop's 8192 samples are moved to baseband at the probe's
/// frequency, block-averaged and decimated by `D` to 128 samples over ±344.5 Hz.
/// The model holds every partial of the key in that band as a complex
/// exponential at its known frequency: the target's at the tracker's
/// instantaneous frequency where the probe carries one, else at the probe's
/// own; the others measured where the capture located them and predicted by
/// the stiff-string law where it did not; and of each the real signal's image
/// at `−f`. Partials outside the band reach it through the block average's
/// response, −3.9 dB at the band's edge and nil at every multiple of `fs/D`, and
/// are not modelled. The estimate is read at the probe's own frequency.
///
/// On white noise `M/(aᴴ Q̂⁻¹ a)` reads low, for two reasons. The fit takes the
/// noise at every modelled frequency with it, a notch the filter sees in full at
/// the target's own line; that is exact in expectation, `E[Q̂] = σ²T` with `T` in
/// closed form from the lines, so the estimate is divided by `M/(aᴴ T⁻¹ a)` for
/// the probe's own model. And the filter, fitted to the same data, finds less
/// noise than there is; that is the median of the ratio on white noise at the
/// same model size, simulated once per size at an even comb. With both, the
/// estimate's median on white noise is the noise's level within 0.15 dB for every
/// layout from one line to A0's twenty-four.
///
/// The estimate is the noise's median magnitude at the probe in its `4/n` units,
/// converted from the power per decimated sample by the block average's unit
/// gain and noise bandwidth `fs/D`, the Goertzel's `4/n` scaling and Hann
/// `Σw² = 0.375n`, and the Rayleigh median. It rests on about four independent
/// periodograms' worth of noise (the power's mean²/variance 3.7–4.2 over those
/// layouts), so its interval, from the 5–95 % range of the ratio on white noise
/// at its model size, is wide: about −3.3 to +3.8 dB around the estimate. The
/// threshold is the estimate times the upper `P_fa` point of
/// `amplitude / estimate` simulated on white noise at the probe's window: the
/// probe's own noise is part of the estimate, the two correlate at 4096 samples,
/// and a known-σ multiplier on a scattered estimate would not hold the rate.
///
/// # References
///
/// - Nørholm, S. M., Jensen, J. R. & Christensen, M. G. (2016). Enhancement and
///   noise statistics estimation for non-stationary voiced speech. IEEE/ACM
///   Trans. Audio, Speech, Lang. Process. 24(4), 645–658.
///   DOI: 10.1109/TASLP.2016.2514492. Eqs. 8, 53–58, §IV.
/// - Nørholm, S. M., Jensen, J. R. & Christensen, M. G. (2013). On the influence
///   of inharmonicities in model-based speech enhancement. Proc. 21st EUSIPCO.
///   Eqs. 1–4, 13–15.
pub(super) struct Apes {
    /// The operating point.
    p_fa: f64,
    /// The workspace probes run on.
    ws: Workspace,
    /// The workspace the white-noise calibrations run on.
    sim: Workspace,
    /// The estimate's law on white noise per model size, simulated on first use.
    calibrations: HashMap<usize, Option<Calibration>>,
    /// The null's squared ratios of amplitude to estimated level per window,
    /// ascending.
    null: HashMap<usize, Vec<f64>>,
    /// Time the null took.
    null_time: Duration,
    /// The threshold over the estimate per window, at the operating point.
    multipliers: HashMap<usize, f64>,
    /// What `finish` reports.
    tally: Tally,
}

impl Apes {
    /// An arm at 10⁻³ until `prepare` sets the operating point; the null is
    /// simulated on the first `prepare`.
    pub(super) fn new() -> Self {
        Self {
            p_fa: 1.0e-3,
            ws: Workspace::new(),
            sim: Workspace::new(),
            calibrations: HashMap::new(),
            null: HashMap::new(),
            null_time: Duration::ZERO,
            multipliers: HashMap::new(),
            tally: Tally::default(),
        }
    }

    /// The estimate's law on white noise at `k` lines, simulated on first use.
    fn calibration(&mut self, k: usize) -> Option<Calibration> {
        if let Some(&known) = self.calibrations.get(&k) {
            return known;
        }
        let start = Instant::now();
        let found = calibrate(k, &mut self.sim);
        self.tally.calibration += start.elapsed();
        self.calibrations.insert(k, found);
        found
    }

    /// The probe's estimate at `f_t`, the target's line at `f_line`.
    fn estimate(
        &mut self,
        audio: &[f32],
        cap: &CaptureCtx,
        index: usize,
        f_t: f64,
        f_line: f64,
    ) -> Result<Estimate, Decline> {
        let (relative, lines) = self.ws.relative_power(audio, cap, index, f_t, f_line)?;
        let cal = self.calibration(lines).ok_or(Decline::Solve)?;
        Ok(Estimate {
            power: relative / cal.median,
            cal,
            lines,
        })
    }

    /// The threshold's null: `NULL_TRIALS` frames of unit white noise through the
    /// estimate at `NULL_HZ` with the target alone in the band, and the probe's
    /// Goertzel over each window's last samples; per window the squared ratio of
    /// amplitude to estimated level, ascending.
    fn simulate_null(&mut self) {
        let start = Instant::now();
        let (ns, f_meas) = ([1usize], [NULL_HZ]);
        let cap = CaptureCtx {
            dir: "null",
            key: 0,
            register: 0,
            ns: &ns,
            f_meas: &f_meas,
            spacing_hz: NULL_HZ,
            b: 0.0,
            sigma_rec: 1.0,
        };
        let hann: Vec<Vec<f32>> = WINDOWS.iter().map(|&w| hann_vec(w)).collect();
        let mut rng = Gaussian::new(SEED);
        let mut x = vec![0.0f32; BASS_WINDOW_SIZE];
        let mut ratios = vec![Vec::with_capacity(NULL_TRIALS); WINDOWS.len()];
        let f = f64::from(NULL_HZ);
        for _ in 0..NULL_TRIALS {
            x.iter_mut().for_each(|s| *s = rng.next());
            let Ok(est) = self.estimate(&x, &cap, 0, f, f) else {
                continue;
            };
            for ((&w, h), r) in WINDOWS.iter().zip(&hann).zip(ratios.iter_mut()) {
                let (amp, _) = goertzel_windowed(&x, SAMPLE_RATE, NULL_HZ, h);
                r.push((f64::from(amp) / est.level(w)).powi(2));
            }
        }
        for (w, mut r) in WINDOWS.into_iter().zip(ratios) {
            r.sort_by(f64::total_cmp);
            self.null.insert(w, r);
        }
        self.null_time = start.elapsed();
    }

    /// The frequency-error sweep: `SWEEP_FRAMES` frames of white noise at
    /// `SWEEP_SIGMA`, each with one line at `SWEEP_HZ` of random phase, read with
    /// the model's line displaced by each of `SWEEP_OFFSETS_BINS`. Per
    /// displacement and level, `20 lg(estimate ÷ the noise's exact level)` at
    /// `SWEEP_WINDOW` over the frames; the last row is the same frames without
    /// the line.
    fn sweep(&mut self) -> Vec<[Vec<f64>; 2]> {
        let (ns, f_meas) = ([1usize], [SWEEP_HZ]);
        let cap = CaptureCtx {
            dir: "sweep",
            key: 0,
            register: 0,
            ns: &ns,
            f_meas: &f_meas,
            spacing_hz: SWEEP_HZ,
            b: 0.0,
            sigma_rec: SWEEP_SIGMA,
        };
        let truth = f64::from(raw_to_gate(awgn_median_bin(SWEEP_SIGMA), SWEEP_WINDOW));
        let fs = f64::from(SAMPLE_RATE);
        let bin_hz = fs / BASS_WINDOW_SIZE as f64;
        let f = f64::from(SWEEP_HZ);
        let mut rng = Gaussian::new(SEED + 1);
        let mut rows: Vec<[Vec<f64>; 2]> = vec![Default::default(); SWEEP_OFFSETS_BINS.len() + 1];
        let mut frame = vec![0.0f32; BASS_WINDOW_SIZE];
        for _ in 0..SWEEP_FRAMES {
            let noise: Vec<f32> = (0..BASS_WINDOW_SIZE)
                .map(|_| SWEEP_SIGMA * rng.next())
                .collect();
            let phase = f64::from(rng.next()).atan2(f64::from(rng.next()));
            for (s, &snr) in SWEEP_SNR_DB.iter().enumerate() {
                let amp = f64::from(tone_amplitude(SWEEP_SIGMA, snr, BASS_WINDOW_SIZE));
                for (i, (out, &v)) in frame.iter_mut().zip(&noise).enumerate() {
                    *out = v + (amp * (TAU * f * i as f64 / fs + phase).cos()) as f32;
                }
                for (row, &offset) in rows.iter_mut().zip(&SWEEP_OFFSETS_BINS) {
                    if let Ok(e) = self.estimate(&frame, &cap, 0, f, f + offset * bin_hz) {
                        row[s].push(20.0 * (e.level(SWEEP_WINDOW) / truth).log10());
                    }
                }
            }
            if let Ok(e) = self.estimate(&noise, &cap, 0, f, f) {
                let db = 20.0 * (e.level(SWEEP_WINDOW) / truth).log10();
                let last = rows.last_mut().expect("the no-line row");
                last[0].push(db);
                last[1].push(db);
            }
        }
        rows
    }

    /// The gate on one probe: the estimate at the probe's frequency, the target's
    /// line at the tracker's instantaneous frequency where the probe carries one
    /// inside the band, and the null's threshold at the probe's window.
    fn verdict(&mut self, hop: &HopCtx, probe: &GateProbe) -> Result<GateVerdict, Decline> {
        let k = *self.multipliers.get(&probe.window).ok_or(Decline::Series)?;
        let f_t = f64::from(probe.target_hz);
        let half = f64::from(SAMPLE_RATE) / DECIMATION as f64 / 2.0;
        let f_inst = probe
            .f_inst
            .map(f64::from)
            .filter(|f| f.is_finite() && (f - f_t).abs() < half);
        let est = self.estimate(
            hop.audio,
            hop.capture,
            probe.index,
            f_t,
            f_inst.unwrap_or(f_t),
        )?;
        let level = est.level(probe.window);
        let threshold = k * level;
        let amplitude = f64::from(probe.amplitude);
        let admit = amplitude >= threshold;
        let t = &mut self.tally;
        t.answered += 1;
        t.from_f_inst += u64::from(f_inst.is_some());
        *t.sizes.entry(est.lines).or_default() += 1;
        if hop.block == Block::Holdout {
            let known_sigma = level * f64::from(median_to_threshold(self.p_fa as f32));
            let s = t.silence.entry(probe.window).or_default();
            s.0 += 1;
            s.1 += u64::from(admit);
            s.2 += u64::from(amplitude >= known_sigma);
        }
        let (lo, hi) = est.interval(probe.window);
        Ok(GateVerdict {
            threshold: Some(threshold as f32),
            estimate: Some(level as f32),
            interval: Some((lo as f32, hi as f32)),
            admit: Some(admit),
        })
    }
}

/// `(median, 5 %, 95 %)` of `v`, sorted in place; NaN when empty.
fn spread(v: &mut [f64]) -> (f64, f64, f64) {
    if v.is_empty() {
        return (f64::NAN, f64::NAN, f64::NAN);
    }
    v.sort_by(f64::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    (at(0.5), at(0.05), at(0.95))
}

impl Arm for Apes {
    fn name(&self) -> &str {
        "apes"
    }

    fn consumers(&self) -> &[Consumer] {
        &[Consumer::G2, Consumer::G3]
    }

    fn prepare(&mut self, set: &SetCtx) {
        self.p_fa = f64::from(set.p_fa);
        self.tally = Tally::default();
        if self.null.is_empty() {
            self.simulate_null();
        }
        let p_fa = self.p_fa;
        self.multipliers = self
            .null
            .iter()
            .filter(|(_, r)| !r.is_empty())
            .map(|(&w, r)| {
                let idx = ((1.0 - p_fa) * (r.len() - 1) as f64).round() as usize;
                (w, r[idx.min(r.len() - 1)].sqrt())
            })
            .collect();
        let known = median_to_threshold(set.p_fa);
        let at = |w: usize| self.multipliers.get(&w).copied().unwrap_or(f64::NAN);
        println!(
            "[{}] arm apes: D = {DECIMATION}, M = {FILTER_LEN}, N = {DECIMATED_LEN} samples over \
             ±{:.1} Hz, {SNAPSHOTS} snapshots; the threshold over the estimate at {:.1e} from \
             {NULL_TRIALS} white-noise trials ({:.1} s): {:.3} at 1024, {:.3} at 4096, against \
             the known-σ {known:.3}",
            set.label,
            f64::from(SAMPLE_RATE) / DECIMATION as f64 / 2.0,
            set.p_fa,
            self.null_time.as_secs_f64(),
            at(1024),
            at(4096)
        );
    }

    fn gate(&mut self, hop: &HopCtx, probe: &GateProbe) -> GateVerdict {
        let start = Instant::now();
        let calibrating = self.tally.calibration;
        let verdict = match self.verdict(hop, probe) {
            Ok(v) => v,
            Err(reason) => {
                *self.tally.declined.entry(reason).or_default() += 1;
                GateVerdict::default()
            }
        };
        let spent_calibrating = self.tally.calibration.saturating_sub(calibrating);
        self.tally.cost += start.elapsed().saturating_sub(spent_calibrating);
        verdict
    }

    fn finish(&mut self, set: &SetCtx) {
        let declined = |r: Decline| self.tally.declined.get(&r).copied().unwrap_or(0);
        println!(
            "[{}] arm apes: {} probes answered, {} with the target's line at the tracker's \
             instantaneous frequency; declined {} without a series or window, {} with more than \
             {MAX_LINES} lines, {} not positive definite",
            set.label,
            self.tally.answered,
            self.tally.from_f_inst,
            declined(Decline::Series),
            declined(Decline::Lines),
            declined(Decline::Solve)
        );
        let mut sizes: Vec<(usize, u64)> = self.tally.sizes.iter().map(|(&k, &c)| (k, c)).collect();
        sizes.sort_unstable();
        if !sizes.is_empty() {
            println!(
                "     the estimate's law on white noise per model size K: the filter's median \
                 under the expectation, and the 90 % interval on the level around the estimate"
            );
        }
        for (k, count) in &sizes {
            if let Some(Some(cal)) = self.calibrations.get(k) {
                println!(
                    "       K {k:>2} ({count:>8} probes): {:.4}, [{:+.1}, {:+.1}] dB",
                    cal.median,
                    -10.0 * cal.hi.log10(),
                    -10.0 * cal.lo.log10()
                );
            }
        }
        let known = median_to_threshold(set.p_fa);
        for w in WINDOWS {
            if let Some(&(n, admitted, at_known)) = self.tally.silence.get(&w) {
                let rate = |k: u64| k as f64 / n.max(1) as f64;
                println!(
                    "     the scored pre-roll at {w}: {n} probes; the arm admits {admitted} ({:.2e}), \
                     the known-σ multiplier {known:.3} on the same estimates would admit {at_known} \
                     ({:.2e})",
                    rate(admitted),
                    rate(at_known)
                );
            }
        }
        let start = Instant::now();
        let mut rows = self.sweep();
        println!(
            "     the frequency-error sweep ({:.1} s): a line at {SWEEP_HZ} Hz in white noise of σ = \
             {SWEEP_SIGMA:.0e}, {SWEEP_FRAMES} frames, the model's line displaced; \
             20 lg(estimate ÷ the noise's exact level) at the {SWEEP_WINDOW}-sample window, \
             median [5 %, 95 %]",
            start.elapsed().as_secs_f64()
        );
        println!(
            "       {:<24} {:>26} {:>26}",
            "displacement",
            format!("{} dB per bin", SWEEP_SNR_DB[0]),
            format!("{} dB per bin", SWEEP_SNR_DB[1])
        );
        let bin_hz = f64::from(SAMPLE_RATE) / BASS_WINDOW_SIZE as f64;
        let labels = SWEEP_OFFSETS_BINS
            .iter()
            .map(|&o| format!("{o:.2} bin ({:.2} Hz)", o * bin_hz))
            .chain(std::iter::once("no line".to_string()));
        for (label, row) in labels.zip(rows.iter_mut()) {
            let cells: Vec<String> = row
                .iter_mut()
                .map(|v| {
                    let (m, lo, hi) = spread(v);
                    format!("{m:+.1} [{lo:+.1}, {hi:+.1}] n{}", v.len())
                })
                .collect();
            println!("       {label:<24} {:>26} {:>26}", cells[0], cells[1]);
        }
        if self.tally.answered > 0 {
            println!(
                "     cost: {:.0} µs per probe over {} probes; the white-noise calibrations of {} \
                 model sizes took {:.1} s and are not in it",
                self.tally.cost.as_secs_f64() * 1.0e6 / self.tally.answered as f64,
                self.tally.answered,
                self.calibrations.len(),
                self.tally.calibration.as_secs_f64()
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gates::stats;

    const SIGMA: f32 = 1.0e-3;
    const FRAMES: usize = 50;

    /// Partials `1..=count` of a key on the stiff-string law, as a capture
    /// would list them.
    fn series(f0: f32, b: f32, count: usize) -> (Vec<usize>, Vec<f32>) {
        let ns: Vec<usize> = (1..=count).collect();
        let f = ns
            .iter()
            .map(|&n| ambient::stiff_partial(f0, b, n))
            .collect();
        (ns, f)
    }

    fn capture<'a>(ns: &'a [usize], f_meas: &'a [f32], f0: f32, b: f32) -> CaptureCtx<'a> {
        CaptureCtx {
            dir: "test",
            key: 0,
            register: 0,
            ns,
            f_meas,
            spacing_hz: f0,
            b,
            sigma_rec: SIGMA,
        }
    }

    /// One hop of white noise at `SIGMA`.
    fn noise(rng: &mut Gaussian) -> Vec<f32> {
        (0..BASS_WINDOW_SIZE).map(|_| SIGMA * rng.next()).collect()
    }

    /// `x` with a cosine of amplitude `amp` at `f` Hz and phase `phase` added.
    fn with_line(x: &[f32], amp: f64, f: f64, phase: f64) -> Vec<f32> {
        let fs = f64::from(SAMPLE_RATE);
        x.iter()
            .enumerate()
            .map(|(i, &v)| v + (amp * (TAU * f * i as f64 / fs + phase).cos()) as f32)
            .collect()
    }

    /// A Hermitian positive-definite system, `B Bᴴ + I`, solved through the
    /// factor reproduces its right-hand side to 10⁻¹⁰, and an indefinite one is
    /// refused.
    #[test]
    fn cholesky_solves_a_hermitian_system() {
        let n = 6;
        let mut rng = Gaussian::new(1);
        let b: Vec<C64> = (0..n * n)
            .map(|_| C64::new(f64::from(rng.next()), f64::from(rng.next())))
            .collect();
        let mut a = vec![C64::new(0.0, 0.0); n * n];
        for i in 0..n {
            for j in 0..n {
                let identity = if i == j { 1.0 } else { 0.0 };
                a[i * n + j] = (0..n)
                    .map(|k| b[i * n + k] * b[j * n + k].conj())
                    .sum::<C64>()
                    + identity;
            }
        }
        let full = a.clone();
        let rhs: Vec<C64> = (0..n).map(|i| C64::new(i as f64, 1.0 - i as f64)).collect();
        let mut x = rhs.clone();
        assert!(cholesky(&mut a, n));
        solve_lower(&a, n, &mut x);
        solve_upper(&a, n, &mut x);
        for i in 0..n {
            let ax: C64 = (0..n).map(|j| full[i * n + j] * x[j]).sum();
            assert!((ax - rhs[i]).norm() < 1e-10, "row {i}: {ax} vs {}", rhs[i]);
        }
        let one = C64::new(1.0, 0.0);
        let mut indefinite = vec![one, C64::new(0.0, 0.0), C64::new(2.0, 0.0), one];
        assert!(!cholesky(&mut indefinite, 2));
    }

    /// White noise of σ leaves the decimator at σ²/D per sample, within 2 % over
    /// 200 hops. A line at the centre comes out as half its amplitude at its
    /// phase, its image falling on a null of the block average; a line 100 Hz
    /// above advances `2π·100·D/fs` radians per sample.
    #[test]
    fn decimation_keeps_lines_and_noise_power() {
        let fs = f64::from(SAMPLE_RATE);
        let mut rng = Gaussian::new(SEED);
        let mut rotor = vec![C64::new(0.0, 0.0); DECIMATION];
        let mut z = vec![C64::new(0.0, 0.0); DECIMATED_LEN];
        let mut power = 0.0;
        for _ in 0..200 {
            decimate(&noise(&mut rng), 1234.5, &mut rotor, &mut z);
            power += z.iter().map(|c| c.norm_sqr()).sum::<f64>();
        }
        let ratio =
            power / (200 * DECIMATED_LEN) as f64 / f64::from(SIGMA).powi(2) * DECIMATION as f64;
        println!("decimated white noise over σ²/D: {ratio:.4}");
        assert!((ratio - 1.0).abs() < 0.02, "{ratio}");

        let f_t = 5.0 * fs / DECIMATION as f64;
        let (amp, phase) = (0.01, 0.3);
        let silent = vec![0.0f32; BASS_WINDOW_SIZE];
        decimate(
            &with_line(&silent, amp, f_t, phase),
            f_t,
            &mut rotor,
            &mut z,
        );
        let expected = C64::from_polar(amp / 2.0, phase);
        for c in &z {
            assert!((c - expected).norm() < 1e-4 * amp, "{c} vs {expected}");
        }
        let nu = 100.0;
        decimate(
            &with_line(&silent, amp, f_t + nu, 0.0),
            f_t,
            &mut rotor,
            &mut z,
        );
        let step = TAU * nu * DECIMATION as f64 / fs;
        for pair in z.windows(2) {
            let advance = (pair[1] / pair[0]).arg();
            assert!((advance - step).abs() < 0.05, "{advance} vs {step}");
        }
    }

    /// A bass probe's model holds the target first at the band's centre, every
    /// partial within the band and the images that fall in it; a treble probe's
    /// holds the target alone.
    #[test]
    fn the_model_holds_the_band() {
        let (f0, b) = (32.7f32, 2.0e-4f32);
        let (ns, f) = series(f0, b, 8);
        let cap = capture(&ns, &f, f0, b);
        let half = f64::from(SAMPLE_RATE) / DECIMATION as f64 / 2.0;
        let f_t = f64::from(f[1]);
        let mut psi = Vec::new();
        assert!(model_lines(&cap, 1, f_t, f_t, &mut psi));
        let expected: usize = (1..60)
            .map(|n| {
                let f_n = f64::from(ambient::stiff_partial(f0, b, n));
                usize::from((f_n - f_t).abs() < half) + usize::from(f_n + f_t < half)
            })
            .sum();
        println!(
            "C1's second partial: {} lines, {expected} expected",
            psi.len()
        );
        assert_eq!(psi[0], 0.0);
        assert_eq!(psi.len(), expected);
        assert!(psi.iter().all(|p| p.abs() <= PI));

        let (f0, b) = (1046.5f32, 4.0e-4f32);
        let (ns, f) = series(f0, b, 4);
        let cap = capture(&ns, &f, f0, b);
        assert!(model_lines(
            &cap,
            0,
            f64::from(f[0]),
            f64::from(f[0]),
            &mut psi
        ));
        assert_eq!(psi, vec![0.0]);
        let flat = capture(&ns, &f, 0.0, b);
        assert!(!model_lines(
            &flat,
            0,
            f64::from(f[0]),
            f64::from(f[0]),
            &mut psi
        ));
    }

    /// The residual form is Eq. 56: on random data with three lines, `Q̂` equals
    /// `R̂ − Gᴴ W⁻¹ G` built from the paper's own sums (Eqs. 54, 55, 58),
    /// `R̂ = (1/L) Σ x(n) x(n)ᴴ`, `G = (1/L) Σ w(n) x(n)ᴴ`, `W = (1/L) Σ w(n) w(n)ᴴ`,
    /// to 10⁻¹² in every entry of the lower triangle.
    #[test]
    fn the_residual_form_is_eq_56() {
        let (k, l, m) = (3, SNAPSHOTS, FILTER_LEN);
        let mut ws = Workspace::new();
        ws.psi.extend([0.0, 0.7, -2.0]);
        assert!(ws.fit_lines());
        let mut rng = Gaussian::new(SEED + 6);
        for s in ws.z.iter_mut() {
            *s = C64::new(f64::from(rng.next()), f64::from(rng.next()));
        }
        ws.residual_covariance();
        let x = |n: usize, i: usize| ws.z[n + i];
        let w = |n: usize, a: usize| C64::from_polar(1.0, ws.psi[a] * n as f64);
        let zero = C64::new(0.0, 0.0);
        let (mut r_hat, mut g, mut w_mat) =
            (vec![zero; m * m], vec![zero; k * m], vec![zero; k * k]);
        for n in 0..l {
            for i in 0..m {
                for j in 0..m {
                    r_hat[i * m + j] += x(n, i) * x(n, j).conj() / l as f64;
                }
                for a in 0..k {
                    g[a * m + i] += w(n, a) * x(n, i).conj() / l as f64;
                }
            }
            for a in 0..k {
                for b in 0..k {
                    w_mat[a * k + b] += w(n, a) * w(n, b).conj() / l as f64;
                }
            }
        }
        assert!(cholesky(&mut w_mat, k));
        let mut worst = 0.0f64;
        let mut column = vec![zero; k];
        let mut w_inv_g = vec![zero; k * m];
        for j in 0..m {
            for a in 0..k {
                column[a] = g[a * m + j];
            }
            solve_lower(&w_mat, k, &mut column);
            solve_upper(&w_mat, k, &mut column);
            for a in 0..k {
                w_inv_g[a * m + j] = column[a];
            }
        }
        for i in 0..m {
            for j in 0..=i {
                let fitted: C64 = (0..k)
                    .map(|a| g[a * m + i].conj() * w_inv_g[a * m + j])
                    .sum();
                worst = worst.max((r_hat[i * m + j] - fitted - ws.q[i * m + j]).norm());
            }
        }
        println!("largest |R̂ − Gᴴ W⁻¹ G − Q̂| over the lower triangle: {worst:.2e}");
        assert!(worst < 1e-12, "{worst}");
    }

    /// `T` is the mean of `Q̂` on unit complex white noise: every entry of the
    /// lower triangle within 0.02 of the mean over 3000 trials, for three lines
    /// placed asymmetrically, and `t(0) = 1 − K/L` exactly.
    #[test]
    fn the_expected_covariance_is_the_mean_of_q() {
        let mut ws = Workspace::new();
        ws.psi.extend([0.0, 0.3, -1.1]);
        assert!(ws.fit_lines());
        ws.expected_covariance();
        let t = ws.t.clone();
        let m = FILTER_LEN;
        let trials = 3000;
        let mut mean = vec![C64::new(0.0, 0.0); m * m];
        let mut rng = Gaussian::new(SEED);
        for _ in 0..trials {
            for s in ws.z.iter_mut() {
                *s = C64::new(f64::from(rng.next()), f64::from(rng.next())) * FRAC_1_SQRT_2;
            }
            ws.residual_covariance();
            for (acc, q) in mean.iter_mut().zip(&ws.q) {
                *acc += q / trials as f64;
            }
        }
        let mut worst = 0.0f64;
        for i in 0..m {
            for j in 0..=i {
                worst = worst.max((mean[i * m + j] - t[i * m + j]).norm());
            }
        }
        println!(
            "largest |mean Q̂ − T| over the lower triangle: {worst:.4}; t(0) = {:.5}",
            t[0].re
        );
        assert!(worst < 0.02, "{worst}");
        assert!((t[0].re - (1.0 - 3.0 / SNAPSHOTS as f64)).abs() < 1e-12);
    }

    /// The conversion's white-noise identity: `σ²/D` per decimated sample is
    /// `raw_to_gate` of the analytic 8192-point level `σ√(0.375 · 8192 · ln 2)`
    /// at both windows.
    #[test]
    fn the_level_is_the_goertzels_on_white_noise() {
        for w in WINDOWS {
            let ours = gate_level(f64::from(SIGMA).powi(2) / DECIMATION as f64, w);
            let analytic = f64::from(raw_to_gate(awgn_median_bin(SIGMA), w));
            assert!(
                (ours / analytic - 1.0).abs() < 1e-5,
                "{w}: {ours} vs {analytic}"
            );
        }
    }

    /// White noise: the estimate's median over 50 hops within 1 dB of the
    /// analytic level, `raw_to_gate(σ√(0.375 · 8192 · ln 2), n)`, for a treble
    /// probe alone in its band at 1024 samples and two bass probes among their
    /// keys' modelled partials and images at 4096. The coverage of the stated
    /// interval is printed beside it.
    #[test]
    fn white_noise_reads_the_analytic_level() {
        let mut arm = Apes::new();
        let mut rng = Gaussian::new(SEED + 2);
        let cases = [
            (1046.5f32, 4.0e-4f32, 1usize, 1024usize),
            (55.0, 1.5e-4, 2, 4096),
            (32.7, 2.0e-4, 0, 4096),
        ];
        for (f0, b, index, window) in cases {
            let (ns, f) = series(f0, b, 8);
            let cap = capture(&ns, &f, f0, b);
            let f_t = f64::from(f[index]);
            let truth = f64::from(raw_to_gate(awgn_median_bin(SIGMA), window));
            let mut db = Vec::with_capacity(FRAMES);
            let (mut covered, mut lines) = (0, 0);
            for _ in 0..FRAMES {
                let e = arm
                    .estimate(&noise(&mut rng), &cap, index, f_t, f_t)
                    .expect("an estimate");
                db.push(20.0 * (e.level(window) / truth).log10());
                let (lo, hi) = e.interval(window);
                covered += usize::from(lo <= truth && truth <= hi);
                lines = e.lines;
            }
            let (m, lo, hi) = spread(&mut db);
            println!(
                "f0 {f0} Hz, partial {}, {f_t:.1} Hz at {window}: {lines} lines; estimate over the \
                 analytic level {m:+.2} dB [{lo:+.2}, {hi:+.2}]; the interval covers it on {covered} \
                 of {FRAMES}",
                ns[index]
            );
            assert!(m.abs() < 1.0, "f0 {f0}: {m:+.2} dB");
        }
    }

    /// The two corrections hold across the layouts keys produce: over 1500 hops
    /// of white noise per layout, from a treble probe alone in its band to A0's
    /// twenty-four lines and a probe whose own band holds the noise's mirror
    /// (32.7 Hz), the estimate's median within 0.35 dB of the analytic level
    /// (about four standard errors). Each layout's median, its standard error
    /// and the power ratio's mean²/variance are printed.
    #[test]
    fn the_correction_holds_across_layouts() {
        let mut arm = Apes::new();
        let mut rng = Gaussian::new(SEED + 5);
        let hops = 1500;
        let layouts = [
            (1046.5f32, 4.0e-4f32, 1usize, 1024usize),
            (220.0, 2.0e-4, 1, 1024),
            (110.0, 2.0e-4, 1, 1024),
            (55.0, 1.5e-4, 2, 4096),
            (32.7, 2.0e-4, 0, 4096),
            (32.7, 2.0e-4, 5, 4096),
            (27.5, 2.0e-4, 3, 4096),
        ];
        for (f0, b, index, window) in layouts {
            let (ns, f) = series(f0, b, 8);
            let cap = capture(&ns, &f, f0, b);
            let f_t = f64::from(f[index]);
            let truth = f64::from(raw_to_gate(awgn_median_bin(SIGMA), window));
            let mut db = Vec::with_capacity(hops);
            let mut power = Vec::with_capacity(hops);
            let mut lines = 0;
            for _ in 0..hops {
                let e = arm
                    .estimate(&noise(&mut rng), &cap, index, f_t, f_t)
                    .expect("an estimate");
                let ratio = e.level(window) / truth;
                db.push(20.0 * ratio.log10());
                power.push(ratio * ratio);
                lines = e.lines;
            }
            let n = hops as f64;
            let mean = power.iter().sum::<f64>() / n;
            let var = power.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
            let (m, lo, hi) = spread(&mut db);
            let sd = (db.iter().map(|d| (d - m).powi(2)).sum::<f64>() / n).sqrt();
            let se = 1.2533 * sd / n.sqrt();
            println!(
                "f0 {f0} Hz, partial {}, {f_t:.1} Hz at {window}: {lines} lines; median {m:+.3} dB \
                 ± {se:.3} [{lo:+.2}, {hi:+.2}]; power mean²/variance {:.2}",
                ns[index],
                mean * mean / var
            );
            assert!(m.abs() < 0.35, "f0 {f0}, partial {}: {m:+.3} dB", ns[index]);
        }
    }

    /// One 40 dB line at the probe with the model's frequency exact, as the
    /// probe's own and as an instantaneous frequency: the estimate's median over
    /// 50 hops within 1.5 dB of the noise's level, the fit having taken the line.
    #[test]
    fn an_exact_model_removes_the_line() {
        let mut arm = Apes::new();
        let mut rng = Gaussian::new(SEED + 3);
        let f = f64::from(SWEEP_HZ);
        let (ns, f_meas) = ([1usize], [SWEEP_HZ]);
        let cap = capture(&ns, &f_meas, SWEEP_HZ, 0.0);
        let amp = f64::from(tone_amplitude(SIGMA, 40.0, BASS_WINDOW_SIZE));
        let truth = f64::from(raw_to_gate(awgn_median_bin(SIGMA), SWEEP_WINDOW));
        let mut db = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let phase = f64::from(rng.next()).atan2(f64::from(rng.next()));
            let x = with_line(&noise(&mut rng), amp, f, phase);
            let e = arm.estimate(&x, &cap, 0, f, f).expect("an estimate");
            db.push(20.0 * (e.level(SWEEP_WINDOW) / truth).log10());
        }
        let (m, lo, hi) = spread(&mut db);
        println!("40 dB line, model exact: {m:+.2} dB [{lo:+.2}, {hi:+.2}] over the noise's level");
        assert!(m.abs() < 1.5, "{m:+.2} dB");
    }

    /// The same line with the model half an 8192-point bin off: the residual's
    /// excess over the noise's level, printed. A measurement, no bar.
    #[test]
    fn a_half_bin_error_leaves_the_line() {
        let mut arm = Apes::new();
        let mut rng = Gaussian::new(SEED + 3);
        let f = f64::from(SWEEP_HZ);
        let offset = 0.5 * f64::from(SAMPLE_RATE) / BASS_WINDOW_SIZE as f64;
        let (ns, f_meas) = ([1usize], [SWEEP_HZ]);
        let cap = capture(&ns, &f_meas, SWEEP_HZ, 0.0);
        let amp = f64::from(tone_amplitude(SIGMA, 40.0, BASS_WINDOW_SIZE));
        let truth = f64::from(raw_to_gate(awgn_median_bin(SIGMA), SWEEP_WINDOW));
        let mut db = Vec::with_capacity(FRAMES);
        for _ in 0..FRAMES {
            let phase = f64::from(rng.next()).atan2(f64::from(rng.next()));
            let x = with_line(&noise(&mut rng), amp, f, phase);
            let e = arm
                .estimate(&x, &cap, 0, f, f + offset)
                .expect("an estimate");
            db.push(20.0 * (e.level(SWEEP_WINDOW) / truth).log10());
        }
        let (m, lo, hi) = spread(&mut db);
        println!(
            "40 dB line, model {offset:.2} Hz off: {m:+.2} dB [{lo:+.2}, {hi:+.2}] over the noise's level"
        );
        assert!(m.is_finite());
    }

    /// Through the trait at 10⁻²: on fresh white noise at three frequencies the
    /// null's threshold admits a share of probes whose 95 % Clopper–Pearson
    /// interval holds 10⁻², at each window; the scored pre-roll's tally counts
    /// every probe; a capture without a series is declined.
    #[test]
    fn the_null_holds_its_rate() {
        let p = 1.0e-2;
        let mut arm = Apes::new();
        arm.prepare(&SetCtx {
            label: "test",
            p_fa: p as f32,
            sigma_override: None,
            nu_half_per_frame: None,
        });
        let mut rng = Gaussian::new(SEED + 4);
        let probes = 3000;
        for window in WINDOWS {
            let hann = hann_vec(window);
            let mut admitted = 0u64;
            for i in 0..probes {
                let f = [700.0f32, 1000.0, 2500.0][i % 3];
                let (ns, f_meas) = ([1usize], [f]);
                let cap = capture(&ns, &f_meas, f, 0.0);
                let audio = noise(&mut rng);
                let hop = HopCtx {
                    hop: -1,
                    t_since: -1.0,
                    bucket: 0,
                    is_silence: true,
                    block: Block::Holdout,
                    audio: &audio,
                    spectrum: &[],
                    mag: &[],
                    hz_per_bin: SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32,
                    sigma: SIGMA,
                    truth_level: None,
                    oracle: &[],
                    capture: &cap,
                };
                let (amplitude, _) = goertzel_windowed(&audio, SAMPLE_RATE, f, &hann);
                let probe = GateProbe {
                    index: 0,
                    target_hz: f,
                    window,
                    amplitude,
                    f_inst: None,
                    fixed: true,
                    truth_level: None,
                };
                let v = arm.gate(&hop, &probe);
                admitted += u64::from(v.admit.expect("an answer"));
            }
            let (lo, hi) = stats::clopper_pearson(admitted, probes as u64, 0.05);
            println!(
                "{window}: {admitted} / {probes} admitted at threshold {:.3} × the estimate, 95 % CP \
                 [{lo:.2e}, {hi:.2e}]",
                arm.multipliers[&window]
            );
            assert!(lo <= p && p <= hi, "{window}: [{lo:e}, {hi:e}]");
            assert_eq!(arm.tally.silence[&window].0, probes as u64);
        }
        let (ns, f_meas) = ([1usize], [1000.0f32]);
        let flat = capture(&ns, &f_meas, 0.0, 0.0);
        let audio = noise(&mut rng);
        let hop = HopCtx {
            hop: 0,
            t_since: 0.1,
            bucket: 0,
            is_silence: false,
            block: Block::Note,
            audio: &audio,
            spectrum: &[],
            mag: &[],
            hz_per_bin: SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32,
            sigma: SIGMA,
            truth_level: None,
            oracle: &[],
            capture: &flat,
        };
        let probe = GateProbe {
            index: 0,
            target_hz: 1000.0,
            window: 1024,
            amplitude: 1.0,
            f_inst: Some(1001.0),
            fixed: false,
            truth_level: None,
        };
        assert!(arm.gate(&hop, &probe).admit.is_none());
    }
}
