//! Report 0020's lock replay at phases of the hop grid, with a G1 arm from
//! `gates arms` in place of the shipped peak floor.
//!
//! Each capture's full event, or its `audio.raw` where it kept none, is cut at
//! `phases` offsets spread evenly over one hop, `k · HOP_SIZE / phases` samples
//! from its head (0, 64, …, 960 at sixteen), and every cut runs hop by hop
//! through the front end `engine lock` drives: a fresh Gatekeeper at the default
//! thresholds and the capture's logged silence threshold, reading the newest 2048
//! samples, then the 8192-point bass FFT. The lock is the real `Engine::process`
//! in auto mode, as `engine lock` runs it, or, with `--peak-arm`, a replica of its
//! discovery state machine that thresholds each bin at the arm's level where the
//! engine applies its one Neyman–Pearson scalar (Kay 1998, eq. 7.26). The replica
//! then runs beside the engine on the same hops and every (capture, cut) is
//! compared: under arm `A`, the shipped scalar at the recorded σ, the two must
//! commit the same key on the same hop everywhere.
//!
//! The arm is prepared at the peak floor's operating point, told about the
//! capture before its hops, and shown every hop before it is asked for a
//! threshold, a Gatekeeper-silent hop as one it may learn from. By default a
//! fresh arm runs each capture at each cut, so a running floor restarts there
//! and knows only the silent hops before that capture's strike; it sees the hops
//! up to the later of the two commits. With `--carry` one arm per cut runs the
//! set in recording order, sees every hop of every capture and carries what it
//! learned from one capture to the next. `finish` is not called: an arm's own
//! tables are `gates arms`'s. The picker does not know the key, so the arm is
//! given no partial spacing (`NaN`) unless `--known-key` hands it the key's
//! equal-tempered f₀.
//!
//! Time is report 0020's clock, one for every cut: a hop's window end on the
//! original recording minus the strike's first hop there, the first of the
//! recording's first 30 hops of 1024 samples whose RMS passes a tenth of the
//! loudest's. A commit's latency and the arm's `t_since` are both read on it.
//!
//! The tables are report 0020's yardstick: per cut the pass, no-lock and wrong
//! counts, overall and in `engine lock`'s three registers; over the cuts each
//! count's range, mean and population σ, the cuts being the whole grid and not a
//! sample of it; and the captures that pass at every cut, at none, and those the
//! grid decides. Cut 0 is printed again in `engine lock`'s own lines, so the two
//! outputs diff.
//!
//! Usage: `cargo lab engine replay ROOT [--phases N] [--peak-arm NAME]
//! [--known-key] [--carry] [--regen REGEN] [--out CSV]`

use std::cmp::Ordering;
use std::fs::{self, File};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use realfft::{RealFftPlanner, RealToComplex};
use rustfft::num_complex::Complex;

use crate::gatekeeper::replay as gate_replay;
use crate::gates::ambient;
use crate::gates::arms::{self, Arm, Availability, Block, CaptureCtx, Consumer, HopCtx, SetCtx};
use crate::{capture, raw};
use tuner_core::algorithms::twm::TwmConfig;
use tuner_core::algorithms::{discovery, peaks, spectral};
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE};
use tuner_core::engine::Engine;
use tuner_core::gatekeeper::{GateResult, GatekeeperConfig, SignalState};
use tuner_core::models::{KeyProfile, NOTES, SpectralPeak};
use tuner_core::pipeline::ProcessingFrame;

/// Votes a key must hold in the window to commit: `Engine::process`'s M, of its
/// binary-integration rule (Schwartz 1956).
// The engine's constants are private, so the replica restates them; the engine
// running beside it under arm A guards the copy. report 0010
const LOCK_VOTES_M: usize = 7;

/// Discovery winners the vote counts over: `Engine::process`'s N.
const LOCK_WINDOW_N: usize = 8;

/// Local maxima `peaks::extract_peaks` keeps: the first in bin order.
const MAX_MAXIMA: usize = 128;

/// Peaks `Engine::process` hands `peaks::mask_peaks`: the strongest of those kept.
const MAX_PEAKS: usize = 64;

/// False-alarm probability of `Engine::process`'s peak floor, the operating point
/// every arm is prepared at.
const PEAK_FLOOR_P_FA: f32 = 1.0e-3;

/// The silence threshold `engine lock` takes for a capture that logs none.
const DEFAULT_NOISE_FLOOR: f32 = 0.001;

/// Hops of 1024 samples from the recording's start searched for the strike.
// The onset report 0020's latencies count from, by its replay's own rule.
// report 0020
const ONSET_SEARCH_HOPS: usize = 30;

/// A hop's RMS over the loudest hop's at which the strike has begun: −20 dB.
// report 0020
const ONSET_FRACTION: f64 = 0.1;

/// Magnitude bins of the 8192-point frame, the ones `extract_peaks` scans.
const BINS: usize = BASS_WINDOW_SIZE / 2;

/// `engine lock`'s registers by key: bass to 26, mid to 59, treble above.
const REGISTERS: [&str; 3] = ["bass", "mid", "treble"];

/// The same registers as report 0020's tables label them.
const REGISTER_LABELS: [&str; 3] = ["bass (0–26)", "mid (27–59)", "treble (60–87)"];

/// First key of report 0020's treble high, 75–87, the group its yardstick counts
/// the captures the grid decides in.
const TREBLE_HIGH_FROM: u8 = 75;

/// The replay's command-line choices.
pub struct Options<'a> {
    /// Cuts of the hop grid, spread evenly over one hop: 1 to `HOP_SIZE`.
    pub phases: usize,
    /// The G1 arm in place of the peak floor, by `gates arms`'s name; the engine
    /// alone when `None`.
    pub peak_arm: Option<&'a str>,
    /// A `mat regen` dump whose fitted curve the templates take, in place of the
    /// prior.
    pub regen: Option<&'a Path>,
    /// Where one CSV row per (capture, cut) is written.
    pub out: Option<&'a Path>,
    /// Hand the arm the key's partial spacing, which the picker does not have.
    pub known_key: bool,
    /// One arm per cut over the whole set in recording order, in place of a fresh
    /// one per capture.
    pub carry: bool,
}

/// One capture, read once.
struct Capture {
    /// The directory's name, as `engine lock` lists it.
    name: String,
    /// The key it was captured as: `analysis.json`'s `key_index`.
    key: u8,
    /// Its logged silence threshold: the gate's, and σ for the peak floor.
    noise_floor: f32,
    /// The full event, or the analysed record where none was kept.
    audio: Vec<f32>,
    /// The strike's first sample on the clock, `None` where it has none.
    onset: Option<usize>,
}

/// A commit: the key, and the hop of its cut's grid it landed on. Hop `h` reads
/// samples `h·HOP_SIZE` to `h·HOP_SIZE + 8192` of the cut audio.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Commit {
    key: u8,
    hop: usize,
}

/// A capture's outcome at one cut: its first commit against the key it was
/// captured as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Pass,
    NoLock,
    Wrong,
}

impl Outcome {
    /// The outcomes in the order of report 0020's tables.
    const ALL: [Outcome; 3] = [Outcome::Pass, Outcome::NoLock, Outcome::Wrong];

    /// The outcome of `lock` for a capture of `key`.
    fn of(lock: Option<Commit>, key: u8) -> Self {
        match lock {
            Some(c) if c.key == key => Outcome::Pass,
            Some(_) => Outcome::Wrong,
            None => Outcome::NoLock,
        }
    }

    /// The outcome as report 0020's tables and the CSV name it.
    fn label(self) -> &'static str {
        match self {
            Outcome::Pass => "pass",
            Outcome::NoLock => "no-lock",
            Outcome::Wrong => "wrong",
        }
    }
}

/// One capture at one cut.
#[derive(Clone, Copy, Debug)]
struct Run {
    /// The first commit of the lock under test.
    lock: Option<Commit>,
    /// `Engine::process`'s first commit on the same hops, where the replica ran
    /// beside it.
    engine: Option<Option<Commit>>,
}

/// Runs the replay over the capture set at `root` and prints its tables.
pub fn run(root: &Path, opts: &Options) -> Result<()> {
    if !(1..=HOP_SIZE).contains(&opts.phases) {
        bail!("--phases takes 1 to {HOP_SIZE}, not {}", opts.phases);
    }
    let cuts = cuts(opts.phases);
    if let Some(name) = opts.peak_arm {
        let arm = arms::arm_by_name(name)?;
        if !arm.consumers().contains(&Consumer::G1) {
            bail!("arm {name} does not serve the picker (G1)");
        }
    }
    let betas = super::template_b(opts.regen);
    let profiles: [KeyProfile; 88] =
        core::array::from_fn(|i| KeyProfile::new(NOTES[i].frequency, betas[i]));
    let (caps, left_out) = load(root, cuts[cuts.len() - 1])?;
    if caps.is_empty() {
        bail!("no capture under {} holds a window", root.display());
    }
    let label = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("set")
        .to_string();
    print_header(root, &caps, left_out, &cuts, opts);

    let set = SetCtx {
        label: &label,
        p_fa: PEAK_FLOOR_P_FA,
        sigma_override: None,
        nu_half_per_frame: None,
    };
    let order = if opts.carry {
        recording_order(&caps)
    } else {
        (0..caps.len()).collect()
    };
    let mut front = Front::new();
    let mut replica = Replica::new(&profiles);
    let mut runs: Vec<Vec<Run>> = vec![Vec::with_capacity(cuts.len()); caps.len()];
    let mut elapsed = Vec::with_capacity(cuts.len());

    print_cut_header(opts.peak_arm.is_some());
    for (p, &cut) in cuts.iter().enumerate() {
        let started = Instant::now();
        let mut carried = match opts.peak_arm {
            Some(name) if opts.carry => Some(prepared_arm(name, &set)?),
            _ => None,
        };
        for &c in &order {
            let cap = &caps[c];
            let run = match opts.peak_arm {
                None => replay_cut(cap, cut, &profiles, &mut front, None),
                Some(name) => {
                    let spacing_hz = if opts.known_key {
                        profiles[cap.key as usize].f0_et
                    } else {
                        f32::NAN
                    };
                    let ctx = CaptureCtx {
                        dir: &cap.name,
                        key: cap.key,
                        register: ambient::register_index(cap.key),
                        ns: &[],
                        f_meas: &[],
                        spacing_hz,
                        b: betas[cap.key as usize],
                        sigma_rec: cap.noise_floor,
                    };
                    let mut fresh = None;
                    let arm: &mut dyn Arm = match carried.as_mut() {
                        Some(arm) => arm.as_mut(),
                        None => fresh.insert(prepared_arm(name, &set)?).as_mut(),
                    };
                    arm.begin_capture(&ctx);
                    let beside = Beside {
                        replica: &mut replica,
                        arm,
                        capture: &ctx,
                        observe_all: opts.carry,
                    };
                    replay_cut(cap, cut, &profiles, &mut front, Some(beside))
                }
            };
            runs[c].push(run);
        }
        elapsed.push(started.elapsed());
        print_cut_row(&caps, &runs, p, cut, elapsed[p]);
    }

    print_yardstick(&caps, &runs);
    print_latency(&caps, &runs, &cuts);
    match opts.peak_arm {
        None => print_lock_form("ENGINE AUTO LOCK", &caps, &runs),
        Some(name) => {
            print_lock_form(&format!("ARM {name} LOCK"), &caps, &runs);
            print_tally(name, &replica.tally);
            print_comparison(name, &caps, &runs, &cuts);
        }
    }
    let total: Duration = elapsed.iter().sum();
    println!(
        "\nRuntime: {:.1} s over {} cut{}, {:.2} s per cut, {:.2} ms per capture and cut",
        total.as_secs_f64(),
        cuts.len(),
        if cuts.len() == 1 { "" } else { "s" },
        total.as_secs_f64() / cuts.len() as f64,
        total.as_secs_f64() * 1.0e3 / (cuts.len() * caps.len()) as f64
    );
    if let Some(path) = opts.out {
        write_csv(path, &caps, &cuts, &runs)?;
        println!("Wrote {}", path.display());
    }
    Ok(())
}

/// The head cuts of `phases` phases, spread evenly over one hop, in samples.
fn cuts(phases: usize) -> Vec<usize> {
    (0..phases).map(|k| k * HOP_SIZE / phases).collect()
}

/// Every capture under `root` whose audio holds the first window at `last_cut`,
/// in `capture::find`'s order, and how many were left out, so every cut scores
/// one population. The audio is the full event, else `audio.raw`, as `engine
/// lock` reads it.
fn load(root: &Path, last_cut: usize) -> Result<(Vec<Capture>, usize)> {
    let dirs = capture::find(root).with_context(|| format!("read dir {}", root.display()))?;
    let mut caps = Vec::with_capacity(dirs.len());
    let mut left_out = 0;
    for dir in &dirs {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let key =
            capture::key_of(dir).ok_or_else(|| anyhow!("{name}: no key_index in analysis.json"))?;
        match raw::full_event(dir).or_else(|| raw::stable(dir)) {
            Some(audio) if audio.len() >= last_cut + BASS_WINDOW_SIZE => caps.push(Capture {
                name,
                key,
                noise_floor: logged_noise_floor(dir),
                onset: strike_onset(&audio),
                audio,
            }),
            _ => left_out += 1,
        }
    }
    Ok((caps, left_out))
}

/// The capture's logged silence threshold, `metadata.noise_floor`, or
/// [`DEFAULT_NOISE_FLOOR`] where it logs none, as `engine lock` reads it.
fn logged_noise_floor(dir: &Path) -> f32 {
    fs::read_to_string(dir.join("analysis.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|j| j["metadata"]["noise_floor"].as_f64())
        .map(|v| v as f32)
        .filter(|v| *v > 0.0)
        .unwrap_or(DEFAULT_NOISE_FLOOR)
}

/// The strike's first sample on report 0020's clock: the start of the first of
/// the recording's first [`ONSET_SEARCH_HOPS`] hops of 1024 samples whose RMS
/// exceeds [`ONSET_FRACTION`] of the loudest hop's, `None` where none does.
fn strike_onset(audio: &[f32]) -> Option<usize> {
    let rms: Vec<f64> = audio
        .as_chunks::<HOP_SIZE>()
        .0
        .iter()
        .map(|hop| {
            let power = hop.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / HOP_SIZE as f64;
            power.sqrt()
        })
        .collect();
    let loudest = rms.iter().copied().fold(0.0, f64::max);
    rms.iter()
        .take(ONSET_SEARCH_HOPS)
        .position(|&r| r > ONSET_FRACTION * loudest)
        .map(|h| h * HOP_SIZE)
}

/// Samples from `onset` to the end of hop `hop`'s window at `cut`, both on the
/// original recording: report 0020's clock.
fn samples_since(onset: usize, cut: usize, hop: usize) -> i64 {
    (cut + hop * HOP_SIZE + BASS_WINDOW_SIZE) as i64 - onset as i64
}

/// A commit's latency in ms on report 0020's clock; `None` without a strike.
fn latency_ms(cap: &Capture, cut: usize, commit: Commit) -> Option<f64> {
    cap.onset
        .map(|onset| samples_since(onset, cut, commit.hop) as f64 / SAMPLE_RATE as f64 * 1.0e3)
}

/// `engine lock`'s register of `key`, an index into [`REGISTERS`].
fn register(key: u8) -> usize {
    match key {
        0..=26 => 0,
        27..=59 => 1,
        _ => 2,
    }
}

/// Capture indices in recording order: by the directory name's trailing serial,
/// as `gates arms` orders a set, then by name.
fn recording_order(caps: &[Capture]) -> Vec<usize> {
    let serial = |name: &str| name.rsplit('_').next().and_then(|s| s.parse::<u64>().ok());
    let mut order: Vec<usize> = (0..caps.len()).collect();
    order.sort_by(|&a, &b| {
        (serial(&caps[a].name), &caps[a].name).cmp(&(serial(&caps[b].name), &caps[b].name))
    });
    order
}

/// A fresh arm by name, prepared for `set`.
fn prepared_arm(name: &str, set: &SetCtx) -> Result<Box<dyn Arm>> {
    let mut arm = arms::arm_by_name(name)?;
    arm.prepare(set);
    Ok(arm)
}

/// The Gatekeeper's verdict on every hop of `audio`, as `engine lock` runs it: a
/// fresh gate at the default thresholds, with the capture's silence threshold.
fn gate_verdicts(audio: &[f32], noise_floor: f32) -> Vec<GateResult> {
    let config = GatekeeperConfig {
        silence_threshold: noise_floor,
        ..GatekeeperConfig::default()
    };
    gate_replay::run(audio, config)
        .into_iter()
        .map(|h| h.result)
        .collect()
}

/// The pipeline frame the engine reads, and the bass FFT plan.
struct Front {
    frame: ProcessingFrame,
    plan: Arc<dyn RealToComplex<f32>>,
}

impl Front {
    /// A zeroed frame and the plan for its 8192-point FFT.
    fn new() -> Self {
        Self {
            frame: ProcessingFrame::new(),
            plan: RealFftPlanner::<f32>::new().plan_fft_forward(BASS_WINDOW_SIZE),
        }
    }

    /// One hop's 8192 samples into the frame's audio buffer.
    fn load(&mut self, window: &[f32]) {
        self.frame.audio_buffer[..BASS_WINDOW_SIZE].copy_from_slice(window);
    }

    /// The window's 8192-point Hann FFT and its magnitudes, into the frame's bass
    /// buffers, as `engine lock` fills them.
    fn bass_spectrum(&mut self) {
        let frame = &mut self.frame;
        spectral::fft(
            &frame.audio_buffer[..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..BASS_WINDOW_SIZE],
            &mut frame.bass_frequency_buffer[..],
            &self.plan,
            BASS_WINDOW_SIZE,
        );
        spectral::magnitude_spectrum(
            &frame.bass_frequency_buffer,
            BASS_WINDOW_SIZE,
            &mut frame.bass_magnitude_buffer[..BINS],
        );
    }
}

/// `Engine::process`'s acquisition window: the last [`LOCK_WINDOW_N`] discovery
/// winners in a ring, with its fill length and cursor, kept as `record_winner`
/// and `reset_lock_window` keep them.
struct LockWindow {
    slots: [u8; LOCK_WINDOW_N],
    len: usize,
    head: usize,
}

impl LockWindow {
    /// An empty window.
    fn new() -> Self {
        Self {
            slots: [0; LOCK_WINDOW_N],
            len: 0,
            head: 0,
        }
    }

    /// Records `key` and returns whether it now holds [`LOCK_VOTES_M`] of the
    /// filled slots. M > N/2, so no other key can.
    fn record(&mut self, key: u8) -> bool {
        self.slots[self.head] = key;
        self.head = (self.head + 1) % LOCK_WINDOW_N;
        self.len = (self.len + 1).min(LOCK_WINDOW_N);
        self.slots[..self.len].iter().filter(|&&k| k == key).count() >= LOCK_VOTES_M
    }

    /// Empties the window.
    fn clear(&mut self) {
        self.len = 0;
        self.head = 0;
    }
}

/// What the arm answered on the hops the replica scanned.
#[derive(Default)]
struct Tally {
    /// Hops that ran discovery.
    hops: u64,
    /// Of those, hops the arm declined.
    declined: u64,
    /// Answered hops on which more than [`MAX_MAXIMA`] maxima cleared the
    /// threshold, so the cap in bin order bound.
    capped: u64,
    /// Maxima at or above the threshold per answered hop, before the cap.
    cleared: Vec<u32>,
}

/// `Engine::process`'s discovery state machine with a per-bin peak threshold, one
/// cut at a time, and what the arm answered over the run.
struct Replica<'p> {
    profiles: &'p [KeyProfile; 88],
    twm: TwmConfig,
    window: LockWindow,
    /// The arm's threshold per magnitude bin.
    thresholds: Vec<f32>,
    /// The maxima kept, before the magnitude sort.
    maxima: Vec<SpectralPeak>,
    /// The strongest maxima kept, which `mask_peaks` thins in place.
    peaks: Vec<SpectralPeak>,
    tally: Tally,
}

impl<'p> Replica<'p> {
    /// A replica scoring against `profiles` with the shipped TWM configuration.
    fn new(profiles: &'p [KeyProfile; 88]) -> Self {
        Self {
            profiles,
            twm: TwmConfig::default(),
            window: LockWindow::new(),
            thresholds: vec![f32::INFINITY; BINS],
            maxima: Vec::with_capacity(MAX_MAXIMA),
            peaks: vec![SpectralPeak::default(); MAX_PEAKS],
            tally: Tally::default(),
        }
    }

    /// One unlocked hop in `Engine::process`'s order: a silent hop clears the
    /// window, an onset's bypass clears it and casts no vote, and a new onset
    /// clears it before the hop votes. A voting hop takes the arm's threshold,
    /// extracts and masks the peaks, scans the 88 keys with refinement and records
    /// the winner. A hop the arm declines has no peaks, and `discover` on none
    /// names key 0, the vote the engine casts there. Returns the key this hop
    /// commits.
    fn hop(&mut self, hop: &HopCtx, gate: &GateResult, arm: &mut dyn Arm) -> Option<u8> {
        if gate.state == SignalState::Silence || gate.is_transient_bypass {
            self.window.clear();
            return None;
        }
        if gate.is_new_onset {
            self.window.clear();
        }
        self.tally.hops += 1;
        self.thresholds.fill(f32::INFINITY);
        let count = match arm.g1_threshold(hop, &mut self.thresholds) {
            Availability::Available => {
                let arm: &dyn Arm = arm;
                let (count, cleared) = extract_peaks_per_bin(
                    hop.mag,
                    hop.spectrum,
                    &self.thresholds,
                    |bin| arm.g1_keep_peak(hop, bin),
                    &mut self.maxima,
                    &mut self.peaks,
                );
                self.tally.capped += u64::from(cleared > MAX_MAXIMA);
                self.tally.cleared.push(cleared as u32);
                count
            }
            Availability::Unavailable => {
                self.tally.declined += 1;
                0
            }
        };
        let valid = peaks::mask_peaks(&mut self.peaks[..count]);
        let winner =
            discovery::discover(&self.peaks[..valid], self.profiles, &self.twm, true).key_index;
        self.window.record(winner).then_some(winner)
    }
}

/// `peaks::extract_peaks` with a threshold per bin. Every strict local maximum of
/// `mag` on bins `1..len − 1` at or above its bin's `threshold` that `keep`
/// accepts is refined by `spectral::jacobsen` on `spectrum`; the first
/// [`MAX_MAXIMA`] of positive frequency, in bin order, are sorted strongest first
/// and as many as `out` holds written there. Returns how many were written, and
/// how many maxima of positive frequency cleared the threshold before the cap.
/// With one threshold at every bin it writes what `extract_peaks` writes, except
/// for a maximum exactly at the threshold, which that function's strict test
/// drops.
fn extract_peaks_per_bin(
    mag: &[f32],
    spectrum: &[Complex<f32>],
    threshold: &[f32],
    mut keep: impl FnMut(usize) -> bool,
    maxima: &mut Vec<SpectralPeak>,
    out: &mut [SpectralPeak],
) -> (usize, usize) {
    maxima.clear();
    let mut cleared = 0;
    for bin in 1..mag.len().saturating_sub(1) {
        let m = mag[bin];
        if !(m >= threshold[bin] && m > mag[bin - 1] && m > mag[bin + 1] && keep(bin)) {
            continue;
        }
        let frequency = spectral::jacobsen(spectrum, bin, BASS_WINDOW_SIZE, SAMPLE_RATE);
        if frequency > 0.0 {
            cleared += 1;
            if maxima.len() < MAX_MAXIMA {
                maxima.push(SpectralPeak {
                    frequency,
                    magnitude: m,
                });
            }
        }
    }
    maxima.sort_unstable_by(|a, b| {
        b.magnitude
            .partial_cmp(&a.magnitude)
            .unwrap_or(Ordering::Equal)
    });
    let count = maxima.len().min(out.len());
    out[..count].copy_from_slice(&maxima[..count]);
    (count, cleared)
}

/// The replica beside the engine for one capture at one cut.
struct Beside<'a, 'p> {
    replica: &'a mut Replica<'p>,
    arm: &'a mut dyn Arm,
    capture: &'a CaptureCtx<'a>,
    /// Keep showing the arm hops past both commits, to the cut's last.
    observe_all: bool,
}

/// One capture at one cut: the Gatekeeper's verdicts on every hop, then the hops
/// in order to the first commit. With `beside`, the replica and the engine run on
/// the same frames to the later of their commits, or to the last hop when the
/// arm observes all.
fn replay_cut(
    cap: &Capture,
    cut: usize,
    profiles: &[KeyProfile; 88],
    front: &mut Front,
    beside: Option<Beside>,
) -> Run {
    let audio = &cap.audio[cut..];
    let verdicts = gate_verdicts(audio, cap.noise_floor);
    let mut engine = Engine::new(SAMPLE_RATE);
    engine.noise_floor = cap.noise_floor;

    let Some(Beside {
        replica,
        arm,
        capture,
        observe_all,
    }) = beside
    else {
        for (h, gate) in verdicts.iter().enumerate() {
            let start = h * HOP_SIZE;
            front.load(&audio[start..start + BASS_WINDOW_SIZE]);
            let is_silence = gate.state == SignalState::Silence;
            // `Engine::process` reads the spectrum only on a hop it scans.
            if !is_silence && !gate.is_transient_bypass {
                front.bass_spectrum();
            }
            engine.process(
                &front.frame,
                profiles,
                is_silence,
                gate.is_new_onset,
                gate.is_transient_bypass,
                None,
            );
            if let Some(key) = engine.identified_key {
                return Run {
                    lock: Some(Commit { key, hop: h }),
                    engine: None,
                };
            }
        }
        return Run {
            lock: None,
            engine: None,
        };
    };

    replica.window.clear();
    let hz_per_bin = SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32;
    let mut lock = None;
    let mut engine_lock = None;
    for (h, gate) in verdicts.iter().enumerate() {
        let start = h * HOP_SIZE;
        front.load(&audio[start..start + BASS_WINDOW_SIZE]);
        front.bass_spectrum();
        let is_silence = gate.state == SignalState::Silence;
        // A recording with no strike counts from its first sample.
        let since = samples_since(cap.onset.unwrap_or(0), cut, h);
        let hop = HopCtx {
            hop: since.div_euclid(HOP_SIZE as i64),
            t_since: since as f32 / SAMPLE_RATE as f32,
            bucket: 0,
            is_silence,
            block: if is_silence {
                Block::Build
            } else {
                Block::Note
            },
            audio: &front.frame.audio_buffer[..BASS_WINDOW_SIZE],
            spectrum: &front.frame.bass_frequency_buffer[..BINS + 1],
            mag: &front.frame.bass_magnitude_buffer[..BINS],
            hz_per_bin,
            sigma: cap.noise_floor,
            truth_level: None,
            oracle: &[],
            capture,
        };
        arm.observe(&hop);
        if engine_lock.is_none() {
            engine.process(
                &front.frame,
                profiles,
                is_silence,
                gate.is_new_onset,
                gate.is_transient_bypass,
                None,
            );
            engine_lock = engine.identified_key.map(|key| Commit { key, hop: h });
        }
        if lock.is_none() {
            lock = replica
                .hop(&hop, gate, arm)
                .map(|key| Commit { key, hop: h });
        }
        if lock.is_some() && engine_lock.is_some() && !observe_all {
            break;
        }
    }
    Run {
        lock,
        engine: Some(engine_lock),
    }
}

/// Mean, population σ, minimum and maximum of per-cut counts: the cuts are the
/// whole grid, not a sample of it.
fn spread(counts: &[usize]) -> (f64, f64, usize, usize) {
    let n = counts.len() as f64;
    let mean = counts.iter().sum::<usize>() as f64 / n;
    let var = counts
        .iter()
        .map(|&c| (c as f64 - mean).powi(2))
        .sum::<f64>()
        / n;
    let lo = counts.iter().copied().min().unwrap_or(0);
    let hi = counts.iter().copied().max().unwrap_or(0);
    (mean, var.sqrt(), lo, hi)
}

/// The median, the mean of the middle two for an even count; `NaN` for none.
fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        0.5 * (values[mid - 1] + values[mid])
    }
}

/// Counts by outcome at cut `p`, overall and then by register.
fn cut_counts(caps: &[Capture], runs: &[Vec<Run>], p: usize) -> [[usize; 3]; 4] {
    let mut n = [[0usize; 3]; 4];
    for (cap, r) in caps.iter().zip(runs) {
        let o = Outcome::of(r[p].lock, cap.key) as usize;
        n[0][o] += 1;
        n[1 + register(cap.key)][o] += 1;
    }
    n
}

/// The run's first line: the set, the cuts and the lock under test.
fn print_header(root: &Path, caps: &[Capture], left_out: usize, cuts: &[usize], opts: &Options) {
    let cut_list = if cuts.len() <= 4 {
        cuts.iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    } else {
        format!("{}, {}, …, {}", cuts[0], cuts[1], cuts[cuts.len() - 1])
    };
    let lock = match opts.peak_arm {
        None => "lock: Engine::process in auto mode".to_string(),
        Some(name) => format!(
            "lock: the replica, arm {name} in place of the peak floor, beside Engine::process; {}; partial spacing {}",
            if opts.carry {
                "one arm per cut, carried across captures in recording order"
            } else {
                "a fresh arm per capture and cut"
            },
            if opts.known_key {
                "the key's f0"
            } else {
                "unknown (NaN)"
            }
        ),
    };
    println!(
        "engine replay | base={} | {} captures{} | {} cut{} of the hop grid at {} samples | {lock}",
        root.display(),
        caps.len(),
        if left_out > 0 {
            format!(" ({left_out} without a window at every cut, left out)")
        } else {
            String::new()
        },
        cuts.len(),
        if cuts.len() == 1 { "" } else { "s" },
        cut_list
    );
}

/// The column heads of the per-cut table; `beside` adds the agreement with the
/// engine.
fn print_cut_header(beside: bool) {
    println!(
        "\n{:>5}  {:<14} {:<14} {:<14} {:<14}{}{:>8}",
        "cut",
        "all p/nl/w",
        "bass p/nl/w",
        "mid p/nl/w",
        "treble p/nl/w",
        if beside { "  = engine" } else { "" },
        "s"
    );
}

/// One cut's pass / no-lock / wrong counts, overall and by register, how many
/// captures the replica and the engine agree on, and the cut's seconds.
fn print_cut_row(caps: &[Capture], runs: &[Vec<Run>], p: usize, cut: usize, took: Duration) {
    let n = cut_counts(caps, runs, p);
    let cell = |g: usize| format!("{}/{}/{}", n[g][0], n[g][1], n[g][2]);
    let agree = runs
        .iter()
        .filter_map(|r| r[p].engine.map(|e| e == r[p].lock))
        .collect::<Vec<_>>();
    let agree = if agree.is_empty() {
        String::new()
    } else {
        format!(
            "  {:>8}",
            format!("{}/{}", agree.iter().filter(|&&a| a).count(), agree.len())
        )
    };
    println!(
        "{cut:>5}  {:<14} {:<14} {:<14} {:<14}{agree}{:>8.2}",
        cell(0),
        cell(1),
        cell(2),
        cell(3),
        took.as_secs_f64()
    );
}

/// Report 0020's yardstick over the cuts, and the captures the grid decides.
fn print_yardstick(caps: &[Capture], runs: &[Vec<Run>]) {
    let cuts = runs[0].len();
    let per_cut: Vec<[[usize; 3]; 4]> = (0..cuts).map(|p| cut_counts(caps, runs, p)).collect();
    println!(
        "\nThe yardstick over {cuts} cut{}: min–max (mean ± population σ)",
        if cuts == 1 { "" } else { "s" }
    );
    println!(
        "  {:<16} {:>4}   {:<26} {:<26} wrong",
        "group", "n", "pass", "no-lock"
    );
    for g in 0..4 {
        let label = if g == 0 {
            "all"
        } else {
            REGISTER_LABELS[g - 1]
        };
        let column = |o: Outcome| {
            let counts: Vec<usize> = per_cut.iter().map(|n| n[g][o as usize]).collect();
            let (mean, sd, lo, hi) = spread(&counts);
            format!("{lo}–{hi} ({mean:.2} ± {sd:.2})")
        };
        println!(
            "  {label:<16} {:>4}   {:<26} {:<26} {}",
            per_cut[0][g].iter().sum::<usize>(),
            column(Outcome::Pass),
            column(Outcome::NoLock),
            column(Outcome::Wrong)
        );
    }

    let (mut every, mut none) = (0, 0);
    let mut decided = [0usize; 3];
    let mut decided_high = 0;
    for (cap, r) in caps.iter().zip(runs) {
        let passes = r
            .iter()
            .filter(|run| Outcome::of(run.lock, cap.key) == Outcome::Pass)
            .count();
        if passes == r.len() {
            every += 1;
        } else if passes == 0 {
            none += 1;
        } else {
            decided[register(cap.key)] += 1;
            decided_high += usize::from(cap.key >= TREBLE_HIGH_FROM);
        }
    }
    println!(
        "Captures passing at every cut {every}, at none {none}; the grid decides {} (bass {}, mid {}, treble {}; keys {TREBLE_HIGH_FROM}–87 {decided_high})",
        decided.iter().sum::<usize>(),
        decided[0],
        decided[1],
        decided[2]
    );
}

/// The median latency per register on report 0020's clock, over every commit and
/// over the passes.
fn print_latency(caps: &[Capture], runs: &[Vec<Run>], cuts: &[usize]) {
    println!("\nLatency on report 0020's clock, ms from the strike to the commit's window end");
    println!(
        "  {:<16} {:>8} {:>9} {:>8} {:>9}",
        "register", "commits", "median", "passes", "median"
    );
    for g in 0..4 {
        let (mut all, mut pass) = (Vec::new(), Vec::new());
        for (cap, r) in caps.iter().zip(runs) {
            if g > 0 && register(cap.key) != g - 1 {
                continue;
            }
            for (run, &cut) in r.iter().zip(cuts) {
                let Some(c) = run.lock else {
                    continue;
                };
                if let Some(ms) = latency_ms(cap, cut, c) {
                    all.push(ms);
                    if c.key == cap.key {
                        pass.push(ms);
                    }
                }
            }
        }
        let label = if g == 0 {
            "all"
        } else {
            REGISTER_LABELS[g - 1]
        };
        println!(
            "  {label:<16} {:>8} {:>9.2} {:>8} {:>9.2}",
            all.len(),
            median(&mut all),
            pass.len(),
            median(&mut pass)
        );
    }
}

/// Cut 0 in `engine lock`'s own lines, under `title`.
fn print_lock_form(title: &str, caps: &[Capture], runs: &[Vec<Run>]) {
    let n = cut_counts(caps, runs, 0);
    let total = |g: usize| n[g].iter().sum::<usize>();
    println!(
        "\nCut 0 in `engine lock`'s form:\n{title}: {}/{}   bass {}/{}  mid {}/{}  treble {}/{}",
        n[0][0],
        total(0),
        n[1][0],
        total(1),
        n[2][0],
        total(2),
        n[3][0],
        total(3)
    );
    let fails: Vec<String> = caps
        .iter()
        .zip(runs)
        .filter_map(|(cap, r)| match r[0].lock {
            Some(c) if c.key == cap.key => None,
            Some(c) => Some(format!("{} -> locked {}", cap.name, c.key)),
            None => Some(format!("{} -> never locked", cap.name)),
        })
        .collect();
    if !fails.is_empty() {
        println!("FAILURES ({}):", fails.len());
        for f in &fails {
            println!("  {f}");
        }
    }
}

/// What the arm answered on the hops the replica scanned.
fn print_tally(name: &str, tally: &Tally) {
    let mut cleared: Vec<f64> = tally.cleared.iter().map(|&c| c as f64).collect();
    let answered = cleared.len();
    let mid = median(&mut cleared);
    let at = |q: f64| {
        cleared
            .get(((answered.max(1) - 1) as f64 * q).round() as usize)
            .copied()
            .unwrap_or(f64::NAN)
    };
    let pct = |k: u64, n: u64| {
        if n == 0 {
            f64::NAN
        } else {
            100.0 * k as f64 / n as f64
        }
    };
    println!(
        "\nArm {name} on the {} hops the replica scanned: declined {} ({:.1} %); maxima at or above its threshold per answered hop: median {}, 5–95 % {}–{}; more than {MAX_MAXIMA}, so only the first {MAX_MAXIMA} in bin order kept, on {} ({:.1} %)",
        tally.hops,
        tally.declined,
        pct(tally.declined, tally.hops),
        mid,
        at(0.05),
        at(0.95),
        tally.capped,
        pct(tally.capped, answered as u64)
    );
}

/// The replica against the engine run beside it: agreement, outcome against
/// outcome, and every capture they disagree on.
fn print_comparison(name: &str, caps: &[Capture], runs: &[Vec<Run>], cuts: &[usize]) {
    let mut matrix = [[0usize; 3]; 3];
    let (mut agree, mut total) = (0, 0);
    for (cap, r) in caps.iter().zip(runs) {
        for run in r {
            let Some(engine) = run.engine else {
                continue;
            };
            total += 1;
            agree += usize::from(engine == run.lock);
            matrix[Outcome::of(engine, cap.key) as usize]
                [Outcome::of(run.lock, cap.key) as usize] += 1;
        }
    }
    println!(
        "\nArm {name}'s replica against Engine::process on the same hops: {agree} of {total} (capture, cut) pairs commit the same key on the same hop"
    );
    println!(
        "  {:<16} {:>9} {:>9} {:>9}",
        "engine \\ arm", "pass", "no-lock", "wrong"
    );
    for o in Outcome::ALL {
        let row = matrix[o as usize];
        println!(
            "  {:<16} {:>9} {:>9} {:>9}",
            format!("engine {}", o.label()),
            row[0],
            row[1],
            row[2]
        );
    }
    let describe = |lock: Option<Commit>| match lock {
        Some(c) => format!("locked {} at hop {}", c.key, c.hop),
        None => "never locked".to_string(),
    };
    for (cap, r) in caps.iter().zip(runs) {
        let differ: Vec<usize> = r
            .iter()
            .enumerate()
            .filter(|(_, run)| run.engine.is_some_and(|e| e != run.lock))
            .map(|(p, _)| p)
            .collect();
        let Some(&first) = differ.first() else {
            continue;
        };
        let run = &r[first];
        println!(
            "  {} (key {}): {} of {} cuts; at cut {}: engine {}, arm {}",
            cap.name,
            cap.key,
            differ.len(),
            r.len(),
            cuts[first],
            describe(run.engine.flatten()),
            describe(run.lock)
        );
    }
}

/// One row per (capture, cut): the directory, its key and register, the cut in
/// samples, the outcome, the committed key, the commit's hop on the cut's grid and
/// its latency in ms on report 0020's clock.
fn write_csv(path: &Path, caps: &[Capture], cuts: &[usize], runs: &[Vec<Run>]) -> Result<()> {
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        fs::create_dir_all(dir).with_context(|| format!("create {}", dir.display()))?;
    }
    let file = File::create(path).with_context(|| format!("create {}", path.display()))?;
    let mut w = BufWriter::new(file);
    writeln!(
        w,
        "dir,key,register,phase_samples,outcome,lock_key,commit_hop,latency_ms"
    )?;
    for (cap, r) in caps.iter().zip(runs) {
        for (run, &cut) in r.iter().zip(cuts) {
            let (lock_key, hop, latency) = match run.lock {
                Some(c) => (
                    c.key.to_string(),
                    c.hop.to_string(),
                    latency_ms(cap, cut, c)
                        .map(|ms| format!("{ms:.2}"))
                        .unwrap_or_default(),
                ),
                None => (String::new(), String::new(), String::new()),
            };
            writeln!(
                w,
                "{},{},{},{},{},{},{},{}",
                cap.name,
                cap.key,
                REGISTERS[register(cap.key)],
                cut,
                Outcome::of(run.lock, cap.key).label(),
                lock_key,
                hop,
                latency
            )?;
        }
    }
    w.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;
    use crate::truth::synth_tone;
    use tuner_core::models::get_expected_beta;

    /// Uniform noise of standard deviation `sigma` from a xorshift generator.
    fn noise(len: usize, sigma: f32, seed: u64) -> Vec<f32> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let u = (state >> 11) as f32 / (1u64 << 53) as f32 - 0.5;
                u * sigma * 12f32.sqrt()
            })
            .collect()
    }

    /// The reference M-of-N rule `Engine`'s own tests hold `record_winner` to, a
    /// port of `eval_rule` in `replay_lock_rules.py`: the key and index of the
    /// first lock.
    fn ref_eval(seq: &[u8], m: usize, n: usize) -> Option<(u8, usize)> {
        let mut win: VecDeque<u8> = VecDeque::with_capacity(n);
        for (t, &w) in seq.iter().enumerate() {
            if win.len() == n {
                win.pop_front();
            }
            win.push_back(w);
            if win.iter().filter(|&&k| k == w).count() >= m {
                return Some((w, t));
            }
        }
        None
    }

    #[test]
    fn lock_window_matches_the_reference_rule() {
        let seqs: &[&[u8]] = &[
            &[],
            &[5],
            &[5, 5, 5, 5, 5, 5, 5],
            &[5, 5, 5, 5, 5, 5],
            &[1, 2, 1, 2, 1, 5, 5, 5, 5, 5, 5, 5],
            &[0, 1, 0, 1, 0, 1, 0, 1, 0, 1],
            &[5, 5, 5, 5, 5, 5, 9, 5],
            &[9, 5, 5, 5, 5, 5, 5, 5],
            &[5, 5, 5, 9, 5, 5, 5, 5, 5],
            &[3, 3, 3, 3, 4, 4, 4, 4, 4, 4, 4],
        ];
        for seq in seqs {
            let mut window = LockWindow::new();
            let got = seq
                .iter()
                .enumerate()
                .find(|&(_, &k)| window.record(k))
                .map(|(t, &k)| (k, t));
            assert_eq!(got, ref_eval(seq, LOCK_VOTES_M, LOCK_WINDOW_N), "{seq:?}");
        }
        // A clear forgets every vote: six, a clear, then seven more to commit.
        let mut window = LockWindow::new();
        for _ in 0..LOCK_VOTES_M - 1 {
            assert!(!window.record(5));
        }
        window.clear();
        for _ in 0..LOCK_VOTES_M - 1 {
            assert!(!window.record(5));
        }
        assert!(window.record(5));
    }

    /// With one threshold at every bin the per-bin extraction writes what
    /// `peaks::extract_peaks` writes, bit for bit, including where more than
    /// [`MAX_MAXIMA`] maxima clear it and the cap in bin order binds.
    #[test]
    fn per_bin_extraction_is_extract_peaks_at_a_scalar() {
        let n = BASS_WINDOW_SIZE;
        let mut signal = noise(n, 1.0e-3, 0x9E37_79B9_7F4A_7C15);
        for (i, s) in signal.iter_mut().enumerate() {
            let t = i as f32 / SAMPLE_RATE as f32;
            for (k, f) in [55.0f32, 196.0, 261.6, 523.3, 1046.5, 2093.0, 4186.0, 7040.0]
                .iter()
                .enumerate()
            {
                *s += 0.02 / (k as f32 + 1.0) * (std::f32::consts::TAU * f * t).sin();
            }
        }
        let plan = RealFftPlanner::<f32>::new().plan_fft_forward(n);
        let mut time = vec![0.0f32; n];
        let mut spectrum = vec![Complex::new(0.0f32, 0.0); n / 2 + 1];
        let mut mag = vec![0.0f32; BINS];
        spectral::fft(&signal, &mut time, &mut spectrum, &plan, n);
        spectral::magnitude_spectrum(&spectrum, n, &mut mag);

        let mut sorted = mag.clone();
        sorted.sort_by(f32::total_cmp);
        let median = sorted[sorted.len() / 2];
        let mut maxima = Vec::new();
        let mut capped = false;
        for t in [0.5 * median, 2.0 * median, 6.0 * median, 40.0 * median] {
            assert!(mag.iter().all(|&m| m != t), "a magnitude sits on {t}");
            let thresholds = vec![t; BINS];
            let mut ours = vec![SpectralPeak::default(); MAX_PEAKS];
            let (count, cleared) = extract_peaks_per_bin(
                &mag,
                &spectrum,
                &thresholds,
                |_| true,
                &mut maxima,
                &mut ours,
            );
            let mut shipped = vec![SpectralPeak::default(); MAX_PEAKS];
            let expected = peaks::extract_peaks(&mag, &spectrum, SAMPLE_RATE, n, t, &mut shipped);
            assert_eq!(count, expected, "count at {t}");
            for (a, b) in ours[..count].iter().zip(&shipped[..expected]) {
                assert_eq!(
                    a.frequency.to_bits(),
                    b.frequency.to_bits(),
                    "frequency at {t}"
                );
                assert_eq!(
                    a.magnitude.to_bits(),
                    b.magnitude.to_bits(),
                    "magnitude at {t}"
                );
            }
            capped |= cleared > MAX_MAXIMA;
        }
        assert!(
            capped,
            "no threshold let more than {MAX_MAXIMA} maxima through"
        );
    }

    /// Report 0020's own per-phase pass counts give its yardstick: the shipped
    /// rule 564–571, mean 567.19, σ 2.04; the vote from the onset 564–571, mean
    /// 566.94, σ 2.16.
    #[test]
    fn spread_reproduces_report_0020s_yardstick() {
        let shipped = [
            568, 570, 568, 569, 565, 564, 565, 564, 566, 566, 567, 569, 567, 567, 569, 571,
        ];
        let (mean, sd, lo, hi) = spread(&shipped);
        assert_eq!((lo, hi), (564, 571));
        assert_eq!(format!("{mean:.2} {sd:.2}"), "567.19 2.04");
        let onset = [
            568, 570, 568, 569, 565, 564, 565, 564, 565, 565, 566, 569, 566, 567, 569, 571,
        ];
        let (mean, sd, lo, hi) = spread(&onset);
        assert_eq!((lo, hi), (564, 571));
        assert_eq!(format!("{mean:.2} {sd:.2}"), "566.94 2.16");
    }

    #[test]
    fn cuts_spread_over_one_hop() {
        assert_eq!(cuts(1), vec![0]);
        assert_eq!(cuts(16), (0..16).map(|k| 64 * k).collect::<Vec<_>>());
        assert_eq!(cuts(3), vec![0, 341, 682]);
    }

    /// Report 0020's clock: the strike is the first of the first 30 hops past a
    /// tenth of the loudest hop's RMS, and a commit's latency is its window end on
    /// the original recording minus the strike, whatever the cut.
    #[test]
    fn the_clock_is_report_0020s() {
        let mut audio = vec![0.0f32; 40 * HOP_SIZE];
        audio[14 * HOP_SIZE..].fill(0.5);
        audio[13 * HOP_SIZE + 1000] = 0.5; // RMS 0.0156, under a tenth of 0.5
        assert_eq!(strike_onset(&audio), Some(14 * HOP_SIZE));
        let mut late = vec![0.0f32; 40 * HOP_SIZE];
        late[31 * HOP_SIZE..].fill(0.5);
        assert_eq!(strike_onset(&late), None);

        // A commit on hop 17 at cut 0 ends its window 11 hops after a strike on
        // hop 14: 11 264 samples, 255.42 ms.
        let cap = Capture {
            name: String::new(),
            key: 0,
            noise_floor: DEFAULT_NOISE_FLOOR,
            audio: Vec::new(),
            onset: Some(14 * HOP_SIZE),
        };
        let ms = latency_ms(&cap, 0, Commit { key: 0, hop: 17 }).unwrap();
        assert_eq!(format!("{ms:.2}"), "255.42");
        let moved = latency_ms(&cap, 64, Commit { key: 0, hop: 17 }).unwrap();
        assert!((moved - ms - 64.0 / 44.1).abs() < 1e-9, "{moved}");
        let none = Capture { onset: None, ..cap };
        assert_eq!(latency_ms(&none, 0, Commit { key: 0, hop: 17 }), None);
    }

    #[test]
    fn registers_split_as_engine_lock_splits() {
        assert_eq!([0u8, 26, 27, 59, 60, 87].map(register), [0, 0, 1, 1, 2, 2]);
    }

    /// A synthetic C4 after a quiet pre-roll: the replica under arm A commits the
    /// key the engine commits, on the hop it commits it, at several cuts, and arm
    /// N's ungated picker still names the note.
    #[test]
    fn replica_under_arm_a_is_the_engine_on_a_synthetic_note() {
        let key = 39u8;
        let f0 = NOTES[key as usize].frequency;
        let preroll = (0.35 * SAMPLE_RATE as f32) as usize;
        let len = (1.5 * SAMPLE_RATE as f32) as usize;
        let amps: Vec<f32> = (1..=12).map(|n| 1.0 / n as f32).collect();
        let note = synth_tone(f0, get_expected_beta(key), &amps, len - preroll, 1.5);
        let mut audio = noise(len, 1.0e-4, 0x5EED_0039);
        for (a, s) in audio[preroll..].iter_mut().zip(&note) {
            *a += s;
        }
        let cap = Capture {
            name: "synthetic".to_string(),
            key,
            noise_floor: 1.0e-3,
            onset: strike_onset(&audio),
            audio,
        };
        let profiles: [KeyProfile; 88] = core::array::from_fn(|i| {
            KeyProfile::new(NOTES[i].frequency, get_expected_beta(i as u8))
        });
        let set = SetCtx {
            label: "synthetic",
            p_fa: PEAK_FLOOR_P_FA,
            sigma_override: None,
            nu_half_per_frame: None,
        };
        let ctx = CaptureCtx {
            dir: "synthetic",
            key,
            register: ambient::register_index(key),
            ns: &[],
            f_meas: &[],
            spacing_hz: f32::NAN,
            b: get_expected_beta(key),
            sigma_rec: cap.noise_floor,
        };
        let mut front = Front::new();
        let mut replica = Replica::new(&profiles);
        for cut in [0, 320, 960] {
            let alone = replay_cut(&cap, cut, &profiles, &mut front, None);
            let engine = alone
                .lock
                .expect("the engine commits on the synthetic note");
            assert_eq!(engine.key, key, "cut {cut}");
            for (name, must_match) in [("A", true), ("N", false)] {
                let mut arm = prepared_arm(name, &set).expect("a known arm");
                arm.begin_capture(&ctx);
                let run = replay_cut(
                    &cap,
                    cut,
                    &profiles,
                    &mut front,
                    Some(Beside {
                        replica: &mut replica,
                        arm: arm.as_mut(),
                        capture: &ctx,
                        observe_all: false,
                    }),
                );
                assert_eq!(run.engine, Some(Some(engine)), "arm {name}, cut {cut}");
                if must_match {
                    assert_eq!(run.lock, Some(engine), "arm {name}, cut {cut}");
                } else {
                    assert_eq!(run.lock.map(|c| c.key), Some(key), "arm {name}, cut {cut}");
                }
            }
        }
        assert_eq!(replica.tally.declined, 0);
    }
}
