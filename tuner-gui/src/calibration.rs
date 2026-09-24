//! # Threshold calibration
//!
//! The state behind the three gate-threshold panels, and its per-tick logic: the
//! silence calibration, which sets the silence threshold from the loudest RMS the
//! room shows over a short window, and the onset and sustain scopes, which trace
//! their metric against the threshold the operator sets.

use std::collections::VecDeque;

use tuner_core::audio::HOP_RATE_HZ;

use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;

/// Margin over the loudest RMS seen: the silence threshold is that RMS times this.
pub const DEFAULT_NOISE_MULTIPLIER: f32 = 1.5;

/// How long to ignore input before measuring, so the measurement outlasts the
/// EMA decay of the interface's start-up artifacts.
const WARMUP_SECS: f32 = 1.0;

/// How long to read the room's RMS for.
const CALIBRATION_SECS: f32 = 2.0;

/// [`WARMUP_SECS`] in hops.
const WARMUP_FRAMES: u32 = (WARMUP_SECS * HOP_RATE_HZ) as u32;

/// [`CALIBRATION_SECS`] in hops.
pub const CALIBRATION_FRAMES: u32 = (CALIBRATION_SECS * HOP_RATE_HZ) as u32;

/// Ticks the scope keeps scrolling after a strike clears the threshold before
/// it freezes: ≈ 1.5 s at the 16 ms tick, so the whole strike is on screen.
const FREEZE_AFTER_TICKS: u32 = 90;

#[derive(Debug, Clone)]
struct RmsCalibrationState {
    warmup_hops: Option<u32>,
    countdown: Option<u32>,
    max_seen_rms: f32,
}

impl RmsCalibrationState {
    /// A calibration at its start: the warm-up, then the measuring window.
    fn new() -> Self {
        Self {
            warmup_hops: Some(WARMUP_FRAMES),
            countdown: Some(CALIBRATION_FRAMES),
            max_seen_rms: 0.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NoiseFloorSettings {
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
    calibration_complete: bool,
    pub visible: bool,
    active_calibration: Option<RmsCalibrationState>,
}

impl NoiseFloorSettings {
    /// Restarts the silence calibration from its warm-up.
    pub fn recalibrate(&mut self) {
        self.calibration_complete = false;
        self.active_calibration = Some(RmsCalibrationState::new());
    }

    /// The silence calibration has not yet set a threshold.
    pub fn is_calibrating(&self) -> bool {
        !self.calibration_complete
    }

    /// Hops the calibration in progress has measured, out of
    /// [`CALIBRATION_FRAMES`]; `None` once it has completed.
    pub fn calibration_progress(&self) -> Option<usize> {
        let countdown = self.active_calibration.as_ref()?.countdown?;
        Some(CALIBRATION_FRAMES.saturating_sub(countdown) as usize)
    }
}

#[derive(Debug, Clone)]
pub struct TransientSettings {
    pub visible: bool,
    pub is_frozen: bool,
    pub freeze_countdown: Option<u32>,
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
}

#[derive(Debug, Clone)]
pub struct SustainSettings {
    pub visible: bool,
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
}

#[derive(Debug, Clone)]
pub struct SettingsDisplayData {
    pub rms: NoiseFloorSettings,
    pub transient: TransientSettings,
    pub sustain: SustainSettings,
}

impl Default for SettingsDisplayData {
    /// Every panel hidden, and the silence calibration pending.
    fn default() -> Self {
        Self {
            rms: NoiseFloorSettings {
                history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                current_threshold: 0.005,
                calibration_complete: false,
                visible: false,
                active_calibration: Some(RmsCalibrationState::new()),
            },
            transient: TransientSettings {
                visible: false,
                is_frozen: false,
                freeze_countdown: None,
                history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                current_threshold: 0.5,
            },
            sustain: SustainSettings {
                visible: false,
                history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                current_threshold: 10.0,
            },
        }
    }
}

/// Advances the silence calibration by one tick, returning the silence
/// threshold on the tick it completes, which also marks it complete.
pub fn process_silence_tick(
    settings: &mut NoiseFloorSettings,
    current_rms: f32,
    new_frame_arrived: bool,
) -> Option<f32> {
    // Only a new frame advances it.
    if !new_frame_arrived {
        return None;
    }

    let active = settings.active_calibration.as_mut()?;

    // Still warming up: count the hop down without measuring it.
    if let Some(mut warmup) = active.warmup_hops {
        if warmup == 0 {
            active.warmup_hops = None;
        } else {
            warmup -= 1;
            active.warmup_hops = Some(warmup);
        }
        return None;
    }

    // Counted in hops, not ticks, so the window is the same on any machine.
    if let Some(mut countdown) = active.countdown {
        if current_rms > active.max_seen_rms {
            active.max_seen_rms = current_rms;
        }

        if countdown == 0 {
            active.countdown = None;
            let final_threshold = active.max_seen_rms * DEFAULT_NOISE_MULTIPLIER;
            settings.calibration_complete = true;
            return Some(final_threshold);
        } else {
            countdown -= 1;
            active.countdown = Some(countdown);
        }
    }

    None
}

pub fn process_transient_tick(settings: &mut TransientSettings, flux: f32, current_threshold: f32) {
    if settings.is_frozen {
        return;
    }

    settings.history.push_back(flux);
    if settings.history.len() > ENVELOPE_HISTORY_LENGTH {
        settings.history.pop_front();
    }

    if let Some(mut countdown) = settings.freeze_countdown {
        if countdown == 0 {
            settings.is_frozen = true;
            settings.freeze_countdown = None;
        } else {
            countdown -= 1;
            settings.freeze_countdown = Some(countdown);
        }
    } else if flux >= current_threshold {
        settings.freeze_countdown = Some(FREEZE_AFTER_TICKS);
    }
}

pub fn process_sustain_tick(settings: &mut SustainSettings, sustain_stability: f32) {
    settings.history.push_back(sustain_stability);
    if settings.history.len() > ENVELOPE_HISTORY_LENGTH {
        settings.history.pop_front();
    }
}
