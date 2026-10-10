//! One arm over one or more dump sets.
//!
//! An arm is a candidate noise estimate or detection rule for one of the four
//! consumers: the picker's per-bin threshold over the 8192-point frame (G1), the
//! tracker's and the strobe's gate on a Goertzel amplitude at a known frequency
//! (G2, G3), and the background level MAT reads its partials against on the 2¹⁶
//! window (BG). The driver runs every hop of every capture through the arm as
//! the pipeline would — the same window, the same hop grid, the Gatekeeper's own
//! verdict on each hop — and writes one row per (capture, hop, partial) with
//! `gates ambient`'s columns: the register, the time bucket, the estimate, the
//! truth where it is known, the threshold, the decision, the two SNRs and the
//! estimation error. A synthetic set (`gates truth`) carries the exact noise
//! beside each capture, so the truth is per bin; a real set's truth is the
//! record's oracle, the median of the inter-partial gaps (report 0015 §1).
//!
//! Hold-outs are by time blocks: the first half of a capture's screened silent
//! hops may teach an arm, the second half scores it, never a parity split.

pub(crate) mod apes;
pub(crate) mod excision;
pub(crate) mod floor;
pub(crate) mod ftest;
pub(crate) mod local;
pub(crate) mod null;
pub(crate) mod shape;
pub(crate) mod shipped;
pub(crate) mod signal;

use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use super::ambient::{self, REGISTERS, T_EDGES, T_LABELS};
use super::stats;
use super::truth::{self as synth, Synthetic};
use crate::gatekeeper::replay::{self as gate_replay, Overrides, Thresholds};
use crate::raw;
use crate::regen::{Capture, load as load_regen};
use crate::truth::{SAMPLE_RATE, TAU, hann_vec, register_window};
use tuner_core::algorithms::spectral::{self, goertzel_windowed, neyman_pearson_k};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::gatekeeper::SignalState;
use tuner_core::pipeline::CAPTURE_ANALYSIS_SAMPLES;
use tuner_core::strobe::MAX_STROBE_REFS;

/// The four consumers an arm can serve.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) enum Consumer {
    /// Discovery's peak picker: a per-bin threshold over the 8192-point frame.
    G1,
    /// The tracker's gate on a Goertzel amplitude at its adaptive target.
    G2,
    /// The strobe's gate on a Goertzel amplitude at a fixed reference.
    G3,
    /// The background MAT reads partials against, on the 2¹⁶ window.
    Bg,
}

impl Consumer {
    fn label(self) -> &'static str {
        match self {
            Consumer::G1 => "G1",
            Consumer::G2 => "G2",
            Consumer::G3 => "G3",
            Consumer::Bg => "BG",
        }
    }
}

/// Whether an arm answered for this hop. An arm declines where its premise does
/// not hold (no reference yet, a register it does not serve); declined rows are
/// counted and excluded from every rate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Availability {
    Available,
    Unavailable,
}

/// Which time block a screened silent hop belongs to. A capture with sixteen or
/// more silent windows is split within itself, the earlier half building and
/// the later half scoring, with [`BLOCK_GAP_HOPS`] between them so no scored
/// window shares samples with a learned one; a shorter pre-roll makes whole
/// captures the blocks, alternating in the order they were recorded.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Block {
    /// Not silence: a hop of the note.
    Note,
    /// An arm may learn from this hop.
    Build,
    /// Scored, never learned from.
    Holdout,
}

/// Hops left out between a capture's build and hold-out blocks: one 8192-sample
/// window, so the two share no samples.
const BLOCK_GAP_HOPS: usize = BASS_WINDOW_SIZE / HOP_SIZE;

/// Hops between a pre-roll window's end and the onset: the strike's first hop
/// may lead the −20 dB onset by the gate's own latency.
const ONSET_GUARD_HOPS: usize = 4;

/// How the kept record is located inside the full event.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Preroll {
    /// The first exact match of the record's opening samples: nearly every
    /// capture has a pre-roll.
    Aligned,
    /// The record is the full event's suffix: only captures whose note decayed
    /// before the record filled, report 0015's population.
    Strict,
}

/// Fewest silent windows for a within-capture split.
const MIN_SPLIT_WINDOWS: usize = 16;

/// Hann main-lobe half-width in bins (null to null is four bins).
pub(crate) const LOBE_HALF_BINS: usize = 2;

/// What an arm answers for one gate probe.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct GateVerdict {
    /// The threshold the amplitude is tested against, in the probe's units.
    pub(crate) threshold: Option<f32>,
    /// The arm's estimate of the noise's median magnitude at the target, in
    /// the probe's units; `None` for a signal-side rule.
    pub(crate) estimate: Option<f32>,
    /// The arm's own 5–95 % interval on that estimate, where it states one.
    pub(crate) interval: Option<(f32, f32)>,
    /// The decision; `None` where the arm declines this probe.
    pub(crate) admit: Option<bool>,
}

/// One Goertzel probe at a known frequency.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GateProbe {
    /// Index of the partial in the capture's series (0-based).
    pub(crate) index: usize,
    /// The frequency evaluated, Hz: the fixed reference (G3) or the tracker's
    /// adaptive target (G2).
    pub(crate) target_hz: f32,
    /// Goertzel window, 1024 or 4096 by the register rule.
    pub(crate) window: usize,
    /// Goertzel amplitude in the `4/n` units.
    pub(crate) amplitude: f32,
    /// The phase vocoder's instantaneous frequency, once a previous hop exists.
    pub(crate) f_inst: Option<f32>,
    /// `true` for G3's fixed reference, `false` for G2's adaptive target.
    pub(crate) fixed: bool,
    /// The exact noise level at this probe in its `4/n` units, where the set is
    /// synthetic and the level is read per hop at the probe's window; `None`
    /// elsewhere (the oracle arm then takes the frame's level or N_local).
    pub(crate) truth_level: Option<f32>,
}

/// What one set offers every arm before its first hop.
pub(crate) struct SetCtx<'a> {
    pub(crate) label: &'a str,
    /// The false-alarm probability every arm is set to: the operating point.
    pub(crate) p_fa: f32,
    /// σ an arm that takes the shipped scalar should use, where the run fixes one.
    pub(crate) sigma_override: Option<f32>,
    /// ν/2, the equivalent independent periodograms per 8192-point frame
    /// averaged at the hop, as Stage B measured it; `None` takes the analytic
    /// Hann value.
    pub(crate) nu_half_per_frame: Option<f32>,
}

/// One capture, before its hops.
pub(crate) struct CaptureCtx<'a> {
    pub(crate) dir: &'a str,
    pub(crate) key: u8,
    pub(crate) register: usize,
    /// The capture's partial series: numbers and measured frequencies.
    pub(crate) ns: &'a [usize],
    pub(crate) f_meas: &'a [f32],
    /// The key's partial spacing, ≈ f₀.
    pub(crate) spacing_hz: f32,
    pub(crate) b: f32,
    /// The σ the capture recorded.
    pub(crate) sigma_rec: f32,
}

/// One hop as the pipeline sees it.
pub(crate) struct HopCtx<'a> {
    /// Hop index on the record's grid; pre-roll hops are negative.
    pub(crate) hop: i64,
    /// Seconds since the onset; negative in the pre-roll.
    pub(crate) t_since: f32,
    pub(crate) bucket: usize,
    /// The Gatekeeper's verdict on this hop.
    pub(crate) is_silence: bool,
    pub(crate) block: Block,
    /// The freshest 8192 samples.
    pub(crate) audio: &'a [f32],
    /// Their Hann FFT, 4097 complex bins, unnormalised.
    pub(crate) spectrum: &'a [Complex<f32>],
    /// `|spectrum|` over 4096 bins, the units `extract_peaks` thresholds.
    pub(crate) mag: &'a [f32],
    pub(crate) hz_per_bin: f32,
    /// The σ arms taking the shipped scalar use this hop.
    pub(crate) sigma: f32,
    /// The noise's median magnitude per bin where the truth is exact.
    pub(crate) truth_level: Option<&'a [f32]>,
    /// The oracle per partial in `mag`'s units: 3.157 × this is the ideal
    /// threshold. `None` where a gap is unmeasurable.
    pub(crate) oracle: &'a [Option<f32>],
    pub(crate) capture: &'a CaptureCtx<'a>,
}

/// A candidate noise estimate or detection rule.
pub(crate) trait Arm {
    fn name(&self) -> &str;
    fn consumers(&self) -> &[Consumer];
    /// Once per set, before any capture.
    fn prepare(&mut self, _set: &SetCtx) {}
    /// Once per capture, in chronological order, before its hops.
    fn begin_capture(&mut self, _cap: &CaptureCtx) {}
    /// Every hop, in order, before the consumers are asked: where a running arm
    /// learns.
    fn observe(&mut self, _hop: &HopCtx) {}
    /// Once per set, after the last capture: where an arm prints what only it
    /// can measure about itself.
    fn finish(&mut self, _set: &SetCtx) {}
    /// Whether the arm's G1 threshold encodes a decision rather than a level:
    /// `out[bin] = mag[bin]` where it accepts, `INFINITY` where it does not. No
    /// estimate or error is read from such a threshold.
    fn decision_only(&self) -> bool {
        false
    }
    /// BG: the arm's own multiplier on its floor, where its acceptance rule is a
    /// stated margin rather than the operating point's Rayleigh threshold.
    fn bg_multiplier(&self) -> Option<f32> {
        None
    }
    /// G1: the threshold per bin of `hop.mag`, in its units; `INFINITY` declines
    /// a bin.
    fn g1_threshold(&mut self, _hop: &HopCtx, _out: &mut [f32]) -> Availability {
        Availability::Unavailable
    }
    /// G1's second stage: whether a local maximum that clears the threshold is
    /// kept as a partial.
    fn g1_keep_peak(&self, _hop: &HopCtx, _bin: usize) -> bool {
        true
    }
    /// G2 and G3: the gate on one Goertzel probe.
    fn gate(&mut self, _hop: &HopCtx, _probe: &GateProbe) -> GateVerdict {
        GateVerdict::default()
    }
    /// BG: the noise's median magnitude at each partial on the 2¹⁶ window, in
    /// `spectrum`'s units; `NAN` declines a partial.
    fn bg_floor(
        &mut self,
        _cap: &CaptureCtx,
        _spectrum: &[f32],
        _hz_per_bin: f32,
        _partials: &[f32],
        _out: &mut [f32],
    ) -> Availability {
        Availability::Unavailable
    }
}

/// The arms this run knows, by the name the command line gives.
pub(crate) fn arm_by_name(name: &str) -> Result<Box<dyn Arm>> {
    Ok(match name {
        "A" => Box::new(shipped::Shipped::recorded()),
        "A@1e-3" => Box::new(shipped::Shipped::fixed(1.0e-3)),
        "B" => Box::new(shipped::Oracle::new()),
        "C" => Box::new(shipped::Cfar::new()),
        "N" => Box::new(shipped::NoGate),
        "1" => Box::new(floor::RunningFloor::new()),
        "finst" => Box::new(signal::FrequencyConsistency::new()),
        "ftest" => Box::new(ftest::FTest::new(3)),
        "ftest-nw2" => Box::new(ftest::FTest::new(2)),
        "ftest-nw4" => Box::new(ftest::FTest::new(4)),
        "fcme" => Box::new(excision::Fcme::new()),
        "sb9" => Box::new(local::Neighbours::new()),
        "laurenti" => Box::new(local::Laurenti::new()),
        "bgmed" => Box::new(local::BgMedian::new(local::Margin::Rayleigh)),
        "bgmed-iso6" => Box::new(local::BgMedian::new(local::Margin::Iso6)),
        "bgmed-iso15" => Box::new(local::BgMedian::new(local::Margin::Iso15)),
        "bgmed-opp" => Box::new(local::BgMedian::new(local::Margin::Oppenheimer)),
        "shape" => Box::new(shape::PeakShape::new()),
        "apes" => Box::new(apes::Apes::new()),
        "7" => Box::new(null::EmpiricalNull::new()),
        _ => bail!(
            "unknown arm {name}; known: A, A@1e-3, B, C, N, 1 (the running per-bin floor), finst \
             (frequency consistency), ftest, ftest-nw2, ftest-nw4 (Thomson's harmonic F-test), fcme \
             (consecutive mean excision), sb9 (neighbour bins), laurenti, bgmed, bgmed-iso6, bgmed-iso15, \
             bgmed-opp (the BG median and its margins), shape (the peak classifier), apes, 7 (the empirical \
             null's scalar)"
        ),
    })
}

/// Multiplier taking a median magnitude to the threshold at `p_fa` on a Rayleigh
/// bin: `√(ln(1/p_fa) / ln 2)`, 3.157 at 10⁻³ (report 0015 §4.1).
pub(crate) fn median_to_threshold(p_fa: f32) -> f32 {
    (p_fa.ln() / 0.5f32.ln()).sqrt()
}

/// The Hann window's squared normalised autocorrelation at shifts of one to
/// seven hops: the power correlation of two frames `m` hops apart.
fn hann_hop_correlation() -> &'static [f64; 8] {
    static TABLE: std::sync::OnceLock<[f64; 8]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let n = BASS_WINDOW_SIZE;
        let w = hann_vec(n);
        let w2: f64 = w.iter().map(|&x| x as f64 * x as f64).sum();
        let mut table = [0.0f64; 8];
        for (m, slot) in table.iter_mut().enumerate().skip(1) {
            let shift = m * HOP_SIZE;
            let s: f64 = w[..n - shift]
                .iter()
                .zip(&w[shift..])
                .map(|(&a, &b)| a as f64 * b as f64)
                .sum();
            *slot = (s / w2).powi(2);
        }
        table
    })
}

/// The equivalent independent periodograms in `k` consecutive Hann frames of
/// [`BASS_WINDOW_SIZE`] samples at a hop of [`HOP_SIZE`]: Percival & Walden's
/// EDOF/2 for the average, `k / (1 + 2 Σ_m (1 − m/k) ρ(m)²)` with ρ the window's
/// normalised autocorrelation at `m` hops. 20.8 for 79 frames; the analytic value
/// Stage B's measurement replaces.
pub(crate) fn analytic_nu_half(k: usize) -> f32 {
    if k == 0 {
        return 0.0;
    }
    let rho = hann_hop_correlation();
    let mut sum = 0.0;
    for (m, &r) in rho.iter().enumerate().skip(1).take(k.saturating_sub(1)) {
        sum += (1.0 - m as f64 / k as f64) * r;
    }
    (k as f64 / (1.0 + 2.0 * sum)) as f32
}

/// One scored row: the common columns, kept in memory for the tables and
/// written to the CSV as they are made.
#[derive(Clone, Debug)]
struct Row {
    register: usize,
    bucket: usize,
    consumer: Consumer,
    population: &'static str,
    amplitude: f32,
    estimate: f32,
    threshold: f32,
    admit: Option<bool>,
    truth_hop: f32,
    truth_level: f32,
    snr_gate: f32,
    snr_true: f32,
    err_db: f32,
    covered: Option<bool>,
    /// Whether the exact threshold on the truth level would admit this row's
    /// amplitude: the control's own nominal for a lobe search.
    exact_admit: Option<bool>,
    /// Scan rows: local maxima above the arm's threshold, bins scanned, those in
    /// the rumble band, and local maxima above the exact threshold where the
    /// truth is known.
    scan: Option<(u32, u32, u32, u32)>,
}

const CSV_HEADER: &str = "set,capture,key,register,hop,t_since_s,bucket,consumer,arm,window,n,f_hz,population,amplitude,estimate,threshold,admit,truth_hop,truth_level,snr_gate,snr_true,err_db,covered,scan_count,scan_bins,scan_rumble,scan_exact";

/// The rumble region whose false candidates are reported apart: below the DC
/// blocker's corner and the lowest keys' fundamentals.
const RUMBLE_HZ: f32 = 100.0;

fn fmt_opt_bool(b: Option<bool>) -> &'static str {
    match b {
        Some(true) => "1",
        Some(false) => "0",
        None => "",
    }
}

fn fmt_f(v: f32) -> String {
    if v.is_finite() {
        format!("{v:.6e}")
    } else {
        String::new()
    }
}

struct Study {
    arm: String,
    set: String,
    p_fa: f32,
    rows: Vec<Row>,
    declined: HashMap<Consumer, u64>,
    captures: u64,
    captures_with_preroll: u64,
    silent_hops: (u64, u64),
    synthetic: bool,
    csv: Option<BufWriter<File>>,
}

/// Where a row was taken: the hop, its time, the window and the partial.
#[derive(Clone, Copy)]
struct Loc {
    hop: i64,
    t_since: f32,
    window: usize,
    /// Partial number; 0 for a hop-level row.
    n: usize,
    f_hz: f32,
}

impl Study {
    fn push(&mut self, row: Row, cap: &CaptureCtx, at: Loc) -> Result<()> {
        let Loc {
            hop,
            t_since,
            window,
            n,
            f_hz,
        } = at;
        if let Some(w) = self.csv.as_mut() {
            let (sc, sb, sr, se) = row
                .scan
                .map(|(a, b, c, d)| (a.to_string(), b.to_string(), c.to_string(), d.to_string()))
                .unwrap_or_default();
            writeln!(
                w,
                "{},{},{},{},{},{:.4},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                self.set,
                cap.dir,
                cap.key,
                REGISTERS[row.register],
                hop,
                t_since,
                row.bucket,
                row.consumer.label(),
                self.arm,
                window,
                if n == 0 { String::new() } else { n.to_string() },
                fmt_f(f_hz),
                row.population,
                fmt_f(row.amplitude),
                fmt_f(row.estimate),
                fmt_f(row.threshold),
                fmt_opt_bool(row.admit),
                fmt_f(row.truth_hop),
                fmt_f(row.truth_level),
                fmt_f(row.snr_gate),
                fmt_f(row.snr_true),
                fmt_f(row.err_db),
                fmt_opt_bool(row.covered),
                sc,
                sb,
                sr,
                se
            )?;
        }
        self.rows.push(row);
        Ok(())
    }
}

/// The 8192-point Hann FFT and magnitudes of `samples`.
struct Frame {
    spectrum: Vec<Complex<f32>>,
    mag: Vec<f32>,
    time: Vec<f32>,
    plan: Arc<dyn RealToComplex<f32>>,
    n: usize,
}

impl Frame {
    /// The 8192-point bass frame.
    fn new(planner: &mut RealFftPlanner<f32>) -> Self {
        Self::sized(planner, BASS_WINDOW_SIZE)
    }

    /// A Hann frame of `n` points.
    fn sized(planner: &mut RealFftPlanner<f32>, n: usize) -> Self {
        Self {
            spectrum: vec![Complex::new(0.0, 0.0); n / 2 + 1],
            mag: vec![0.0; n / 2],
            time: vec![0.0; n],
            plan: planner.plan_fft_forward(n),
            n,
        }
    }

    fn compute(&mut self, samples: &[f32]) {
        spectral::fft(
            samples,
            &mut self.time,
            &mut self.spectrum,
            &self.plan,
            self.n,
        );
        spectral::magnitude_spectrum(&self.spectrum, self.n, &mut self.mag);
    }
}

/// Where the kept record begins inside the full event: the strict suffix the
/// app's early-decayed captures satisfy, else the first exact match of the
/// record's opening samples, which a record that outran the diagnostic buffer
/// still has. `None` where the record is not inside the full event at all.
pub(crate) fn preroll_offset(full: &[f32], kept: &[f32]) -> Option<usize> {
    preroll_suffix(full, kept).or_else(|| {
        const PROBE: usize = 2048;
        if kept.len() < PROBE || full.len() < PROBE {
            return None;
        }
        let head = &kept[..PROBE];
        (0..=full.len() - PROBE).find(|&off| full[off..off + PROBE] == *head)
    })
}

/// [`preroll_offset`]'s strict half: the record as the full event's suffix.
pub(crate) fn preroll_suffix(full: &[f32], kept: &[f32]) -> Option<usize> {
    (full.len() >= kept.len() && full[full.len() - kept.len()..] == *kept)
        .then(|| full.len() - kept.len())
}

/// The Gatekeeper's verdict on every hop of `full` at the capture's logged
/// thresholds, with σ overridden where the set fixes one (the external stores
/// log none).
fn gate_verdicts(dir: &Path, full: &[f32], sigma: Option<f32>) -> Vec<gate_replay::Hop> {
    let thresholds = Thresholds::resolve(
        dir,
        &Overrides {
            silence: sigma,
            ..Overrides::default()
        },
    );
    gate_replay::run(full, thresholds.config)
}

/// Runs `arm` over every set and writes the tables.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run(
    arm_name: &str,
    sets: &[String],
    out: Option<&Path>,
    p_fa: f32,
    sigma: Option<f32>,
    nu_half: Option<f32>,
    consumers: Option<&[String]>,
    preroll: Preroll,
    exact_median: bool,
) -> Result<()> {
    let sets: Vec<(String, PathBuf, PathBuf)> = sets
        .chunks(3)
        .map(|c| (c[0].clone(), PathBuf::from(&c[1]), PathBuf::from(&c[2])))
        .collect();
    if sets.is_empty() {
        bail!("at least one --set LABEL REGEN ROOT is needed");
    }
    let wanted: Option<Vec<Consumer>> = consumers.map(|cs| {
        cs.iter()
            .filter_map(|c| match c.to_ascii_uppercase().as_str() {
                "G1" => Some(Consumer::G1),
                "G2" => Some(Consumer::G2),
                "G3" => Some(Consumer::G3),
                "BG" => Some(Consumer::Bg),
                _ => None,
            })
            .collect()
    });
    if let Some(dir) = out {
        std::fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    for (label, regen, root) in &sets {
        let mut arm = arm_by_name(arm_name)?;
        let serve: Vec<Consumer> = arm
            .consumers()
            .iter()
            .copied()
            .filter(|c| wanted.as_ref().is_none_or(|w| w.contains(c)))
            .collect();
        println!(
            "\n════ arm {} on {} ({}) ════",
            arm.name(),
            label,
            serve
                .iter()
                .map(|c| c.label())
                .collect::<Vec<_>>()
                .join(", ")
        );
        let study = run_set(
            arm.as_mut(),
            &serve,
            label,
            regen,
            root,
            out,
            p_fa,
            sigma,
            nu_half,
            preroll,
            exact_median,
        )?;
        report(&study, &serve);
    }
    Ok(())
}

/// Sorted chronologically by the directory's trailing serial, the order the
/// session recorded them, so a running arm learns in the order it would live.
fn chronological(mut caps: Vec<Capture>) -> Vec<Capture> {
    let serial = |d: &str| d.rsplit('_').next().and_then(|s| s.parse::<u64>().ok());
    caps.sort_by_key(|c| (serial(&c.dir), c.dir.clone()));
    caps
}

#[allow(clippy::too_many_arguments)]
fn run_set(
    arm: &mut dyn Arm,
    serve: &[Consumer],
    label: &str,
    regen: &Path,
    root: &Path,
    out: Option<&Path>,
    p_fa: f32,
    sigma_override: Option<f32>,
    nu_half: Option<f32>,
    preroll: Preroll,
    exact_median: bool,
) -> Result<Study> {
    let (caps, dropped) = load_regen(regen);
    let caps = chronological(caps);
    let synthetic = synth::load(regen, root);
    println!(
        "[{label}] {} captures loaded, {dropped} dropped as implausible; {}",
        caps.len(),
        match &synthetic {
            Some(s) => format!(
                "synthetic truth for {} ({})",
                s.len(),
                s.values().next().map(|v| v.kind.as_str()).unwrap_or("")
            ),
            None => "a real set: the oracle is the truth from the tenor up".to_string(),
        }
    );
    let csv = match out {
        Some(dir) => {
            let path = dir.join(format!("{}_{label}.csv", arm.name().replace('@', "_at_")));
            let mut w = BufWriter::new(File::create(&path)?);
            writeln!(w, "{CSV_HEADER}")?;
            Some(w)
        }
        None => None,
    };
    let mut study = Study {
        arm: arm.name().to_string(),
        set: label.to_string(),
        p_fa,
        rows: Vec::new(),
        declined: HashMap::new(),
        captures: 0,
        captures_with_preroll: 0,
        silent_hops: (0, 0),
        synthetic: synthetic.is_some(),
        csv,
    };
    arm.prepare(&SetCtx {
        label,
        p_fa,
        sigma_override,
        nu_half_per_frame: nu_half,
    });

    let mut planner = RealFftPlanner::<f32>::new();
    let mut frame = Frame::new(&mut planner);
    let mut noise_frame = Frame::new(&mut planner);
    let hann_probe = hann_vec(ambient::NOISE_WIN);
    for (index, cap) in caps.iter().enumerate() {
        let truth = synthetic.as_ref().and_then(|s| s.get(&cap.dir));
        run_capture(
            arm,
            serve,
            &mut study,
            cap,
            index,
            root,
            truth,
            sigma_override,
            preroll,
            exact_median,
            &mut planner,
            &mut frame,
            &mut noise_frame,
            &hann_probe,
        )?;
    }
    arm.finish(&SetCtx {
        label,
        p_fa,
        sigma_override,
        nu_half_per_frame: nu_half,
    });
    if let Some(w) = study.csv.as_mut() {
        w.flush()?;
    }
    Ok(study)
}

/// Half-width in bins of the running median that reads a T2 hop's exact level
/// from the known noise realisation: 33 bins, 178 Hz at the 8192 frame.
const TRUTH_HALF_BINS: usize = 16;

/// Per-bin median of `mag` over `±half` bins, bin 0 left out, into `out`.
fn running_median(mag: &[f32], half: usize, out: &mut [f32], scratch: &mut Vec<f32>) {
    out[0] = f32::NAN;
    for (b, o) in out.iter_mut().enumerate().take(mag.len()).skip(1) {
        let lo = b.saturating_sub(half).max(1);
        let hi = (b + half).min(mag.len() - 1);
        scratch.clear();
        scratch.extend_from_slice(&mag[lo..=hi]);
        let mid = scratch.len() / 2;
        let (_, m, _) = scratch.select_nth_unstable_by(mid, f32::total_cmp);
        *o = *m;
    }
}

/// The exact noise of one hop, where the set is synthetic: the realisation's
/// spectrum into `noise_frame` for the full-event window ending at `end_full`,
/// and the T2 level refreshed in `level` where the generator gave no closed
/// form. `false` where the noise does not cover the window.
fn exact_hop(
    truth: Option<&Synthetic>,
    closed_form: Option<f32>,
    end_full: usize,
    level: Option<&mut Vec<f32>>,
    noise_frame: &mut Frame,
    scratch: &mut Vec<f32>,
) -> bool {
    let Some(t) = truth else {
        return false;
    };
    let per_hop = closed_form.is_none();
    if end_full < BASS_WINDOW_SIZE || end_full > t.noise.len() {
        if let (true, Some(l)) = (per_hop, level) {
            l.fill(f32::NAN);
        }
        return false;
    }
    noise_frame.compute(&t.noise[end_full - BASS_WINDOW_SIZE..end_full]);
    if let (true, Some(l)) = (per_hop, level) {
        running_median(&noise_frame.mag, TRUTH_HALF_BINS, l, scratch);
    }
    true
}

/// Strict local maximum, as `extract_peaks` defines one.
fn is_local_max(mag: &[f32], i: usize) -> bool {
    i >= 1 && i + 1 < mag.len() && mag[i] > mag[i - 1] && mag[i] > mag[i + 1]
}

/// The bin of the largest magnitude within one main lobe of `f_hz`.
fn lobe_peak_bin(mag: &[f32], f_hz: f32, hz_per_bin: f32) -> usize {
    let c = (f_hz / hz_per_bin).round() as usize;
    let lo = c.saturating_sub(LOBE_HALF_BINS).max(1);
    let hi = (c + LOBE_HALF_BINS).min(mag.len() - 2);
    (lo..=hi)
        .max_by(|&a, &b| mag[a].total_cmp(&mag[b]))
        .unwrap_or(c.min(mag.len() - 2))
}

/// Mask of bins within one main lobe of any predicted partial of the key: the
/// bins a false candidate may not be counted on (rulebook T5(ii)).
fn partial_mask(spacing_hz: f32, b: f32, hz_per_bin: f32, bins: usize) -> Vec<bool> {
    let mut mask = vec![false; bins];
    let f0 = spacing_hz;
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let mut n = 1usize;
    loop {
        let f_n = ambient::stiff_partial(f0, b, n);
        if f_n >= nyquist || n > 400 {
            break;
        }
        let c = (f_n / hz_per_bin).round() as usize;
        let lo = c.saturating_sub(2 * LOBE_HALF_BINS);
        let hi = (c + 2 * LOBE_HALF_BINS).min(bins - 1);
        mask[lo..=hi].iter_mut().for_each(|m| *m = true);
        n += 1;
    }
    mask
}

fn population_of(snr_gate: f32, snr_true: f32) -> &'static str {
    if snr_gate < ambient::SNR_DEAD {
        if snr_true < ambient::SNR_DEAD {
            "h0"
        } else {
            "blind"
        }
    } else if snr_gate < ambient::SNR_LIVE {
        "ambig"
    } else {
        "live"
    }
}

fn err_db(estimate: f32, truth: f32) -> f32 {
    if estimate.is_finite() && estimate > 0.0 && truth.is_finite() && truth > 0.0 {
        20.0 * (estimate / truth).log10()
    } else {
        f32::NAN
    }
}

fn covered(interval: Option<(f32, f32)>, truth: f32) -> Option<bool> {
    interval
        .filter(|_| truth.is_finite())
        .map(|(lo, hi)| lo <= truth && truth <= hi)
}

/// Factor taking a magnitude level in the raw 8192-point frame to the same
/// noise's level in a Goertzel's `4/n` units at window `n`: the `4/N_T`
/// normalisation, then the `√(N_T / n)` processing gain (report 0015 §1).
pub(crate) fn raw_to_gate(level_raw: f32, window: usize) -> f32 {
    level_raw * 4.0 / BASS_WINDOW_SIZE as f32 * (BASS_WINDOW_SIZE as f32 / window as f32).sqrt()
}

#[allow(clippy::too_many_arguments)]
fn run_capture(
    arm: &mut dyn Arm,
    serve: &[Consumer],
    study: &mut Study,
    cap: &Capture,
    cap_index: usize,
    root: &Path,
    truth: Option<&Synthetic>,
    sigma_override: Option<f32>,
    preroll: Preroll,
    exact_median: bool,
    planner: &mut RealFftPlanner<f32>,
    frame: &mut Frame,
    noise_frame: &mut Frame,
    hann_probe: &[f32],
) -> Result<()> {
    let dir = root.join(&cap.dir);
    let Some(audio) = cap.audio(root) else {
        return Ok(());
    };
    if audio.len() < BASS_WINDOW_SIZE + HOP_SIZE || cap.f0 <= 0.0 {
        return Ok(());
    }
    let sigma_rec = ambient::recorded_sigma(root, &cap.dir).unwrap_or(2.9e-3);
    let sigma = sigma_override.unwrap_or(sigma_rec);
    let register = ambient::register_index(cap.key);

    // The capture's series and the neighbours bounding each partial's gaps.
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
        return Ok(());
    }
    let absent: Vec<bool> = ns
        .iter()
        .map(|n| truth.is_some_and(|t| t.absent.contains(n)))
        .collect();
    let cctx = CaptureCtx {
        dir: &cap.dir,
        key: cap.key,
        register,
        ns: &ns,
        f_meas: &f_meas,
        spacing_hz: cap.f0,
        b: cap.b,
        sigma_rec,
    };
    arm.begin_capture(&cctx);
    study.captures += 1;

    // The full event and the pre-roll inside it.
    let full = raw::full_event(&dir);
    let lead = full.as_deref().and_then(|f| match preroll {
        Preroll::Aligned => preroll_offset(f, &audio),
        Preroll::Strict => preroll_suffix(f, &audio),
    });
    if let (Some(t), Some(l)) = (truth, lead)
        && t.onset_sample != l
    {
        eprintln!(
            "[{}] synthetic onset {} disagrees with the aligned pre-roll {l}; the truth is used at the aligned offset",
            cap.dir, t.onset_sample
        );
    }
    let verdicts: Vec<gate_replay::Hop> = match (&full, lead) {
        (Some(f), Some(_)) => gate_verdicts(&dir, f, sigma_override),
        _ => Vec::new(),
    };
    let verdict_at = |end_full: usize| -> Option<SignalState> {
        if end_full < BASS_WINDOW_SIZE {
            return None;
        }
        let idx = (end_full - BASS_WINDOW_SIZE) / HOP_SIZE;
        verdicts.get(idx).map(|h| h.result.state)
    };

    // Exact truth where the set is synthetic: the noise's level per bin, in
    // closed form where the generator knows it (T1), else per hop the median of
    // the known realisation over ±TRUTH_HALF_BINS bins (T2): the bed's level as
    // it is at that hop, strike and segment changes included, read with a
    // median's scatter over 33 Hann bins (about ±2 dB at 5–95 %). The noise is
    // indexed like the full event.
    let closed_form = if exact_median {
        None
    } else {
        truth.and_then(|t| t.level_bin_8192)
    };
    let mut truth_level: Option<Vec<f32>> =
        truth.map(|_| vec![closed_form.unwrap_or(f32::NAN); BASS_WINDOW_SIZE / 2]);
    let mut median_scratch: Vec<f32> = Vec::new();
    let k_exact = median_to_threshold(study.p_fa);
    let hz_per_bin = SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32;
    let onset = ambient::onset_sample(&audio);
    let win = register_window(f_meas[0]);
    let hann_gate = hann_vec(win);
    let k_gate = neyman_pearson_k(win);
    // T2's exact level at the gate's own window: the median over ±TRUTH_HALF_BINS
    // bins of the noise's `win`-point Hann spectrum this hop, in the Goertzel's
    // 4/n units; the 8192 frame's median averages a burst the 1024-sample probe
    // meets at full strength.
    let mut probe_frame =
        (truth.is_some() && closed_form.is_none()).then(|| Frame::sized(planner, win));
    let mut probe_level = vec![f32::NAN; win / 2];
    let probe_hz = SAMPLE_RATE as f32 / win as f32;
    let noise_scale = (ambient::NOISE_WIN as f32 / win as f32).sqrt();
    let mask = partial_mask(cctx.spacing_hz, cctx.b, hz_per_bin, BASS_WINDOW_SIZE / 2);
    let rumble_bins = (RUMBLE_HZ / hz_per_bin) as usize;
    let mut thresholds = vec![f32::INFINITY; BASS_WINDOW_SIZE / 2];
    let mut probe_scratch = Vec::new();
    let mut fft_scratch = Vec::new();

    // ── BG: one 2¹⁶ window at the record's start ──
    if serve.contains(&Consumer::Bg) && audio.len() >= CAPTURE_ANALYSIS_SAMPLES {
        bg_pass(arm, study, &cctx, &audio, truth, lead, planner)?;
    }

    // ── the pre-roll: screened silence, in two time blocks ──
    // The record begins at the gate's Stable verdict, hops after the strike, so
    // the guard counts from the −20 dB onset where that comes first.
    let mut silent_ends: Vec<usize> = Vec::new();
    if let (Some(f), Some(l)) = (&full, lead) {
        let onset_full = ambient::onset_sample(f);
        let strike = if onset_full > 0 && onset_full < l {
            onset_full
        } else {
            l
        };
        let usable = strike.saturating_sub(ONSET_GUARD_HOPS * HOP_SIZE);
        let mut end = BASS_WINDOW_SIZE;
        while end <= usable {
            if verdict_at(end) == Some(SignalState::Silence) {
                silent_ends.push(end);
            }
            end += HOP_SIZE;
        }
        if !silent_ends.is_empty() {
            study.captures_with_preroll += 1;
        }
        let n_silent = silent_ends.len();
        let within = n_silent >= MIN_SPLIT_WINDOWS;
        let (build_end, holdout_start) = if within {
            (
                n_silent / 2 - BLOCK_GAP_HOPS / 2,
                n_silent / 2 + BLOCK_GAP_HOPS / 2,
            )
        } else if cap_index.is_multiple_of(2) {
            (n_silent, n_silent)
        } else {
            (0, 0)
        };
        for (i, &end) in silent_ends.iter().enumerate() {
            let block = if i < build_end {
                Block::Build
            } else if i >= holdout_start {
                Block::Holdout
            } else {
                continue;
            };
            match block {
                Block::Build => study.silent_hops.0 += 1,
                _ => study.silent_hops.1 += 1,
            }
            let samples = &f[end - BASS_WINDOW_SIZE..end];
            frame.compute(samples);
            let has_noise = exact_hop(
                truth,
                closed_form,
                end,
                truth_level.as_mut(),
                noise_frame,
                &mut median_scratch,
            );
            if let (true, Some(pf), Some(t)) = (has_noise, probe_frame.as_mut(), truth) {
                pf.compute(&t.noise[end - win..end]);
                running_median(
                    &pf.mag,
                    TRUTH_HALF_BINS,
                    &mut probe_level,
                    &mut median_scratch,
                );
            }
            let hop = HopCtx {
                hop: -(((l - end) / HOP_SIZE) as i64) - 1,
                t_since: (end as f32 - l as f32) / SAMPLE_RATE as f32,
                bucket: 0,
                is_silence: true,
                block,
                audio: samples,
                spectrum: &frame.spectrum,
                mag: &frame.mag,
                hz_per_bin,
                sigma,
                truth_level: truth_level.as_deref(),
                oracle: &[],
                capture: &cctx,
            };
            arm.observe(&hop);
            if block != Block::Holdout {
                continue;
            }
            // G1: every local maximum above the threshold is a false candidate.
            if serve.contains(&Consumer::G1) {
                thresholds.fill(f32::INFINITY);
                if arm.g1_threshold(&hop, &mut thresholds) == Availability::Available {
                    let exact = truth_level.as_deref().map(|lvl| {
                        (1..frame.mag.len() - 1)
                            .filter(|&i| {
                                is_local_max(&frame.mag, i) && frame.mag[i] >= k_exact * lvl[i]
                            })
                            .count() as u32
                    });
                    let mut count = 0u32;
                    let mut rumble = 0u32;
                    for (i, &t) in thresholds
                        .iter()
                        .enumerate()
                        .take(frame.mag.len() - 1)
                        .skip(1)
                    {
                        if is_local_max(&frame.mag, i)
                            && frame.mag[i] >= t
                            && arm.g1_keep_peak(&hop, i)
                        {
                            count += 1;
                            if i <= rumble_bins {
                                rumble += 1;
                            }
                        }
                    }
                    let row = Row {
                        register,
                        bucket: 0,
                        consumer: Consumer::G1,
                        population: "scan_silence",
                        amplitude: f32::NAN,
                        estimate: f32::NAN,
                        threshold: f32::NAN,
                        admit: None,
                        truth_hop: f32::NAN,
                        truth_level: f32::NAN,
                        snr_gate: f32::NAN,
                        snr_true: f32::NAN,
                        err_db: f32::NAN,
                        covered: None,
                        exact_admit: None,
                        scan: Some((
                            count,
                            (frame.mag.len() - 2) as u32,
                            rumble,
                            exact.unwrap_or(0),
                        )),
                    };
                    study.push(
                        row,
                        &cctx,
                        Loc {
                            hop: hop.hop,
                            t_since: hop.t_since,
                            window: BASS_WINDOW_SIZE,
                            n: 0,
                            f_hz: f32::NAN,
                        },
                    )?;
                } else {
                    *study.declined.entry(Consumer::G1).or_default() += 1;
                }
            }
            // G2/G3: the probes at the partials' frequencies, on room noise.
            for consumer in [Consumer::G2, Consumer::G3] {
                if !serve.contains(&consumer) {
                    continue;
                }
                for (i, &f) in f_meas.iter().enumerate() {
                    let (amp, _) = goertzel_windowed(hop.audio, SAMPLE_RATE, f, &hann_gate);
                    let exact_gate = probe_frame
                        .is_some()
                        .then(|| {
                            probe_level[((f / probe_hz).round() as usize).min(win / 2 - 1)] * 4.0
                                / win as f32
                        })
                        .filter(|v| v.is_finite());
                    let probe = GateProbe {
                        index: i,
                        target_hz: f,
                        window: win,
                        amplitude: amp,
                        f_inst: None,
                        fixed: consumer == Consumer::G3,
                        truth_level: exact_gate,
                    };
                    let v = arm.gate(&hop, &probe);
                    let Some(admit) = v.admit else {
                        *study.declined.entry(consumer).or_default() += 1;
                        continue;
                    };
                    let (truth_hop, level) = match truth {
                        Some(t) => {
                            let (na, _) = goertzel_windowed(
                                &t.noise[end - BASS_WINDOW_SIZE..end],
                                SAMPLE_RATE,
                                f,
                                &hann_gate,
                            );
                            let level = exact_gate.unwrap_or_else(|| {
                                truth_level
                                    .as_deref()
                                    .map(|lvl| {
                                        raw_to_gate(lvl[(f / hz_per_bin).round() as usize], win)
                                    })
                                    .unwrap_or(f32::NAN)
                            });
                            (na, level)
                        }
                        None => (f32::NAN, f32::NAN),
                    };
                    let row = Row {
                        register,
                        bucket: 0,
                        consumer,
                        population: "silence",
                        amplitude: amp,
                        estimate: v.estimate.unwrap_or(f32::NAN),
                        threshold: v.threshold.unwrap_or(f32::NAN),
                        admit: Some(admit),
                        truth_hop,
                        truth_level: level,
                        snr_gate: f32::NAN,
                        snr_true: f32::NAN,
                        err_db: err_db(v.estimate.unwrap_or(f32::NAN), level),
                        covered: covered(v.interval, level),
                        exact_admit: level.is_finite().then_some(amp >= k_exact * level),
                        scan: None,
                    };
                    study.push(
                        row,
                        &cctx,
                        Loc {
                            hop: hop.hop,
                            t_since: hop.t_since,
                            window: win,
                            n: ns[i],
                            f_hz: f,
                        },
                    )?;
                }
            }
        }
    }

    // ── the note: every hop of the kept record ──
    let mut target = f_meas.clone();
    let mut prev_phase_track = vec![0.0f32; ns.len()];
    let mut prev_phase_fixed = vec![0.0f32; ns.len()];
    let mut warm = false;
    let t_hop = HOP_SIZE as f32 / SAMPLE_RATE as f32;
    let mut oracle: Vec<Option<f32>> = vec![None; ns.len()];
    let mut end = BASS_WINDOW_SIZE;
    let mut hop_index = 0i64;
    while end <= audio.len() {
        let samples = &audio[end - BASS_WINDOW_SIZE..end];
        let t_since = (end as f32 - onset as f32) / SAMPLE_RATE as f32;
        let bucket = ambient::t_bucket(t_since.max(0.0));
        frame.compute(samples);
        let is_silence = match lead {
            Some(l) => verdict_at(l + end) == Some(SignalState::Silence),
            None => false,
        };

        // The oracle per partial, from the ×4 probe of the same window.
        let spec = ambient::probe_spectrum(&audio, end, hann_probe, planner, &mut fft_scratch);
        for (i, &n) in ns.iter().enumerate() {
            oracle[i] = spec.as_ref().and_then(|s| {
                let (f_lo, f_hi) = ambient::neighbour_bounds(cap, &f_meas, i, n);
                ambient::partial_probe(s, f_meas[i], f_lo, f_hi, &mut probe_scratch)
                    .filter(|&(_, nl)| nl > 0.0)
                    .map(|(_, nl)| nl * BASS_WINDOW_SIZE as f32 / 4.0)
            });
        }

        // The exact realisation this hop, where the set is synthetic, and the
        // level it refreshes.
        let noise_mag: Option<&[f32]> = lead
            .is_some_and(|l| {
                exact_hop(
                    truth,
                    closed_form,
                    l + end,
                    truth_level.as_mut(),
                    noise_frame,
                    &mut median_scratch,
                )
            })
            .then_some(&noise_frame.mag[..]);
        if let (Some(_), Some(pf), Some(t), Some(l)) =
            (noise_mag, probe_frame.as_mut(), truth, lead)
        {
            pf.compute(&t.noise[l + end - win..l + end]);
            running_median(
                &pf.mag,
                TRUTH_HALF_BINS,
                &mut probe_level,
                &mut median_scratch,
            );
        }

        let hop = HopCtx {
            hop: hop_index,
            t_since,
            bucket,
            is_silence,
            block: Block::Note,
            audio: samples,
            spectrum: &frame.spectrum,
            mag: &frame.mag,
            hz_per_bin,
            sigma,
            truth_level: truth_level.as_deref(),
            oracle: &oracle,
            capture: &cctx,
        };
        arm.observe(&hop);

        if t_since >= 0.0 && serve.contains(&Consumer::G1) {
            thresholds.fill(f32::INFINITY);
            if arm.g1_threshold(&hop, &mut thresholds) == Availability::Available {
                // Per partial: is its lobe peak a kept local maximum above T? Its
                // frequency is the refined one the picker would report.
                for (i, &n) in ns.iter().enumerate() {
                    let bin = lobe_peak_bin(&frame.mag, f_meas[i], hz_per_bin);
                    let f_refined =
                        spectral::jacobsen(hop.spectrum, bin, BASS_WINDOW_SIZE, SAMPLE_RATE);
                    let amp = frame.mag[bin];
                    let admit = is_local_max(&frame.mag, bin)
                        && amp >= thresholds[bin]
                        && arm.g1_keep_peak(&hop, bin);
                    let level = match (&truth_level, oracle[i]) {
                        (Some(lvl), _) => lvl[bin],
                        (None, Some(o)) => o,
                        _ => f32::NAN,
                    };
                    let snr = amp / level;
                    let population = if absent[i] {
                        "absent"
                    } else {
                        population_of(snr, snr)
                    };
                    let estimate = if thresholds[bin].is_finite() && !arm.decision_only() {
                        thresholds[bin] / k_exact
                    } else {
                        f32::NAN
                    };
                    let exact_admit = level
                        .is_finite()
                        .then(|| is_local_max(&frame.mag, bin) && amp >= k_exact * level);
                    let row = Row {
                        register,
                        bucket: hop.bucket,
                        consumer: Consumer::G1,
                        population,
                        amplitude: amp,
                        estimate,
                        threshold: thresholds[bin],
                        admit: Some(admit),
                        truth_hop: noise_mag.map(|m| m[bin]).unwrap_or(f32::NAN),
                        truth_level: level,
                        snr_gate: snr,
                        snr_true: snr,
                        err_db: err_db(estimate, level),
                        covered: None,
                        exact_admit,
                        scan: None,
                    };
                    study.push(
                        row,
                        &cctx,
                        Loc {
                            hop: hop_index,
                            t_since,
                            window: BASS_WINDOW_SIZE,
                            n,
                            f_hz: f_refined,
                        },
                    )?;
                }
                // The hop's false candidates off every predicted partial.
                let mut count = 0u32;
                let mut rumble = 0u32;
                let mut exact = 0u32;
                let mut bins = 0u32;
                for b in 1..frame.mag.len() - 1 {
                    if mask[b] {
                        continue;
                    }
                    bins += 1;
                    if !is_local_max(&frame.mag, b) {
                        continue;
                    }
                    if frame.mag[b] >= thresholds[b] && arm.g1_keep_peak(&hop, b) {
                        count += 1;
                        if b <= rumble_bins {
                            rumble += 1;
                        }
                    }
                    if let Some(lvl) = &truth_level
                        && frame.mag[b] >= k_exact * lvl[b]
                    {
                        exact += 1;
                    }
                }
                let row = Row {
                    register,
                    bucket: hop.bucket,
                    consumer: Consumer::G1,
                    population: "scan",
                    amplitude: f32::NAN,
                    estimate: f32::NAN,
                    threshold: f32::NAN,
                    admit: None,
                    truth_hop: f32::NAN,
                    truth_level: f32::NAN,
                    snr_gate: f32::NAN,
                    snr_true: f32::NAN,
                    err_db: f32::NAN,
                    covered: None,
                    exact_admit: None,
                    scan: Some((count, bins, rumble, exact)),
                };
                study.push(
                    row,
                    &cctx,
                    Loc {
                        hop: hop_index,
                        t_since,
                        window: BASS_WINDOW_SIZE,
                        n: 0,
                        f_hz: f32::NAN,
                    },
                )?;
            } else {
                *study.declined.entry(Consumer::G1).or_default() += 1;
            }
        }

        // G2 (adaptive target) and G3 (fixed reference), at the register window.
        for (i, &n) in ns.iter().enumerate() {
            let (amp_fixed, phase_fixed) =
                goertzel_windowed(hop.audio, SAMPLE_RATE, f_meas[i], &hann_gate);
            let (amp_track, phase_track) =
                goertzel_windowed(hop.audio, SAMPLE_RATE, target[i], &hann_gate);
            let f_inst_fixed = warm.then(|| {
                let d = (phase_fixed - prev_phase_fixed[i] - TAU * f_meas[i] * t_hop
                    + std::f32::consts::PI)
                    .rem_euclid(TAU)
                    - std::f32::consts::PI;
                f_meas[i] + d / (TAU * t_hop)
            });
            let f_inst_track = warm.then(|| {
                let d = (phase_track - prev_phase_track[i] - TAU * target[i] * t_hop
                    + std::f32::consts::PI)
                    .rem_euclid(TAU)
                    - std::f32::consts::PI;
                target[i] + d / (TAU * t_hop)
            });
            prev_phase_fixed[i] = phase_fixed;
            prev_phase_track[i] = phase_track;
            let target_now = target[i];
            // The shipped tracker re-centres on hops the shipped gate admits, at
            // the recorded σ: the trajectory the instrument took.
            if let Some(fi) = f_inst_track
                && amp_track >= sigma_rec * k_gate
                && fi.is_finite()
                && fi > 0.0
            {
                target[i] = 0.95 * target[i] + 0.05 * fi;
            }
            if !warm || t_since < 0.0 {
                continue;
            }
            for (consumer, amp, f_hz, f_inst, fixed) in [
                (Consumer::G2, amp_track, target_now, f_inst_track, false),
                (Consumer::G3, amp_fixed, f_meas[i], f_inst_fixed, true),
            ] {
                if !serve.contains(&consumer) {
                    continue;
                }
                let exact_gate = probe_frame
                    .is_some()
                    .then(|| {
                        probe_level[((f_hz / probe_hz).round() as usize).min(win / 2 - 1)] * 4.0
                            / win as f32
                    })
                    .filter(|v| v.is_finite());
                let probe = GateProbe {
                    index: i,
                    target_hz: f_hz,
                    window: win,
                    amplitude: amp,
                    f_inst,
                    fixed,
                    truth_level: exact_gate,
                };
                let v = arm.gate(&hop, &probe);
                let Some(admit) = v.admit else {
                    *study.declined.entry(consumer).or_default() += 1;
                    continue;
                };
                // The truth at the gate's window: exact from the noise, else the
                // oracle scaled by the processing gain.
                let bin = (f_hz / hz_per_bin).round() as usize;
                let (truth_hop, level, level_probe) = match (truth, lead) {
                    (Some(t), Some(l)) if l + end <= t.noise.len() => {
                        let (na, _) = goertzel_windowed(
                            &t.noise[l + end - BASS_WINDOW_SIZE..l + end],
                            SAMPLE_RATE,
                            f_hz,
                            &hann_gate,
                        );
                        let lvl = truth_level
                            .as_deref()
                            .map(|v| v[bin.min(v.len() - 1)])
                            .unwrap_or(f32::NAN);
                        let at_gate = exact_gate.unwrap_or_else(|| raw_to_gate(lvl, win));
                        (na, at_gate, raw_to_gate(lvl, BASS_WINDOW_SIZE))
                    }
                    _ => match oracle[i] {
                        Some(o) => (
                            f32::NAN,
                            o * 4.0 / BASS_WINDOW_SIZE as f32 * noise_scale,
                            o * 4.0 / BASS_WINDOW_SIZE as f32,
                        ),
                        None => (f32::NAN, f32::NAN, f32::NAN),
                    },
                };
                let snr_gate = amp / level;
                let snr_true = amp / level_probe;
                let population = if absent[i] {
                    "absent"
                } else if level.is_finite() {
                    population_of(snr_gate, snr_true)
                } else {
                    "unmeasurable"
                };
                let row = Row {
                    register,
                    bucket: hop.bucket,
                    consumer,
                    population,
                    amplitude: amp,
                    estimate: v.estimate.unwrap_or(f32::NAN),
                    threshold: v.threshold.unwrap_or(f32::NAN),
                    admit: Some(admit),
                    truth_hop,
                    truth_level: level,
                    snr_gate,
                    snr_true,
                    err_db: err_db(v.estimate.unwrap_or(f32::NAN), level),
                    covered: covered(v.interval, level),
                    exact_admit: level.is_finite().then_some(amp >= k_exact * level),
                    scan: None,
                };
                study.push(
                    row,
                    &cctx,
                    Loc {
                        hop: hop_index,
                        t_since,
                        window: win,
                        n,
                        f_hz,
                    },
                )?;
            }
        }
        warm = true;
        end += HOP_SIZE;
        hop_index += 1;
    }
    Ok(())
}

/// BG on the record's first 2¹⁶ samples: the arm's floor at each partial against
/// the partial's own magnitude and, on a synthetic set, the noise's exact level
/// in the same window.
fn bg_pass(
    arm: &mut dyn Arm,
    study: &mut Study,
    cctx: &CaptureCtx,
    audio: &[f32],
    truth: Option<&Synthetic>,
    lead: Option<usize>,
    planner: &mut RealFftPlanner<f32>,
) -> Result<()> {
    let n = CAPTURE_ANALYSIS_SAMPLES;
    let plan = planner.plan_fft_forward(n);
    let mut time = vec![0.0f32; n];
    let mut spec = vec![Complex::new(0.0, 0.0); n / 2 + 1];
    let mut mag = vec![0.0f32; n / 2];
    spectral::fft(&audio[..n], &mut time, &mut spec, &plan, n);
    spectral::magnitude_spectrum(&spec, n, &mut mag);
    let hz_per_bin = SAMPLE_RATE as f32 / n as f32;
    let mut floor = vec![f32::NAN; cctx.f_meas.len()];
    if arm.bg_floor(cctx, &mag, hz_per_bin, cctx.f_meas, &mut floor) != Availability::Available {
        *study.declined.entry(Consumer::Bg).or_default() += 1;
        return Ok(());
    }
    // The noise alone in the same window, where it is known.
    let noise_mag: Option<Vec<f32>> = match (truth, lead) {
        (Some(t), Some(l)) if l + n <= t.noise.len() => {
            let mut nm = vec![0.0f32; n / 2];
            spectral::fft(&t.noise[l..l + n], &mut time, &mut spec, &plan, n);
            spectral::magnitude_spectrum(&spec, n, &mut nm);
            Some(nm)
        }
        _ => None,
    };
    let absent: Vec<bool> = cctx
        .ns
        .iter()
        .map(|n| truth.is_some_and(|t| t.absent.contains(n)))
        .collect();
    for (i, &f) in cctx.f_meas.iter().enumerate() {
        if floor[i].is_nan() {
            continue;
        }
        let c = (f / hz_per_bin).round() as usize;
        let lo = c.saturating_sub(2).max(1);
        let hi = (c + 2).min(mag.len() - 2);
        let bin = (lo..=hi)
            .max_by(|&a, &b| mag[a].total_cmp(&mag[b]))
            .unwrap_or(c);
        let amp = mag[bin];
        // The exact level: the median of the noise over ±32 bins (± 21.5 Hz).
        let (truth_hop, level) = match &noise_mag {
            Some(nm) => {
                let a = c.saturating_sub(32).max(1);
                let b = (c + 32).min(nm.len() - 1);
                let mut v: Vec<f32> = nm[a..=b].to_vec();
                v.sort_by(f32::total_cmp);
                (nm[bin], v[v.len() / 2])
            }
            None => (f32::NAN, f32::NAN),
        };
        let k_exact = median_to_threshold(study.p_fa);
        let threshold = arm.bg_multiplier().unwrap_or(k_exact) * floor[i];
        let snr = amp / level;
        let population = if absent[i] {
            "absent"
        } else if level.is_finite() {
            population_of(snr, snr)
        } else {
            "unmeasured"
        };
        let row = Row {
            register: cctx.register,
            bucket: 0,
            consumer: Consumer::Bg,
            population,
            amplitude: amp,
            estimate: floor[i],
            threshold,
            admit: Some(amp >= threshold),
            truth_hop,
            truth_level: level,
            snr_gate: snr,
            snr_true: snr,
            err_db: err_db(floor[i], level),
            covered: None,
            exact_admit: level.is_finite().then_some(amp >= k_exact * level),
            scan: None,
        };
        study.push(
            row,
            cctx,
            Loc {
                hop: 0,
                t_since: 0.0,
                window: n,
                n: cctx.ns[i],
                f_hz: f,
            },
        )?;
    }
    Ok(())
}

// ── the tables ───────────────────────────────────────────────────────────────

fn pct(num: u64, den: u64) -> f32 {
    if den == 0 {
        f32::NAN
    } else {
        100.0 * num as f32 / den as f32
    }
}

fn quantiles(v: &mut [f32]) -> (f32, f32, f32) {
    if v.is_empty() {
        return (f32::NAN, f32::NAN, f32::NAN);
    }
    v.sort_by(f32::total_cmp);
    let q = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
    (q(0.5), q(0.05), q(0.95))
}

fn rate_line(label: &str, k: u64, n: u64, nominal: Option<f64>) -> String {
    let (lo, hi) = stats::clopper_pearson(k, n, 0.05);
    let rate = if n == 0 {
        f64::NAN
    } else {
        k as f64 / n as f64
    };
    let verdict = match nominal {
        Some(p) if n > 0 => {
            if lo <= p && p <= hi {
                "inside nominal's interval"
            } else if rate > p {
                "ABOVE nominal"
            } else {
                "BELOW nominal"
            }
        }
        _ => "",
    };
    format!(
        "  {label:<28} {k:>8} / {n:<9} rate {rate:>9.3e}  95 % CP [{lo:.2e}, {hi:.2e}]  {verdict}"
    )
}

fn report(st: &Study, serve: &[Consumer]) {
    println!(
        "  {} captures, {} with screened pre-roll; silent hops build {} / hold-out {}; {} rows",
        st.captures,
        st.captures_with_preroll,
        st.silent_hops.0,
        st.silent_hops.1,
        st.rows.len()
    );
    for (c, n) in &st.declined {
        println!("  {} declined {n} probes or hops", c.label());
    }

    // The live cuts: the pre-registered 3, the sweep's 6.3 / 10 / 20, and ISO
    // 3744's 6 dB and 15 dB margins as partial-to-noise power ratios 2.98 and
    // 30.6 converted to an amplitude over the noise's median magnitude: a
    // partial of power R·σ_b² against a median of σ_b√(ln 2) is √(R / ln 2)
    // medians, 2.07 and 6.64.
    let cuts: [(&str, f32); 6] = [
        ("≥3×", 3.0),
        ("≥6.3×", 6.3),
        ("≥10×", 10.0),
        ("≥20×", 20.0),
        ("ISO 6 dB", (2.98f32 / std::f32::consts::LN_2).sqrt()),
        ("ISO 15 dB", (30.6f32 / std::f32::consts::LN_2).sqrt()),
    ];

    for &consumer in serve {
        let rows: Vec<&Row> = st.rows.iter().filter(|r| r.consumer == consumer).collect();
        if rows.is_empty() {
            continue;
        }
        println!("\n  ── {} ──", consumer.label());
        println!(
            "  {:<12} {:>7} {:>9} {:>7} {:>9} {:>7} {:>9} {:>7} {:>7}   miss% at {}",
            "register",
            "absent",
            "P_fa",
            "H0",
            "P_fa",
            "silence",
            "P_fa",
            "live",
            "blind",
            cuts.iter().map(|c| c.0).collect::<Vec<_>>().join(" / ")
        );
        for (ri, rname) in REGISTERS
            .iter()
            .enumerate()
            .chain(std::iter::once((usize::MAX, &"ALL")))
        {
            let sel: Vec<&&Row> = rows
                .iter()
                .filter(|r| ri == usize::MAX || r.register == ri)
                .collect();
            if sel.is_empty() {
                continue;
            }
            let count = |pop: &str| -> (u64, u64) {
                let s: Vec<&&&Row> = sel.iter().filter(|r| r.population == pop).collect();
                (
                    s.iter().filter(|r| r.admit == Some(true)).count() as u64,
                    s.len() as u64,
                )
            };
            let (a_k, a_n) = count("absent");
            let (h_k, h_n) = count("h0");
            let (s_k, s_n) = count("silence");
            let live: Vec<&&&Row> = sel
                .iter()
                .filter(|r| r.population == "live" || r.population == "ambig")
                .collect();
            let blind = count("blind");
            let misses: Vec<String> = cuts
                .iter()
                .map(|&(_, c)| {
                    let at: Vec<&&&&Row> = live.iter().filter(|r| r.snr_gate >= c).collect();
                    let m = at.iter().filter(|r| r.admit == Some(false)).count();
                    format!("{:>5.1}% ({})", pct(m as u64, at.len() as u64), at.len())
                })
                .collect();
            println!(
                "  {:<12} {:>7} {:>9.2e} {:>7} {:>9.2e} {:>7} {:>9.2e} {:>7} {:>7}   {}",
                rname,
                a_n,
                pct(a_k, a_n) / 100.0,
                h_n,
                pct(h_k, h_n) / 100.0,
                s_n,
                pct(s_k, s_n) / 100.0,
                live.iter().filter(|r| r.snr_gate >= 3.0).count(),
                blind.1,
                misses.join(" / ")
            );
        }

        // Q-A: estimation error against the truth, and coverage.
        let with_err: Vec<&&Row> = rows.iter().filter(|r| r.err_db.is_finite()).collect();
        if !with_err.is_empty() {
            println!(
                "\n  {} Q-A: 20 lg(estimate ÷ truth level), median [5 %, 95 %] by register × time bucket; coverage of the arm's own interval",
                consumer.label()
            );
            print!("  {:<12}", "register");
            for l in T_LABELS {
                print!(" {l:>22}");
            }
            println!("   coverage");
            for (ri, rname) in REGISTERS.iter().enumerate() {
                let sel: Vec<&&&Row> = with_err.iter().filter(|r| r.register == ri).collect();
                if sel.is_empty() {
                    continue;
                }
                print!("  {rname:<12}");
                for tb in 0..T_EDGES.len() {
                    let mut v: Vec<f32> = sel
                        .iter()
                        .filter(|r| r.bucket == tb && r.population != "absent")
                        .map(|r| r.err_db)
                        .collect();
                    if v.is_empty() {
                        print!(" {:>22}", "—");
                    } else {
                        let (m, lo, hi) = quantiles(&mut v);
                        print!(" {:>+6.1} [{:>+5.1},{:>+5.1}] n{:<4}", m, lo, hi, v.len());
                    }
                }
                let cov: Vec<&&&&Row> = sel.iter().filter(|r| r.covered.is_some()).collect();
                let hit = cov.iter().filter(|r| r.covered == Some(true)).count();
                if cov.is_empty() {
                    println!("   —");
                } else {
                    println!(
                        "   {:.1} % of {}",
                        pct(hit as u64, cov.len() as u64),
                        cov.len()
                    );
                }
            }
        }

        // T ÷ T_ideal, the record's own column.
        let with_t: Vec<&&Row> = rows
            .iter()
            .filter(|r| r.threshold.is_finite() && r.truth_level.is_finite() && r.truth_level > 0.0)
            .collect();
        if !with_t.is_empty() {
            println!(
                "\n  {} T ÷ T_ideal (T_ideal = 3.157 × the truth's median level; 1.0 is correct), median by register × bucket",
                consumer.label()
            );
            print!("  {:<12}", "register");
            for l in T_LABELS {
                print!(" {l:>9}");
            }
            println!();
            for (ri, rname) in REGISTERS.iter().enumerate() {
                let sel: Vec<&&&Row> = with_t.iter().filter(|r| r.register == ri).collect();
                if sel.is_empty() {
                    continue;
                }
                print!("  {rname:<12}");
                for tb in 0..T_EDGES.len() {
                    let mut v: Vec<f32> = sel
                        .iter()
                        .filter(|r| r.bucket == tb)
                        .map(|r| r.threshold / (median_to_threshold(st.p_fa) * r.truth_level))
                        .collect();
                    if v.is_empty() {
                        print!(" {:>9}", "—");
                    } else {
                        print!(" {:>9.2}", quantiles(&mut v).0);
                    }
                }
                println!();
            }
        }

        // G1's scan: false candidates per hop and per bin.
        if consumer == Consumer::G1 {
            for pop in ["scan_silence", "scan"] {
                let scans: Vec<(u32, u32, u32, u32)> = rows
                    .iter()
                    .filter(|r| r.population == pop)
                    .filter_map(|r| r.scan)
                    .collect();
                if scans.is_empty() {
                    continue;
                }
                let hops = scans.len() as u64;
                let count: u64 = scans.iter().map(|s| s.0 as u64).sum();
                let bins: u64 = scans.iter().map(|s| s.1 as u64).sum();
                let rumble: u64 = scans.iter().map(|s| s.2 as u64).sum();
                let exact: u64 = scans.iter().map(|s| s.3 as u64).sum();
                println!(
                    "\n  G1 {pop}: {count} local maxima above T over {hops} hops and {bins} off-partial bins — {:.2} per hop, {:.2e} per bin ({rumble} below {RUMBLE_HZ:.0} Hz)",
                    count as f64 / hops as f64,
                    count as f64 / bins.max(1) as f64
                );
                if st.synthetic {
                    println!(
                        "     the exact threshold on the same hops (the Hann geometry's own rate at 10⁻³): {exact} — {:.2} per hop, {:.2e} per bin",
                        exact as f64 / hops as f64,
                        exact as f64 / bins.max(1) as f64
                    );
                }
            }
        }
    }

    // The control's verdict: every H₀ population against nominal. A Goertzel
    // probe's nominal is the operating point itself; the picker's lobe search
    // and scan take the exact threshold's own rate on the same rows, the Hann
    // geometry's rate at that operating point.
    if st.synthetic {
        println!(
            "\n  ── T1 check: realised false-alarm rates against nominal {:.1e} (95 % Clopper–Pearson) ──",
            st.p_fa
        );
        for &consumer in serve {
            let rows: Vec<&Row> = st.rows.iter().filter(|r| r.consumer == consumer).collect();
            for pop in ["absent", "silence", "h0"] {
                let sel: Vec<&&Row> = rows.iter().filter(|r| r.population == pop).collect();
                if sel.is_empty() {
                    continue;
                }
                let k = sel.iter().filter(|r| r.admit == Some(true)).count() as u64;
                let exact: Vec<&&&Row> = sel.iter().filter(|r| r.exact_admit.is_some()).collect();
                let nominal = if consumer == Consumer::G1 || consumer == Consumer::Bg {
                    (!exact.is_empty()).then(|| {
                        exact.iter().filter(|r| r.exact_admit == Some(true)).count() as f64
                            / exact.len() as f64
                    })
                } else {
                    Some(st.p_fa as f64)
                };
                println!(
                    "{}",
                    rate_line(
                        &format!("{} {pop}", consumer.label()),
                        k,
                        sel.len() as u64,
                        nominal
                    )
                );
                if !exact.is_empty() {
                    let ke = exact.iter().filter(|r| r.exact_admit == Some(true)).count() as u64;
                    if consumer == Consumer::G1 || consumer == Consumer::Bg {
                        println!(
                            "     nominal here is the exact threshold's own rate on the same lobe search: {ke} / {}",
                            exact.len()
                        );
                    } else {
                        println!(
                            "     the exact threshold at the probe's window on the same probes: {ke} / {} = {:.3e}",
                            exact.len(),
                            ke as f64 / exact.len() as f64
                        );
                    }
                }
            }
            if consumer == Consumer::G1 {
                for pop in ["scan_silence", "scan"] {
                    let scans: Vec<(u32, u32, u32, u32)> = rows
                        .iter()
                        .filter(|r| r.population == pop)
                        .filter_map(|r| r.scan)
                        .collect();
                    if scans.is_empty() {
                        continue;
                    }
                    let count: u64 = scans.iter().map(|s| s.0 as u64).sum();
                    let bins: u64 = scans.iter().map(|s| s.1 as u64).sum();
                    let exact: u64 = scans.iter().map(|s| s.3 as u64).sum();
                    // Nominal for a scan is the geometry's own rate at a correct
                    // threshold, measured here with the exact threshold.
                    let nominal = if bins > 0 {
                        Some(exact as f64 / bins as f64)
                    } else {
                        None
                    };
                    println!(
                        "{}",
                        rate_line(&format!("G1 {pop} per bin"), count, bins, nominal)
                    );
                    if let Some(p) = nominal {
                        println!(
                            "     nominal here is the exact threshold's own rate {p:.3e} per bin: the Hann geometry's local-maximum rate at this operating point"
                        );
                    }
                }
            }
        }
    }
}
