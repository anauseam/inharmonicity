//! Acquisition: the default input device, recorded through the app's own input
//! stream.
//!
//! `record` opens [`audio::open_input_stream`], the stream the app analyses (the
//! same device, the same negotiated rate, the same DC block), and writes what it
//! delivers as the pipeline would have received it: `audio.raw` in the capture
//! dumps' headerless `f32` form, with `recording.json` beside it. It serves the
//! noise-floor measurement, which needs minutes of the room (report 0015 §13).

use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Subcommand};
use cpal::traits::{DeviceTrait, HostTrait};
use ringbuf::traits::{Consumer, Observer};

use tuner_core::algorithms::metrics;
use tuner_core::audio::{self, SAMPLE_RATE};

use crate::raw;

/// The ring between the audio callback and the recording thread, in seconds of
/// audio. It fills only if the recording thread stalls this long, and a full ring
/// drops samples in the callback, which counts none.
const RING_SECONDS: usize = 10;

/// How often the recording thread drains the ring.
const DRAIN_INTERVAL: Duration = Duration::from_millis(50);

/// Seconds of audio per progress line.
const PROGRESS_SECONDS: usize = 10;

/// How long the stream may deliver nothing before the recording is abandoned. A
/// device delivers every few milliseconds, so a gap this long is a stopped
/// stream, not a slow one.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Args)]
pub struct Cmd {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Records the default input device through the app's own input stream to
    /// `audio.raw`, with `recording.json` beside it: the start, the duration, the
    /// device, the negotiated rate, the label, and when a drain found the ring
    /// full. Prints progress and each ten seconds' RMS on stderr.
    Record {
        /// Directory to write into: created if missing, refused if it already
        /// holds a recording.
        out_dir: PathBuf,
        /// Duration, in whole seconds.
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        seconds: u32,
        /// What the room was doing: ventilation, windows, traffic.
        #[arg(long)]
        label: Option<String>,
    },
}

pub fn run(cmd: Cmd) -> Result<()> {
    match cmd.mode {
        Mode::Record {
            out_dir,
            seconds,
            label,
        } => record(&out_dir, seconds, label.as_deref()),
    }
}

fn record(out_dir: &Path, seconds: u32, label: Option<&str>) -> Result<()> {
    let audio_path = out_dir.join("audio.raw");
    let json_path = out_dir.join("recording.json");
    if audio_path.exists() || json_path.exists() {
        bail!("{} already holds a recording", out_dir.display());
    }
    std::fs::create_dir_all(out_dir).with_context(|| format!("create {}", out_dir.display()))?;

    // Must name the device `open_input_stream` opens, the default host's default
    // input; if that function ever takes a device, pass the same one here.
    let device = cpal::default_host()
        .default_input_device()
        .ok_or_else(|| anyhow!("No input device available"))?
        .description()?;
    let (stream, mut consumer, rate) =
        audio::open_input_stream(RING_SECONDS * SAMPLE_RATE as usize)?;
    let start_unix_s = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();

    let total = seconds as usize * rate as usize;
    let stretch = PROGRESS_SECONDS * rate as usize;
    let mut samples = Vec::with_capacity(total);
    let mut ring_full_at_s = Vec::new();
    let mut reported = 0;
    let mut last_arrival = Instant::now();

    while samples.len() < total {
        thread::sleep(DRAIN_INTERVAL);
        let at_s = samples.len() as f64 / f64::from(rate);
        if consumer.is_full() {
            ring_full_at_s.push(at_s);
            eprintln!("ring full at {at_s:.1} s: samples may have been dropped");
        }

        let before = samples.len();
        samples.extend(consumer.pop_iter().take(total - before));
        if samples.len() > before {
            last_arrival = Instant::now();
        } else if last_arrival.elapsed() > STALL_TIMEOUT {
            bail!(
                "the stream delivered nothing for {} s at {at_s:.1} s; nothing written",
                STALL_TIMEOUT.as_secs()
            );
        }

        while samples.len() - reported >= stretch {
            let level = metrics::rms(&samples[reported..reported + stretch]);
            reported += stretch;
            eprintln!(
                "{:>6} / {seconds} s   RMS {:6.1} dBFS",
                reported / rate as usize,
                20.0 * f64::from(level).log10()
            );
        }
    }
    drop(stream);

    raw::write(&audio_path, &samples).with_context(|| format!("write {}", audio_path.display()))?;
    let recording = serde_json::json!({
        "start_unix_s": start_unix_s,
        "requested_seconds": seconds,
        "device": format!("{device:?}"),
        "sample_rate_hz": rate,
        "label": label,
        "ring_full_at_s": ring_full_at_s,
    });
    std::fs::write(&json_path, serde_json::to_string_pretty(&recording)?)
        .with_context(|| format!("write {}", json_path.display()))?;

    println!(
        "Wrote {seconds} s ({} samples at {rate} Hz) to {}",
        samples.len(),
        audio_path.display()
    );
    if ring_full_at_s.is_empty() {
        println!("The ring was never full.");
    } else {
        println!(
            "The ring was found full at {} drains; samples may be missing \
             (ring_full_at_s in recording.json).",
            ring_full_at_s.len()
        );
    }
    Ok(())
}
