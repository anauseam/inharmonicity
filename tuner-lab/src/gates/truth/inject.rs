//! T2: known partials in real noise. Each synthetic capture mirrors one real
//! capture of a `mat regen` dump: its located partials are re-synthesised from
//! that capture's own measured series and added to a bed of real noise, and the
//! bed is written beside the capture, so the noise at every bin of every hop is
//! known exactly.
//!
//! Two beds. `room` (T2a) is the set's pre-roll as the Gatekeeper screens it —
//! the mirrored capture's own first, then its neighbours' in recording order —
//! tiled with crossfades, with a strike burst added at the onset. `residual`
//! (T2b) is the mirrored capture's own audio less a sinusoidal model of its
//! located partials, the deterministic-plus-stochastic split of Serra & Smith
//! (1990), so the bed keeps what silence lacks: the hammer, decay irregularity,
//! sympathetic strings.
//!
//! Every parameter comes from the record: the partials' frequencies and levels
//! from the mirrored row, their two-stage decays from the extended store's long
//! records (fitted here), the unison splits from the isolation set's as-found
//! strings. The fidelity tables compare the synthetic set with the real one on
//! the quantities the estimators see.
//!
//! # References
//! Serra, X. & Smith, J. O. (1990). "Spectral Modeling Synthesis: A Sound
//! Analysis/Synthesis System Based on a Deterministic plus Stochastic
//! Decomposition." Computer Music Journal 14(4), 12–24.
//!
//! McAulay, R. J. & Quatieri, T. F. (1986). "Speech Analysis/Synthesis Based on
//! a Sinusoidal Representation." IEEE Trans. ASSP 34(4), 744–754.

use std::collections::HashMap;
use std::f64::consts::{FRAC_PI_2, PI, TAU};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use anyhow::{Context, Result, bail};
use realfft::RealFftPlanner;
use rustfft::num_complex::Complex;
use serde_json::{Value, json};

use super::Gaussian;
use crate::gatekeeper::replay::{self as gate_replay, Overrides, Thresholds};
use crate::gates::ambient::{self, MIN_GAP_BINS, REGISTERS};
use crate::gates::arms::{self, LOBE_HALF_BINS};
use crate::raw;
use crate::regen::{self, Capture};
use crate::truth::{SAMPLE_RATE, hann_vec};
use tuner_core::algorithms::mat::MAX_PARTIALS;
use tuner_core::algorithms::spectral;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE};
use tuner_core::gatekeeper::SignalState;
use tuner_core::models::{NOTES, get_expected_beta};
use tuner_core::pipeline::{CAPTURE_ANALYSIS_SAMPLES, CAPTURE_DEFAULT_SAMPLES};
use tuner_core::strobe::MAX_STROBE_REFS;

/// Hops of bed before the onset unless `--preroll-hops` says otherwise: the
/// app's diagnostic pre-roll, about 348 ms.
const PREROLL_HOPS: usize = 15;

/// Goertzel window of the decay tracks and of the residual's model: the long
/// window, whose main lobe (±21.5 Hz) stays inside A0's 27.5 Hz partial spacing.
const TRACK_WIN: usize = 4096;

/// Seam between two room segments, 2048 samples (46.4 ms). The power ramps are
/// a raised-cosine pair, so the gains' squares sum to one and two uncorrelated
/// segments keep their mean power across the seam.
const CROSSFADE: usize = 2048;

/// The span the knee of a two-stage decay is swept over, s.
const KNEE_RANGE_S: (f64, f64) = (0.1, 1.0);

/// Fewest hops a side of a knee: the fewest that over-determine a slope.
const MIN_SEGMENT_HOPS: usize = 3;

/// Hops in the running median a track is cut on, so that one beating dip under
/// the room floor does not end a partial still above it.
const CUT_MEDIAN_HOPS: usize = 5;

/// Fewest fits in a (register, partial) cell for its quartiles to be drawn
/// from; a sparser cell takes the nearest partial number's.
const MIN_CELL_FITS: usize = 4;

/// Records at least this long are the extended store's long ones: 5 s.
const LONG_RECORD_SAMPLES: usize = 5 * SAMPLE_RATE as usize;

/// As-found unison splits at the fundamental, ¢ (report 0014 §2): 0.88–3.67 ¢
/// on C2, F2, C3, D3 and A#3, 9.2–18.5 ¢ on A4, C5 and C6. A capture draws one
/// split, uniformly in its key's range.
const SPLIT_BELOW_A4_CENTS: (f32, f32) = (0.88, 3.67);
const SPLIT_FROM_A4_CENTS: (f32, f32) = (9.2, 18.5);
const A4: u8 = 48;

/// Keys 0–11 carry one string (report 0013), keys to D3 two and the rest three
/// (`capture-sets.md`: D3 is a bichord, A#3 a trichord).
const LAST_SINGLE_KEY: u8 = 11;
const LAST_BICHORD_KEY: u8 = 29;

/// The strike: white noise added at the onset under a two-term envelope, each
/// term a level over the bed's RMS (dB) and an amplitude time constant (s),
/// the whole scaled per register by [`STRIKE_GAIN_DB`]. The fidelity table
/// reads its first-bucket excess against the real captures' and report 0015
/// §8's 2–5×.
const STRIKE: [(f64, f64); 2] = [(20.0, 0.020), (5.0, 0.150)];

/// Gain on the strike per register (bass, tenor, treble, high 76–87), dB, so
/// that the synthetic first-bucket excess matches the mirrored set's; the
/// two-term model at 0 dB is a broadband burst, the real hammer's excess falls
/// from bass to treble. `--strike-gain-db` overrides it.
// Drawn once from the 264-capture piano-2 mirror: with E the real excess
// (8.02, 4.04, 2.32, 1.57), E₀ the synthetic's with the strike off (2.91, 1.20,
// 1.04, 1.02) and E₁ at 0 dB (3.89, 6.25, 7.09, 6.75), g² = (E² − E₀²)/(E₁² −
// E₀²), the strike's power adding to the leakage's. report 0023
const STRIKE_GAIN_DB: [f64; 4] = [9.23, -4.03, -10.58, -14.95];

/// Partials declared absent past the last located one: T5(iii)'s population.
const ABSENT_PARTIALS: usize = 4;

/// The time bucket the strike is read in, 0.1–0.25 s: report 0015 §8's first.
const STRIKE_BUCKET: usize = 2;

/// Highest partial number the amplitude profile is tabled for.
const PROFILE_MAX_N: usize = 8;

/// Report 0009 Analysis 7: the median level of partial n relative to partial 1
/// on instrument 2, dB, for n = 2…6, in the bass and tenor (keys 0–47) and in
/// the treble (keys 62–87).
const A7_BASS_TENOR: [&str; 5] = ["+8.2", "+5.2", "+3.9", "—", "+2.4"];
const A7_TREBLE: [&str; 5] = ["−28.3", "−40.7", "−49.8", "−53.0", "−56 to −60"];

/// The bands the amplitude profile is tabled over: the four registers, then
/// report 0009 Analysis 7's two.
const BANDS: [&str; 6] = [
    "bass",
    "tenor",
    "treble",
    "high 76–87",
    "keys 0–47",
    "keys 62–87",
];

/// The noise the partials are added to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Bed {
    /// T2a: the set's pre-roll as the Gatekeeper screens it, tiled.
    Room,
    /// T2b: each capture less a sinusoidal model of its located partials.
    Residual,
}

impl Bed {
    fn kind(self) -> &'static str {
        match self {
            Bed::Room => "t2a",
            Bed::Residual => "t2b",
        }
    }
}

/// `gates truth t2`'s arguments.
#[derive(clap::Args)]
pub struct Args {
    /// A `mat regen` dump of the real set whose captures are mirrored.
    pub regen: PathBuf,
    /// That set's dump directory.
    pub root: PathBuf,
    /// Directory to write; refused if it exists.
    pub out: PathBuf,
    /// The noise bed: the set's screened pre-roll, or each capture less a
    /// sinusoidal model of its partials.
    #[arg(long, value_enum, default_value_t = Bed::Room)]
    pub bed: Bed,
    /// The extended store's dump directory, whose 5 s records give the
    /// two-stage decays.
    #[arg(long, requires = "long_regen")]
    pub long_root: Option<PathBuf>,
    /// The extended store's `mat regen` dump.
    #[arg(long, requires = "long_root")]
    pub long_regen: Option<PathBuf>,
    /// At most this many captures of each key, the earliest first.
    #[arg(long)]
    pub per_key: Option<usize>,
    /// Hops of bed before the onset.
    #[arg(long, default_value_t = PREROLL_HOPS)]
    pub preroll_hops: usize,
    /// Seed of the generator.
    #[arg(long, default_value_t = 20261009)]
    pub seed: u64,
    /// Only these keys, e.g. `0,12,24`.
    #[arg(long, value_delimiter = ',')]
    pub keys: Option<Vec<u8>>,
    /// Gain on the strike per register, dB, four values (bass, tenor, treble,
    /// high); `-inf` turns it off.
    #[arg(long, num_args = 4, allow_hyphen_values = true, value_name = "DB")]
    pub strike_gain_db: Option<Vec<f64>>,
}

/// Writes the T2 set and its fidelity tables.
pub fn run(a: Args) -> Result<()> {
    let clock = Instant::now();
    if a.out.exists() {
        bail!(
            "{} exists; T2 is written to a fresh directory",
            a.out.display()
        );
    }
    if a.preroll_hops * HOP_SIZE < BASS_WINDOW_SIZE + 4 * HOP_SIZE {
        bail!(
            "--preroll-hops must cover one 8192-sample window and the four-hop guard: at least 12"
        );
    }
    if a.per_key == Some(0) {
        bail!("--per-key must be at least 1");
    }
    let (mirrors, dropped) = load_mirrors(&a.regen)?;
    let chosen = choose(&mirrors, a.keys.as_deref(), a.per_key);
    if chosen.is_empty() {
        bail!("no capture of {} matches the selection", a.regen.display());
    }
    let decays = match (&a.long_root, &a.long_regen) {
        (Some(root), Some(regen)) => DecaySource::Long(DecayTable::fit(root, regen)?),
        _ => DecaySource::Own,
    };
    if let DecaySource::Long(table) = &decays {
        println!("{}", table.summary());
        print!("{}", table.text());
    } else {
        println!(
            "T2: no --long-root / --long-regen, so each partial decays at one slope fitted on the mirrored capture's own 1.5 s record"
        );
    }
    let strike_gain_db: [f64; 4] = match &a.strike_gain_db {
        Some(v) => v.as_slice().try_into().expect("clap takes four values"),
        None => STRIKE_GAIN_DB,
    };
    let pool = match a.bed {
        Bed::Room => room_pool(&mirrors, &a.root),
        Bed::Residual => Vec::new(),
    };
    if a.bed == Bed::Room && pool.is_empty() {
        bail!(
            "no capture of {} has a screened pre-roll to build the room from",
            a.root.display()
        );
    }
    fs::create_dir_all(&a.out).with_context(|| format!("create {}", a.out.display()))?;
    let ctx = Ctx {
        root: &a.root,
        out: &a.out,
        bed: a.bed,
        preroll_hops: a.preroll_hops,
        seed: a.seed,
        decays: &decays,
        pool: &pool,
        strike_gain_db,
    };
    let outcomes = par_map(&chosen, |m| make(m, &ctx));
    let mut made = Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    for (m, outcome) in chosen.iter().zip(outcomes) {
        match outcome? {
            Outcome::Made(x) => made.push(*x),
            Outcome::Skipped(why) => skipped.push((m.cap.dir.clone(), why)),
        }
    }
    let rows: Vec<&Value> = made.iter().map(|x| &x.row).collect();
    fs::write(
        a.out.join("truth.json"),
        serde_json::to_string_pretty(&rows)?,
    )?;

    let head = format!(
        "# T2 fidelity: {} ({} bed), {} captures mirrored from {} ({})\n\n\
         {} rows of the dump were dropped by the ±200 ¢ rule; {} chosen captures were skipped{}. \
         Seed {}; {} hops of bed before each onset; decays from {}; strike gain per register {:?} dB.",
        a.bed.kind(),
        match a.bed {
            Bed::Room => "room",
            Bed::Residual => "residual",
        },
        made.len(),
        a.regen.display(),
        a.root.display(),
        dropped,
        skipped.len(),
        if skipped.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                skipped
                    .iter()
                    .map(|(d, why)| format!("{d}: {why}"))
                    .collect::<Vec<_>>()
                    .join("; ")
            )
        },
        a.seed,
        a.preroll_hops,
        decays.label(),
        strike_gain_db
    );
    let text = fidelity_text(&made, &decays, a.bed, &head)?;
    fs::write(a.out.join("fidelity.md"), &text)?;
    fs::write(a.out.join("fidelity_decay.csv"), decay_csv(&decays, &made))?;
    if a.bed == Bed::Residual {
        fs::write(a.out.join("fidelity_residual.csv"), residual_csv(&made))?;
    }
    println!(
        "\nT2 ({}): {} captures written to {}; truth.json in the regen schema, fidelity.md, fidelity_decay.csv{}",
        a.bed.kind(),
        made.len(),
        a.out.display(),
        if a.bed == Bed::Residual {
            ", fidelity_residual.csv"
        } else {
            ""
        }
    );
    println!("\n{text}");
    println!("runtime {:.1} s", clock.elapsed().as_secs_f64());
    Ok(())
}

// ── the mirrored captures ───────────────────────────────────────────────────

/// One real capture to mirror: its regen row with every located partial.
struct Mirror {
    cap: Capture,
    measured_f0: f32,
    /// Every located partial, `(n, f_n, a_n)`, by number, up to MAT's 32.
    partials: Vec<(usize, f32, f32)>,
    serial: u64,
}

/// The plausible rows of a regen dump in recording order, each with its full
/// partial list (`regen::load` keeps only the first twelve), and how many the
/// ±200 ¢ rule dropped.
fn load_mirrors(path: &Path) -> Result<(Vec<Mirror>, usize)> {
    let (caps, dropped) = regen::load(path);
    let text = fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let rows: Vec<Value> =
        serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
    let by_dir: HashMap<&str, &Value> = rows
        .iter()
        .filter_map(|r| Some((r["source_dir"].as_str()?, r)))
        .collect();
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let mut out = Vec::with_capacity(caps.len());
    for cap in caps {
        let Some(row) = by_dir.get(cap.dir.as_str()) else {
            continue;
        };
        let mut partials: Vec<(usize, f32, f32)> = row["partials"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| {
                let n = p["number"].as_u64()? as usize;
                let f = p["frequency"].as_f64()? as f32;
                let a = p["amplitude"].as_f64()? as f32;
                ((1..=MAX_PARTIALS).contains(&n) && f > 0.0 && f < nyquist && a > 0.0)
                    .then_some((n, f, a))
            })
            .collect();
        partials.sort_by_key(|p| p.0);
        partials.dedup_by_key(|p| p.0);
        if partials.is_empty() || cap.f0 <= 0.0 {
            continue;
        }
        out.push(Mirror {
            measured_f0: row["measured_f0"].as_f64().map_or(cap.f0, |v| v as f32),
            serial: serial_of(&cap.dir),
            partials,
            cap,
        });
    }
    out.sort_by(|x, y| (x.serial, &x.cap.dir).cmp(&(y.serial, &y.cap.dir)));
    Ok((out, dropped))
}

/// The captures to mirror: those of the named keys, at most `per_key` of each,
/// the earliest first.
fn choose<'a>(
    mirrors: &'a [Mirror],
    keys: Option<&[u8]>,
    per_key: Option<usize>,
) -> Vec<&'a Mirror> {
    let mut taken: HashMap<u8, usize> = HashMap::new();
    mirrors
        .iter()
        .filter(|m| keys.is_none_or(|k| k.contains(&m.cap.key)))
        .filter(|m| {
            let count = taken.entry(m.cap.key).or_default();
            let keep = per_key.is_none_or(|p| *count < p);
            if keep {
                *count += 1;
            }
            keep
        })
        .collect()
}

// ── the screened pre-roll ───────────────────────────────────────────────────

/// A capture's full event, where its record begins in it, and the ends of the
/// 8192-sample windows before the record that the Gatekeeper calls silent at
/// the capture's own thresholds, four hops short of the record: the arms'
/// screen.
struct Screened {
    full: Vec<f32>,
    lead: usize,
    silent_ends: Vec<usize>,
}

impl Screened {
    /// `full`, whose record begins at `lead`, screened at the thresholds the
    /// capture in `dir` ran at.
    fn new(dir: &Path, full: Vec<f32>, lead: usize) -> Self {
        let thresholds = Thresholds::resolve(dir, &Overrides::default());
        let usable = lead.saturating_sub(4 * HOP_SIZE);
        let silent_ends = gate_replay::run(&full, thresholds.config)
            .iter()
            .filter(|h| h.result.state == SignalState::Silence)
            .map(|h| h.window_end())
            .filter(|&e| e >= BASS_WINDOW_SIZE && e <= usable)
            .collect();
        Self {
            full,
            lead,
            silent_ends,
        }
    }

    /// The capture in `dir`, whose record is `audio`, where its full event
    /// holds that record.
    fn of_capture(dir: &Path, audio: &[f32]) -> Option<Self> {
        let full = raw::full_event(dir)?;
        let lead = arms::preroll_offset(&full, audio)?;
        Some(Self::new(dir, full, lead))
    }

    /// The longest run of silent windows as one stretch of room, less its last
    /// hop so that a strike's first samples never enter it: `[start, end)`.
    fn room(&self) -> Option<(usize, usize)> {
        let mut best: Option<(usize, usize)> = None;
        let mut run_first = 0;
        for (i, &e) in self.silent_ends.iter().enumerate() {
            if i == 0 || e != self.silent_ends[i - 1] + HOP_SIZE {
                run_first = e;
            }
            if best.is_none_or(|(a, b)| e - run_first > b - a) {
                best = Some((run_first, e));
            }
        }
        best.map(|(first, last)| (first - BASS_WINDOW_SIZE, last - HOP_SIZE))
    }

    /// The room's median Goertzel amplitude at `f` over the silent windows, at
    /// `TRACK_WIN` in the `4/n` units: the floor a track is cut at.
    fn floor_at(&self, f: f32, hann: &[f32]) -> Option<f32> {
        let mut v: Vec<f32> = self
            .silent_ends
            .iter()
            .map(|&e| {
                spectral::goertzel_windowed(&self.full[e - TRACK_WIN..e], SAMPLE_RATE, f, hann).0
            })
            .collect();
        median(&mut v)
    }
}

// ── T2a: the room ───────────────────────────────────────────────────────────

/// One capture's screened pre-roll, as a stretch of room.
struct RoomSegment {
    dir: String,
    serial: u64,
    /// Where it starts in its full event.
    start: usize,
    samples: Vec<f32>,
}

/// Every capture's screened pre-roll in the set.
fn room_pool(mirrors: &[Mirror], root: &Path) -> Vec<RoomSegment> {
    par_map(mirrors, |m| {
        let audio = m.cap.audio(root)?;
        let s = Screened::of_capture(&root.join(&m.cap.dir), &audio)?;
        let (start, end) = s.room()?;
        (end > start + 2 * CROSSFADE).then(|| RoomSegment {
            dir: m.cap.dir.clone(),
            serial: m.serial,
            start,
            samples: s.full[start..end].to_vec(),
        })
    })
    .into_iter()
    .flatten()
    .collect()
}

/// `segments` laid end to end until `len` samples, each seam a `CROSSFADE`
/// whose gains are the square roots of a raised-cosine pair. Returns the bed
/// and how many samples each segment gave, 0 for one too short to use or never
/// reached; `None` when they run out first.
fn tile(segments: &[&[f32]], len: usize) -> Option<(Vec<f32>, Vec<usize>)> {
    let mut bed: Vec<f32> = Vec::with_capacity(len + CROSSFADE);
    let mut taken = vec![0usize; segments.len()];
    let mut last = None;
    for (i, seg) in segments.iter().enumerate() {
        if bed.len() >= len {
            break;
        }
        if seg.len() <= 2 * CROSSFADE {
            continue;
        }
        if bed.is_empty() {
            bed.extend_from_slice(seg);
        } else {
            let base = bed.len() - CROSSFADE;
            for (k, (b, &s)) in bed[base..].iter_mut().zip(seg.iter()).enumerate() {
                let x = FRAC_PI_2 * (k as f64 + 0.5) / CROSSFADE as f64;
                *b = (*b as f64 * x.cos() + s as f64 * x.sin()) as f32;
            }
            bed.extend_from_slice(&seg[CROSSFADE..]);
        }
        taken[i] = seg.len();
        last = Some(i);
    }
    if bed.len() < len {
        return None;
    }
    if let Some(i) = last {
        taken[i] -= bed.len() - len;
    }
    bed.truncate(len);
    Some((bed, taken))
}

/// Adds the strike to `bed` from its first sample: white noise under the
/// envelope `STRIKE` sets relative to `rms`, the bed's own, times `gain_db`.
fn add_strike(bed: &mut [f32], rms: f64, gain_db: f64, rng: &mut Gaussian) {
    let fs = SAMPLE_RATE as f64;
    let gain = 10f64.powf(gain_db / 20.0);
    for (m, s) in bed.iter_mut().enumerate() {
        let t = m as f64 / fs;
        let envelope: f64 = STRIKE
            .iter()
            .map(|&(db, tau)| 10f64.powf(db / 20.0) * (-t / tau).exp())
            .sum();
        *s += (rms * gain * envelope * rng.next() as f64) as f32;
    }
}

// ── decays ──────────────────────────────────────────────────────────────────

/// A two-stage decay in dB against time: `early_db_s` until the knee, then
/// `late_db_s`, 0 dB at t = 0. One stage when the two slopes agree.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Decay {
    early_db_s: f32,
    late_db_s: f32,
    knee_s: f32,
}

impl Decay {
    fn single(slope: f32) -> Self {
        Self {
            early_db_s: slope,
            late_db_s: slope,
            knee_s: 0.0,
        }
    }

    fn db(&self, t: f64) -> f64 {
        let k = self.knee_s as f64;
        self.early_db_s as f64 * t.min(k) + self.late_db_s as f64 * (t - k).max(0.0)
    }

    /// Amplitude at `t` relative to t = 0.
    fn gain(&self, t: f64) -> f64 {
        10f64.powf(self.db(t) / 20.0)
    }

    fn json(&self) -> Value {
        json!({
            "early_db_s": self.early_db_s,
            "late_db_s": self.late_db_s,
            "knee_s": self.knee_s,
        })
    }
}

/// A window of `TRACK_WIN` samples ending at `end` is centred here, in samples.
fn centre(end: usize) -> f64 {
    end as f64 - (TRACK_WIN as f64 + 1.0) / 2.0
}

/// The Goertzel amplitude at `f` hop by hop over `audio`, at `TRACK_WIN` in
/// the `4/n` units: `(window centre in s from the first sample, amplitude)`.
fn track(audio: &[f32], f: f32, hann: &[f32]) -> Vec<(f64, f32)> {
    let mut out = Vec::new();
    let mut end = TRACK_WIN;
    while end <= audio.len() {
        let (a, _) =
            spectral::goertzel_windowed(&audio[end - TRACK_WIN..end], SAMPLE_RATE, f, hann);
        out.push((centre(end) / SAMPLE_RATE as f64, a));
        end += HOP_SIZE;
    }
    out
}

/// How many of `amps` precede the first hop whose running median over
/// `CUT_MEDIAN_HOPS` falls under `floor`: where a track leaves the room.
fn above_floor(amps: &[f32], floor: Option<f32>) -> usize {
    let Some(floor) = floor else {
        return amps.len();
    };
    let h = CUT_MEDIAN_HOPS / 2;
    let mut w = Vec::with_capacity(CUT_MEDIAN_HOPS);
    for k in 0..amps.len() {
        w.clear();
        w.extend_from_slice(&amps[k.saturating_sub(h)..(k + h + 1).min(amps.len())]);
        if median(&mut w).is_some_and(|m| m < floor) {
            return k;
        }
    }
    amps.len()
}

/// Weighted least squares on the leading `p` ≤ 3 columns of `x`: the
/// coefficients and the weighted residual sum of squares. `None` when the
/// normal equations are singular.
fn wls(x: &[[f64; 3]], y: &[f64], w: &[f64], p: usize) -> Option<([f64; 3], f64)> {
    let mut a = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for ((xi, &yi), &wi) in x.iter().zip(y).zip(w) {
        for (r, br) in b.iter_mut().enumerate().take(p) {
            *br += wi * xi[r] * yi;
            for (c, arc) in a[r].iter_mut().enumerate().take(p) {
                *arc += wi * xi[r] * xi[c];
            }
        }
    }
    let beta = solve(a, b, p)?;
    let sse = x
        .iter()
        .zip(y)
        .zip(w)
        .map(|((xi, &yi), &wi)| {
            let fit: f64 = beta.iter().zip(xi).take(p).map(|(c, v)| c * v).sum();
            wi * (yi - fit).powi(2)
        })
        .sum();
    Some((beta, sse))
}

/// Gaussian elimination with partial pivoting on the leading `p`×`p` block.
fn solve(mut a: [[f64; 3]; 3], mut b: [f64; 3], p: usize) -> Option<[f64; 3]> {
    let scale = a.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    if scale == 0.0 {
        return None;
    }
    for col in 0..p {
        let pivot = (col..p).max_by(|&i, &j| a[i][col].abs().total_cmp(&a[j][col].abs()))?;
        if a[pivot][col].abs() <= 1e-12 * scale {
            return None;
        }
        a.swap(col, pivot);
        b.swap(col, pivot);
        for r in col + 1..p {
            let factor = a[r][col] / a[col][col];
            let pivot_row = a[col];
            for (v, pv) in a[r].iter_mut().zip(pivot_row).skip(col).take(p - col) {
                *v -= factor * pv;
            }
            b[r] -= factor * b[col];
        }
    }
    let mut x = [0.0f64; 3];
    for r in (0..p).rev() {
        let tail: f64 = (r + 1..p).map(|c| a[r][c] * x[c]).sum();
        x[r] = (b[r] - tail) / a[r][r];
    }
    x.iter().take(p).all(|v| v.is_finite()).then_some(x)
}

/// The two-stage line `y = c + s₁·min(t, k) + s₂·max(t − k, 0)` by weighted
/// least squares, the knee `k` swept over the points' own times inside
/// `KNEE_RANGE_S` with `MIN_SEGMENT_HOPS` a side, the least residual kept.
/// Returns `(c, s₁, s₂, k)`.
fn hinge(t: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64, f64, f64)> {
    let mut best: Option<(f64, [f64; 3], f64)> = None;
    let mut x = vec![[0.0f64; 3]; t.len()];
    for j in MIN_SEGMENT_HOPS..=t.len().saturating_sub(MIN_SEGMENT_HOPS) {
        let k = t[j];
        if k < KNEE_RANGE_S.0 || k > KNEE_RANGE_S.1 {
            continue;
        }
        for (xi, &ti) in x.iter_mut().zip(t) {
            *xi = [1.0, ti.min(k), (ti - k).max(0.0)];
        }
        if let Some((beta, sse)) = wls(&x, y, w, 3)
            && best.is_none_or(|b| sse < b.0)
        {
            best = Some((sse, beta, k));
        }
    }
    best.map(|(_, b, k)| (b[0], b[1], b[2], k))
}

/// The weighted least-squares line `y = c + s·t`: `(c, s)`.
fn line(t: &[f64], y: &[f64], w: &[f64]) -> Option<(f64, f64)> {
    let x: Vec<[f64; 3]> = t.iter().map(|&ti| [1.0, ti, 0.0]).collect();
    wls(&x, y, w, 2).map(|(b, _)| (b[0], b[1]))
}

/// What a decay fit on one partial's track found.
#[derive(Clone, Copy, Debug)]
enum Fit {
    Decay(Decay),
    /// The track was under the room floor from its first hops.
    UnderFloor,
    /// Too few hops above the floor for the model.
    TooShort,
}

/// The decay of a partial from its track, cut where it falls under `floor`:
/// the two-stage least-squares fit in dB, or one slope.
fn decay_from_track(tr: &[(f64, f32)], floor: Option<f32>, two_stage: bool) -> Fit {
    let amps: Vec<f32> = tr.iter().map(|p| p.1).collect();
    let keep = above_floor(&amps, floor);
    if keep == 0 {
        return Fit::UnderFloor;
    }
    let (t, y): (Vec<f64>, Vec<f64>) = tr[..keep]
        .iter()
        .filter(|p| p.1 > 0.0)
        .map(|&(t, a)| (t, 20.0 * (a as f64).log10()))
        .unzip();
    let w = vec![1.0; t.len()];
    let fitted = if two_stage {
        hinge(&t, &y, &w).map(|(_, s1, s2, k)| Decay {
            early_db_s: s1 as f32,
            late_db_s: s2 as f32,
            knee_s: k as f32,
        })
    } else if t.len() >= MIN_SEGMENT_HOPS {
        line(&t, &y, &w).map(|(_, s)| Decay::single(s as f32))
    } else {
        None
    };
    fitted.map_or(Fit::TooShort, Fit::Decay)
}

/// Median and quartiles of one quantity.
#[derive(Clone, Copy, Debug)]
struct Spread {
    median: f32,
    q25: f32,
    q75: f32,
    count: usize,
}

fn spread(v: &mut [f32]) -> Option<Spread> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let q = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
    Some(Spread {
        median: q(0.5),
        q25: q(0.25),
        q75: q(0.75),
        count: v.len(),
    })
}

/// One (register, partial) cell of a decay table.
#[derive(Clone, Copy)]
struct Cell {
    early: Spread,
    late: Spread,
    knee: Spread,
}

/// The cells of `fits`, keyed by (register, partial number).
fn cells_of(fits: impl Iterator<Item = (usize, usize, Decay)>) -> HashMap<(usize, usize), Cell> {
    let mut by: HashMap<(usize, usize), Vec<Decay>> = HashMap::new();
    for (r, n, d) in fits {
        by.entry((r, n)).or_default().push(d);
    }
    by.into_iter()
        .filter_map(|(k, v)| {
            let mut e: Vec<f32> = v.iter().map(|d| d.early_db_s).collect();
            let mut l: Vec<f32> = v.iter().map(|d| d.late_db_s).collect();
            let mut kn: Vec<f32> = v.iter().map(|d| d.knee_s).collect();
            Some((
                k,
                Cell {
                    early: spread(&mut e)?,
                    late: spread(&mut l)?,
                    knee: spread(&mut kn)?,
                },
            ))
        })
        .collect()
}

/// Two-stage decays fitted on the extended store's long records, every located
/// partial of every open (or undeclared) record of at least 5 s, read whole from
/// `audio.raw` rather than bounded to the Worker's 1.5 s.
struct DecayTable {
    cells: HashMap<(usize, usize), Cell>,
    /// Every partial's fits, `(register, n, whole record, first 1.5 s)`.
    fits: Vec<(usize, usize, Fit, Fit)>,
    root: PathBuf,
    long_records: usize,
    solos: usize,
    unscreened: usize,
}

/// One long record's outcome.
enum RecordFits {
    Solo,
    Unscreened,
    Fitted(Vec<(usize, usize, Fit, Fit)>),
}

impl DecayTable {
    fn fit(root: &Path, regen_path: &Path) -> Result<Self> {
        let (mirrors, _) = load_mirrors(regen_path)?;
        let long: Vec<&Mirror> = mirrors
            .iter()
            .filter(|m| {
                fs::metadata(root.join(&m.cap.dir).join("audio.raw"))
                    .is_ok_and(|md| md.len() as usize >= 4 * LONG_RECORD_SAMPLES)
            })
            .collect();
        if long.is_empty() {
            bail!("no record of 5 s or longer under {}", root.display());
        }
        let outcomes = par_map(&long, |m| fit_record(m, root));
        let mut fits = Vec::new();
        let (mut solos, mut unscreened) = (0, 0);
        for o in outcomes {
            match o {
                RecordFits::Solo => solos += 1,
                RecordFits::Unscreened => unscreened += 1,
                RecordFits::Fitted(v) => fits.extend(v),
            }
        }
        let cells = cells_of(fits.iter().filter_map(|&(r, n, whole, _)| match whole {
            Fit::Decay(d) => Some((r, n, d)),
            _ => None,
        }));
        Ok(Self {
            cells,
            fits,
            root: root.to_path_buf(),
            long_records: long.len(),
            solos,
            unscreened,
        })
    }

    /// A decay for partial `n` in `register`, uniform inside each quartile
    /// range of its cell, or of the nearest partial number's cell that holds
    /// `MIN_CELL_FITS`; and whether it was borrowed.
    fn draw(&self, register: usize, n: usize, rng: &mut Gaussian) -> Option<(Decay, bool)> {
        let (&(_, m), cell) = self
            .cells
            .iter()
            .filter(|((r, _), c)| *r == register && c.early.count >= MIN_CELL_FITS)
            .min_by_key(|((_, m), _)| (m.abs_diff(n), *m))?;
        let mut within = |s: Spread| s.q25 + (s.q75 - s.q25) * rng.uniform() as f32;
        let decay = Decay {
            early_db_s: within(cell.early),
            late_db_s: within(cell.late),
            knee_s: within(cell.knee),
        };
        Some((decay, m != n))
    }

    fn summary(&self) -> String {
        let fitted = self
            .fits
            .iter()
            .filter(|f| matches!(f.2, Fit::Decay(_)))
            .count();
        let under = self
            .fits
            .iter()
            .filter(|f| matches!(f.2, Fit::UnderFloor))
            .count();
        let short = self
            .fits
            .iter()
            .filter(|f| matches!(f.2, Fit::TooShort))
            .count();
        format!(
            "T2 decays: the extended store's records of 5 s or more under {}, read whole from audio.raw: {} records, \
             {} skipped as declared solos, {} without a screened pre-roll; {} partials, {} fitted in two stages, \
             {} under the room floor from the start, {} too short above it",
            self.root.display(),
            self.long_records,
            self.solos,
            self.unscreened,
            self.fits.len(),
            fitted,
            under,
            short
        )
    }

    /// The table, one line per (register, partial).
    fn text(&self) -> String {
        let mut keys: Vec<&(usize, usize)> = self.cells.keys().collect();
        keys.sort();
        let mut s = format!(
            "{:<11} {:>3} {:>5}   {:>24}   {:>24}   {:>21}\n",
            "register", "n", "fits", "early dB/s [IQR]", "late dB/s [IQR]", "knee s [IQR]"
        );
        for k in keys {
            let c = &self.cells[k];
            let _ = writeln!(
                s,
                "{:<11} {:>3} {:>5}   {:>7.1} [{:>6.1}, {:>6.1}]   {:>7.1} [{:>6.1}, {:>6.1}]   {:>5.2} [{:>5.2}, {:>5.2}]",
                REGISTERS[k.0],
                k.1,
                c.early.count,
                c.early.median,
                c.early.q25,
                c.early.q75,
                c.late.median,
                c.late.q25,
                c.late.q75,
                c.knee.median,
                c.knee.q25,
                c.knee.q75
            );
        }
        s
    }
}

/// Every located partial of one long record: its track over the whole record
/// and over the first 1.5 s, each cut at the room floor and fitted in two
/// stages.
fn fit_record(m: &Mirror, root: &Path) -> RecordFits {
    if m.cap.sounding.is_some_and(|s| !s.is_open()) {
        return RecordFits::Solo;
    }
    let dir = root.join(&m.cap.dir);
    let Some(audio) = raw::stable(&dir) else {
        return RecordFits::Unscreened;
    };
    let Some(s) = Screened::of_capture(&dir, &audio).filter(|s| !s.silent_ends.is_empty()) else {
        return RecordFits::Unscreened;
    };
    let hann = hann_vec(TRACK_WIN);
    let register = ambient::register_index(m.cap.key);
    // Hops whose window ends inside the Worker's 1.5 s.
    let short_hops = (CAPTURE_DEFAULT_SAMPLES - TRACK_WIN) / HOP_SIZE + 1;
    RecordFits::Fitted(
        m.partials
            .iter()
            .map(|&(n, f, _)| {
                let floor = s.floor_at(f, &hann);
                let tr = track(&audio, f, &hann);
                let whole = decay_from_track(&tr, floor, true);
                let short = decay_from_track(&tr[..tr.len().min(short_hops)], floor, true);
                (register, n, whole, short)
            })
            .collect(),
    )
}

/// Where the generator's decays come from.
enum DecaySource {
    /// The extended store's two-stage table.
    Long(DecayTable),
    /// The mirrored capture's own 1.5 s record, one slope per partial.
    Own,
}

impl DecaySource {
    fn label(&self) -> String {
        match self {
            DecaySource::Long(t) => format!(
                "the extended store's long records ({}), two stages drawn per register and partial inside the quartiles",
                t.root.display()
            ),
            DecaySource::Own => {
                "each mirrored capture's own 1.5 s record, one slope per partial (no --long-root)"
                    .to_string()
            }
        }
    }
}

/// One slope per located partial from the capture's own record, cut at its
/// room floor; a partial too short to fit takes the median of the others, and
/// 0 dB/s when none fits.
fn own_slopes(m: &Mirror, audio: &[f32], screened: Option<&Screened>, hann: &[f32]) -> Vec<Decay> {
    let fits: Vec<Option<f32>> = m
        .partials
        .iter()
        .map(|&(_, f, _)| {
            let floor = screened.and_then(|s| s.floor_at(f, hann));
            match decay_from_track(&track(audio, f, hann), floor, false) {
                Fit::Decay(d) => Some(d.late_db_s),
                _ => None,
            }
        })
        .collect();
    let mut found: Vec<f32> = fits.iter().flatten().copied().collect();
    let fallback = median(&mut found).unwrap_or(0.0);
    fits.into_iter()
        .map(|s| Decay::single(s.unwrap_or(fallback)))
        .collect()
}

// ── the partials ────────────────────────────────────────────────────────────

/// One string of one partial: frequency (Hz), amplitude at the onset, phase.
#[derive(Clone, Copy, Debug)]
struct Line {
    freq: f64,
    amp: f64,
    phase: f64,
}

/// One present partial of a synthetic capture.
struct Present {
    n: usize,
    f: f32,
    /// The mirrored capture's Worker reading.
    a_n: f32,
    /// Time-domain amplitude at the onset, all strings in power.
    onset_amp: f64,
    decay: Decay,
    borrowed: bool,
    lines: Vec<Line>,
}

/// The strings of `key`.
fn strings_on(key: u8) -> usize {
    if key <= LAST_SINGLE_KEY {
        1
    } else if key <= LAST_BICHORD_KEY {
        2
    } else {
        3
    }
}

/// The as-found split range of `key`, ¢ at the fundamental.
fn split_range(key: u8) -> (f32, f32) {
    if key < A4 {
        SPLIT_BELOW_A4_CENTS
    } else {
        SPLIT_FROM_A4_CENTS
    }
}

/// The strings' frequencies for a partial at `f` under the split ratio `delta`:
/// `f·(1 ± Δ/2)`, and `f` itself for a third string. The split in Hz is then
/// `n·f₀·√(1 + Bn²)·Δ`, n times the fundamental's to within `√(1 + Bn²)`.
fn string_freqs(f: f64, delta: f64, strings: usize) -> Vec<f64> {
    match strings {
        1 => vec![f],
        2 => vec![f * (1.0 - delta / 2.0), f * (1.0 + delta / 2.0)],
        _ => vec![f * (1.0 - delta / 2.0), f, f * (1.0 + delta / 2.0)],
    }
}

/// The onset amplitude at which a partial of envelope `decay` at `f` reads
/// `a_n` as the Worker reads it: the magnitude at the rounded bin of the
/// `hann.len()`-point Hann FFT of the record's first samples. That reading is
/// `(A₀/2)·|Σ_m w[m]·g(m)·e^{i2π(f − f_bin)m/fs}|`, so for a stationary partial
/// on its bin it is the coherent gain, `A₀ = 2a_n/Σw`, `Σw ≈ N/2`; off the bin
/// it carries the window's scalloping, and under a decay the envelope's
/// weighting.
fn onset_amplitude(a_n: f32, f: f32, decay: Decay, hann: &[f32]) -> f64 {
    let fs = SAMPLE_RATE as f64;
    let hz = fs / hann.len() as f64;
    let offset = f as f64 - (f as f64 / hz).round() * hz;
    let step = Complex::from_polar(1.0, TAU * offset / fs);
    let mut p = Complex::new(1.0f64, 0.0);
    let mut sum = Complex::new(0.0f64, 0.0);
    for (m, &w) in hann.iter().enumerate() {
        sum += p * (w as f64 * decay.gain(m as f64 / fs));
        p *= step;
    }
    2.0 * a_n as f64 / sum.norm()
}

/// Adds `lines`, all under `decay` from the first sample of `out`, each as
/// `amp·g(t)·sin(2πft + phase)`.
fn add_partial(out: &mut [f32], lines: &[Line], decay: Decay) {
    let fs = SAMPLE_RATE as f64;
    let steps: Vec<Complex<f64>> = lines
        .iter()
        .map(|l| Complex::from_polar(1.0, TAU * l.freq / fs))
        .collect();
    let mut phasors: Vec<Complex<f64>> = lines
        .iter()
        .map(|l| Complex::from_polar(l.amp, l.phase))
        .collect();
    for (m, s) in out.iter_mut().enumerate() {
        let sum: f64 = phasors.iter().map(|p| p.im).sum();
        *s += (decay.gain(m as f64 / fs) * sum) as f32;
        for (p, step) in phasors.iter_mut().zip(&steps) {
            *p *= step;
        }
    }
}

// ── T2b: the residual ───────────────────────────────────────────────────────

/// One located partial's sinusoidal model, as subtracted from the record.
#[derive(Clone, Copy, Debug)]
struct PartialModel {
    n: usize,
    f: f32,
    /// Seconds from the onset to the last hop above the room floor.
    life_s: f32,
    decay: Decay,
    /// The fitted frequency's drift, Hz/s.
    glide_hz_s: f32,
}

/// The centre-referenced complex amplitude of `win` at `f`:
/// `(2/Σw)·Σ_m w[m]·x[m]·e^{−i2πf(m − m_c)/fs}`, m_c the window's centre. For a
/// partial `A·cos φ(t)` inside the main lobe it is `A·e^{iφ(t_c)}` scaled by the
/// window's real response at the offset, so its phase is the partial's at the
/// centre whatever `f` is: the Hann window is symmetric about its centre.
fn phasor(win: &[f32], f: f64, hann: &[f32], hann_sum: f64) -> Complex<f64> {
    let fs = SAMPLE_RATE as f64;
    let mid = (hann.len() as f64 - 1.0) / 2.0;
    let step = Complex::from_polar(1.0, -TAU * f / fs);
    let mut p = Complex::from_polar(1.0, TAU * f * mid / fs);
    let mut sum = Complex::new(0.0f64, 0.0);
    for (&x, &w) in win.iter().zip(hann) {
        sum += p * (x as f64 * w as f64);
        p *= step;
    }
    sum * (2.0 / hann_sum)
}

/// `x` wrapped into [−π, π).
fn wrap(x: f64) -> f64 {
    (x + PI).rem_euclid(TAU) - PI
}

/// What the residual's model shares across one capture's partials: where the
/// note begins, the shipped tracker gate the target re-centres under, and the
/// window.
struct ModelCtx<'a> {
    onset: usize,
    gate: f32,
    hann: &'a [f32],
    hann_sum: f64,
}

/// Tracks the partial at `f_n` in `x` from the onset on, fits its model and
/// subtracts it from `out`. `None` where the track stays under `floor`.
///
/// The track is the `TRACK_WIN` phasor hop by hop at the tracker's adaptive
/// target: the phase unwrapped by the phase vocoder's step and the target
/// re-centred as `Engine` re-centres it (McAulay & Quatieri 1986). The model is
/// fitted to it rather than interpolated through it — a two-stage decay in dB
/// and a quadratic phase, by ordinary least squares over the hops before the
/// track falls under the room — and resynthesised sample by sample: amplitude
/// the decay, frequency linear in time, a one-hop ramp in from the onset and
/// out after the last hop.
fn subtract_partial(
    x: &[f32],
    out: &mut [f32],
    ctx: &ModelCtx,
    n: usize,
    f_n: f32,
    floor: f32,
) -> Option<PartialModel> {
    let fs = SAMPLE_RATE as f64;
    let t_hop = HOP_SIZE as f64 / fs;
    let f0 = f_n as f64;
    let mut target = f0;
    let mut prev: Option<f64> = None;
    let mut psi = 0.0f64;
    let (mut centres, mut amps, mut psis) = (Vec::new(), Vec::new(), Vec::new());
    let mut end = ctx.onset + TRACK_WIN;
    while end <= x.len() {
        let z = phasor(&x[end - TRACK_WIN..end], target, ctx.hann, ctx.hann_sum);
        let c = centre(end);
        let theta = z.arg();
        match prev {
            None => psi = wrap(theta - TAU * f0 * c / fs),
            Some(p) => {
                let f_inst = target + wrap(theta - p - TAU * target * t_hop) / (TAU * t_hop);
                psi += TAU * (f_inst - f0) * t_hop;
                if z.norm() >= ctx.gate as f64 && f_inst.is_finite() && f_inst > 0.0 {
                    target = 0.95 * target + 0.05 * f_inst;
                }
            }
        }
        prev = Some(theta);
        centres.push(c);
        amps.push(z.norm() as f32);
        psis.push(psi);
        end += HOP_SIZE;
    }
    let life = above_floor(&amps, Some(floor));
    if life < MIN_SEGMENT_HOPS {
        return None;
    }
    let onset = ctx.onset as f64;
    let t: Vec<f64> = centres[..life].iter().map(|&c| (c - onset) / fs).collect();
    let db: Vec<f64> = amps[..life]
        .iter()
        .map(|&a| 20.0 * (a.max(f32::MIN_POSITIVE) as f64).log10())
        .collect();
    let w = vec![1.0f64; life];
    let (c0, s1, s2, knee) =
        hinge(&t, &db, &w).or_else(|| line(&t, &db, &w).map(|(c, s)| (c, s, s, 0.0)))?;
    let w_sum: f64 = w.iter().sum();
    let t_mean = t.iter().zip(&w).map(|(a, b)| a * b).sum::<f64>() / w_sum;
    let xq: Vec<[f64; 3]> = t
        .iter()
        .map(|&ti| {
            let u = ti - t_mean;
            [1.0, u, u * u]
        })
        .collect();
    let order = if life >= 2 * MIN_SEGMENT_HOPS { 3 } else { 2 };
    let (q, _) = wls(&xq, &psis[..life], &w, order)?;
    // The model absorbs the noise its own parameters span. Subtracted hop by
    // hop as measured, the track leaves the bed 15–21 dB under the noise at
    // f_n in white noise; these six parameters, unweighted, 0.9 dB (asserted:
    // subtracting_the_track_as_measured_notches_the_bed,
    // residual_of_one_partial_sits_at_the_noise). Weighting by the amplitude's
    // square concentrates the fit on the early hops and deepens it there.
    let last = centres[life - 1];
    let hop = HOP_SIZE as f64;
    let stop = ((last + hop).ceil() as usize).min(x.len());
    for (m, o) in out.iter_mut().enumerate().take(stop).skip(ctx.onset) {
        let mf = m as f64;
        let ti = (mf - onset) / fs;
        let level = c0 + s1 * ti.min(knee) + s2 * (ti - knee).max(0.0);
        let ramp = ((mf - onset) / hop).min(1.0) * (1.0 - (mf - last) / hop).clamp(0.0, 1.0);
        let u = ti - t_mean;
        let phase = TAU * f0 * mf / fs + q[0] + q[1] * u + q[2] * u * u;
        *o -= (10f64.powf(level / 20.0) * ramp * phase.cos()) as f32;
    }
    Some(PartialModel {
        n,
        f: f_n,
        life_s: t[life - 1] as f32,
        decay: Decay {
            early_db_s: s1 as f32,
            late_db_s: s2 as f32,
            knee_s: knee as f32,
        },
        glide_hz_s: (q[2] / PI) as f32,
    })
}

/// `combined` less the model of each located partial that rises above the
/// room: the T2b bed, indexed like `combined`, the models, and how many
/// partials went unmodelled.
fn residual(
    combined: &[f32],
    onset: usize,
    partials: &[(usize, f32, f32)],
    s: &Screened,
    gate: f32,
) -> (Vec<f32>, Vec<PartialModel>, usize) {
    let hann = hann_vec(TRACK_WIN);
    let hann_sum: f64 = hann.iter().map(|&w| w as f64).sum();
    let ctx = ModelCtx {
        onset,
        gate,
        hann: &hann,
        hann_sum,
    };
    let mut out = combined.to_vec();
    let mut models = Vec::new();
    let mut unmodelled = 0;
    for &(n, f, _) in partials {
        match s
            .floor_at(f, &hann)
            .and_then(|floor| subtract_partial(combined, &mut out, &ctx, n, f, floor))
        {
            Some(m) => models.push(m),
            None => unmodelled += 1,
        }
    }
    (out, models, unmodelled)
}

/// One 8192-point Hann spectrum per window of `signal` ending at each of `ends`.
fn spectra(
    signal: &[f32],
    ends: &[usize],
    planner: &mut RealFftPlanner<f32>,
) -> Vec<Vec<Complex<f32>>> {
    let plan = planner.plan_fft_forward(BASS_WINDOW_SIZE);
    let mut time = vec![0.0f32; BASS_WINDOW_SIZE];
    ends.iter()
        .map(|&e| {
            let mut spec = vec![Complex::new(0.0f32, 0.0); BASS_WINDOW_SIZE / 2 + 1];
            spectral::fft(
                &signal[e - BASS_WINDOW_SIZE..e],
                &mut time,
                &mut spec,
                &plan,
                BASS_WINDOW_SIZE,
            );
            spec
        })
        .collect()
}

/// The residual at one partial over a record's 8192-point frames, read at
/// f_n's bin against the median of the neighbouring gaps (lobes excluded, as
/// `N_local` excludes them): `(original dB, remnant dB, MSC)`, the MSC the
/// magnitude-squared coherence of original and residual at that bin over the
/// frames. `None` where the gaps hold fewer than `MIN_GAP_BINS` bins.
fn remnant_at(
    orig: &[Vec<Complex<f32>>],
    resid: &[Vec<Complex<f32>>],
    f: f32,
    f_lo: f32,
    f_hi: f32,
) -> Option<(f32, f32, f32)> {
    let hz = SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32;
    let lobe = LOBE_HALF_BINS as f32 * hz;
    let bins = BASS_WINDOW_SIZE / 2;
    let b = (f / hz).round() as usize;
    let lo1 = ((f_lo + lobe).max(20.0) / hz).ceil() as usize;
    let hi1 = ((f - lobe) / hz).floor().max(0.0) as usize;
    let lo2 = ((f + lobe) / hz).ceil() as usize;
    let hi2 = (((f_hi - lobe) / hz).floor().max(0.0) as usize).min(bins - 1);
    let gaps: Vec<usize> = (lo1..=hi1).chain(lo2..=hi2).collect();
    if gaps.len() < MIN_GAP_BINS || b == 0 || b >= bins || orig.is_empty() {
        return None;
    }
    let (mut r, mut x, mut g) = (Vec::new(), Vec::new(), Vec::new());
    let (mut cross, mut px, mut pr) = (Complex::new(0.0f64, 0.0), 0.0f64, 0.0f64);
    let mut gap_mags = Vec::with_capacity(gaps.len());
    for (so, sr) in orig.iter().zip(resid) {
        r.push(sr[b].norm());
        x.push(so[b].norm());
        gap_mags.clear();
        gap_mags.extend(gaps.iter().map(|&k| sr[k].norm()));
        g.push(median(&mut gap_mags)?);
        let xo = Complex::new(so[b].re as f64, so[b].im as f64);
        let xr = Complex::new(sr[b].re as f64, sr[b].im as f64);
        cross += xo * xr.conj();
        px += xo.norm_sqr();
        pr += xr.norm_sqr();
    }
    let level = median(&mut g)?;
    let msc = if px > 0.0 && pr > 0.0 {
        (cross.norm_sqr() / (px * pr)) as f32
    } else {
        f32::NAN
    };
    Some((
        20.0 * (median(&mut x)? / level).log10(),
        20.0 * (median(&mut r)? / level).log10(),
        msc,
    ))
}

/// The residual at one modelled partial of one capture.
#[derive(Clone, Debug)]
struct Remnant {
    capture: String,
    key: u8,
    register: usize,
    model: PartialModel,
    orig_db: f32,
    remnant_db: f32,
    msc: f32,
    frames: usize,
}

// ── one capture ─────────────────────────────────────────────────────────────

/// What every capture's generation shares.
struct Ctx<'a> {
    root: &'a Path,
    out: &'a Path,
    bed: Bed,
    preroll_hops: usize,
    seed: u64,
    decays: &'a DecaySource,
    pool: &'a [RoomSegment],
    strike_gain_db: [f64; 4],
}

/// What `make` produced for one mirrored capture.
enum Outcome {
    Made(Box<Made>),
    Skipped(&'static str),
}

/// One synthetic capture's truth row and what the fidelity tables read from it.
struct Made {
    row: Value,
    key: u8,
    register: usize,
    /// `20·lg(a_n/a_1)` by n: the regen's reading of the real capture, and the
    /// Worker's reading of the synthetic one.
    real_profile: Vec<(usize, f32)>,
    synth_profile: Vec<(usize, f32)>,
    /// The decays drawn, and the two-stage fit re-run on the synthetic record.
    drawn: Vec<(usize, Decay)>,
    refit: Vec<Decay>,
    strike_real: Option<f32>,
    strike_synth: Option<f32>,
    strings: usize,
    split_cents: Option<f32>,
    beat_hz: Option<f32>,
    borrowed: usize,
    absent: Vec<usize>,
    unmodelled: usize,
    remnants: Vec<Remnant>,
}

/// Mirrors one capture: draws its synthetic parameters, builds its bed, writes
/// its directory and reads what the fidelity tables compare.
fn make(m: &Mirror, ctx: &Ctx) -> Result<Outcome> {
    let dir = ctx.root.join(&m.cap.dir);
    let Some(real) = m.cap.audio(ctx.root) else {
        return Ok(Outcome::Skipped("no audio.raw"));
    };
    // The Worker's FFT of the mirrored record, which its a_n were read from.
    let n_fft = pow2_floor(real.len().min(CAPTURE_ANALYSIS_SAMPLES));
    if n_fft < 2048 {
        return Ok(Outcome::Skipped("record under 2048 samples"));
    }
    let screened = Screened::of_capture(&dir, &real);
    let key = m.cap.key;
    let register = ambient::register_index(key);
    let meta = metadata(&dir);
    let sigma_rec = meta["noise_floor"].as_f64().map(|v| v as f32);
    let mut rng = Gaussian::new(capture_seed(ctx.seed, m.serial));
    let mut planner = RealFftPlanner::<f32>::new();
    let hann_track = hann_vec(TRACK_WIN);

    // ── the unison: strings, and one split for the capture ──
    let strings = strings_on(key);
    let split_cents = (strings > 1).then(|| {
        let (lo, hi) = split_range(key);
        lo + (hi - lo) * rng.uniform() as f32
    });
    let delta = split_cents.map_or(0.0, |c| 2f64.powf(c as f64 / 1200.0) - 1.0);

    // ── the present partials: the mirrored series, each under a drawn decay ──
    let own = match ctx.decays {
        DecaySource::Own => own_slopes(m, &real, screened.as_ref(), &hann_track),
        DecaySource::Long(_) => Vec::new(),
    };
    let hann_worker = hann_vec(n_fft);
    let mut present = Vec::with_capacity(m.partials.len());
    for (i, &(n, f, a_n)) in m.partials.iter().enumerate() {
        let (decay, borrowed) = match ctx.decays {
            DecaySource::Long(table) => table
                .draw(register, n, &mut rng)
                .unwrap_or((Decay::single(0.0), true)),
            DecaySource::Own => (own[i], false),
        };
        let onset_amp = onset_amplitude(a_n, f, decay, &hann_worker);
        let per_string = onset_amp / (strings as f64).sqrt();
        let lines = string_freqs(f as f64, delta, strings)
            .into_iter()
            .map(|freq| Line {
                freq,
                amp: per_string,
                phase: TAU * rng.uniform(),
            })
            .collect();
        present.push(Present {
            n,
            f,
            a_n,
            onset_amp,
            decay,
            borrowed,
            lines,
        });
    }

    // ── declared absent: past the last located partial, inside the band ──
    let nyquist = SAMPLE_RATE as f32 / 2.0;
    let lobe_hz = LOBE_HALF_BINS as f32 * SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32;
    let last = m.partials.last().map_or(0, |p| p.0);
    let absent: Vec<(usize, f32)> = (last + 1..=last + ABSENT_PARTIALS)
        .map(|n| (n, ambient::stiff_partial(m.cap.f0, m.cap.b, n)))
        .filter(|&(_, f)| f > 0.0 && f < nyquist - lobe_hz)
        .collect();

    // ── the bed ──
    let mut remnants = Vec::new();
    let mut unmodelled = 0;
    let (noise, pre, bed_json, strike_json) = match ctx.bed {
        Bed::Room => {
            let pre = ctx.preroll_hops * HOP_SIZE;
            let mut order: Vec<&RoomSegment> = ctx.pool.iter().collect();
            order.sort_by(|x, y| {
                (x.dir != m.cap.dir, x.serial.abs_diff(m.serial), &x.dir).cmp(&(
                    y.dir != m.cap.dir,
                    y.serial.abs_diff(m.serial),
                    &y.dir,
                ))
            });
            let slices: Vec<&[f32]> = order.iter().map(|s| s.samples.as_slice()).collect();
            let Some((mut bed, taken)) = tile(&slices, pre + CAPTURE_DEFAULT_SAMPLES) else {
                return Ok(Outcome::Skipped("the room pool is shorter than one bed"));
            };
            let rms =
                (bed.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / bed.len() as f64).sqrt();
            let gain_db = ctx.strike_gain_db[register];
            add_strike(&mut bed[pre..], rms, gain_db, &mut rng);
            let segments: Vec<Value> = order
                .iter()
                .zip(&taken)
                .filter(|(_, t)| **t > 0)
                .map(|(s, &t)| json!({ "capture": s.dir, "start": s.start, "end": s.start + t }))
                .collect();
            (
                bed,
                pre,
                json!({ "kind": "room", "segments": segments, "crossfade_samples": CROSSFADE, "rms": rms }),
                json!({
                    "kind": "white noise from the onset, in noise.raw",
                    "terms": STRIKE.iter().map(|&(db, tau)| json!({ "db_over_bed_rms": db, "tau_s": tau })).collect::<Vec<_>>(),
                    "register_gain_db": gain_db,
                    "bed_rms": rms,
                }),
            )
        }
        Bed::Residual => {
            let Some(s) = screened.as_ref().filter(|s| !s.silent_ends.is_empty()) else {
                return Ok(Outcome::Skipped("no screened pre-roll"));
            };
            let rec_len = real.len().min(CAPTURE_DEFAULT_SAMPLES);
            let pre = ctx.preroll_hops.min(s.lead / HOP_SIZE) * HOP_SIZE;
            if pre < BASS_WINDOW_SIZE + 4 * HOP_SIZE {
                return Ok(Outcome::Skipped("under twelve hops before the record"));
            }
            let mut combined = s.full[..s.lead].to_vec();
            combined.extend_from_slice(&real[..rec_len]);
            let onset = ambient::onset_sample(&combined);
            let gate = sigma_rec.map_or(f32::INFINITY, |sg| {
                sg * spectral::neyman_pearson_k(TRACK_WIN)
            });
            let (resid, models, missed) = residual(&combined, onset, &m.partials, s, gate);
            unmodelled = missed;
            let ends: Vec<usize> = (s.lead + BASS_WINDOW_SIZE..=s.lead + rec_len)
                .step_by(HOP_SIZE)
                .collect();
            let so = spectra(&combined, &ends, &mut planner);
            let sr = spectra(&resid, &ends, &mut planner);
            let f_meas: Vec<f32> = m.partials.iter().map(|p| p.1).collect();
            let mut model_json = Vec::new();
            for model in &models {
                let Some(i) = m.partials.iter().position(|p| p.0 == model.n) else {
                    continue;
                };
                let (f_lo, f_hi) = ambient::neighbour_bounds(&m.cap, &f_meas, i, model.n);
                let read = remnant_at(&so, &sr, model.f, f_lo, f_hi);
                model_json.push(json!({
                    "number": model.n,
                    "life_s": model.life_s,
                    "decay": model.decay.json(),
                    "glide_hz_s": model.glide_hz_s,
                    "orig_db": read.map(|r| r.0),
                    "remnant_db": read.map(|r| r.1),
                    "msc": read.map(|r| r.2),
                }));
                if let Some((orig_db, remnant_db, msc)) = read {
                    remnants.push(Remnant {
                        capture: m.cap.dir.clone(),
                        key,
                        register,
                        model: *model,
                        orig_db,
                        remnant_db,
                        msc,
                        frames: ends.len(),
                    });
                }
            }
            (
                resid[s.lead - pre..s.lead + rec_len].to_vec(),
                pre,
                json!({
                    "kind": "residual",
                    "source": m.cap.dir,
                    "source_lead": s.lead,
                    "source_onset": onset,
                    "taken_from": s.lead - pre,
                    "model": "per located partial: the 4096-sample phasor hop by hop under the vocoder's step, fitted by least squares in a two-stage dB decay and a quadratic phase, resynthesised from the onset to one hop past the room floor",
                    "unmodelled": unmodelled,
                    "partials": model_json,
                }),
                Value::Null,
            )
        }
    };

    // ── the capture: the partials from the onset, the bed's pre-roll in front ──
    let rec_len = noise.len() - pre;
    let mut audio = noise[pre..].to_vec();
    for p in &present {
        add_partial(&mut audio, &p.lines, p.decay);
    }
    let mut full = noise[..pre].to_vec();
    full.extend_from_slice(&audio);

    let name = format!("key_{key:03}_{}_{}", NOTES[key as usize].name, m.serial);
    let out_dir = ctx.out.join(&name);
    fs::create_dir(&out_dir).with_context(|| format!("create {}", out_dir.display()))?;
    raw::write(&out_dir.join("audio.raw"), &audio)?;
    raw::write(&out_dir.join("audio_full_event.raw"), &full)?;
    raw::write(&out_dir.join("noise.raw"), &noise)?;
    let mut metadata = json!({
        "key_index": key,
        "sample_rate": SAMPLE_RATE,
        "measured_f0": m.measured_f0,
        "f0_et": NOTES[key as usize].frequency,
        "synthetic": ctx.bed.kind(),
    });
    // The thresholds the mirrored capture ran at, so the Gatekeeper screens the
    // synthetic pre-roll as it screened the real one.
    for field in [
        "noise_floor",
        "nhwrsf_threshold",
        "sustain_stability_threshold",
    ] {
        if meta[field].is_number() {
            metadata[field] = meta[field].clone();
        }
    }
    fs::write(
        out_dir.join("analysis.json"),
        serde_json::to_string_pretty(&json!({ "key_index": key, "metadata": metadata }))?,
    )?;

    let mut partials_json: Vec<Value> = present
        .iter()
        .map(|p| json!({ "number": p.n, "frequency": p.f, "amplitude": p.a_n }))
        .collect();
    partials_json.extend(
        absent
            .iter()
            .map(|&(n, f)| json!({ "number": n, "frequency": f, "amplitude": 0.0 })),
    );
    let series_json: Vec<Value> = present
        .iter()
        .map(|p| {
            json!({
                "number": p.n,
                "frequency": p.f,
                "amplitude": p.onset_amp,
                "worker_amplitude": p.a_n,
                "strings": p.lines.iter().map(|l| l.freq).collect::<Vec<_>>(),
                "string_amplitude": p.lines.first().map(|l| l.amp),
                "phases": p.lines.iter().map(|l| l.phase).collect::<Vec<_>>(),
                "decay": p.decay.json(),
                "decay_borrowed": p.borrowed,
            })
        })
        .collect();
    let row = json!({
        "key_index": key,
        "source_dir": name,
        "measured_f0": m.measured_f0,
        "mat_f0": m.cap.f0,
        "calculated_b": m.cap.b,
        "prior_b": get_expected_beta(key),
        "partials": partials_json,
        "synthetic": {
            "kind": ctx.bed.kind(),
            "mirrors": m.cap.dir,
            "bed": bed_json,
            "onset_sample": pre,
            "noise_file": "noise.raw",
            "present": present.iter().map(|p| p.n).collect::<Vec<_>>(),
            "absent": absent.iter().map(|p| p.0).collect::<Vec<_>>(),
            "strings": strings,
            "split_cents": split_cents,
            "partials": series_json,
            "strike": strike_json,
            "decay_source": ctx.decays.label(),
            "worker_fft": n_fft,
            "recorded_sigma": sigma_rec,
            "record_samples": rec_len,
        },
    });

    // ── what the fidelity tables read ──
    let reading = worker_reading(
        &audio,
        &present.iter().map(|p| p.f).collect::<Vec<_>>(),
        &mut planner,
    );
    let synth_profile = profile(&present.iter().map(|p| p.n).zip(reading).collect::<Vec<_>>());
    let real_profile = profile(
        &m.partials
            .iter()
            .map(|&(n, _, a)| (n, a))
            .collect::<Vec<_>>(),
    );
    let synth_screen = Screened::new(&dir, full, pre);
    let refit: Vec<Decay> = present
        .iter()
        .filter_map(|p| {
            match decay_from_track(
                &track(&audio, p.f, &hann_track),
                synth_screen.floor_at(p.f, &hann_track),
                true,
            ) {
                Fit::Decay(d) => Some(d),
                _ => None,
            }
        })
        .collect();
    let strike_real = screened
        .as_ref()
        .and_then(|s| strike_excess(&real, s, &m.cap, &mut planner));
    let strike_synth = strike_excess(&audio, &synth_screen, &m.cap, &mut planner);

    Ok(Outcome::Made(Box::new(Made {
        row,
        key,
        register,
        real_profile,
        synth_profile,
        drawn: present.iter().map(|p| (p.n, p.decay)).collect(),
        refit,
        strike_real,
        strike_synth,
        strings,
        split_cents,
        beat_hz: split_cents.map(|_| (m.cap.f0 as f64 * delta) as f32),
        borrowed: present.iter().filter(|p| p.borrowed).count(),
        absent: absent.iter().map(|p| p.0).collect(),
        unmodelled,
        remnants,
    })))
}

// ── the fidelity reads ──────────────────────────────────────────────────────

/// The Worker's reading of `audio` at each of `freqs`: the magnitude at the
/// rounded bin of the Hann FFT over the largest power of two of samples up to
/// 1.5 s.
fn worker_reading(audio: &[f32], freqs: &[f32], planner: &mut RealFftPlanner<f32>) -> Vec<f32> {
    let n = pow2_floor(audio.len().min(CAPTURE_ANALYSIS_SAMPLES));
    let plan = planner.plan_fft_forward(n);
    let mut time = vec![0.0f32; n];
    let mut spec = vec![Complex::new(0.0f32, 0.0); n / 2 + 1];
    let mut mag = vec![0.0f32; n / 2];
    spectral::fft(&audio[..n], &mut time, &mut spec, &plan, n);
    spectral::magnitude_spectrum(&spec, n, &mut mag);
    let hz = SAMPLE_RATE as f32 / n as f32;
    freqs
        .iter()
        .map(|&f| mag.get((f / hz).round() as usize).copied().unwrap_or(0.0))
        .collect()
}

/// `20·lg(a_n / a_1)` for each partial n ≥ 2 of a series that locates partial 1.
fn profile(series: &[(usize, f32)]) -> Vec<(usize, f32)> {
    let Some(&(_, a1)) = series.iter().find(|p| p.0 == 1) else {
        return Vec::new();
    };
    if a1 <= 0.0 {
        return Vec::new();
    }
    series
        .iter()
        .filter(|p| p.0 >= 2 && p.1 > 0.0)
        .map(|&(n, a)| (n, 20.0 * (a / a1).log10()))
        .collect()
}

/// The local-noise probe of report 0015 §1 over one capture's series.
struct Probe<'a> {
    cap: &'a Capture,
    ns: Vec<usize>,
    f_meas: Vec<f32>,
    hann: Vec<f32>,
    fft_scratch: Vec<f32>,
    scratch: Vec<f32>,
}

impl<'a> Probe<'a> {
    /// The capture's first twelve located partials, as the gate studies take them.
    fn new(cap: &'a Capture) -> Option<Self> {
        let (ns, f_meas): (Vec<usize>, Vec<f32>) = (1..=MAX_STROBE_REFS)
            .filter_map(|n| {
                let (f, _) = cap.partials.get(n).copied().flatten()?;
                (f > 0.0 && f < SAMPLE_RATE as f32 / 2.0).then_some((n, f))
            })
            .unzip();
        (!ns.is_empty()).then(|| Self {
            cap,
            ns,
            f_meas,
            hann: hann_vec(ambient::NOISE_WIN),
            fft_scratch: Vec::new(),
            scratch: Vec::new(),
        })
    }

    /// Appends each partial's `N_local` in the window of `signal` ending at `end`.
    fn add(
        &mut self,
        signal: &[f32],
        end: usize,
        planner: &mut RealFftPlanner<f32>,
        out: &mut [Vec<f32>],
    ) {
        let Some(spec) =
            ambient::probe_spectrum(signal, end, &self.hann, planner, &mut self.fft_scratch)
        else {
            return;
        };
        for (i, &n) in self.ns.iter().enumerate() {
            let (f_lo, f_hi) = ambient::neighbour_bounds(self.cap, &self.f_meas, i, n);
            if let Some((_, nl)) =
                ambient::partial_probe(&spec, self.f_meas[i], f_lo, f_hi, &mut self.scratch)
                && nl > 0.0
            {
                out[i].push(nl);
            }
        }
    }
}

/// The noise beside a capture's partials in record time 0.1–0.25 s over the
/// same gaps in its screened pre-roll, both as `N_local`, the ratio's median
/// over the partials: report 0015 §8's first-bucket excess, in amplitude.
fn strike_excess(
    audio: &[f32],
    s: &Screened,
    cap: &Capture,
    planner: &mut RealFftPlanner<f32>,
) -> Option<f32> {
    if s.silent_ends.is_empty() {
        return None;
    }
    let mut probe = Probe::new(cap)?;
    let mut floor = vec![Vec::new(); probe.ns.len()];
    for &e in &s.silent_ends {
        probe.add(&s.full, e, planner, &mut floor);
    }
    let onset = ambient::onset_sample(audio);
    let mut bucket = vec![Vec::new(); probe.ns.len()];
    let mut end = BASS_WINDOW_SIZE;
    while end <= audio.len() {
        let t = (end as f32 - onset as f32) / SAMPLE_RATE as f32;
        if t >= 0.0 && ambient::t_bucket(t) == STRIKE_BUCKET {
            probe.add(audio, end, planner, &mut bucket);
        }
        end += HOP_SIZE;
    }
    let mut ratios: Vec<f32> = floor
        .iter_mut()
        .zip(bucket.iter_mut())
        .filter_map(|(fl, bu)| Some(median(bu)? / median(fl)?))
        .filter(|r| r.is_finite())
        .collect();
    median(&mut ratios)
}

// ── the tables ──────────────────────────────────────────────────────────────

/// Whether `key` falls in band `band` of `BANDS`.
fn in_band(band: usize, key: u8) -> bool {
    match band {
        0..=3 => ambient::register_index(key) == band,
        4 => key <= 47,
        _ => key >= 62,
    }
}

fn fmt_spread(s: Option<Spread>, digits: usize) -> String {
    match s {
        Some(s) => format!(
            "{:+.d$} [{:+.d$}, {:+.d$}]",
            s.median,
            s.q25,
            s.q75,
            d = digits
        ),
        None => "—".to_string(),
    }
}

fn fmt_median(v: &mut [f32]) -> String {
    let count = v.len();
    match median(v) {
        Some(m) => format!("{m:+.1} ({count})"),
        None => "—".to_string(),
    }
}

/// The fidelity tables of rulebook §2.2.1 this generator can measure itself.
fn fidelity_text(made: &[Made], decays: &DecaySource, bed: Bed, head: &str) -> Result<String> {
    let mut s = String::new();
    writeln!(s, "{head}")?;

    // ── the amplitude profile ──
    writeln!(
        s,
        "\n## Amplitude profile: median 20 lg(a_n / a_1), dB (captures)\n"
    )?;
    writeln!(
        s,
        "Each synthetic partial is declared at its mirrored capture's own a_n, so the declared profile is the real \
         one by construction. The re-measured row is the Worker's reading of the synthetic record (the Hann FFT of \
         its first 2¹⁶ samples at the rounded bin), unison beating, decay, strike and bed included.\n"
    )?;
    write!(s, "| band | source |")?;
    for n in 2..=PROFILE_MAX_N {
        write!(s, " n = {n} |")?;
    }
    writeln!(s)?;
    writeln!(s, "|---|---|{}", "---|".repeat(PROFILE_MAX_N - 1))?;
    for (band, label) in BANDS.iter().enumerate() {
        let sel: Vec<&Made> = made.iter().filter(|x| in_band(band, x.key)).collect();
        if sel.is_empty() {
            continue;
        }
        for (source, pick) in [("real (regen)", 0usize), ("synthetic, re-measured", 1)] {
            write!(s, "| {label} | {source} |")?;
            for n in 2..=PROFILE_MAX_N {
                let mut v: Vec<f32> = sel
                    .iter()
                    .flat_map(|x| {
                        if pick == 0 {
                            &x.real_profile
                        } else {
                            &x.synth_profile
                        }
                    })
                    .filter(|p| p.0 == n)
                    .map(|p| p.1)
                    .collect();
                write!(s, " {} |", fmt_median(&mut v))?;
            }
            writeln!(s)?;
        }
        let reference = match band {
            4 => Some(A7_BASS_TENOR),
            5 => Some(A7_TREBLE),
            _ => None,
        };
        if let Some(r) = reference {
            write!(s, "| {label} | report 0009 Analysis 7 |")?;
            for n in 2..=PROFILE_MAX_N {
                write!(s, " {} |", r.get(n - 2).copied().unwrap_or("—"))?;
            }
            writeln!(s)?;
        }
    }

    // ── the decays ──
    writeln!(
        s,
        "\n## Decay: early and late slope (dB/s) and knee (s), median [IQR], pooled over partials\n"
    )?;
    writeln!(s, "| register | source | fits | early | late | knee |")?;
    writeln!(s, "|---|---|---|---|---|---|")?;
    for (r, label) in REGISTERS.iter().enumerate() {
        let mut rows: Vec<(&str, Vec<Decay>)> = Vec::new();
        if let DecaySource::Long(t) = decays {
            for (source, whole) in [
                ("real, whole long record", true),
                ("real, its first 1.5 s", false),
            ] {
                rows.push((
                    source,
                    t.fits
                        .iter()
                        .filter(|f| f.0 == r)
                        .filter_map(|f| match if whole { f.2 } else { f.3 } {
                            Fit::Decay(d) => Some(d),
                            _ => None,
                        })
                        .collect(),
                ));
            }
        }
        let sel: Vec<&Made> = made.iter().filter(|x| x.register == r).collect();
        rows.push((
            match decays {
                DecaySource::Long(_) => "synthetic, drawn",
                DecaySource::Own => "real, own record, one slope (drawn)",
            },
            sel.iter()
                .flat_map(|x| x.drawn.iter().map(|d| d.1))
                .collect(),
        ));
        rows.push((
            "synthetic, re-fitted on its 1.5 s",
            sel.iter().flat_map(|x| x.refit.iter().copied()).collect(),
        ));
        for (source, v) in rows {
            if v.is_empty() {
                continue;
            }
            let mut e: Vec<f32> = v.iter().map(|d| d.early_db_s).collect();
            let mut l: Vec<f32> = v.iter().map(|d| d.late_db_s).collect();
            let mut k: Vec<f32> = v.iter().map(|d| d.knee_s).collect();
            writeln!(
                s,
                "| {label} | {source} | {} | {} | {} | {} |",
                v.len(),
                fmt_spread(spread(&mut e), 1),
                fmt_spread(spread(&mut l), 1),
                fmt_spread(spread(&mut k), 2)
            )?;
        }
    }
    let borrowed: usize = made.iter().map(|x| x.borrowed).sum();
    let drawn: usize = made.iter().map(|x| x.drawn.len()).sum();
    if matches!(decays, DecaySource::Long(_)) {
        writeln!(
            s,
            "\n{borrowed} of {drawn} drawn decays came from the nearest partial number's cell, their own (register, n) \
             cell holding fewer than {MIN_CELL_FITS} fits."
        )?;
    }

    // ── the strike ──
    writeln!(
        s,
        "\n## Strike: N_local at 0.1–0.25 s over the screened pre-roll's at the same gaps, median [IQR] over captures\n"
    )?;
    writeln!(s, "| register | real | synthetic | report 0015 §8 |")?;
    writeln!(s, "|---|---|---|---|")?;
    for (r, label) in REGISTERS.iter().enumerate() {
        let sel: Vec<&Made> = made.iter().filter(|x| x.register == r).collect();
        if sel.is_empty() {
            continue;
        }
        let mut real: Vec<f32> = sel.iter().filter_map(|x| x.strike_real).collect();
        let mut syn: Vec<f32> = sel.iter().filter_map(|x| x.strike_synth).collect();
        let (nr, ns) = (real.len(), syn.len());
        writeln!(
            s,
            "| {label} | {} ({nr}) | {} ({ns}) | 2–5× in the first 0.5 s |",
            fmt_ratio(spread(&mut real)),
            fmt_ratio(spread(&mut syn))
        )?;
    }
    if bed == Bed::Room {
        writeln!(
            s,
            "\nThe synthetic record begins at its strike; a real record begins at the Gatekeeper's verdict, after its \
             strike, so the same bucket of record time lies later after the strike in the real captures."
        )?;
    }

    // ── the unison ──
    writeln!(
        s,
        "\n## Unison drawn: strings, split at the fundamental and its beat\n"
    )?;
    writeln!(
        s,
        "| register | captures | strings | split ¢ min / median / max | beat at f₀ Hz min / median / max |"
    )?;
    writeln!(s, "|---|---|---|---|---|")?;
    for (r, label) in REGISTERS.iter().enumerate() {
        let sel: Vec<&Made> = made.iter().filter(|x| x.register == r).collect();
        if sel.is_empty() {
            continue;
        }
        let mut strings: Vec<usize> = sel.iter().map(|x| x.strings).collect();
        strings.sort_unstable();
        strings.dedup();
        let mut split: Vec<f32> = sel.iter().filter_map(|x| x.split_cents).collect();
        let mut beat: Vec<f32> = sel.iter().filter_map(|x| x.beat_hz).collect();
        writeln!(
            s,
            "| {label} | {} | {} | {} | {} |",
            sel.len(),
            strings
                .iter()
                .map(|k| k.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            fmt_range(&mut split, 2),
            fmt_range(&mut beat, 3)
        )?;
    }

    // ── the declared-absent population ──
    writeln!(s, "\n## Declared absent: the T5(iii) population\n")?;
    writeln!(
        s,
        "| register | captures | absent declared | of them n ≤ {MAX_STROBE_REFS}, the ones `regen::load` keeps |"
    )?;
    writeln!(s, "|---|---|---|---|")?;
    for (r, label) in REGISTERS.iter().enumerate() {
        let sel: Vec<&Made> = made.iter().filter(|x| x.register == r).collect();
        if sel.is_empty() {
            continue;
        }
        let total: usize = sel.iter().map(|x| x.absent.len()).sum();
        let seen: usize = sel
            .iter()
            .map(|x| x.absent.iter().filter(|&&n| n <= MAX_STROBE_REFS).count())
            .sum();
        writeln!(s, "| {label} | {} | {total} | {seen} |", sel.len())?;
    }

    // ── T2b's leftover ──
    if bed == Bed::Residual {
        writeln!(
            s,
            "\n## T2b remnant: the residual at f_n's 8192-point bin over the gaps beside it, dB, and the coherence of \
             original and residual there\n"
        )?;
        writeln!(
            s,
            "| register | modelled | unmodelled | read | remnant dB median [IQR] | > +1 dB | MSC median [IQR] | original dB median |"
        )?;
        writeln!(s, "|---|---|---|---|---|---|---|---|")?;
        for (r, label) in REGISTERS.iter().enumerate() {
            let sel: Vec<&Made> = made.iter().filter(|x| x.register == r).collect();
            if sel.is_empty() {
                continue;
            }
            let rem: Vec<&Remnant> = sel.iter().flat_map(|x| x.remnants.iter()).collect();
            let modelled: usize = sel
                .iter()
                .filter_map(|x| {
                    x.row["synthetic"]["bed"]["partials"]
                        .as_array()
                        .map(|a| a.len())
                })
                .sum();
            let unmodelled: usize = sel.iter().map(|x| x.unmodelled).sum();
            let mut db: Vec<f32> = rem.iter().map(|x| x.remnant_db).collect();
            let above = db.iter().filter(|&&v| v > 1.0).count();
            let mut msc: Vec<f32> = rem
                .iter()
                .map(|x| x.msc)
                .filter(|v| v.is_finite())
                .collect();
            let mut orig: Vec<f32> = rem.iter().map(|x| x.orig_db).collect();
            writeln!(
                s,
                "| {label} | {modelled} | {unmodelled} | {} | {} | {above} | {} | {} |",
                rem.len(),
                fmt_spread(spread(&mut db), 1),
                fmt_spread(spread(&mut msc), 2),
                fmt_median(&mut orig)
            )?;
        }
        writeln!(
            s,
            "\nRead over the record's frames from its first sample: the first 50 ms after the onset are kept. A \
             partial goes unmodelled where its track never rises above the room floor; its energy stays in the bed. \
             The declared-absent partials sit on the real capture's unlocated ones, which the residual keeps."
        )?;
    }
    Ok(s)
}

fn fmt_ratio(s: Option<Spread>) -> String {
    match s {
        Some(s) => format!("{:.2}× [{:.2}, {:.2}]", s.median, s.q25, s.q75),
        None => "—".to_string(),
    }
}

fn fmt_range(v: &mut [f32], digits: usize) -> String {
    if v.is_empty() {
        return "—".to_string();
    }
    v.sort_by(f32::total_cmp);
    format!(
        "{:.d$} / {:.d$} / {:.d$}",
        v[0],
        v[v.len() / 2],
        v[v.len() - 1],
        d = digits
    )
}

/// The decay table as written to `fidelity_decay.csv`: the extended store's
/// cells, or without it the own-record slopes drawn, per (register, n).
fn decay_csv(decays: &DecaySource, made: &[Made]) -> String {
    let (source, cells) = match decays {
        DecaySource::Long(t) => ("extended store long records: two stages", t.cells.clone()),
        DecaySource::Own => (
            "own 1.5 s records: one slope",
            cells_of(
                made.iter()
                    .flat_map(|x| x.drawn.iter().map(move |&(n, d)| (x.register, n, d))),
            ),
        ),
    };
    let mut keys: Vec<&(usize, usize)> = cells.keys().collect();
    keys.sort();
    let mut s = String::from(
        "source,register,n,fits,early_median_db_s,early_q25,early_q75,late_median_db_s,late_q25,late_q75,knee_median_s,knee_q25,knee_q75\n",
    );
    for k in keys {
        let c = &cells[k];
        let _ = writeln!(
            s,
            "{source},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.3},{:.3},{:.4},{:.4},{:.4}",
            REGISTERS[k.0],
            k.1,
            c.early.count,
            c.early.median,
            c.early.q25,
            c.early.q75,
            c.late.median,
            c.late.q25,
            c.late.q75,
            c.knee.median,
            c.knee.q25,
            c.knee.q75
        );
    }
    s
}

/// T2b's remnant per capture and partial, as written to `fidelity_residual.csv`.
fn residual_csv(made: &[Made]) -> String {
    let mut s = String::from(
        "capture,key,register,n,f_hz,life_s,early_db_s,late_db_s,knee_s,glide_hz_s,orig_db,remnant_db,msc,frames\n",
    );
    for r in made.iter().flat_map(|x| x.remnants.iter()) {
        let _ = writeln!(
            s,
            "{},{},{},{},{:.3},{:.3},{:.3},{:.3},{:.4},{:.4},{:.2},{:.2},{:.4},{}",
            r.capture,
            r.key,
            REGISTERS[r.register],
            r.model.n,
            r.model.f,
            r.model.life_s,
            r.model.decay.early_db_s,
            r.model.decay.late_db_s,
            r.model.decay.knee_s,
            r.model.glide_hz_s,
            r.orig_db,
            r.remnant_db,
            r.msc,
            r.frames
        );
    }
    s
}

// ── plumbing ────────────────────────────────────────────────────────────────

/// `f` over `items` on every available core, the results in the items' order.
fn par_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let mut out: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                scope.spawn(|| {
                    let mut local = Vec::new();
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        if i >= items.len() {
                            break;
                        }
                        local.push((i, f(&items[i])));
                    }
                    local
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("a worker thread panicked"))
            .collect()
    });
    out.sort_by_key(|(i, _)| *i);
    out.into_iter().map(|(_, r)| r).collect()
}

/// Middle of the sorted values, the upper of the middle pair.
fn median(v: &mut [f32]) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

/// The largest power of two not above `n`, as `mat regen` sizes the Worker's FFT.
fn pow2_floor(n: usize) -> usize {
    if n == 0 {
        0
    } else {
        1 << (usize::BITS - 1 - n.leading_zeros())
    }
}

/// A dump directory's trailing serial: the order the session recorded it in.
fn serial_of(dir: &str) -> u64 {
    dir.rsplit('_')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// The generator's seed for one capture, so its draws do not depend on which
/// other captures run beside it.
fn capture_seed(seed: u64, serial: u64) -> u64 {
    seed ^ serial.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

/// A capture's `analysis.json` metadata, `Null` where there is none.
fn metadata(dir: &Path) -> Value {
    fs::read_to_string(dir.join("analysis.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .map_or(Value::Null, |j| j["metadata"].clone())
}

#[cfg(test)]
mod tests {
    use super::super::tone_amplitude;
    use super::*;

    fn white(rng: &mut Gaussian, sigma: f32, len: usize) -> Vec<f32> {
        (0..len).map(|_| sigma * rng.next()).collect()
    }

    /// `amp·g(t)·sin(2πft + phase)` added to `x` from its sample `from`.
    fn add_tone(x: &mut [f32], from: usize, f: f32, amp: f32, decay: Decay, phase: f64) {
        let fs = SAMPLE_RATE as f64;
        for (m, s) in x[from..].iter_mut().enumerate() {
            let t = m as f64 / fs;
            *s += (amp as f64 * decay.gain(t) * (TAU * f as f64 * t + phase).sin()) as f32;
        }
    }

    /// Screened silence over `x`'s first `pre` samples, every hop of it.
    fn silent(x: Vec<f32>, pre: usize) -> Screened {
        Screened {
            silent_ends: (BASS_WINDOW_SIZE..=pre).step_by(HOP_SIZE).collect(),
            lead: pre,
            full: x,
        }
    }

    /// The Worker reads a stationary tone on its bin at `A·Σw/2 ≈ A·N/4`, and
    /// the inverse recovers `A`; off its bin and decaying, the inverse carries
    /// the scalloping and the envelope and still recovers it.
    #[test]
    fn worker_reading_inverts_to_the_amplitude() {
        let n = 1 << 16;
        let hann = hann_vec(n);
        let mut planner = RealFftPlanner::<f32>::new();
        let a = 0.01f32;
        let f = 1000.0 * SAMPLE_RATE as f32 / n as f32;
        let mut tone = vec![0.0f32; n];
        add_tone(&mut tone, 0, f, a, Decay::single(0.0), 0.3);
        let reading = worker_reading(&tone, &[f], &mut planner)[0];
        assert!(
            (reading / (a * n as f32 / 4.0) - 1.0).abs() < 1e-3,
            "{reading}"
        );
        let back = onset_amplitude(reading, f, Decay::single(0.0), &hann) as f32;
        assert!((back / a - 1.0).abs() < 1e-4, "{back}");

        let decay = Decay {
            early_db_s: -30.0,
            late_db_s: -6.0,
            knee_s: 0.3,
        };
        let f = 1000.37 * SAMPLE_RATE as f32 / n as f32;
        let mut tone = vec![0.0f32; n];
        add_tone(&mut tone, 0, f, a, decay, 1.1);
        let reading = worker_reading(&tone, &[f], &mut planner)[0];
        let back = onset_amplitude(reading, f, decay, &hann) as f32;
        assert!((back / a - 1.0).abs() < 2e-3, "{back}");
    }

    /// Uncorrelated segments tiled with the crossfade keep their mean power,
    /// over the whole bed and across the seams alone.
    #[test]
    fn crossfade_tiling_keeps_the_mean_power() {
        let mut rng = Gaussian::new(7);
        let segments: Vec<Vec<f32>> = (0..40)
            .map(|i| white(&mut rng, 1e-3, 9000 + 137 * i))
            .collect();
        let slices: Vec<&[f32]> = segments.iter().map(|s| s.as_slice()).collect();
        let len = 300_000;
        let (bed, taken) = tile(&slices, len).expect("the segments cover the bed");
        assert_eq!(bed.len(), len);
        let power = |v: &[f32]| v.iter().map(|&x| (x as f64).powi(2)).sum::<f64>() / v.len() as f64;
        let used: Vec<f32> = slices
            .iter()
            .zip(&taken)
            .flat_map(|(s, &t)| s[..t].iter().copied())
            .collect();
        let whole = power(&bed) / power(&used);
        assert!((whole - 1.0).abs() < 0.01, "bed ÷ source power {whole}");
        let mut seams = Vec::new();
        let mut start = 0;
        for (i, &t) in taken.iter().enumerate().filter(|(_, t)| **t > 0) {
            if i > 0 {
                seams.extend_from_slice(&bed[start..start + CROSSFADE]);
            }
            start += t - CROSSFADE;
        }
        let at_seams = power(&seams) / power(&used);
        assert!(
            (at_seams - 1.0).abs() < 0.05,
            "seam ÷ source power {at_seams}"
        );
    }

    /// A partial decaying in two known stages in white noise: the track, cut at
    /// the room floor, gives back both slopes and the knee.
    #[test]
    fn two_stage_fit_recovers_a_known_decay() {
        let truth = Decay {
            early_db_s: -30.0,
            late_db_s: -6.0,
            knee_s: 0.4,
        };
        let mut rng = Gaussian::new(11);
        let pre = 16 * HOP_SIZE;
        let mut x = white(&mut rng, 1e-3, pre + 3 * SAMPLE_RATE as usize);
        let f = 523.3;
        add_tone(&mut x, pre, f, 0.1, truth, 0.0);
        let s = silent(x, pre);
        let hann = hann_vec(TRACK_WIN);
        let fit = decay_from_track(&track(&s.full[pre..], f, &hann), s.floor_at(f, &hann), true);
        let Fit::Decay(d) = fit else {
            panic!("no two-stage fit: {fit:?}");
        };
        assert!((d.early_db_s - truth.early_db_s).abs() < 1.0, "{d:?}");
        assert!((d.late_db_s - truth.late_db_s).abs() < 0.3, "{d:?}");
        assert!((d.knee_s - truth.knee_s).abs() < 0.03, "{d:?}");
    }

    /// The track subtracted as measured: each hop's phasor, demodulated, joined
    /// linearly between the hop centres.
    fn subtract_as_measured(x: &[f32], out: &mut [f32], onset: usize, f: f32, hann: &[f32]) {
        let hann_sum: f64 = hann.iter().map(|&w| w as f64).sum();
        let omega = TAU * f as f64 / SAMPLE_RATE as f64;
        let mut points = Vec::new();
        let mut end = onset + TRACK_WIN;
        while end <= x.len() {
            let z = phasor(&x[end - TRACK_WIN..end], f as f64, hann, hann_sum);
            let c = centre(end);
            points.push((c, z * Complex::from_polar(1.0, -omega * c)));
            end += HOP_SIZE;
        }
        for pair in points.windows(2) {
            let ((c0, b0), (c1, b1)) = (pair[0], pair[1]);
            for (m, o) in out
                .iter_mut()
                .enumerate()
                .take(c1.ceil() as usize)
                .skip(c0.ceil() as usize)
            {
                let b = b0 + (b1 - b0) * ((m as f64 - c0) / (c1 - c0));
                *o -= (b * Complex::from_polar(1.0, omega * m as f64)).re as f32;
            }
        }
    }

    /// The residual's level at f_n's 8192-point bin against the noise alone at
    /// the same bin, dB, over the frames after `pre`.
    fn against_the_noise(resid: &[f32], bed: &[f32], f: f32, pre: usize) -> f32 {
        let mut planner = RealFftPlanner::<f32>::new();
        let ends: Vec<usize> = (pre + BASS_WINDOW_SIZE..=resid.len())
            .step_by(HOP_SIZE)
            .collect();
        let b = (f / (SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32)).round() as usize;
        let mut r: Vec<f32> = spectra(resid, &ends, &mut planner)
            .iter()
            .map(|v| v[b].norm())
            .collect();
        let mut n: Vec<f32> = spectra(bed, &ends, &mut planner)
            .iter()
            .map(|v| v[b].norm())
            .collect();
        20.0 * (median(&mut r).unwrap() / median(&mut n).unwrap()).log10()
    }

    /// Why the residual's model is fitted: the per-hop track carries the noise
    /// inside the window's band with the partial, and subtracted as measured
    /// it cancels that noise and leaves a deep notch at f_n.
    #[test]
    fn subtracting_the_track_as_measured_notches_the_bed() {
        let sigma = 1e-3f32;
        let amp = tone_amplitude(sigma, 30.0, BASS_WINDOW_SIZE);
        let pre = 16 * HOP_SIZE;
        let hann = hann_vec(TRACK_WIN);
        let mut notch = Vec::new();
        for draw in 0..4u64 {
            let mut rng = Gaussian::new(200 + draw);
            let bed = white(&mut rng, sigma, pre + CAPTURE_DEFAULT_SAMPLES);
            let f = 1000.3 + 41.0 * draw as f32;
            let mut x = bed.clone();
            add_tone(&mut x, pre, f, amp, Decay::single(-6.0), 0.7);
            let mut out = x.clone();
            subtract_as_measured(&x, &mut out, pre, f, &hann);
            notch.push(against_the_noise(&out, &bed, f, pre));
        }
        assert!(notch.iter().all(|&v| v < -10.0), "{notch:?}");
    }

    /// One partial at 30 dB per 8192-point bin in white noise, decaying in two
    /// stages: after the model's subtraction the T2b remnant at f_n is under
    /// 1 dB, and the residual sits within the model's own notch of the noise at
    /// that bin, pooled over twelve draws (one draw's remnant scatters by about
    /// 1.2 dB, so the pair against the noise is the precise read).
    #[test]
    fn residual_of_one_partial_sits_at_the_noise() {
        let sigma = 1e-3f32;
        let amp = tone_amplitude(sigma, 30.0, BASS_WINDOW_SIZE);
        let decay = Decay {
            early_db_s: -12.0,
            late_db_s: -4.0,
            knee_s: 0.5,
        };
        let pre = 16 * HOP_SIZE;
        let len = CAPTURE_DEFAULT_SAMPLES;
        let hann = hann_vec(TRACK_WIN);
        let hann_sum: f64 = hann.iter().map(|&w| w as f64).sum();
        let mut planner = RealFftPlanner::<f32>::new();
        let ends: Vec<usize> = (pre + BASS_WINDOW_SIZE..=pre + len)
            .step_by(HOP_SIZE)
            .collect();
        let (mut remnant, mut against_noise, mut msc) = (Vec::new(), Vec::new(), Vec::new());
        for draw in 0..12u64 {
            let mut rng = Gaussian::new(100 + draw);
            let bed = white(&mut rng, sigma, pre + len);
            let f = 700.3 + 37.1 * draw as f32;
            let mut x = bed.clone();
            add_tone(&mut x, pre, f, amp, decay, 0.7 + draw as f64);
            let s = silent(x.clone(), pre);
            let ctx = ModelCtx {
                onset: pre,
                gate: sigma * spectral::neyman_pearson_k(TRACK_WIN),
                hann: &hann,
                hann_sum,
            };
            let mut out = x.clone();
            let floor = s.floor_at(f, &hann).expect("a pre-roll");
            subtract_partial(&x, &mut out, &ctx, 1, f, floor).expect("the partial is modelled");
            let so = spectra(&x, &ends, &mut planner);
            let sr = spectra(&out, &ends, &mut planner);
            let sn = spectra(&bed, &ends, &mut planner);
            let (_, rem, coherence) = remnant_at(&so, &sr, f, f - 150.0, f + 150.0).expect("gaps");
            remnant.push(rem);
            msc.push(coherence);
            let b = (f / (SAMPLE_RATE as f32 / BASS_WINDOW_SIZE as f32)).round() as usize;
            let mut r: Vec<f32> = sr.iter().map(|v| v[b].norm()).collect();
            let mut nz: Vec<f32> = sn.iter().map(|v| v[b].norm()).collect();
            against_noise.push(20.0 * (median(&mut r).unwrap() / median(&mut nz).unwrap()).log10());
        }
        let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
        let (rem, vs_noise) = (mean(&remnant), mean(&against_noise));
        assert!(rem < 1.0, "remnant {rem:.2} dB: {remnant:?}");
        assert!(
            vs_noise < 1.0 && vs_noise > -1.5,
            "against the noise {vs_noise:.2} dB: {against_noise:?}"
        );
        assert!(median(&mut msc).unwrap() < 0.1, "MSC {msc:?}");
    }
}
