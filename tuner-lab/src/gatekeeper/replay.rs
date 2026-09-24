//! One pass of a capture through the signal validator, and the thresholds
//! it runs at.
//!
//! The front end copies `AudioPipeline::process_cola_hop`'s: an 8192-sample window
//! advances one hop at a time, and the gate reads the newest 2048 samples and
//! their spectrum. `dump` tabulates the pass and `plot` draws it.

use std::fs;
use std::path::Path;

use realfft::RealFftPlanner;

use tuner_core::algorithms::spectral;
use tuner_core::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use tuner_core::gatekeeper::{GateResult, Gatekeeper, GatekeeperConfig};
use tuner_core::models::ProfileSettings;
use tuner_core::pipeline::ProcessingFrame;

/// The gate's verdict on one hop.
pub struct Hop {
    /// First sample of the window the gate analysed.
    pub window_start: usize,
    pub result: GateResult,
}

impl Hop {
    /// The sample at which the verdict exists: the end of its window.
    pub fn window_end(&self) -> usize {
        self.window_start + WINDOW_SIZE
    }
}

/// Runs `audio` through a fresh gate at `config`, one hop per `HOP_SIZE`,
/// starting at the first full 8192-sample window. Empty when `audio` is shorter
/// than one window.
pub fn run(audio: &[f32], config: GatekeeperConfig) -> Vec<Hop> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WINDOW_SIZE);
    let mut frame = ProcessingFrame::new();
    let mut gatekeeper = Gatekeeper::new();
    gatekeeper.config = config;

    let newest_start = BASS_WINDOW_SIZE - WINDOW_SIZE;
    let mut hops = Vec::new();
    let mut cursor = 0;
    while cursor + BASS_WINDOW_SIZE <= audio.len() {
        frame.audio_buffer[..BASS_WINDOW_SIZE]
            .copy_from_slice(&audio[cursor..cursor + BASS_WINDOW_SIZE]);
        spectral::fft(
            &frame.audio_buffer[newest_start..BASS_WINDOW_SIZE],
            &mut frame.time_buffer[..WINDOW_SIZE],
            &mut frame.frequency_buffer[..],
            &fft,
            WINDOW_SIZE,
        );
        hops.push(Hop {
            window_start: cursor + newest_start,
            result: gatekeeper.process_frame(&frame),
        });
        cursor += HOP_SIZE;
    }
    hops
}

/// Where a threshold came from.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A command-line flag.
    Override,
    /// The capture's own `analysis.json`.
    Logged,
    /// The instrument profile named on the command line.
    Profile,
    /// `GatekeeperConfig::default()`: nothing else supplied it.
    GateDefault,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::Override => "override",
            Source::Logged => "logged",
            Source::Profile => "profile",
            Source::GateDefault => "gate default, not logged",
        }
    }
}

/// What may replace or fill in a capture's thresholds.
#[derive(Default)]
pub struct Overrides {
    pub silence: Option<f32>,
    pub nhwrsf: Option<f32>,
    pub sustain: Option<f32>,
    /// Fills in a threshold the capture did not log. It has no silence
    /// threshold: that is the rig's, measured at each launch.
    pub profile: Option<ProfileSettings>,
}

/// The thresholds a capture's replay runs at, and where each came from.
pub struct Thresholds {
    pub config: GatekeeperConfig,
    pub silence: Source,
    pub nhwrsf: Source,
    pub sustain: Source,
}

impl Thresholds {
    /// Resolves each threshold for the capture in `dir`: a flag first, then what
    /// the capture logged, then the profile, then the gate's default. A logged
    /// value beats the profile because the log is what the capture ran at, and
    /// the profile is only what the instrument holds now.
    pub fn resolve(dir: &Path, overrides: &Overrides) -> Thresholds {
        let metadata = fs::read_to_string(dir.join("analysis.json"))
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .map(|json| json["metadata"].clone());
        let logged = |key: &str| {
            metadata
                .as_ref()
                .and_then(|m| m[key].as_f64())
                .map(|v| v as f32)
                .filter(|v| *v > 0.0)
        };
        let pick = |flag: Option<f32>, key: &str, profile: Option<f32>, default: f32| {
            if let Some(v) = flag {
                (v, Source::Override)
            } else if let Some(v) = logged(key) {
                (v, Source::Logged)
            } else if let Some(v) = profile {
                (v, Source::Profile)
            } else {
                (default, Source::GateDefault)
            }
        };

        let defaults = GatekeeperConfig::default();
        let profile = overrides.profile.as_ref();
        let (silence, silence_source) = pick(
            overrides.silence,
            "noise_floor",
            None,
            defaults.silence_threshold,
        );
        let (nhwrsf, nhwrsf_source) = pick(
            overrides.nhwrsf,
            "nhwrsf_threshold",
            profile.map(|p| p.nhwrsf_threshold),
            defaults.nhwrsf_threshold,
        );
        let (sustain, sustain_source) = pick(
            overrides.sustain,
            "sustain_stability_threshold",
            profile.map(|p| p.sustain_stability_threshold),
            defaults.sustain_stability_threshold,
        );

        Thresholds {
            config: GatekeeperConfig {
                silence_threshold: silence,
                nhwrsf_threshold: nhwrsf,
                sustain_stability_threshold: sustain,
                ..defaults
            },
            silence: silence_source,
            nhwrsf: nhwrsf_source,
            sustain: sustain_source,
        }
    }
}
