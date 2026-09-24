//! The gate's verdict on a capture, drawn.
//!
//! The waveform is shaded by gate state, and beneath it NHWRSF, RMS and sustain
//! stability each run against their threshold. Each hop's verdict is drawn from
//! the end of the window it analysed, the moment the gate could first give it,
//! until the next verdict. The header says where each threshold came from. A set
//! also gets the wait from onset to `Stable`, per register.

use std::path::Path;

use anyhow::{Result, anyhow};
use plotters::prelude::*;

use tuner_core::audio::{HOP_SIZE, SAMPLE_RATE};
use tuner_core::gatekeeper::{GatekeeperConfig, SignalState};
use tuner_core::models;

use super::replay::{self, Hop, Overrides, Thresholds};
use crate::figure::{self, Canvas, draw_error};
use crate::{capture, raw};

/// Image size in pixels.
const SIZE: (u32, u32) = (1800, 1800);

/// Bottom of the RMS axis, in dBFS: below any room a capture was taken in, so the
/// silence threshold always lands on the axis.
const DB_FLOOR: f64 = -90.0;

const UNSTABLE: RGBColor = RGBColor(0xE7, 0x4C, 0x3C);
const STABLE: RGBColor = RGBColor(0x27, 0xAE, 0x60);
const NHWRSF: RGBColor = RGBColor(0x29, 0x80, 0xB9);
const RMS: RGBColor = RGBColor(0xE6, 0x7E, 0x22);
const SUSTAIN: RGBColor = RGBColor(0x8E, 0x44, 0xAD);

/// How long the gate held a note between its onset and its first `Stable`
/// verdict.
#[derive(Clone, Copy)]
enum Wait {
    /// Samples from the onset verdict to the `Stable` one.
    Stable(usize),
    NeverStable,
    NoOnset,
}

impl Wait {
    fn of(hops: &[Hop]) -> Wait {
        let Some(onset) = hops.iter().position(|h| h.result.is_new_onset) else {
            return Wait::NoOnset;
        };
        match hops[onset + 1..]
            .iter()
            .find(|h| h.result.state == SignalState::Stable)
        {
            Some(stable) => Wait::Stable(stable.window_end() - hops[onset].window_end()),
            None => Wait::NeverStable,
        }
    }

    fn describe(self) -> String {
        match self {
            Wait::Stable(samples) => format!("{:.0} ms", figure::ms(samples, SAMPLE_RATE)),
            Wait::NeverStable => "never Stable".to_string(),
            Wait::NoOnset => "no onset".to_string(),
        }
    }
}

/// One capture's result, kept for the summary.
struct Drawn {
    name: String,
    key: Option<u8>,
    wait: Wait,
}

pub fn run(path: &Path, out: Option<&Path>, overrides: &Overrides) -> Result<()> {
    let captures = if is_capture(path) {
        vec![path.to_path_buf()]
    } else {
        capture::find(path)?
    };
    if captures.is_empty() {
        return Err(anyhow!("no captures under {}", path.display()));
    }
    if let Some(out) = out {
        std::fs::create_dir_all(out)?;
    }

    let mut drawn = Vec::with_capacity(captures.len());
    for dir in &captures {
        let name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("capture")
            .to_string();
        // The full event carries the strike; `audio.raw` begins at `Stable`.
        let Some(audio) = raw::full_event(dir).or_else(|| raw::stable(dir)) else {
            eprintln!("skipped {name}: no audio");
            continue;
        };
        let thresholds = Thresholds::resolve(dir, overrides);
        let hops = replay::run(&audio, thresholds.config.clone());
        if hops.is_empty() {
            eprintln!("skipped {name}: shorter than one analysis window");
            continue;
        }
        let key = capture::key_of(dir).or_else(|| capture::key_from_dirname(&name));
        let wait = Wait::of(&hops);
        let png = match out {
            Some(out) => out.join(format!("{name}.png")),
            None => dir.join("gatekeeper.png"),
        };
        draw(&png, &name, key, &audio, &hops, &thresholds, wait)?;
        drawn.push(Drawn { name, key, wait });
    }

    match out {
        Some(out) => println!("Wrote {} images to {}", drawn.len(), out.display()),
        None => println!("Wrote {} images beside their captures", drawn.len()),
    }
    summarise(&drawn);
    Ok(())
}

/// A single capture rather than a set: it holds the audio itself.
fn is_capture(path: &Path) -> bool {
    path.join("audio.raw").exists() || path.join("audio_full_event.raw").exists()
}

/// When `hop`'s verdict exists, in ms from the start of the audio.
fn at(hop: &Hop) -> f64 {
    figure::ms(hop.window_end(), SAMPLE_RATE)
}

/// Gate state as a shade; silence is left clear.
fn shade(state: SignalState) -> Option<RGBColor> {
    match state {
        SignalState::Silence => None,
        SignalState::Unstable => Some(UNSTABLE),
        SignalState::Stable => Some(STABLE),
    }
}

/// A verdict's span, and where the state changes.
struct Timeline {
    /// Each verdict holds from the end of its window until the next verdict.
    spans: Vec<(f64, f64, SignalState)>,
    changes: Vec<(f64, SignalState)>,
}

impl Timeline {
    fn of(hops: &[Hop]) -> Timeline {
        let spans: Vec<(f64, f64, SignalState)> = hops
            .iter()
            .enumerate()
            .map(|(i, hop)| {
                let from = at(hop);
                let to = hops
                    .get(i + 1)
                    .map(at)
                    .unwrap_or(from + figure::ms(HOP_SIZE, SAMPLE_RATE));
                (from, to, hop.result.state)
            })
            .collect();
        let changes = spans
            .windows(2)
            .filter(|pair| pair[0].2 != pair[1].2)
            .map(|pair| (pair[1].0, pair[1].2))
            .collect();
        Timeline { spans, changes }
    }

    /// Marks each change of state down `chart`, from `bottom` to `top`.
    fn mark(&self, chart: &mut figure::Chart, bottom: f64, top: f64) -> Result<()> {
        for &(t, state) in &self.changes {
            if let Some(color) = shade(state) {
                chart
                    .draw_series(figure::dashed((t, bottom), (t, top), color))
                    .map_err(draw_error)?;
            }
        }
        Ok(())
    }
}

fn draw(
    png: &Path,
    name: &str,
    key: Option<u8>,
    audio: &[f32],
    hops: &[Hop],
    thresholds: &Thresholds,
    wait: Wait,
) -> Result<()> {
    let canvas = figure::canvas(png, SIZE)?;
    let (header, body) = canvas.split_vertically(100);
    let (wave, rest) = body.split_vertically(580);
    let (onset, rest) = rest.split_vertically(370);
    let (level, sustain) = rest.split_vertically(370);

    let note = match key {
        Some(k) => format!("{} (key {k})", models::find_nearest_note_by_index(k).0),
        None => "unknown key".to_string(),
    };
    let config = &thresholds.config;
    let settings = format!(
        "silence {:.4} ({}) · NHWRSF {:.3} ({}) · sustain {:.1} ({}) · onset to Stable: {}",
        config.silence_threshold,
        thresholds.silence.label(),
        config.nhwrsf_threshold,
        thresholds.nhwrsf.label(),
        config.sustain_stability_threshold,
        thresholds.sustain.label(),
        wait.describe(),
    );
    header
        .draw_text(
            &format!("{note} · {name}"),
            &(figure::FONT, figure::TITLE)
                .into_font()
                .color(&figure::INK),
            (24, 16),
        )
        .map_err(draw_error)?;
    header
        .draw_text(
            &settings,
            &(figure::FONT, figure::SUBTITLE)
                .into_font()
                .color(&figure::INK),
            (24, 62),
        )
        .map_err(draw_error)?;

    let end = figure::ms(audio.len(), SAMPLE_RATE);
    let timeline = Timeline::of(hops);
    draw_waveform(&wave, audio, &timeline, end)?;
    draw_onset(&onset, hops, config, &timeline, end)?;
    draw_level(&level, hops, config, &timeline, end)?;
    draw_sustain(&sustain, hops, config, &timeline, end)?;
    canvas.present().map_err(draw_error)?;
    Ok(())
}

fn draw_waveform(area: &Canvas, audio: &[f32], timeline: &Timeline, end: f64) -> Result<()> {
    let peak = audio.iter().fold(1e-6_f32, |m, s| m.max(s.abs())) as f64 * 1.05;
    let mut chart = figure::time_chart(
        area,
        "Waveform and gate state",
        end,
        -peak..peak,
        "Amplitude",
        1,
        false,
    )?;

    let shaded = || {
        timeline
            .spans
            .iter()
            .filter_map(|&(from, to, state)| shade(state).map(|c| (from, to, c)))
    };
    chart
        .draw_series(
            shaded().map(|(from, to, c)| {
                Rectangle::new([(from, -peak), (to, peak)], c.mix(0.22).filled())
            }),
        )
        .map_err(draw_error)?;
    // A hairline at each verdict, so the hops a state lasted can be counted.
    chart
        .draw_series(
            shaded().map(|(from, _, c)| {
                PathElement::new(vec![(from, -peak), (from, peak)], c.mix(0.5))
            }),
        )
        .map_err(draw_error)?;
    figure::waveform(&mut chart, audio, SAMPLE_RATE, figure::WAVE)?;

    for (label, color) in [("Unstable", UNSTABLE), ("Stable", STABLE)] {
        chart
            .draw_series(std::iter::empty::<Rectangle<(f64, f64)>>())
            .map_err(draw_error)?
            .label(label)
            .legend(figure::block_swatch(color.mix(0.35)));
    }
    figure::legend(&mut chart)
}

fn draw_onset(
    area: &Canvas,
    hops: &[Hop],
    config: &GatekeeperConfig,
    timeline: &Timeline,
    end: f64,
) -> Result<()> {
    let threshold = config.nhwrsf_threshold as f64;
    let top = hops
        .iter()
        .map(|h| h.result.nhwrsf as f64)
        .fold(threshold, f64::max)
        * 1.15;
    let mut chart = figure::time_chart(area, "Onset", end, 0.0..top, "NHWRSF", 1, false)?;
    timeline.mark(&mut chart, 0.0, top)?;
    chart
        .draw_series(LineSeries::new(
            hops.iter().map(|h| (at(h), h.result.nhwrsf as f64)),
            NHWRSF.stroke_width(3),
        ))
        .map_err(draw_error)?
        .label("NHWRSF")
        .legend(figure::line_swatch(NHWRSF));
    chart
        .draw_series(figure::dashed((0.0, threshold), (end, threshold), NHWRSF))
        .map_err(draw_error)?
        .label(format!("onset threshold {threshold:.3}"))
        .legend(figure::dashed_swatch(NHWRSF));
    figure::legend(&mut chart)
}

fn draw_level(
    area: &Canvas,
    hops: &[Hop],
    config: &GatekeeperConfig,
    timeline: &Timeline,
    end: f64,
) -> Result<()> {
    let mut chart = figure::time_chart(area, "Level", end, DB_FLOOR..0.0, "RMS (dBFS)", 0, false)?;
    timeline.mark(&mut chart, DB_FLOOR, 0.0)?;
    let silence = figure::dbfs(config.silence_threshold, DB_FLOOR);
    chart
        .draw_series(LineSeries::new(
            hops.iter()
                .map(|h| (at(h), figure::dbfs(h.result.rms_ema, DB_FLOOR))),
            RMS.stroke_width(3),
        ))
        .map_err(draw_error)?
        .label("RMS, smoothed")
        .legend(figure::line_swatch(RMS));
    chart
        .draw_series(figure::dashed((0.0, silence), (end, silence), RMS))
        .map_err(draw_error)?
        .label(format!(
            "silence threshold {:.4} ({silence:.0} dBFS)",
            config.silence_threshold
        ))
        .legend(figure::dashed_swatch(RMS));
    figure::legend(&mut chart)
}

fn draw_sustain(
    area: &Canvas,
    hops: &[Hop],
    config: &GatekeeperConfig,
    timeline: &Timeline,
    end: f64,
) -> Result<()> {
    let threshold = config.sustain_stability_threshold as f64;
    let top = hops
        .iter()
        .map(|h| {
            h.result
                .sustain_stability_ema
                .max(h.result.sustain_stability_raw) as f64
        })
        .fold(threshold * 2.0, f64::max)
        * 1.08;
    let mut chart = figure::time_chart(
        area,
        "Sustain stability",
        end,
        0.0..top,
        "Sustain stability",
        0,
        true,
    )?;
    timeline.mark(&mut chart, 0.0, top)?;
    let faint = SUSTAIN.mix(0.35);
    chart
        .draw_series(LineSeries::new(
            hops.iter()
                .map(|h| (at(h), h.result.sustain_stability_raw as f64)),
            faint.stroke_width(2),
        ))
        .map_err(draw_error)?
        .label("per hop")
        .legend(figure::line_swatch(faint));
    chart
        .draw_series(LineSeries::new(
            hops.iter()
                .map(|h| (at(h), h.result.sustain_stability_ema as f64)),
            SUSTAIN.stroke_width(3),
        ))
        .map_err(draw_error)?
        .label("smoothed, which the gate compares")
        .legend(figure::line_swatch(SUSTAIN));
    chart
        .draw_series(figure::dashed((0.0, threshold), (end, threshold), SUSTAIN))
        .map_err(draw_error)?
        .label(format!("stability threshold {threshold:.1}"))
        .legend(figure::dashed_swatch(SUSTAIN));
    figure::legend(&mut chart)
}

/// The wait from onset to `Stable`, per register, and the captures that never
/// reached it.
fn summarise(drawn: &[Drawn]) {
    println!("\nOnset to Stable:");
    println!(
        "{:<8} {:>5} {:>10} {:>7} {:>7} {:>9} {:>13}",
        "register", "n", "median ms", "min ms", "max ms", "no onset", "never Stable"
    );
    let row = |label: &str, waits: Vec<Wait>| {
        let mut ms: Vec<f64> = waits
            .iter()
            .filter_map(|w| match w {
                Wait::Stable(samples) => Some(figure::ms(*samples, SAMPLE_RATE)),
                _ => None,
            })
            .collect();
        ms.sort_by(f64::total_cmp);
        let no_onset = waits.iter().filter(|w| matches!(w, Wait::NoOnset)).count();
        let never = waits
            .iter()
            .filter(|w| matches!(w, Wait::NeverStable))
            .count();
        let stat = |v: Option<&f64>| v.map_or("—".to_string(), |x| format!("{x:.0}"));
        println!(
            "{:<8} {:>5} {:>10} {:>7} {:>7} {:>9} {:>13}",
            label,
            waits.len(),
            stat(ms.get(ms.len() / 2)),
            stat(ms.first()),
            stat(ms.last()),
            no_onset,
            never
        );
    };
    for register in capture::CURVE_REGISTERS {
        let waits: Vec<Wait> = drawn
            .iter()
            .filter(|d| d.key.map(capture::curve_register) == Some(register))
            .map(|d| d.wait)
            .collect();
        if !waits.is_empty() {
            row(register, waits);
        }
    }
    row("all", drawn.iter().map(|d| d.wait).collect());

    for d in drawn {
        match d.wait {
            Wait::NoOnset => println!("  no onset:     {}", d.name),
            Wait::NeverStable => println!("  never Stable: {}", d.name),
            Wait::Stable(_) => {}
        }
    }
}
