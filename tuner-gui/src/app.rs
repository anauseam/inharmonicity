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

use crate::utils::view_utils::initialize_done_timer;
use crate::views::{main_view::create_main_view, settings_view::create_settings_view};
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use iced::{self, Element, Subscription, Theme};
use std::collections::VecDeque;
use tuner_core::{
    FrameOutput,
    algorithms::tuning,
    audio::{self, AudioSource, HostHandle},
    capture_processing::{self, ProcessingOperation},
    models::InharmonicityProfile,
    pipeline::{PipelineHandle, load_f32, store_f32},
};

// Audio processing constants
const SMOOTHING_FACTOR: usize = 5; // Number of samples for cent smoothing
const STABILITY_TARGET: usize = 20; // Number of stable frames required for capture
const STABILITY_CONFIDENCE_THRESHOLD: f32 = 0.9; // Confidence threshold for stability

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

/// State for the stability-gated capture system.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureState {
    Off,       // Not capturing
    Armed,     // Ready to capture (button shows "Off")
    Capturing, // Actively capturing (button shows "Capturing")
    Done,      // Capture is complete, data is being processed
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
    pub capture_state: CaptureState,
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
    // TODO: Remove when capture_processing.rs is replaced by Worker pipeline.
    #[allow(deprecated)]
    stability_buffer: VecDeque<tuner_core::AnalysisResult>,
    inharmonicity_profile: InharmonicityProfile,

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
            stability_buffer: VecDeque::with_capacity(STABILITY_TARGET),
            inharmonicity_profile: InharmonicityProfile::default(),
            pipeline_handle: PipelineHandle::default(),
            display_data: AppDisplayData {
                audio_worker_active: false,
                last_frame: None,
                last_note_index: None,
                last_frequency: None,
                last_confidence: None,
                last_cents: None,
                smoothing_buffer: Vec::new(),
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
                capture_state: CaptureState::Off,
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
                {
                    if *current_key == key_index {
                        // Same key clicked again - switch to auto mode
                        self.display_data.tuning_mode = TuningMode::Auto;
                        self.display_data.smoothing_buffer.clear();
                        return iced::Task::none();
                    }
                }

                // Different key or not in manual mode - switch to manual mode with new key
                let (note_name, target_freq) =
                    tuner_core::models::find_nearest_note_by_index(key_index);
                self.display_data.tuning_mode = TuningMode::Manual {
                    key_index,
                    note_name,
                    target_freq,
                };
                self.display_data.smoothing_buffer.clear();
            }
            Message::SwitchToAutoMode => {
                self.display_data.tuning_mode = TuningMode::Auto;
                self.display_data.smoothing_buffer.clear();
            }
            Message::ToggleMeasurementMode => {
                // This toggles the measurement mode on/off
                self.display_data.capture_state = match self.display_data.capture_state {
                    CaptureState::Off => {
                        eprintln!("[MAIN] Measurement mode ON - starting in Armed state");
                        CaptureState::Armed // Start in Armed state (ready to capture)
                    }
                    CaptureState::Armed => {
                        eprintln!("[MAIN] Measurement mode OFF");
                        self.stability_buffer.clear();
                        CaptureState::Off
                    }
                    CaptureState::Capturing => {
                        eprintln!("[MAIN] Measurement mode OFF (from Capturing)");
                        self.stability_buffer.clear();
                        CaptureState::Off
                    }
                    CaptureState::Done => {
                        // If it's done, clicking again resets it
                        eprintln!("[MAIN] Measurement mode OFF (from Done)");
                        CaptureState::Off
                    }
                };
            }
            Message::CaptureButtonClicked => {
                // This handles the capture button click behavior
                match self.display_data.capture_state {
                    CaptureState::Armed => {
                        eprintln!("[MAIN] Capture button clicked - starting capture");
                        self.display_data.capture_state = CaptureState::Capturing;
                    }
                    CaptureState::Capturing => {
                        eprintln!("[MAIN] Capture button clicked - stopping capture");
                        self.display_data.capture_state = CaptureState::Armed;
                    }
                    CaptureState::Done => {
                        eprintln!("[MAIN] Capture button clicked - resetting to Off");
                        self.display_data.capture_state = CaptureState::Off;
                    }
                    CaptureState::Off => {
                        eprintln!("[MAIN] Capture button clicked - but not in measurement mode");
                        // Do nothing - button shouldn't be visible in Off state
                    }
                }
            }
            Message::SaveProfile => {
                match save_profile(&self.inharmonicity_profile, "tuning_profile.json") {
                    Ok(_) => eprintln!("[MAIN] Tuning profile saved successfully."),
                    Err(e) => eprintln!("[MAIN] Error saving profile: {}", e),
                }
            }
            Message::LoadProfile => match load_profile("tuning_profile.json") {
                Ok(profile) => {
                    self.inharmonicity_profile = profile;
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
                self.display_data.settings_data.rms.active_calibration = Some(crate::app::RmsCalibrationState {
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
                if let Some(ref mut frame_rx) = self.frame_rx {
                    if frame_rx.update() {
                        frame_pushed = true;
                        let frame = frame_rx.read().clone();
                        self.display_data.last_frame = Some(frame.clone());

                        // 1. Independent Decoupling of Scalar Data
                        // Assign values individually. If one drops out (e.g. confidence),
                        // we still display the others.
                        if let Some(freq) = frame.detected_frequency {
                            self.display_data.last_frequency = Some(freq);
                        }
                        if let Some(idx) = frame.note_index {
                            self.display_data.last_note_index = Some(idx);
                        }
                        if let Some(conf) = frame.confidence {
                            self.display_data.last_confidence = Some(conf);
                        }
                        if let Some(cents) = frame.cents_deviation {
                            self.display_data.last_cents = Some(cents);

                            // Update smoothing buffer
                            let cents_for_smoothing = match self.display_data.tuning_mode {
                                TuningMode::Auto => Some(cents),
                                TuningMode::Manual { target_freq, .. } => {
                                    if let Some(f) = frame.detected_frequency {
                                        Some(tuning::calculate_cents_deviation(f, target_freq))
                                    } else {
                                        None
                                    }
                                }
                            };
                            if let Some(c) = cents_for_smoothing {
                                self.display_data.smoothing_buffer.push(c);
                                if self.display_data.smoothing_buffer.len() > SMOOTHING_FACTOR {
                                    self.display_data.smoothing_buffer.remove(0);
                                }
                            }
                        }

                        // If no note data was detected at all (Silence or Unstable routing gap),
                        // we clear the smoothing buffer, but preserve the last scalar integers
                        // so the UI floats down instead of abruptly vanishing.
                        if frame.detected_frequency.is_none() && frame.cents_deviation.is_none() {
                            self.display_data.smoothing_buffer.clear();
                        }

                        // 2. Route payload to Legacy Capture System
                        // TODO: Remove this call when capture_processing is retired.
                        self.process_legacy_capture(&frame);
                    }
                }

                if self.display_data.settings_view_visible {
                    if self.display_data.settings_data.rms.visible {
                        let rms = load_f32(&self.pipeline_handle.atomics.runtime.current_rms_ema);
                        let history = &mut self.display_data.settings_data.rms.history;
                        history.push_back(rms);
                        if history.len() > ENVELOPE_HISTORY_LENGTH {
                            history.pop_front();
                        }
                        self.display_data.settings_data.rms.current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.silence_threshold);
                    } else if self.display_data.settings_data.transient.visible {
                        let flux = load_f32(&self.pipeline_handle.atomics.runtime.current_nhwrsf);
                        let current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold);

                        crate::views::transient_calibration::process_telemetry_tick(
                            &mut self.display_data.settings_data.transient,
                            flux,
                            current_threshold,
                        );

                        self.display_data.settings_data.transient.current_threshold = current_threshold;
                    }
                }

                // ── Calibration Hook ──
                if self.display_data.is_calibrating {
                    let current_rms =
                        load_f32(&self.pipeline_handle.atomics.runtime.current_rms_ema);
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
                        if let Some(active) = &self.display_data.settings_data.rms.active_calibration {
                            self.display_data.settings_data.transient.noise_floor_baseline = active.max_seen_rms;
                        }

                        eprintln!(
                            "[MAIN] Lock-Free Calibration complete. Threshold set to: {:.6}",
                            silence_val
                        );
                    } else if let Some(active) = &self.display_data.settings_data.rms.active_calibration {
                        if let Some(countdown) = active.countdown {
                            self.display_data.calibration_progress = (crate::calibration::CALIBRATION_FRAMES.saturating_sub(countdown)) as usize;
                        }
                    }
                }

                if self.display_data.capture_state == CaptureState::Done {
                    eprintln!("[MAIN] Capture complete. Resetting state to Armed.");
                    self.display_data.capture_state = CaptureState::Armed;
                }
            }
        }
        iced::Task::none()
    }

    /// Legacy stability-gated capture bridge logic.
    ///
    /// Extracted into its own method to isolate deprecated structures from the
    /// modern fast-path UI rendering loop above.
    ///
    /// TODO: Remove when capture_processing.rs is fully replaced by the new Worker pipeline.
    fn process_legacy_capture(&mut self, frame: &FrameOutput) {
        if let (Some(note_index), Some(frequency), Some(confidence), Some(cents_deviation)) = (
            frame.note_index,
            frame.detected_frequency,
            frame.confidence,
            frame.cents_deviation,
        ) {
            // --- Stability-Gated Capture Bridge ---
            // TODO: Remove when capture_processing.rs is replaced by Worker pipeline.
            #[allow(deprecated)]
            if self.display_data.capture_state == CaptureState::Capturing {
                let (note_name, _) = tuner_core::models::find_nearest_note_by_index(note_index);

                let compat_result = tuner_core::AnalysisResult {
                    detected_frequency: Some(frequency),
                    confidence: Some(confidence),
                    cents_deviation: Some(cents_deviation),
                    note_name: Some(note_name),
                };

                self.stability_buffer.push_back(compat_result);
                if self.stability_buffer.len() > STABILITY_TARGET {
                    self.stability_buffer.pop_front();
                }

                if self.stability_buffer.len() == STABILITY_TARGET {
                    if check_stability(&self.stability_buffer) {
                        eprintln!("[MAIN] STABILITY DETECTED! Capturing...");
                        self.display_data.capture_state = CaptureState::Done;
                        let stability_data: Vec<tuner_core::AnalysisResult> =
                            self.stability_buffer.drain(..).collect();
                        if let Some(measurement) = capture_processing::process(
                            stability_data,
                            ProcessingOperation::BestConfidence,
                        ) {
                            self.inharmonicity_profile
                                .measurements
                                .insert(measurement.key_index, measurement);
                        }
                        initialize_done_timer();
                    }
                }
            }
        }
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

/// Checks if all frames in the stability buffer have the same note with high confidence.
///
/// TODO: Remove when capture_processing.rs is replaced by Worker pipeline.
#[allow(deprecated)]
fn check_stability(buffer: &VecDeque<tuner_core::AnalysisResult>) -> bool {
    if buffer.is_empty() {
        return false;
    }

    // Get the note name from the first frame. If it's None, it's not stable.
    let first_note = match &buffer[0].note_name {
        Some(n) => n,
        None => return false,
    };

    // Use `iter().all()` to efficiently check every frame against the criteria.
    buffer.iter().all(|frame| {
        // 1. Check confidence
        let high_confidence = frame
            .confidence
            .map_or(false, |c| c > STABILITY_CONFIDENCE_THRESHOLD);

        // 2. Check for matching note name
        let matching_note = frame.note_name.as_ref().map_or(false, |n| n == first_note);

        high_confidence && matching_note
    })
}

// --- New Profile Save/Load Functions ---

use serde_json;
use std::fs::File;
use std::io::{Read, Write};

/// Saves the inharmonicity profile to a JSON file.
///
/// Serializes the complete inharmonicity profile (including all measured
/// partials and calculated B values) to a JSON file for persistent storage.
/// This allows users to save their piano's unique inharmonicity characteristics
/// and reload them in future tuning sessions.
///
/// # Arguments
/// * `profile` - The inharmonicity profile to save
/// * `path` - File path where the profile should be saved (e.g., "tuning_profile.json")
///
/// # Returns
/// * `Ok(())` - Profile saved successfully
/// * `Err(io::Error)` - File I/O error or JSON serialization error
fn save_profile(profile: &InharmonicityProfile, path: &str) -> std::io::Result<()> {
    let json_string = serde_json::to_string_pretty(profile)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let mut file = File::create(path)?;
    file.write_all(json_string.as_bytes())?;
    Ok(())
}

/// Loads an inharmonicity profile from a JSON file.
///
/// Deserializes a previously saved inharmonicity profile from a JSON file.
/// This allows users to restore their piano's unique inharmonicity characteristics
/// from a previous tuning session, maintaining consistency across tuning sessions.
///
/// # Arguments
/// * `path` - File path to load the profile from (e.g., "tuning_profile.json")
///
/// # Returns
/// * `Ok(InharmonicityProfile)` - Successfully loaded profile
/// * `Err(io::Error)` - File I/O error or JSON deserialization error
fn load_profile(path: &str) -> std::io::Result<InharmonicityProfile> {
    let mut file = File::open(path)?;
    let mut data = String::new();
    file.read_to_string(&mut data)?;
    let profile: InharmonicityProfile = serde_json::from_str(&data)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    Ok(profile)
}
