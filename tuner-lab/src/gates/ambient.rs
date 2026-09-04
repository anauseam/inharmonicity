//! # The one ambient scalar, at all three sites that threshold against it
//!
//! `config.silence_threshold` is calibrated once from ambient silence and then
//! thresholded at three hot-path detectors — `engine.rs`'s FFT-magnitude peak
//! gate and Goertzel tracker gate, and `strobe.rs`'s fixed-reference gate. Per
//! hop and per partial this measures the signal and the noise present beside it
//! in the *same* window, classifies the partial live or dead against that, and
//! scores all three gates over a σ sweep.
//!
//! Several populations run together so their columns are directly comparable.
//! Reproduces [ADR 0015](../../../docs/adr/0015-ambient-sigma-gates-measured.md).

use std::path::{Path, PathBuf};

use realfft::RealFftPlanner;

use crate::capture::strobe_register as register;
use crate::regen::{Capture, ET_PLAUSIBLE_CENTS, load as load_regen};
use crate::truth::*;
use tuner_core::algorithms::spectral::{goertzel_windowed, neyman_pearson_k};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

/// Analysis length for the local-noise probe, and the window discovery's own
/// peak floor already runs at. 8192 is the shortest window whose Hann main lobe
/// (±2·fs/N ≈ ±10.8 Hz) still leaves a gap between A0's 27.5 Hz-spaced
/// partials; at the 1024- and 4096-sample windows the tracker and strobe gates
/// use, the lobes tile the bass and no off-partial probe exists at all.
const NOISE_WIN: usize = BASS_WINDOW_SIZE;

/// Zero-padding factor for the probe spectrum: takes the bin grid to 1.35 Hz so
/// a 6 Hz gap still holds the three bins [`MIN_GAP_BINS`] requires. An integer
/// factor also makes the unpadded 8192 spectrum a subsequence of this one —
/// `mag[NOISE_PAD·k]` is bin `k` of the FFT the shipped peak floor thresholds.
const NOISE_PAD: usize = 4;

/// Minimum zero-padded bins an inter-partial gap must hold before it may
/// contribute a noise sample (ADR 0015 §1, pre-registered). Below it the row is
/// unmeasurable and is reported as such rather than defaulted.
const MIN_GAP_BINS: usize = 3;

/// Median magnitude → the P_fa = 10⁻³ threshold, for Rayleigh bins:
/// `T/median = √(ln(1/P_fa) / ln 2) = 3.157`. This is [`cfar_multiplier`] at the
/// median quantile — the same conversion the coarse read's OS-CFAR gate uses, so
/// the two are calibrated alike rather than merely similar.
///
/// Without it a threshold compared against `N_local` is being compared against
/// the noise's *median*, which a correctly-specified detector must sit 3.157×
/// above. An earlier revision of ADR 0015 omitted it and so read the bass
/// threshold as "0.77× — nearly right" when it is ≈ 4× too **low**.
fn threshold_for_median(n_local: f32) -> f32 {
    n_local * cfar_multiplier(0.5)
}

/// `SNR_gate` below which a partial counts as **dead** — indistinguishable from
/// its own neighbourhood in the window the gate actually uses.
const SNR_DEAD: f32 = 1.0;

/// `SNR_gate` at or above which a partial counts as **live**. Anchored to
/// ADR 0014 §8a, which measured the strobe gate closing while partials were
/// still 3–15× above the noise beside them: 3 is the bottom of a range already
/// measured on this project's data, not a chosen bar (`07` §2).
const SNR_LIVE: f32 = 3.0;

/// Onset reference: the first hop reaching −20 dB of the record's loudest
/// 1024-sample RMS. Dimensionless by construction (`07` §7), so it survives a
/// gain change and needs no calibrated floor — the quantity under scrutiny here.
const ONSET_REL_DB: f32 = -20.0;

/// Hann main-lobe half-width (null-to-null / 2) for an `n`-sample window.
fn lobe_hw_hz(n: usize) -> f32 {
    2.0 * SAMPLE_RATE as f32 / n as f32
}

/// Stiff-string partial frequency, `fₙ = n·f₀·√(1 + B n²)`.
fn stiff_partial(f0: f32, b: f32, n: usize) -> f32 {
    n as f32 * f0 * (1.0 + b * (n * n) as f32).sqrt()
}

/// The three hot-path detectors that share `config.silence_threshold`. All
/// three reduce to `T = σ·K(N)` in physical amplitude units — discovery's
/// `√(−σ²·0.375N·ln P_fa)` is `σ·K(N)` once the `4/N` normalization is applied,
/// so they differ only in what they evaluate and over how long.
#[derive(Clone, Copy, PartialEq, Eq)]
enum GateSite {
    /// `engine.rs:252` — discovery's FFT peak floor, always 8192. The only one
    /// of the three that *scans*, so ADR 0011 §5's search loss applies to it
    /// and to neither of the others.
    Peaks,
    /// `engine.rs:394` — the tracker's Goertzel gate at an adaptive centre.
    Tracker,
    /// `strobe.rs:253` — the bank's D3 gate at a fixed reference.
    Strobe,
}

impl GateSite {
    fn label(self) -> &'static str {
        match self {
            GateSite::Peaks => "#1 peaks  ",
            GateSite::Tracker => "#2 tracker",
            GateSite::Strobe => "#3 strobe ",
        }
    }
}

const GATES: [GateSite; 3] = [GateSite::Peaks, GateSite::Tracker, GateSite::Strobe];

/// One hop's probe spectrum in the `4/N` physical-amplitude units the Goertzel
/// evaluators return, so a sinusoid of amplitude A peaks at A whatever the
/// window length. Noise carries no such invariance — it falls as 1/√N — and
/// that asymmetry *is* the processing gain, corrected explicitly wherever a
/// gate's own window differs from this one (ADR 0015 §1).
struct ProbeSpectrum {
    mag: Vec<f32>,
    hz_per_bin: f32,
}

impl ProbeSpectrum {
    /// Largest magnitude in `[lo_hz, hi_hz]`, over every zero-padded bin.
    fn peak(&self, lo_hz: f32, hi_hz: f32) -> f32 {
        let lo = (lo_hz / self.hz_per_bin).floor().max(1.0) as usize;
        let hi = ((hi_hz / self.hz_per_bin).ceil() as usize).min(self.mag.len() - 1);
        if lo > hi {
            return 0.0;
        }
        self.mag[lo..=hi].iter().copied().fold(0.0, f32::max)
    }

    /// Largest magnitude in `[lo_hz, hi_hz]` over the **unpadded** 8192 bins
    /// only — what discovery's peak extractor sees, scalloping loss included.
    fn peak_coarse(&self, lo_hz: f32, hi_hz: f32) -> f32 {
        let lo = (lo_hz / self.hz_per_bin).floor().max(1.0) as usize;
        let hi = ((hi_hz / self.hz_per_bin).ceil() as usize).min(self.mag.len() - 1);
        let mut best = 0.0f32;
        let mut k = lo.div_ceil(NOISE_PAD) * NOISE_PAD;
        while k <= hi {
            best = best.max(self.mag[k]);
            k += NOISE_PAD;
        }
        best
    }

    /// Appends every bin in `[lo_hz, hi_hz]` to `out` and returns how many. The
    /// caller pools both flanking gaps before taking one median, so a narrow
    /// gap contributes its weight rather than half the answer.
    fn collect_band(&self, lo_hz: f32, hi_hz: f32, out: &mut Vec<f32>) -> usize {
        let lo = (lo_hz / self.hz_per_bin).ceil().max(1.0) as usize;
        let hi = ((hi_hz / self.hz_per_bin).floor() as usize).min(self.mag.len() - 1);
        if lo > hi {
            return 0;
        }
        out.extend_from_slice(&self.mag[lo..=hi]);
        hi - lo + 1
    }
}

/// The probe spectrum over the freshest `win` samples ending at `end`, in the
/// `4/win` physical units. `hann` must be `win` long.
fn probe_spectrum_at(
    signal: &[f32],
    end: usize,
    win: usize,
    hann: &[f32],
    planner: &mut RealFftPlanner<f32>,
    scratch: &mut Vec<f32>,
) -> Option<ProbeSpectrum> {
    if end < win || end > signal.len() {
        return None;
    }
    let n = win * NOISE_PAD;
    scratch.clear();
    scratch.resize(n, 0.0);
    for (i, w) in hann.iter().enumerate() {
        scratch[i] = signal[end - win + i] * w;
    }
    let fftp = planner.plan_fft_forward(n);
    let mut spec = fftp.make_output_vec();
    fftp.process(scratch, &mut spec).ok()?;
    let norm = 4.0 / win as f32;
    Some(ProbeSpectrum {
        mag: spec.iter().map(|c| c.norm() * norm).collect(),
        hz_per_bin: SAMPLE_RATE as f32 / n as f32,
    })
}

/// [`probe_spectrum_at`] at the standing [`NOISE_WIN`] probe window.
fn probe_spectrum(
    signal: &[f32],
    end: usize,
    hann: &[f32],
    planner: &mut RealFftPlanner<f32>,
    scratch: &mut Vec<f32>,
) -> Option<ProbeSpectrum> {
    probe_spectrum_at(signal, end, NOISE_WIN, hann, planner, scratch)
}

/// **S and N_local for one partial**, both read from the same window.
///
/// `f_n` is the partial's measured frequency; `f_lo`/`f_hi` its neighbours'
/// predicted positions, which bound the two gaps the noise is read from. Each
/// gap is inset by one main-lobe half-width at both ends, excluding the
/// target's lobe and the neighbours'. What leaks past that is Hann sidelobe at
/// −31 dB, which inflates N_local and so understates every SNR below — the
/// conservative direction for the claim under test (ADR 0014 §8a made the same
/// argument).
///
/// `None` where neither gap reaches [`MIN_GAP_BINS`].
fn partial_probe_at(
    spec: &ProbeSpectrum,
    win: usize,
    f_n: f32,
    f_lo: f32,
    f_hi: f32,
    scratch: &mut Vec<f32>,
) -> Option<(f32, f32)> {
    let l = lobe_hw_hz(win);
    // The string drifts a few cents across a 5 s note; the main lobe is the
    // widest the peak can wander without the gaps moving with it.
    let s = spec.peak(f_n - l, f_n + l);

    scratch.clear();
    let mut bins = 0;
    // Below f₁ there is no partial, only rumble — still local noise, but
    // clamped away from DC where the blocker's own 35 Hz corner shapes it
    // (`audio.rs`, and the Prompt P finding that raising α hurt the coarse read).
    let lo_start = (f_lo + l).max(20.0);
    if f_n - l > lo_start {
        bins += spec.collect_band(lo_start, f_n - l, scratch);
    }
    if f_hi - l > f_n + l {
        bins += spec.collect_band(f_n + l, f_hi - l, scratch);
    }
    if bins < MIN_GAP_BINS {
        return None;
    }
    scratch.sort_by(f32::total_cmp);
    Some((s, scratch[scratch.len() / 2]))
}

/// [`partial_probe_at`] at the standing [`NOISE_WIN`] probe window.
fn partial_probe(
    spec: &ProbeSpectrum,
    f_n: f32,
    f_lo: f32,
    f_hi: f32,
    scratch: &mut Vec<f32>,
) -> Option<(f32, f32)> {
    partial_probe_at(spec, NOISE_WIN, f_n, f_lo, f_hi, scratch)
}

/// The two neighbouring partials that bound partial `n`'s noise gaps. Below f₁
/// there is no partial, so the lower bound is reflected from the f₁–f₂ spacing.
fn neighbour_bounds(cap: &Capture, f_meas: &[f32], i: usize, n: usize) -> (f32, f32) {
    let f_hi = stiff_partial(cap.f0, cap.b, n + 1);
    let f_lo = if n >= 2 {
        stiff_partial(cap.f0, cap.b, n - 1)
    } else {
        f_meas[i] - (stiff_partial(cap.f0, cap.b, 2) - stiff_partial(cap.f0, cap.b, 1))
    };
    (f_lo, f_hi)
}

/// First sample at which the record reaches [`ONSET_REL_DB`] of its loudest
/// 1024-sample RMS.
fn onset_sample(signal: &[f32]) -> usize {
    let mut rms = Vec::new();
    let mut c = 0usize;
    while c + HOP_SIZE <= signal.len() {
        let e: f32 = signal[c..c + HOP_SIZE].iter().map(|x| x * x).sum();
        rms.push((e / HOP_SIZE as f32).sqrt());
        c += HOP_SIZE;
    }
    let peak = rms.iter().copied().fold(0.0f32, f32::max);
    if peak <= 0.0 {
        return 0;
    }
    let bar = peak * 10f32.powf(ONSET_REL_DB / 20.0);
    rms.iter().position(|&r| r >= bar).unwrap_or(0) * HOP_SIZE
}

/// σ sweep grid — eight points per decade from 1e−5 to 1e−1, spanning the
/// values the sets actually recorded (2.8e−3 … 7.8e−3) by two decades either
/// way. ADR 0015 §2 sweeps σ rather than choosing one: it is a user-movable
/// slider (`Message::SilenceThresholdChanged`), so no single value is the
/// honest one, and `07` §7 requires the family rather than a crossing.
fn sigma_grid() -> Vec<f32> {
    (0..=32)
        .map(|i| 10f32.powf(-5.0 + i as f32 / 8.0))
        .collect()
}

/// Time-since-onset buckets, in seconds. Coarse early where the decay is fast,
/// wide late where only the long records reach.
const T_EDGES: [f32; 8] = [0.05, 0.1, 0.25, 0.5, 1.0, 1.5, 2.5, f32::INFINITY];
const T_LABELS: [&str; 8] = [
    "0–.05",
    ".05–.1",
    ".1–.25",
    ".25–.5",
    ".5–1",
    "1–1.5",
    "1.5–2.5",
    "2.5+",
];

fn t_bucket(t: f32) -> usize {
    T_EDGES
        .iter()
        .position(|&e| t < e)
        .unwrap_or(T_EDGES.len() - 1)
}

const REGISTERS: [&str; 4] = ["bass", "tenor", "treble", "high 76–87"];

fn register_index(key: u8) -> usize {
    REGISTERS
        .iter()
        .position(|&r| r == register(key))
        .unwrap_or(0)
}

/// Counts for one (gate, register, σ) cell. The three populations are disjoint
/// and each answers a different question, so they are never pooled: `h0` is the
/// only one a false-alarm rate may be quoted against.
#[derive(Default, Clone, Copy)]
struct Tally {
    /// Dead **and** absent at the probe window — H₀ proper.
    h0: u64,
    h0_pass: u64,
    /// Dead at the gate's window but present at the probe's: admitting one is
    /// not a false alarm, and not a detection either.
    blind: u64,
    blind_pass: u64,
    live: u64,
    live_miss: u64,
    ambig: u64,
}

impl Tally {
    fn add(&mut self, class: u8, present: bool, pass: bool) {
        match class {
            0 if present => {
                self.blind += 1;
                self.blind_pass += u64::from(pass);
            }
            0 => {
                self.h0 += 1;
                self.h0_pass += u64::from(pass);
            }
            1 => self.ambig += 1,
            _ => {
                self.live += 1;
                self.live_miss += u64::from(!pass);
            }
        }
    }
}

/// Everything one population accumulates.
struct GateStudy {
    label: String,
    sigma: Vec<f32>,
    /// `[gate][register][σ]` — the pre-registered rates.
    tally: Vec<Vec<Vec<Tally>>>,
    /// `[gate][register][t_bucket]` — the shipped threshold ÷ the correctly
    /// specified one at the same P_fa, `σ_rec·K(N) / (3.157·N_local)`. **1.0 is
    /// correct**; > 1 over-rejects, < 1 over-admits. Verified against AWGN of
    /// known σ, where it reads 0.97–0.99.
    ratio: Vec<Vec<Vec<Vec<f32>>>>,
    /// `[gate][register][t_bucket]` — SNR_gate over the same rows, so a
    /// stranded threshold can be told from a partial that simply died.
    snr: Vec<Vec<Vec<Vec<f32>>>>,
    /// The shipped gate's verdict on each of those rows. Paired with `snr`, it
    /// lets the **live cut be swept after the fact** — necessary because the
    /// pre-registered cut of 3 turns out to sit at a correctly-specified gate's
    /// own threshold (3.157× the noise median), so it cannot separate a
    /// stranded partial from one at the detection limit.
    snr_pass: Vec<Vec<Vec<Vec<bool>>>>,
    /// `[register][t_bucket]` — the σ that would put the threshold exactly at
    /// the noise present, so the registers can be compared without the
    /// calibration each session happened to hold.
    sigma_local: Vec<Vec<Vec<f32>>>,
    /// `[register][t_bucket]` — σ_local read at the **gate's own** window
    /// instead of the 8192 probe's. Only defined where the partial spacing
    /// clears 2.5 main-lobe half-widths at that window, which excludes the bass
    /// entirely; where it is defined it resolves the attack the 186 ms probe
    /// window cannot see, and cross-checks the √(N_T/N) scaling.
    sigma_local_win: Vec<Vec<Vec<f32>>>,
    /// **Candidate C** (ADR 0015 §12): the per-bin startup floor, built once per
    /// set from the odd-indexed silence-screened pre-roll hops of every capture.
    /// Empty when the set kept no usable pre-roll.
    floor_bins: Vec<f32>,
    /// `[register][t_bucket]` — `T_C ÷ T_ideal`, which reduces to
    /// `floor(f_n) / N_local`: how well a floor measured before the strike
    /// predicts the noise actually beside the partial during the note.
    ratio_c: Vec<Vec<Vec<f32>>>,
    /// Held-out silence (even-indexed pre-roll hops) under C — the guard that
    /// stops a floor built from silence being scored on the silence it saw.
    c_holdout_rows: u64,
    c_holdout_pass: u64,
    /// `[register]` — σ_local measured by the *same* probe on pre-onset silence.
    /// The control that decides whether the in-note figure is the room's own
    /// floor or an artifact of the probe: if the note's tail agrees with this,
    /// the noise has already reached the room and cannot fall further.
    sigma_local_silence: Vec<Vec<f32>>,
    /// `[gate][register][t_bucket]` at the recorded σ — Task 1 item 2 asks for
    /// the rates against time-since-onset, not just the threshold ratio.
    shipped_t: Vec<Vec<Vec<Tally>>>,
    /// `[gate][register]` at each capture's own recorded σ — the gate as it
    /// actually shipped on that instrument. No single point on the sweep grid
    /// represents it, because the calibration differs from session to session.
    shipped: Vec<Vec<Tally>>,
    captures: u64,
    /// The σ each capture recorded. It is a slider, not a constant, so a set
    /// has a distribution rather than a value — and piano #2's two sessions
    /// differ by more than a decade on the same instrument.
    sigmas: Vec<f32>,
    rows: u64,
    unmeasurable: u64,
    /// Pre-onset room noise — the honest H₀, and the true-silence rejection the
    /// entry credits these gates with.
    silence_rows: Vec<u64>,
    silence_pass: Vec<Vec<u64>>,
    /// Pre-roll hops seen, before the Gatekeeper silence screen — the
    /// denominator that says how much of the pre-roll was actually quiet.
    preroll_rows: u64,
    /// Bass keys, partial n ≥ 6, dead: the entry's second credit, "widely-spaced
    /// dead partials during a bass-dominated sustain".
    bass_hi_dead: Vec<u64>,
    bass_hi_dead_pass: Vec<u64>,
    /// Gate #3 fidelity: hops where the **shipped** `Strobe::process` and this
    /// harness's replica of its amplitude test agree / disagree on `gated`, at
    /// the recorded σ with `is_silence = false`. The replica has no standing
    /// unless this is ~100 % — the check ADR 0011 ran as `--verify-shipped`.
    strobe_agree: u64,
    strobe_disagree: u64,
    /// Tracker rows the non-physical-`f_inst` guard rejected while the
    /// amplitude test would have passed them — the part of `#2` that is not σ.
    finst_only: u64,
}

impl GateStudy {
    fn new(label: &str) -> Self {
        let sigma = sigma_grid();
        let ns = sigma.len();
        Self {
            label: label.to_string(),
            tally: vec![vec![vec![Tally::default(); ns]; REGISTERS.len()]; GATES.len()],
            ratio: vec![vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()]; GATES.len()],
            snr: vec![vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()]; GATES.len()],
            snr_pass: vec![vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()]; GATES.len()],
            sigma_local: vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()],
            sigma_local_win: vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()],
            sigma_local_silence: vec![Vec::new(); REGISTERS.len()],
            floor_bins: Vec::new(),
            ratio_c: vec![vec![Vec::new(); T_EDGES.len()]; REGISTERS.len()],
            c_holdout_rows: 0,
            c_holdout_pass: 0,
            shipped: vec![vec![Tally::default(); REGISTERS.len()]; GATES.len()],
            shipped_t: vec![
                vec![vec![Tally::default(); T_EDGES.len()]; REGISTERS.len()];
                GATES.len()
            ],
            sigma,
            captures: 0,
            sigmas: Vec::new(),
            rows: 0,
            unmeasurable: 0,
            silence_rows: vec![0; REGISTERS.len()],
            silence_pass: vec![vec![0; REGISTERS.len()]; GATES.len()],
            preroll_rows: 0,
            bass_hi_dead: vec![0; GATES.len()],
            bass_hi_dead_pass: vec![0; GATES.len()],
            finst_only: 0,
            strobe_agree: 0,
            strobe_disagree: 0,
        }
    }
}

/// The σ the app actually held when this capture was taken.
///
/// `metadata.noise_floor` is session *config*, not a MAT output, so reading it
/// from `analysis.json` does not touch what `06` requires `regenerate_partials`
/// for — the stale fields there are `measured_f0`, `calculated_b` and the
/// partial list, none of which this uses.
fn recorded_sigma(root: &Path, dir: &str) -> Option<f32> {
    let text = std::fs::read_to_string(root.join(dir).join("analysis.json")).ok()?;
    let j: serde_json::Value = serde_json::from_str(&text).ok()?;
    j["metadata"]["noise_floor"].as_f64().map(|v| v as f32)
}

/// Pre-onset audio, where the capture kept any: `audio_full_event.raw` leads
/// `audio.raw` by whatever the pipeline held in its pre-roll ring, so the kept
/// segment is the file's **tail**. Verified rather than assumed — a capture
/// whose tail does not reproduce `audio.raw` is skipped, because a misaligned
/// slice would score the note as if it were the room.
fn preroll(root: &Path, dir: &str, kept: &[f32]) -> Option<Vec<f32>> {
    let full = crate::raw::read(&root.join(dir).join("audio_full_event.raw"))?;
    if full.len() <= kept.len() {
        return None;
    }
    let lead = full.len() - kept.len();
    if full[lead..] != *kept {
        return None;
    }
    Some(full[..lead].to_vec())
}

/// **Candidate C's floor.** Per-bin median of the probe spectrum over the
/// odd-indexed silence-screened pre-roll hops of every capture in the set —
/// the offline stand-in for the calibration recording the app already takes and
/// currently reduces to a single scalar (ADR 0015 §12).
///
/// Parity split rather than a random one so the build and holdout halves are
/// interleaved in time and neither gets a quieter stretch of the session.
fn build_startup_floor(
    caps: &[Capture],
    root: &Path,
    hann_probe: &[f32],
    planner: &mut RealFftPlanner<f32>,
    quantile: f32,
) -> (Vec<f32>, usize) {
    let mut cols: Vec<Vec<f32>> = Vec::new();
    let mut hops = 0usize;
    let mut fft_scratch = Vec::new();
    for cap in caps {
        let Some(audio) = cap.audio(root) else {
            continue;
        };
        let sigma_rec = recorded_sigma(root, &cap.dir).unwrap_or(2.9e-3);
        let Some(pre) = preroll(root, &cap.dir, &audio) else {
            continue;
        };
        let usable = pre.len().saturating_sub(4 * HOP_SIZE);
        let mut c = 0usize;
        let mut idx = 0usize;
        while c + NOISE_WIN <= usable {
            let end = c + NOISE_WIN;
            c += HOP_SIZE;
            let sl = &pre[end - NOISE_WIN..end];
            let rms = (sl[NOISE_WIN - HOP_SIZE..]
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                / HOP_SIZE as f32)
                .sqrt();
            if rms >= sigma_rec {
                continue;
            }
            idx += 1;
            if idx.is_multiple_of(2) {
                continue; // even hops are the holdout
            }
            let Some(spec) = probe_spectrum(&pre, end, hann_probe, planner, &mut fft_scratch)
            else {
                continue;
            };
            if cols.is_empty() {
                cols = vec![Vec::new(); spec.mag.len()];
            }
            for (col, m) in cols.iter_mut().zip(&spec.mag) {
                col.push(*m);
            }
            hops += 1;
        }
    }
    let floor = cols
        .into_iter()
        .map(|mut v| {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(f32::total_cmp);
            let i = ((v.len() - 1) as f32 * quantile).round() as usize;
            let q = v[i.min(v.len() - 1)];
            // At the median the Rayleigh conversion applies. Above it the
            // quantile IS the threshold: a q-quantile of the silence admits
            // (1 − q) of it by construction, so the budget is set empirically
            // rather than by assuming a tail shape the room does not have.
            if quantile <= 0.5 {
                threshold_for_median(q)
            } else {
                q
            }
        })
        .collect();
    (floor, hops)
}

/// One capture's contribution to the study.
fn study_capture(
    st: &mut GateStudy,
    cap: &Capture,
    root: &Path,
    hann_probe: &[f32],
    planner: &mut RealFftPlanner<f32>,
    snr_live: f32,
) {
    let Some(audio) = cap.audio(root) else { return };
    if audio.len() < NOISE_WIN + HOP_SIZE || cap.f0 <= 0.0 {
        return;
    }
    let sigma_rec = recorded_sigma(root, &cap.dir).unwrap_or(2.9e-3);
    let reg = register_index(cap.key);
    st.sigmas.push(sigma_rec);
    let onset = onset_sample(&audio);

    // Which partials this capture measured, and the neighbours that bound each
    // one's noise gaps. The target is the measured frequency; the neighbours
    // are the stiff-string predictions from the capture's own (f₀, B), so a
    // gap is defined even where the neighbour itself was never resolved.
    let mut ns: Vec<usize> = Vec::new();
    let mut f_meas: Vec<f32> = Vec::new();
    for n in 1..=MAX_STROBE_REFS {
        if let Some((f, _)) = cap.partials.get(n).copied().flatten()
            && f > 0.0
            && f < SAMPLE_RATE as f32 / 2.0
        {
            ns.push(n);
            f_meas.push(f);
        }
    }
    if ns.is_empty() {
        return;
    }

    // The register rule both gates share, keyed on the first reference exactly
    // as `engine.rs` and `strobe.rs` key it.
    let win = register_window(f_meas[0]);
    let hann_gate = hann_vec(win);
    let k_gate = neyman_pearson_k(win);
    let k_peaks = neyman_pearson_k(NOISE_WIN);
    let lobe_probe = lobe_hw_hz(NOISE_WIN);
    // Incoherent noise falls as 1/√N, coherent signal does not; this is the
    // processing gain that separates the probe window from the gate's.
    let noise_scale = (NOISE_WIN as f32 / win as f32).sqrt();

    // ── the tracker's adaptive state, one per partial ──
    let mut target = f_meas.clone();
    let mut prev_phase = vec![0.0f32; ns.len()];
    let mut warm = false;
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;

    // The shipped bank, driven over the same audio on the same hop grid.
    let mut strobe = Strobe::new(SAMPLE_RATE);
    let mut strobe_refs = [0.0f32; MAX_STROBE_REFS];
    strobe_refs[..ns.len()].copy_from_slice(&f_meas);
    strobe.retarget(StrobeRefUpdate {
        count: ns.len(),
        refs: strobe_refs,
        coarse_index: 0,
        spacing_hz: f_meas[0],
    });
    let mut frame_buf = tuner_core::pipeline::ProcessingFrame::new();
    let mut strobe_warm = false;

    let mut scratch = Vec::new();
    let mut fft_scratch = Vec::new();
    // The same-window probe needs a gap of its own: at 1024 the Hann lobe is
    // ±86 Hz, so only keys whose partials are further apart than 2.5 lobe
    // half-widths admit one. That excludes the bass by construction, which is
    // the point — there is no off-partial frequency to probe there at all.
    let hann_win_pad = hann_vec(win);
    let win_probe_ok = f_meas[0] > 2.5 * lobe_hw_hz(win);

    let mut cursor = 0usize;
    while cursor + win <= audio.len() {
        let end = cursor + win;
        cursor += HOP_SIZE;
        let slice = &audio[end - win..end];

        // Goertzel evaluations first: the tracker adapts every hop whether or
        // not the probe spectrum is usable, exactly as the engine does.
        let mut amp_track = vec![0.0f32; ns.len()];
        let mut amp_strobe = vec![0.0f32; ns.len()];
        let mut finst_ok = vec![true; ns.len()];
        for i in 0..ns.len() {
            let (a_s, _) = goertzel_windowed(slice, SAMPLE_RATE, f_meas[i], &hann_gate);
            amp_strobe[i] = a_s;
            let (a_t, phase) = goertzel_windowed(slice, SAMPLE_RATE, target[i], &hann_gate);
            amp_track[i] = a_t;
            if !warm {
                prev_phase[i] = phase;
                continue;
            }
            let expected = TAU * target[i] * t_hop;
            let delta = (phase - prev_phase[i] - expected + std::f32::consts::PI).rem_euclid(TAU)
                - std::f32::consts::PI;
            prev_phase[i] = phase;
            let f_inst = target[i] + delta / (TAU * t_hop);
            finst_ok[i] = f_inst.is_finite() && f_inst > 0.0;
            // The engine re-centres only on a hop the shipped gate admits, so
            // the adaptation is driven at the recorded σ — the trajectory the
            // instrument actually took.
            if a_t >= sigma_rec * k_gate && finst_ok[i] {
                target[i] = 0.95 * target[i] + 0.05 * f_inst;
            }
        }
        if !warm {
            warm = true;
            continue;
        }

        // Shipped-bank verdicts, once a full buffer exists. Its first call is
        // its own warm-up, so the comparison starts one hop later.
        if end >= BASS_WINDOW_SIZE {
            frame_buf.audio_buffer[..BASS_WINDOW_SIZE]
                .copy_from_slice(&audio[end - BASS_WINDOW_SIZE..end]);
            let fr = strobe.process(&frame_buf, sigma_rec, false);
            if strobe_warm {
                for (i, &a) in amp_strobe.iter().enumerate() {
                    let replica_gated = a < sigma_rec * k_gate;
                    if fr.gated[i] == replica_gated {
                        st.strobe_agree += 1;
                    } else {
                        st.strobe_disagree += 1;
                    }
                }
            }
            strobe_warm = true;
        }

        let t_since = (end as f32 - onset as f32) / SAMPLE_RATE as f32;
        if t_since < 0.0 {
            continue;
        }
        let tb = t_bucket(t_since);

        // The gate's own window resolves the attack; the 8192 probe cannot see
        // anything before 186 ms, because that is how long its window is.
        if win_probe_ok
            && let Some(spec_w) =
                probe_spectrum_at(&audio, end, win, &hann_win_pad, planner, &mut fft_scratch)
        {
            {
                for (i, &n) in ns.iter().enumerate() {
                    let (f_lo, f_hi) = neighbour_bounds(cap, &f_meas, i, n);
                    if let Some((_, nl)) =
                        partial_probe_at(&spec_w, win, f_meas[i], f_lo, f_hi, &mut scratch)
                        && nl > 0.0
                    {
                        // Expressed at the probe window's scale so it is
                        // directly comparable with `sigma_local`. Both carry
                        // the same median → P_fa conversion; omitting it here
                        // read this column 3.157× low.
                        st.sigma_local_win[reg][tb]
                            .push(threshold_for_median(nl) / (k_peaks * noise_scale));
                    }
                }
            }
        }

        let Some(spec) = probe_spectrum(&audio, end, hann_probe, planner, &mut fft_scratch) else {
            continue;
        };

        for (i, &n) in ns.iter().enumerate() {
            let f_n = f_meas[i];
            let (f_lo, f_hi) = neighbour_bounds(cap, &f_meas, i, n);
            let Some((s, n_local)) = partial_probe(&spec, f_n, f_lo, f_hi, &mut scratch) else {
                st.unmeasurable += 1;
                continue;
            };
            if n_local <= 0.0 {
                st.unmeasurable += 1;
                continue;
            }
            st.rows += 1;
            st.sigma_local[reg][tb].push(threshold_for_median(n_local) / k_peaks);
            // Candidate C: T_C ÷ T_ideal = floor(f_n) ÷ N_local, the 3.157
            // cancelling between the two.
            if !st.floor_bins.is_empty() {
                let b = (f_n / spec.hz_per_bin).round() as usize;
                if let Some(&fl) = st.floor_bins.get(b)
                    && fl > 0.0
                {
                    st.ratio_c[reg][tb].push(fl / threshold_for_median(n_local));
                }
            }

            let snr_true = s / n_local;
            let present = snr_true >= SNR_DEAD;
            let n_at_gate = n_local * noise_scale;
            let amp_peaks = spec.peak_coarse(f_n - lobe_probe, f_n + lobe_probe);

            if warm && !finst_ok[i] && amp_track[i] >= sigma_rec * k_gate {
                st.finst_only += 1;
            }

            for (gi, gate) in GATES.iter().enumerate() {
                // Gate #1 runs at the probe's own window, so its SNR needs no
                // conversion and it can have no gate-blind rows by construction.
                let (amp, k, snr_g) = match gate {
                    GateSite::Peaks => (amp_peaks, k_peaks, snr_true),
                    GateSite::Tracker => (amp_track[i], k_gate, s / n_at_gate),
                    GateSite::Strobe => (amp_strobe[i], k_gate, s / n_at_gate),
                };
                let class = if snr_g < SNR_DEAD {
                    0
                } else if snr_g < snr_live {
                    1
                } else {
                    2
                };
                let noise_here = if matches!(gate, GateSite::Peaks) {
                    n_local
                } else {
                    n_at_gate
                };
                st.ratio[gi][reg][tb].push(sigma_rec * k / threshold_for_median(noise_here));
                st.snr[gi][reg][tb].push(snr_g);
                st.snr_pass[gi][reg][tb].push(amp >= sigma_rec * k);

                st.shipped[gi][reg].add(class, present, amp >= sigma_rec * k);
                st.shipped_t[gi][reg][tb].add(class, present, amp >= sigma_rec * k);
                for (si, &sg) in st.sigma.iter().enumerate() {
                    st.tally[gi][reg][si].add(class, present, amp >= sg * k);
                }
                if reg == 0 && n >= 6 && class == 0 {
                    st.bass_hi_dead[gi] += 1;
                    st.bass_hi_dead_pass[gi] += u64::from(amp >= sigma_rec * k);
                }
            }
        }
    }

    // ── what the gates buy: admissions on true pre-onset room noise ──
    // The tracker has not re-centred yet at this point, so its target is still
    // the fixed reference and gates #2 and #3 coincide here by construction.
    if let Some(pre) = preroll(root, &cap.dir, &audio) {
        // The capture arms on the strike, so the pre-roll's *last* samples
        // already contain the attack. Stay four hops clear of its end, or the
        // "silence" being scored is the note.
        let usable = pre.len().saturating_sub(4 * HOP_SIZE);
        let mut c = 0usize;
        let mut sil_idx = 0usize;
        while c + NOISE_WIN <= usable {
            let end = c + NOISE_WIN;
            c += HOP_SIZE;
            let sl = &pre[end - NOISE_WIN..end];
            // The gates' true-silence credit only applies where the Gatekeeper
            // says silence — engine-side it resets first, strobe-side
            // `is_silence` gates every reference. The pre-roll ring also holds
            // whatever was played before, so without this screen a set captured
            // key-by-key scores the *previous* note's decay as room noise.
            let rms = (sl[NOISE_WIN - HOP_SIZE..]
                .iter()
                .map(|x| x * x)
                .sum::<f32>()
                / HOP_SIZE as f32)
                .sqrt();
            st.preroll_rows += 1;
            if rms >= sigma_rec {
                continue;
            }
            sil_idx += 1;
            let Some(spec) = probe_spectrum(&pre, end, hann_probe, planner, &mut fft_scratch)
            else {
                continue;
            };
            for (i, &f) in f_meas.iter().enumerate() {
                st.silence_rows[reg] += 1;
                if !st.floor_bins.is_empty() && sil_idx.is_multiple_of(2) {
                    let b = (f / spec.hz_per_bin).round() as usize;
                    if let Some(&fl) = st.floor_bins.get(b)
                        && fl > 0.0
                    {
                        st.c_holdout_rows += 1;
                        let a_p = spec.peak_coarse(f - lobe_probe, f + lobe_probe);
                        st.c_holdout_pass += u64::from(a_p >= fl);
                    }
                }
                let (f_lo, f_hi) = neighbour_bounds(cap, &f_meas, i, ns[i]);
                if let Some((_, nl)) = partial_probe(&spec, f, f_lo, f_hi, &mut scratch)
                    && nl > 0.0
                {
                    st.sigma_local_silence[reg].push(threshold_for_median(nl) / k_peaks);
                }
                let (a_s, _) = goertzel_windowed(sl, SAMPLE_RATE, f, &hann_gate);
                let a_p = spec.peak_coarse(f - lobe_probe, f + lobe_probe);
                st.silence_pass[0][reg] += u64::from(a_p >= sigma_rec * k_peaks);
                st.silence_pass[1][reg] += u64::from(a_s >= sigma_rec * k_gate);
                st.silence_pass[2][reg] += u64::from(a_s >= sigma_rec * k_gate);
            }
        }
    }

    st.captures += 1;
}

/// Median of an already-collected sample, or NaN when empty.
fn med_of(v: &[f32]) -> f32 {
    if v.is_empty() {
        return f32::NAN;
    }
    let mut w = v.to_vec();
    w.sort_by(f32::total_cmp);
    w[w.len() / 2]
}

fn pct(num: u64, den: u64) -> f32 {
    if den == 0 {
        f32::NAN
    } else {
        100.0 * num as f32 / den as f32
    }
}

fn pool(t: &[Tally]) -> Tally {
    let mut o = Tally::default();
    for x in t {
        o.h0 += x.h0;
        o.h0_pass += x.h0_pass;
        o.blind += x.blind;
        o.blind_pass += x.blind_pass;
        o.live += x.live;
        o.live_miss += x.live_miss;
        o.ambig += x.ambig;
    }
    o
}

fn report(st: &GateStudy) {
    println!("\n════ {} ════", st.label);
    println!(
        "  {} captures, {} scored (key, hop, partial) rows, {} unmeasurable ({:.1} % — gaps under {} zero-padded bins)",
        st.captures,
        st.rows,
        st.unmeasurable,
        pct(st.unmeasurable, st.rows + st.unmeasurable),
        MIN_GAP_BINS
    );
    let mut sg = st.sigmas.clone();
    sg.sort_by(f32::total_cmp);
    if !sg.is_empty() {
        println!(
            "  recorded σ: median {:.4e}, range {:.4e} … {:.4e}",
            sg[sg.len() / 2],
            sg[0],
            sg[sg.len() - 1]
        );
    }

    println!("\n  ── As shipped (each capture's own recorded σ) ──");
    println!(
        "  {:<11} {:<12} {:>8} {:>9} {:>8} {:>8} {:>8} {:>8}",
        "gate", "register", "H0 rows", "P_fa", "live", "miss%", "blind%", "ambig"
    );
    for (gi, gate) in GATES.iter().enumerate() {
        for (ri, rname) in REGISTERS.iter().enumerate() {
            let t = st.shipped[gi][ri];
            if t.h0 + t.live + t.blind + t.ambig == 0 {
                continue;
            }
            // Gate #1 thresholds the very spectrum the H₀ label is derived from
            // (a lobe max below a gap median), so every H₀ row is a guaranteed
            // rejection and its rate carries no information. Gates #2/#3 read a
            // different window, so theirs is a real dead-partial pass rate —
            // verified at 1.9e-3 against a 1e-3 nominal on AWGN.
            let pfa = if matches!(gate, GateSite::Peaks) {
                f32::NAN
            } else {
                pct(t.h0_pass, t.h0) / 100.0
            };
            println!(
                "  {:<11} {:<12} {:>8} {:>9.2e} {:>8} {:>7.1}% {:>7.1}% {:>8}",
                gate.label(),
                rname,
                t.h0,
                pfa,
                t.live,
                pct(t.live_miss, t.live),
                pct(t.blind_pass, t.blind),
                t.ambig
            );
        }
        let t = pool(&st.shipped[gi]);
        let pfa_all = if matches!(gate, GateSite::Peaks) {
            f32::NAN
        } else {
            pct(t.h0_pass, t.h0) / 100.0
        };
        println!(
            "  {:<11} {:<12} {:>8} {:>9.2e} {:>8} {:>7.1}% {:>7.1}% {:>8}   (nominal P_fa 1.00e-3)",
            "",
            "ALL",
            t.h0,
            pfa_all,
            t.live,
            pct(t.live_miss, t.live),
            pct(t.blind_pass, t.blind),
            t.ambig
        );
    }

    println!("\n  ── σ sweep: realized P_fa and miss rate, pooled over registers ──");
    print!("  {:<9}", "σ");
    for gate in GATES.iter() {
        print!(
            "  {:>10} {:>7}",
            format!("{} Pfa", gate.label().trim()),
            "miss%"
        );
    }
    println!();
    for (si, &sg) in st.sigma.iter().enumerate() {
        print!("  {sg:<9.2e}");
        for gi in 0..GATES.len() {
            let t = pool(&st.tally[gi].iter().map(|r| r[si]).collect::<Vec<_>>());
            print!(
                "  {:>10.2e} {:>6.1}%",
                pct(t.h0_pass, t.h0) / 100.0,
                pct(t.live_miss, t.live)
            );
        }
        println!();
    }

    // `T ÷ N_gate` is the same number at every window length, and that is an
    // identity rather than a coincidence: `K(N) ∝ 1/√N` and incoherent noise
    // also falls as `1/√N`, so the two scalings cancel —
    // `σ·K(N) / (N_local·√(N_T/N)) = σ·K(N_T) / N_local`. The measured columns
    // agreed to the printed precision across all three gates, which is the
    // empirical check. **No choice of window rescues this gate**; only the
    // reference σ can.
    println!("\n  ── Per register at three fixed σ, so the register gradient is separable ──");
    println!(
        "  {:<9} {:<11} {:<12} {:>9} {:>9} {:>9} {:>7}",
        "σ", "gate", "register", "H0 rows", "P_fa", "live", "miss%"
    );
    for &want in &[1.0e-3f32, 1.0e-2, 3.8e-2] {
        let si = st
            .sigma
            .iter()
            .enumerate()
            .min_by(|a, b| {
                (a.1.log10() - want.log10())
                    .abs()
                    .total_cmp(&(b.1.log10() - want.log10()).abs())
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        for (gi, gate) in GATES.iter().enumerate() {
            for (ri, rname) in REGISTERS.iter().enumerate() {
                let t = st.tally[gi][ri][si];
                if t.h0 + t.live == 0 {
                    continue;
                }
                println!(
                    "  {:<9.2e} {:<11} {:<12} {:>9} {:>9.2e} {:>9} {:>6.1}%",
                    st.sigma[si],
                    gate.label(),
                    rname,
                    t.h0,
                    pct(t.h0_pass, t.h0) / 100.0,
                    t.live,
                    pct(t.live_miss, t.live)
                );
            }
        }
    }

    println!("\n  ── Miss rate vs the live cut (EXPLORATORY — pre-registered cut is 3) ──");
    println!(
        "     The cut of 3 sits at a correct gate's OWN threshold (3.157× the noise median), so it\n     \
         admits partials a well-specified detector rejects half the time. Cuts far above it\n     \
         separate a stranded partial from one at the detection limit."
    );
    let cuts = [3.0f32, 6.3, 10.0, 20.0, 50.0];
    print!("  {:<11} {:<12}", "gate", "register");
    for c in cuts.iter() {
        print!("  {:>9}", format!("≥{c:.0}×"));
    }
    println!("     rows ≥50×");
    for (gi, gate) in GATES.iter().enumerate() {
        for (ri, rname) in REGISTERS.iter().enumerate() {
            if st.snr[gi][ri].iter().all(|v| v.is_empty()) {
                continue;
            }
            print!("  {:<11} {:<12}", gate.label(), rname);
            let mut widest = 0u64;
            for &c in cuts.iter() {
                let (mut n, mut missed) = (0u64, 0u64);
                for tb in 0..T_EDGES.len() {
                    for (v, p) in st.snr[gi][ri][tb].iter().zip(&st.snr_pass[gi][ri][tb]) {
                        if *v >= c {
                            n += 1;
                            missed += u64::from(!*p);
                        }
                    }
                }
                widest = n;
                print!("  {:>8.1}%", pct(missed, n));
            }
            println!("     {widest}");
        }
    }

    println!("\n  ── Miss rate vs time since onset, at the shipped σ (Task 1 item 2) ──");
    print!("  {:<11} {:<12}", "gate", "register");
    for l in T_LABELS.iter() {
        print!(" {l:>9}");
    }
    println!();
    for (gi, gate) in GATES.iter().enumerate() {
        for (ri, rname) in REGISTERS.iter().enumerate() {
            if st.shipped_t[gi][ri].iter().all(|t| t.live == 0) {
                continue;
            }
            print!("  {:<11} {:<12}", gate.label(), rname);
            for tb in 0..T_EDGES.len() {
                let t = st.shipped_t[gi][ri][tb];
                if t.live == 0 {
                    print!(" {:>9}", "—");
                } else {
                    print!(" {:>8.1}%", pct(t.live_miss, t.live));
                }
            }
            println!();
        }
    }

    println!("\n  ── T ÷ T_ideal: the shipped threshold against the correctly specified one ──");
    println!(
        "     T_ideal = 3.157·N_local, the P_fa = 1e-3 threshold for the noise measured beside the\n     \
         partial. **1.0 is correct**; > 1 over-rejects, < 1 over-admits. Reads 0.96–0.99 on AWGN of\n     \
         known σ, and σ_local recovers that σ to 4 % — the calibration of these two columns."
    );
    println!(
        "     The 1/√N cancellation that makes T÷T_ideal window-independent holds for INCOHERENT\n     \
         noise. In the bass the gaps sit inside neighbouring skirts, so N_local is coherent leakage\n     \
         and the two probe columns diverge; σ_l@win also carries a measured 1.26× sidelobe inflation\n     \
         in the first bucket. Read σ_l@win for the attack, σ_local for the tail."
    );
    println!(
        "  {:<12} {:<8} {:>9} {:>10} {:>10} {:>10} {:>10} {:>10}",
        "register", "t (s)", "rows", "T÷T_ideal", "σ_local", "σ_l@win", "SNR@probe", "SNR@win"
    );
    for (ri, rname) in REGISTERS.iter().enumerate() {
        for (tb, tlabel) in T_LABELS.iter().enumerate() {
            let n = st.ratio[0][ri][tb].len();
            let nw = st.sigma_local_win[ri][tb].len();
            if n == 0 && nw == 0 {
                continue;
            }
            println!(
                "  {:<12} {:<8} {:>9} {:>10.2} {:>10.2e} {:>10.2e} {:>10.1} {:>10.1}",
                rname,
                tlabel,
                n,
                med_of(&st.ratio[0][ri][tb]),
                med_of(&st.sigma_local[ri][tb]),
                med_of(&st.sigma_local_win[ri][tb]),
                med_of(&st.snr[0][ri][tb]),
                med_of(&st.snr[1][ri][tb])
            );
        }
    }

    if st.sigma_local_silence.iter().any(|v| !v.is_empty()) {
        println!("\n  ── Control: σ_local on pre-onset silence, same probe, same frequencies ──");
        println!(
            "     If the note's tail agrees with this, the noise is already at the room floor."
        );
        for (ri, rname) in REGISTERS.iter().enumerate() {
            if st.sigma_local_silence[ri].is_empty() {
                continue;
            }
            let last = st.sigma_local[ri]
                .iter()
                .rev()
                .find(|v| !v.is_empty())
                .map(|v| med_of(v))
                .unwrap_or(f32::NAN);
            let sil = med_of(&st.sigma_local_silence[ri]);
            println!(
                "  {:<12} silence {:>10.2e} (n={:>6})   note's last bucket {:>10.2e}   ratio {:>6.2}×",
                rname,
                sil,
                st.sigma_local_silence[ri].len(),
                last,
                last / sil
            );
        }
    }

    if !st.ratio_c[0].iter().all(|v| v.is_empty()) || !st.ratio_c[3].iter().all(|v| v.is_empty()) {
        println!("\n  ── Candidate C: the per-bin startup floor (ADR 0015 §12) ──");
        println!(
            "     T_C ÷ T_ideal = floor(f_n) ÷ N_local. **1.0 is correct.** Compared against the\n     \
             shipped gate's own T ÷ T_ideal on the identical rows."
        );
        println!(
            "  {:<12} {:>10} {:>10} {:>12} {:>12} {:>9}",
            "register", "A shipped", "C floor", "|log10| A", "|log10| C", "rows"
        );
        let mut spread: Vec<f32> = Vec::new();
        for (ri, rname) in REGISTERS.iter().enumerate() {
            let (mut a, mut c) = (Vec::new(), Vec::new());
            for tb in 0..T_EDGES.len() {
                a.extend_from_slice(&st.ratio[0][ri][tb]);
                c.extend_from_slice(&st.ratio_c[ri][tb]);
            }
            if c.is_empty() {
                continue;
            }
            let med_c = med_of(&c);
            spread.push(med_c);
            let la: Vec<f32> = a.iter().map(|v| v.log10().abs()).collect();
            let lc: Vec<f32> = c.iter().map(|v| v.log10().abs()).collect();
            println!(
                "  {:<12} {:>10.2} {:>10.2} {:>12.2} {:>12.2} {:>9}",
                rname,
                med_of(&a),
                med_c,
                med_of(&la),
                med_of(&lc),
                c.len()
            );
        }
        if spread.len() > 1 {
            let hi = spread.iter().copied().fold(f32::MIN, f32::max);
            let lo = spread.iter().copied().fold(f32::MAX, f32::min);
            println!(
                "  compass spread of median T_C÷T_ideal: {:.2}× (criterion 1: < 5×) over {} registers",
                hi / lo,
                spread.len()
            );
        } else {
            println!(
                "  compass spread: NOT EVALUABLE — only {} register has pre-roll",
                spread.len()
            );
        }
        println!(
            "  held-out silence under C: {:.3e} admitted ({} of {} even-indexed probes; guard: ≤ 1e-3)",
            if st.c_holdout_rows == 0 {
                f32::NAN
            } else {
                st.c_holdout_pass as f32 / st.c_holdout_rows as f32
            },
            st.c_holdout_pass,
            st.c_holdout_rows
        );
    }

    println!("\n  ── What the gates buy ──");
    let sil_total: u64 = st.silence_rows.iter().sum();
    if sil_total > 0 {
        println!(
            "  pre-onset room noise admitted at the shipped σ; {sil_total} (hop, partial) probes \
             from the\n  pre-roll hops passing the Gatekeeper silence screen, out of {} seen. \
             Nominal P_fa = 1e-3\n  predicts ≈{:.0} admissions: a rate FAR below nominal is the \
             same over-rejection seen from the\n  silent side, not a virtue. The bar a \
             replacement must clear is ≤ nominal, not 0.",
            st.preroll_rows,
            sil_total as f32 * 1e-3
        );
        print!("  {:<11}", "gate");
        for (ri, rname) in REGISTERS.iter().enumerate() {
            print!(" {:>13}", format!("{rname} (n={})", st.silence_rows[ri]));
        }
        println!(" {:>9}", "ALL");
        for (gi, gate) in GATES.iter().enumerate() {
            print!("  {:<11}", gate.label());
            for ri in 0..REGISTERS.len() {
                print!(
                    " {:>12.2}%",
                    pct(st.silence_pass[gi][ri], st.silence_rows[ri])
                );
            }
            println!(
                " {:>8.2}%",
                pct(st.silence_pass[gi].iter().sum::<u64>(), sil_total)
            );
        }
    } else {
        println!("  (no capture in this set kept usable pre-roll)");
    }
    for (gi, gate) in GATES.iter().enumerate() {
        if st.bass_hi_dead[gi] == 0 {
            continue;
        }
        println!(
            "  {}  bass n ≥ 6 dead partials: {:.1} % of {} admitted",
            gate.label(),
            pct(st.bass_hi_dead_pass[gi], st.bass_hi_dead[gi]),
            st.bass_hi_dead[gi]
        );
    }
    println!(
        "  #2 tracker: {} rows where the non-physical-f_inst guard rejected what the amplitude test admitted",
        st.finst_only
    );
    println!(
        "  #3 strobe fidelity: shipped Strobe::process vs replica agree on {} of {} (hop, ref) verdicts ({:.3} % disagree)",
        st.strobe_agree,
        st.strobe_agree + st.strobe_disagree,
        pct(st.strobe_disagree, st.strobe_agree + st.strobe_disagree)
    );
}

/// **Task 1 — the three ambient-σ gates, measured as they ship** (ADR 0015).
fn np_gate_study(sets: &[(String, PathBuf, PathBuf)], snr_live: f32, floor_q: f32) {
    let mut planner = RealFftPlanner::<f32>::new();
    let hann_probe = hann_vec(NOISE_WIN);
    if snr_live != SNR_LIVE {
        println!("EXPLORATORY: live cut overridden to {snr_live} (pre-registered {SNR_LIVE}).\n");
    }

    println!(
        "Ambient-σ gate measurement (ADR 0015, pre-registered).\n\
         Probe window {NOISE_WIN} ×{NOISE_PAD} zero-pad; S = peak in the target's main lobe, \
         N_local = median of the flanking inter-partial gaps, both in 4/N physical units.\n\
         SNR_gate = S / (N_local·√({NOISE_WIN}/N_win)) — dead < {SNR_DEAD}, live ≥ {SNR_LIVE} \
         (ADR 0014 §8a); 'blind' = dead at the gate's window but present at the probe's.\n\
         P_fa is quoted on the H₀ subset only (dead AND absent at the probe window). \
         Gates #2/#3 take no argmax, so no search-loss correction applies to them; #1 scans.\n"
    );

    for (label, regen, root) in sets {
        let (caps, dropped) = load_regen(regen);
        let mut st = GateStudy::new(label);
        let (floor, floor_hops) =
            build_startup_floor(&caps, root, &hann_probe, &mut planner, floor_q);
        st.floor_bins = floor;
        if floor_hops > 0 {
            println!(
                "[{label}] Candidate C floor: {floor_hops} odd-indexed silence hops, quantile {floor_q}"
            );
        }
        for cap in &caps {
            study_capture(&mut st, cap, root, &hann_probe, &mut planner, snr_live);
        }
        println!(
            "\n[{}] {} captures loaded, {dropped} dropped as implausible (±{:.0} ¢ rule)",
            label,
            caps.len(),
            ET_PLAUSIBLE_CENTS
        );
        report(&st);
    }
}

// ── Mode entry point ─────────────────────────────────────────────────────────

use anyhow::Result;

/// `--set <label> <regen.json> <dump root>`, repeatable: several populations in
/// one run so their columns are directly comparable.
pub(super) fn run(sets: &[String], live: Option<f32>, floor_q: f32) -> Result<()> {
    let sets: Vec<(String, PathBuf, PathBuf)> = sets
        .chunks(3)
        .map(|c| (c[0].clone(), PathBuf::from(&c[1]), PathBuf::from(&c[2])))
        .collect();
    np_gate_study(&sets, live.unwrap_or(SNR_LIVE), floor_q);
    Ok(())
}
