//! Harnesses for the strobe bank: rotation, the unison lines, and the number
//! the panel displays.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Subcommand};

use tuner_core::algorithms::peaks::MAX_UNISON_LINES;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_RATE_HZ, HOP_SIZE, SAMPLE_RATE};
use tuner_core::models::UnisonLine;
use tuner_core::strobe::unison::UnisonVerdict;
use tuner_core::strobe::{MAX_STROBE_REFS, Strobe, StrobeRefUpdate};

mod isolation;
mod readout;
mod replay;

/// What one reference published at the end of a run.
#[derive(Clone, Copy, Default)]
pub struct Resolved {
    pub count: u8,
    pub resolution_hz: f32,
    pub lines: [UnisonLine; MAX_UNISON_LINES],
    /// Hop the record ended on, so a caller can rebuild the exact span it
    /// covered.
    pub hop: usize,
    /// Record length in hops at that point.
    pub record: usize,
}

/// Drives the shipped bank over `audio` and returns, per reference, the
/// best record it reached — the hop with the longest unbroken record, which is
/// what the panel would be showing when the tuner looks at it.
pub fn run_unison(
    audio: &[f32],
    refs: &[f32; MAX_STROBE_REFS],
    count: usize,
    spacing_hz: f32,
    noise_floor: f32,
) -> (Vec<Resolved>, UnisonVerdict) {
    let mut strobe = Strobe::new(SAMPLE_RATE);
    strobe.retarget(StrobeRefUpdate {
        count,
        refs: *refs,
        coarse_index: 0, // phase/baseband only; the coarse read is E1–E5's business
        spacing_hz,
    });

    let hops = (audio.len().saturating_sub(BASS_WINDOW_SIZE)) / HOP_SIZE;
    let mut best = vec![Resolved::default(); count];
    let mut verdict = UnisonVerdict::Undetermined;
    let mut frame = tuner_core::pipeline::ProcessingFrame::new();
    let mut record = vec![0usize; count];
    for h in 0..hops {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[h * HOP_SIZE..h * HOP_SIZE + BASS_WINDOW_SIZE]);
        let out = strobe.process(&frame, noise_floor, false);
        for i in 0..count {
            // The published resolution is 2·f_hop/L, so it is the record length.
            record[i] = if out.line_resolution_hz[i] > 0.0 {
                (2.0 * HOP_RATE_HZ / out.line_resolution_hz[i]).round() as usize
            } else {
                0
            };
            if record[i] > best[i].record {
                best[i] = Resolved {
                    count: out.line_count[i],
                    resolution_hz: out.line_resolution_hz[i],
                    lines: out.lines[i],
                    hop: h,
                    record: record[i],
                };
                verdict = out.verdict;
            }
        }
    }
    (best, verdict)
}

/// Options shared by the readout modes, all defaulting to the shipped value.
#[derive(Args, Clone)]
pub struct ReadOpts {
    /// Capture-set root, or a single capture directory.
    #[arg(default_value = "diagnostics")]
    pub root: PathBuf,
    /// Only these keys, e.g. `19,24,29`.
    #[arg(long, value_delimiter = ',')]
    pub keys: Option<Vec<u8>>,
    /// Coarse-read search band, ± cents.
    #[arg(long, default_value_t = 100.0)]
    pub span: f32,
    /// Search-band floor, in bins.
    #[arg(long, default_value_t = 4.0)]
    pub min_bins: f32,
}

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// The shipped bank over real captures: detuning coherence (E1), the bass
    /// window A/B (E2), per-hop delta noise (E3), fit-window jitter versus
    /// motion lag (E4) and the shipped rate against an independent refit (E5);
    /// the unison experiments (E6–E8) and the bass attribution (E10–E12).
    /// E1–E5 are report 0011's and must not move.
    Replay {
        /// Capture-set root.
        #[arg(default_value = "diagnostics")]
        root: PathBuf,
    },
    /// The unison panel against isolation truth on the mute-isolation set:
    /// the false-beat positive control, availability per register, and a JSON
    /// dump of per-capture line positions (report 0014 §§3–5).
    Isolation {
        /// A `mat regen` dump of the isolation set.
        regen: PathBuf,
        /// That set's dump directory.
        root: PathBuf,
        /// Write per-capture line positions here, for `isolation_truth.py`.
        #[arg(long)]
        json: Option<PathBuf>,
        /// Admit captures carrying no `sounding_strings` declaration.
        #[arg(long)]
        all: bool,
        /// Override the amplitude gate's ambient-silence RMS.
        #[arg(long, default_value_t = 3e-3)]
        noise_floor: f32,
    },
    /// The shipped readout against the hi-res DFT truth and YIN, per capture.
    Truth(ReadOpts),
    /// Estimator bias of the reference itself, on synthetic tones of known
    /// pitch. Any nonzero here is bias, not a reading.
    Selftest,
    /// YIN's sharpness against inharmonicity B and partial richness.
    Inharm,
    /// The bounded read swept across detuning, synthetic.
    Detune {
        #[command(flatten)]
        opts: ReadOpts,
        /// Flank-floor minimum, Hz.
        #[arg(long)]
        flank_hz: Option<f32>,
    },
    /// Per-hop reading either side of the phase-vocoder alias boundary.
    Alias(ReadOpts),
    /// Longest ungated run and the band read, per fit-window length.
    Window(ReadOpts),
    /// Tracker as-is / tracker + long window / bounded spectral peak.
    Readout(ReadOpts),
    /// Whether the band/coarse regime switch chatters near its boundary.
    Chatter(ReadOpts),
    /// Fixed n* (register table) against strongest-margin-per-hop.
    Policy {
        #[command(flatten)]
        opts: ReadOpts,
        /// Reference the capture's own measured B rather than the Rigaud prior.
        #[arg(long)]
        measured_b: bool,
    },
    /// The fixed-n register table re-run on the capture's own measured B.
    FixedN(ReadOpts),
    /// Partial-centered bass read at each partial's prior-B target.
    BassPartials(ReadOpts),
    /// How far off pitch the coarse read still reads.
    Reach(ReadOpts),
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Replay { root } => replay::run(&root),
        Mode::Isolation {
            regen,
            root,
            json,
            all,
            noise_floor,
        } => {
            isolation::run(&regen, &root, json.as_deref(), all, noise_floor);
            Ok(())
        }
        Mode::Truth(o) => readout::truth(&o),
        Mode::Selftest => readout::selftest_mode(),
        Mode::Inharm => readout::inharm_mode(),
        Mode::Detune { opts, flank_hz } => readout::detune_mode(&opts, flank_hz),
        Mode::Alias(o) => readout::alias(&o),
        Mode::Window(o) => readout::window(&o),
        Mode::Readout(o) => readout::readout(&o),
        Mode::Chatter(o) => readout::chatter(&o),
        Mode::Policy { opts, measured_b } => readout::policy(&opts, measured_b),
        Mode::FixedN(o) => readout::fixed_n(&o),
        Mode::BassPartials(o) => readout::bass_partials_mode(&o),
        Mode::Reach(o) => readout::reach(&o),
    }
}
