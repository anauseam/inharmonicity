//! # Inharmonicity - Professional Piano Tuning GUI
//!
//! This module contains the main GUI application for the Inharmonicity piano tuning software.
//! It provides a real-time interface for audio analysis, spectrogram visualization, and
//! interactive piano keyboard controls.
//!
//! ## Architecture
//! - **Main Thread**: Iced GUI application with dark theme
//! - **Audio Thread**: Dedicated thread for real-time audio processing
//! - **Communication**: Wait-free SPSC primitives (rtrb + triple_buffer + atomics)
//! - **Updates**: 60 FPS continuous updates via subscription system

use crate::views::{main_view::create_main_view, settings_view::create_settings_view};
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use iced::{self, Element, Subscription, Theme};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use tuner_core::{
    FrameOutput,
    algorithms::tuning,
    audio::{self, AudioSource, HostHandle},
    models::{InharmonicityProfile, KeyMeasurement},
    pipeline::{CaptureState, PipelineHandle, load_f32, store_f32},
};

// Audio processing constants
const SMOOTHING_FACTOR: usize = 5; // Number of samples for cent smoothing

/// Main entry point for the Inharmonicity application.
///
/// Initializes the Iced GUI application with dark theme, real-time audio processing,
/// and continuous updates for smooth visualization.
pub fn main() -> iced::Result {
    eprintln!("[MAIN] Starting Inharmonicity application...");
    eprintln!("[MAIN] Initializing GUI framework...");
    let result = iced::application(TunerApp::new, TunerApp::update, TunerApp::view)
        .title("Inharmonicity")
        .subscription(TunerApp::subscription)
        .theme(TunerApp::theme)
        // Disable instant exit on close to allow the audio thread
        // to cleanly join/drop without causing a CPAL/ALSA segfault.
        .window(iced::window::Settings {
            exit_on_close_request: false,
            ..Default::default()
        })
        .run();
    eprintln!("[MAIN] Application finished with result: {:?}", result);
    result
}

/// Application message types for the Iced GUI framework.
///
/// These messages are sent between the GUI and the application logic to handle
/// user interactions, audio processing updates, and tool visibility toggles.
#[derive(Debug, Clone)]
pub enum Message {
    // Piano keyboard interactions
    KeySelected(u8),  // User selected a piano key (0-87)
    SwitchToAutoMode, // Switch from manual to automatic pitch detection

    // --- Messages for Inharmonicity Measurement & Profile ---
    ToggleMeasurementMode, // Toggle the partial measurement mode
    CaptureButtonClicked,  // Capture button was clicked (behavior depends on current state)
    UndoLastCapture,       // Reverts the last target key overwrite (Manual or Auto)
    SaveProfile,           // Save the current inharmonicity profile
    LoadProfile,           // Load an inharmonicity profile from file
    // ----------------------------------------------

    // Settings menu items (placeholder for future implementation)
    Temperament,     // Temperament selection
    TuningStandard,  // Tuning standard (A440, etc.)
    InharmonicCurve, // Inharmonicity curve adjustment
    SampleBuffer,    // Sample buffer size adjustment
    TuningProfile,   // Tuning profile management

    // Application control
    Exit, // Application exit request

    // Working tool visibility toggles
    ToggleSpectrogram, // Show/hide spectrogram panel
    ToggleCentMeter,   // Show/hide cent meter panel
    ToggleKeySelect,   // Show/hide piano keyboard
    TogglePartials,    // Show/hide partials panel
    // ToggleInharmonicityGraph, // Show/hide inharmonicity graph

    // Settings view toggles
    ToggleSettingsView,           // Toggle main view versus settings view
    ToggleNoiseFloorAdjustment,   // Show/hide noise floor envelope viewer
    SilenceThresholdChanged(f32), // User dragged the silence threshold slider
    RecalibrateNoiseFloor,        // User clicked the recalibrate button

    // --- Transient Calibration Messages ---
    ToggleTransientCalibration,
    ResetTransientScope,
    NhwrsfThresholdChanged(f32),

    // Continuous update message
    Tick, // Timer tick for real-time updates
}

/// Tuning mode for the piano tuner application.
///
/// Determines whether the application is in automatic pitch detection mode
/// or manual key selection mode.
#[derive(Debug, Clone, PartialEq)]
pub enum TuningMode {
    /// Automatic pitch detection mode - detects any note being played
    Auto,
    /// Manual mode - user has selected a specific piano key to tune
    Manual {
        key_index: u8,     // Piano key index (0-87)
        note_name: String, // Note name (e.g., "A4", "C#3")
        target_freq: f32,  // Target frequency in Hz
    },
}

#[derive(Debug, Clone)]
pub struct RmsCalibrationState {
    pub warmup_hops: Option<u32>,
    pub countdown: Option<u32>,
    pub max_seen_rms: f32,
}

#[derive(Debug, Clone)]
pub struct NoiseFloorSettings {
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
    pub calibration_complete: bool,
    pub visible: bool,
    pub active_calibration: Option<RmsCalibrationState>,
}

#[derive(Debug, Clone)]
pub struct TransientSettings {
    pub noise_floor_baseline: f32,
    pub visible: bool,
    pub is_frozen: bool,
    pub freeze_countdown: Option<u32>,
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
}

/// Settings-view-specific display data.
#[derive(Debug, Clone)]
pub struct SettingsDisplayData {
    pub rms: NoiseFloorSettings,
    pub transient: TransientSettings,
}

/// UI-specific data needed for rendering the interface.
///
/// This struct contains only the data that the UI components need
#[derive(Debug, Clone)]
pub struct AppDisplayData {
    // Audio state
    pub audio_worker_active: bool,
    /// Most recent visualization frame from the triple buffer.
    pub last_frame: Option<FrameOutput>,
    /// Most recent note index from NoteEvent (0–87), or None if no note locked.
    pub last_note_index: Option<u8>,
    /// Most recent detected frequency in Hz.
    pub last_frequency: Option<f32>,
    /// Most recent detection confidence (0.0–1.0).
    pub last_confidence: Option<f32>,
    /// Most recent cents deviation from nearest ET note.
    pub last_cents: Option<f32>,
    pub smoothing_buffer: Vec<f32>,
    /// Whether the last confident pitch metric is currently stale
    pub is_stale: bool,

    // Calibration state
    pub is_calibrating: bool,
    pub calibration_progress: usize,
    pub calibration_total: usize,

    // UI visibility states
    pub spectrogram_visible: bool,
    pub cent_meter_visible: bool,
    pub key_select_visible: bool,
    pub partials_visible: bool,
    // pub inharmonicity_graph_visible: bool,

    // View state
    pub settings_view_visible: bool,

    // Settings view data
    pub settings_data: SettingsDisplayData,

    // Tuning mode
    pub tuning_mode: TuningMode,

    // Capture state
    pub measurement_mode_active: bool,
    pub capture_state: CaptureState,
    pub undo_target_note: Option<String>,
}

/// Main application state for the Inharmonicity piano tuner.
///
/// Contains all the state necessary for the GUI application including
/// audio processing, analysis results, and UI visibility controls.
pub struct TunerApp {
    // Audio processing — managed by tuner_core::audio::HostHandle
    host_handle: Option<HostHandle>,

    /// Triple buffer output for continuous visualization frames (lossy, freshest only).
    frame_rx: Option<triple_buffer::Output<FrameOutput>>,

    // --- Inharmonicity State ---
    inharmonicity_profile: InharmonicityProfile,
    undo_history: VecDeque<(u8, Option<KeyMeasurement>)>,

    // Frontend handle to the AudioPipeline's shared atomic state
    pipeline_handle: PipelineHandle,

    // Single source of truth for all display data
    pub display_data: AppDisplayData,
}

impl std::fmt::Debug for TunerApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunerApp")
            .field("host_handle", &self.host_handle)
            .field("has_frame_rx", &self.frame_rx.is_some())
            .field("pipeline_handle", &self.pipeline_handle)
            .field("display_data", &self.display_data)
            .finish()
    }
}

impl Default for TunerApp {
    /// Creates a new TunerApp instance with default settings.
    ///
    /// Initializes the application state but does NOT start audio processing.
    /// Audio processing starts after calibration completes (see `CalibrationComplete`).
    fn default() -> Self {
        eprintln!("[MAIN] Creating TunerApp...");

        Self {
            host_handle: None,
            frame_rx: None,
            inharmonicity_profile: InharmonicityProfile::default(),
            undo_history: VecDeque::new(),
            pipeline_handle: PipelineHandle::default(),
            display_data: AppDisplayData {
                audio_worker_active: false,
                last_frame: None,
                last_note_index: None,
                last_frequency: None,
                last_confidence: None,
                last_cents: None,
                smoothing_buffer: Vec::new(),
                is_stale: false,
                is_calibrating: true,
                calibration_progress: 0,
                calibration_total: crate::calibration::CALIBRATION_FRAMES as usize,
                spectrogram_visible: true,
                cent_meter_visible: true,
                key_select_visible: true,
                partials_visible: true,
                // inharmonicity_graph_visible: true,
                settings_view_visible: false,
                settings_data: SettingsDisplayData {
                    rms: NoiseFloorSettings {
                        history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                        current_threshold: 0.005,
                        calibration_complete: false,
                        visible: false,
                        active_calibration: Some(RmsCalibrationState {
                            warmup_hops: Some(crate::calibration::WARMUP_FRAMES),
                            countdown: Some(crate::calibration::CALIBRATION_FRAMES),
                            max_seen_rms: 0.0,
                        }),
                    },
                    transient: TransientSettings {
                        noise_floor_baseline: 0.0,
                        visible: false,
                        is_frozen: false,
                        freeze_countdown: None,
                        history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                        current_threshold: 0.5,
                    },
                },
                tuning_mode: TuningMode::Auto,
                measurement_mode_active: false,
                capture_state: CaptureState::Idle,
                undo_target_note: None,
            },
        }
    }
}

impl TunerApp {
    /// Creates the app and initiates audio processing immediately.
    /// Noise floor calibration runs wait-free from the UI's Tick loop.
    pub fn new() -> (Self, iced::Task<Message>) {
        let mut app = Self::default();
        app.start_audio_processing();
        (app, iced::Task::none())
    }
    /// Starts the dedicated audio processing thread via [`audio::spawn_analysis_thread()`].
    ///
    /// All audio thread boilerplate (CPAL setup, ring buffer polling, analysis loop)
    /// is handled by the `tuner-core` host extension. This method simply calls it
    /// and stores the returned [`HostHandle`] channels.
    #[allow(unreachable_code)]
    fn start_audio_processing(&mut self) {
        // Prevent headless tests from hanging indefinitely while trying to initialize physical audio hardware
        #[cfg(test)]
        {
            eprintln!("[AUDIO-THREAD] Disabled for unit testing.");
            return;
        }

        match audio::spawn_analysis_thread(AudioSource::Default) {
            Ok(mut handle) => {
                eprintln!("[AUDIO] Hardware stream active.");

                // Write the current threshold to the new pipeline's config
                // (since AudioPipeline::new() creates fresh defaults)
                store_f32(
                    &handle.pipeline_handle.atomics.config.silence_threshold,
                    self.display_data.settings_data.rms.current_threshold,
                );

                self.pipeline_handle = handle.pipeline_handle.clone();
                self.frame_rx = handle.frame_rx.take();
                self.host_handle = Some(handle);
                self.display_data.audio_worker_active = true;
            }
            Err(e) => {
                eprintln!("[AUDIO ERROR] Could not start hardware: {}", e);
            }
        }
    }

    /// Handles application state updates based on incoming messages.
    pub fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::Exit => {
                eprintln!("[MAIN] Window close requested - starting cleanup...");
                if let Some(mut handle) = self.host_handle.take() {
                    eprintln!("[MAIN] Shutting down audio host...");
                    handle.stop();
                    eprintln!("[MAIN] Audio host stopped.");
                }
                eprintln!("[MAIN] Clearing channels...");
                self.frame_rx = None;
                eprintln!("[MAIN] Cleanup completed - forcing clean exit");
                std::process::exit(0);
            }
            Message::KeySelected(key_index) => {
                // Check if the same key is already selected - if so, switch to auto mode
                if let TuningMode::Manual {
                    key_index: current_key,
                    ..
                } = &self.display_data.tuning_mode
                    && *current_key == key_index
                {
                    // Same key clicked again - switch to auto mode
                    self.display_data.tuning_mode = TuningMode::Auto;
                    self.pipeline_handle
                        .atomics
                        .config
                        .target_note
                        .store(255, Ordering::Relaxed);
                    self.display_data.smoothing_buffer.clear();
                    return iced::Task::none();
                }

                // Different key or not in manual mode - switch to manual mode with new key
                let (note_name, target_freq) =
                    tuner_core::models::find_nearest_note_by_index(key_index);
                self.display_data.tuning_mode = TuningMode::Manual {
                    key_index,
                    note_name,
                    target_freq,
                };
                self.pipeline_handle
                    .atomics
                    .config
                    .target_note
                    .store(key_index, Ordering::Relaxed);
                self.display_data.smoothing_buffer.clear();
            }
            Message::SwitchToAutoMode => {
                self.display_data.tuning_mode = TuningMode::Auto;
                self.pipeline_handle
                    .atomics
                    .config
                    .target_note
                    .store(255, Ordering::Relaxed);
                self.display_data.smoothing_buffer.clear();
            }
            Message::ToggleMeasurementMode => {
                // This toggles the measurement mode on/off
                self.display_data.measurement_mode_active =
                    !self.display_data.measurement_mode_active;
                let mut new_state = CaptureState::Idle;

                if self.display_data.measurement_mode_active {
                    eprintln!("[MAIN] Measurement mode ON - starting in Armed state");
                    new_state = CaptureState::Armed;
                } else {
                    eprintln!("[MAIN] Measurement mode OFF");
                }

                self.display_data.capture_state = new_state.clone();
                self.pipeline_handle
                    .atomics
                    .capture_state
                    .store(new_state as u8, Ordering::Relaxed);
            }
            Message::CaptureButtonClicked => {
                // Clicked the active button
                // Wait-free: if we are in measurement mode, and state is Idle, we Arm it.
                // If it is Armed, we can optionally go to Idle (to cancel), but measurement mode remains active.
                if self.display_data.measurement_mode_active {
                    let mut new_state = self.display_data.capture_state.clone();
                    if new_state == CaptureState::Idle {
                        new_state = CaptureState::Armed;
                    } else if new_state == CaptureState::Armed {
                        new_state = CaptureState::Idle;
                    }
                    self.display_data.capture_state = new_state.clone();
                    self.pipeline_handle
                        .atomics
                        .capture_state
                        .store(new_state as u8, Ordering::Relaxed);
                }
            }
            Message::UndoLastCapture => {
                if let Some((idx, old_data)) = self.undo_history.pop_back() {
                    if let Some(m) = old_data {
                        self.inharmonicity_profile.measurements.insert(idx, m);
                    } else {
                        self.inharmonicity_profile.measurements.remove(&idx);
                    }
                    eprintln!("[MAIN] Undoing profile change at index {}", idx);
                }
            }
            Message::SaveProfile => {
                match self.inharmonicity_profile.to_file("tuning_profile.json") {
                    Ok(_) => eprintln!("[MAIN] Tuning profile saved successfully."),
                    Err(e) => eprintln!("[MAIN] Error saving profile: {}", e),
                }
            }
            Message::LoadProfile => match InharmonicityProfile::from_file("tuning_profile.json") {
                Ok(profile) => {
                    self.inharmonicity_profile = profile;
                    self.undo_history.clear();
                    eprintln!("[MAIN] Tuning profile loaded successfully.");
                }
                Err(e) => eprintln!("[MAIN] Error loading profile: {}", e),
            },
            // ------------------------------------------
            Message::Temperament => {
                // Placeholder for temperament settings
            }
            Message::TuningStandard => {
                // Placeholder for tuning standard settings
            }
            Message::InharmonicCurve => {
                // Placeholder for inharmonic curve adjustment
            }
            Message::SampleBuffer => {
                // Placeholder for sample buffer adjustment
            }
            Message::TuningProfile => {
                // Placeholder for tuning profile settings
            }
            Message::ToggleSpectrogram => {
                eprintln!(
                    "[MAIN] Toggling spectrogram visibility: {} -> {}",
                    self.display_data.spectrogram_visible, !self.display_data.spectrogram_visible
                );
                self.display_data.spectrogram_visible = !self.display_data.spectrogram_visible;
            }
            Message::ToggleCentMeter => {
                eprintln!(
                    "[MAIN] Toggling cent meter visibility: {} -> {}",
                    self.display_data.cent_meter_visible, !self.display_data.cent_meter_visible
                );
                self.display_data.cent_meter_visible = !self.display_data.cent_meter_visible;
            }
            Message::ToggleKeySelect => {
                eprintln!(
                    "[MAIN] Toggling key select visibility: {} -> {}",
                    self.display_data.key_select_visible, !self.display_data.key_select_visible
                );
                self.display_data.key_select_visible = !self.display_data.key_select_visible;
            }
            Message::TogglePartials => {
                eprintln!(
                    "[MAIN] Toggling partials visibility: {} -> {}",
                    self.display_data.partials_visible, !self.display_data.partials_visible
                );
                self.display_data.partials_visible = !self.display_data.partials_visible;
            }

            // Message::ToggleInharmonicityGraph => {
            //     eprintln!(
            //         "[MAIN] Toggling inharmonicity graph visibility: {} -> {}",
            //         self.display_data.inharmonicity_graph_visible,
            //         !self.display_data.inharmonicity_graph_visible
            //     );
            //     self.display_data.inharmonicity_graph_visible =
            //         !self.display_data.inharmonicity_graph_visible;
            // }
            Message::ToggleSettingsView => {
                eprintln!(
                    "[MAIN] Toggling settings view visibility: {} -> {}",
                    self.display_data.settings_view_visible,
                    !self.display_data.settings_view_visible
                );
                self.display_data.settings_view_visible = !self.display_data.settings_view_visible;
            }
            Message::ToggleNoiseFloorAdjustment => {
                let vis = !self.display_data.settings_data.rms.visible;
                self.display_data.settings_data.rms.visible = vis;
                if vis {
                    self.display_data.settings_data.transient.visible = false;
                }
            }
            Message::SilenceThresholdChanged(value) => {
                // Write to shared atomics so the audio thread picks it up immediately
                store_f32(
                    &self.pipeline_handle.atomics.config.silence_threshold,
                    value,
                );
                // Update local display data for immediate UI feedback
                self.display_data.settings_data.rms.current_threshold = value;
            }
            Message::RecalibrateNoiseFloor => {
                self.display_data.is_calibrating = true;
                self.display_data.settings_data.rms.calibration_complete = false;
                self.display_data.settings_data.rms.active_calibration =
                    Some(crate::app::RmsCalibrationState {
                        warmup_hops: Some(crate::calibration::WARMUP_FRAMES),
                        countdown: Some(crate::calibration::CALIBRATION_FRAMES),
                        max_seen_rms: 0.0,
                    });
            }
            Message::ToggleTransientCalibration => {
                let vis = !self.display_data.settings_data.transient.visible;
                self.display_data.settings_data.transient.visible = vis;

                if self.display_data.settings_data.transient.visible {
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.transient.is_frozen = false;
                    self.display_data.settings_data.transient.freeze_countdown = None;
                    self.display_data.settings_data.transient.history.clear();
                }
            }
            Message::ResetTransientScope => {
                self.display_data.settings_data.transient.is_frozen = false;
                self.display_data.settings_data.transient.freeze_countdown = None;
                self.display_data.settings_data.transient.history.clear();
            }
            Message::NhwrsfThresholdChanged(val) => {
                store_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold, val);
                self.display_data.settings_data.transient.current_threshold = val;
            }
            Message::Tick => {
                let mut frame_pushed = false;

                // ── Read freshest FrameOutput from triple buffer ──
                if let Some(ref mut frame_rx) = self.frame_rx
                    && frame_rx.update()
                {
                    frame_pushed = true;
                    let frame = frame_rx.read().clone();
                    self.display_data.last_frame = Some(frame.clone());

                    // 1. Independent Decoupling of Scalar Data
                    // Assign values individually. If one drops out (e.g. confidence),
                    // we still display the others.
                    self.display_data.last_frequency = frame.detected_frequency;
                    self.display_data.last_confidence = frame.confidence;
                    self.display_data.last_cents = frame.cents_deviation;

                    if let Some(idx) = frame.note_index {
                        self.display_data.last_note_index = Some(idx);
                    }

                    if let Some(cents) = frame.cents_deviation {
                        // Update smoothing buffer
                        let cents_for_smoothing = match self.display_data.tuning_mode {
                            TuningMode::Auto => Some(cents),
                            TuningMode::Manual { target_freq, .. } => frame
                                .detected_frequency
                                .map(|f| tuning::calculate_cents_deviation(f, target_freq)),
                        };
                        if let Some(c) = cents_for_smoothing {
                            self.display_data.smoothing_buffer.push(c);
                            if self.display_data.smoothing_buffer.len() > SMOOTHING_FACTOR {
                                self.display_data.smoothing_buffer.remove(0);
                            }
                        }
                    } else {
                        self.display_data.smoothing_buffer.clear();
                    }

                    // Render State Logic: Silence vs Stale vs Valid
                    if frame.is_silence {
                        // Valid Silence: Drop all old measurements.
                        self.display_data.last_frequency = None;
                        self.display_data.last_note_index = None;
                        self.display_data.last_confidence = None;
                        self.display_data.last_cents = None;
                        self.display_data.smoothing_buffer.clear();
                        self.display_data.is_stale = false;
                    } else if frame.note_index.is_none() {
                        // Valid Audio but No Pitch Lock: Freeze scalars but flag as stale to mute visual output
                        self.display_data.smoothing_buffer.clear();
                        self.display_data.is_stale = true;
                    } else {
                        // Valid Lock: Ensure we are not stale.
                        self.display_data.is_stale = false;
                    }

                    // Sync capture state from atomics for UI rendering
                    let state_val = self
                        .pipeline_handle
                        .atomics
                        .capture_state
                        .load(Ordering::Relaxed);
                    self.display_data.capture_state = match state_val {
                        1 => CaptureState::Armed,
                        2 => CaptureState::Recording,
                        3 => CaptureState::Processing,
                        _ => CaptureState::Idle,
                    };
                }

                // ── Drain Result Channel from Worker ──
                while let Ok(measurement) = self.pipeline_handle.result_rx.try_recv() {
                    let target_idx = measurement.key_index;

                    // Backup old data for Undo History
                    let old_data = self
                        .inharmonicity_profile
                        .measurements
                        .get(&target_idx)
                        .cloned();
                    self.undo_history.push_back((target_idx, old_data));
                    if self.undo_history.len() > 100 {
                        self.undo_history.pop_front();
                    }

                    // Apply to profile
                    self.inharmonicity_profile
                        .measurements
                        .insert(target_idx, measurement);

                    eprintln!(
                        "[MAIN] Successfully slotted new capture data into Inharmonicity Profile at index {}",
                        target_idx
                    );

                    // Re-arm automatically if in Auto mode
                    if let TuningMode::Auto = self.display_data.tuning_mode {
                        eprintln!("[MAIN] Auto-mode rearming...");
                        self.pipeline_handle
                            .atomics
                            .capture_state
                            .store(CaptureState::Armed as u8, Ordering::Relaxed);
                        self.display_data.capture_state = CaptureState::Armed;
                    }
                }

                self.display_data.undo_target_note = self
                    .undo_history
                    .back()
                    .map(|(idx, _)| tuner_core::models::find_nearest_note_by_index(*idx).0);

                if self.display_data.settings_view_visible {
                    if self.display_data.settings_data.rms.visible {
                        let rms = self
                            .display_data
                            .last_frame
                            .as_ref()
                            .map(|f| f.rms_ema)
                            .unwrap_or(0.0);
                        let history = &mut self.display_data.settings_data.rms.history;
                        history.push_back(rms);
                        if history.len() > ENVELOPE_HISTORY_LENGTH {
                            history.pop_front();
                        }
                        self.display_data.settings_data.rms.current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.silence_threshold);
                    } else if self.display_data.settings_data.transient.visible {
                        let flux = self
                            .display_data
                            .last_frame
                            .as_ref()
                            .map(|f| f.nhwrsf)
                            .unwrap_or(0.0);
                        let current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold);

                        crate::views::transient_calibration::process_telemetry_tick(
                            &mut self.display_data.settings_data.transient,
                            flux,
                            current_threshold,
                        );

                        self.display_data.settings_data.transient.current_threshold =
                            current_threshold;
                    }
                }

                // ── Calibration Hook ──
                if self.display_data.is_calibrating {
                    let current_rms = self
                        .display_data
                        .last_frame
                        .as_ref()
                        .map(|f| f.rms_ema)
                        .unwrap_or(0.0);
                    if let Some(silence_val) = crate::calibration::process_calibration_tick(
                        &mut self.display_data.settings_data.rms,
                        current_rms,
                        frame_pushed,
                    ) {
                        // Finished
                        self.display_data.is_calibrating = false;
                        self.display_data.settings_data.rms.calibration_complete = true;

                        store_f32(
                            &self.pipeline_handle.atomics.config.silence_threshold,
                            silence_val,
                        );
                        self.display_data.settings_data.rms.current_threshold = silence_val;

                        // Seed the transient wizard's baseline directly from this calculation point:
                        if let Some(active) =
                            &self.display_data.settings_data.rms.active_calibration
                        {
                            self.display_data
                                .settings_data
                                .transient
                                .noise_floor_baseline = active.max_seen_rms;
                        }

                        eprintln!(
                            "[MAIN] Lock-Free Calibration complete. Threshold set to: {:.6}",
                            silence_val
                        );
                    } else if let Some(active) =
                        &self.display_data.settings_data.rms.active_calibration
                        && let Some(countdown) = active.countdown
                    {
                        self.display_data.calibration_progress =
                            (crate::calibration::CALIBRATION_FRAMES.saturating_sub(countdown))
                                as usize;
                    }
                }
            }
        }
        iced::Task::none()
    }

    /// Renders the main application interface.
    pub fn view(&self) -> Element<'_, Message> {
        if self.display_data.settings_view_visible {
            create_settings_view(&self.display_data)
        } else {
            create_main_view(
                &self.display_data,
                &self.inharmonicity_profile,
                Message::CaptureButtonClicked,
            )
        }
    }

    /// Creates a subscription for continuous application updates.
    ///
    /// Returns a timer subscription that fires every 16ms (60 FPS) to ensure
    /// smooth real-time audio visualization and responsive UI updates.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _status, _window_id| match event {
                iced::Event::Window(iced::window::Event::CloseRequested) => Some(Message::Exit),
                _ => None,
            }),
        ])
    }

    /// Returns the application theme.
    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
