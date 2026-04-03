//! # Inharmonicity - Professional Piano Tuning GUI
//!
//! This module contains the main GUI application for the Inharmonicity piano tuning software.
//! It provides a real-time interface for audio analysis, spectrogram visualization, and
//! interactive piano keyboard controls.
//!
//! ## Architecture
//! - **Main Thread**: Iced GUI application with dark theme
//! - **Audio Thread**: Dedicated thread for real-time audio processing
//! - **Communication**: Crossbeam channels for thread-safe data exchange
//! - **Updates**: 60 FPS continuous updates via subscription system

use crate::utils::view_utils::initialize_done_timer;
use crate::views::{main_view::create_main_view, settings_view::create_settings_view};
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use crossbeam_channel::Receiver;
use iced::{self, Element, Subscription, Theme};
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tuner_core::{
    AnalysisResult,
    algorithms::tuning,
    audio::{self, AudioSource, HostHandle},
    calibration::{self, NoiseFloorResult},
    capture_processing::{self, ProcessingOperation},
    models::InharmonicityProfile,
    pipeline::PipelineHandle,
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
    CalibrationComplete(Result<NoiseFloorResult, String>), // Calibration finished

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

/// Settings-view-specific display data.
#[derive(Debug, Clone)]
pub struct SettingsDisplayData {
    /// Rolling history of smoothed RMS values for the Envelope Viewer.
    pub rms_history: VecDeque<f32>,
    /// Current silence threshold (read from shared ConfigState).
    pub current_silence_threshold: f32,
    /// Whether noise-floor calibration has completed at least once.
    pub calibration_complete: bool,
    /// Whether the Noise Floor Adjustment panel is visible.
    pub noise_floor_adjustment_visible: bool,
    /// Peak NHWRSF observed during the last noise-floor calibration ($N_{max}$).
    /// Used as the lower bound of the transient threshold slider in the wizard.
    pub nhwrsf_noise_floor: f32,
}

/// UI-specific data needed for rendering the interface.
///
/// This struct contains only the data that the UI components need
#[derive(Debug, Clone)]
pub struct AppDisplayData {
    // Audio state
    pub audio_worker_active: bool,
    pub last_analysis: Option<AnalysisResult>,
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
#[derive(Debug)]
pub struct TunerApp {
    // Audio processing — managed by tuner_core::audio::HostHandle
    host_handle: Option<HostHandle>,
    analysis_receiver: Option<Receiver<AnalysisResult>>,

    // --- New Inharmonicity State ---
    stability_buffer: VecDeque<AnalysisResult>, // Buffer for checking note stability
    inharmonicity_profile: InharmonicityProfile,
    // ---------------------------------

    // Frontend handle to the AudioPipeline's shared state
    pipeline_handle: PipelineHandle,

    // Single source of truth for all display data
    pub display_data: AppDisplayData,

    // Calibration progress counter (shared with the calibration task)
    calibration_progress: Arc<AtomicUsize>,
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
            analysis_receiver: None,
            stability_buffer: VecDeque::with_capacity(STABILITY_TARGET),
            inharmonicity_profile: InharmonicityProfile::default(),
            pipeline_handle: PipelineHandle::default(),
            calibration_progress: Arc::new(AtomicUsize::new(0)),
            display_data: AppDisplayData {
                audio_worker_active: false,
                last_analysis: None,
                smoothing_buffer: Vec::new(),
                is_calibrating: true,
                calibration_progress: 0,
                calibration_total: calibration::DEFAULT_CALIBRATION_FRAMES,
                spectrogram_visible: true,
                cent_meter_visible: true,
                key_select_visible: true,
                partials_visible: true,
                // inharmonicity_graph_visible: true,
                settings_view_visible: false,
                settings_data: SettingsDisplayData {
                    rms_history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                    current_silence_threshold: 0.005,
                    calibration_complete: false,
                    noise_floor_adjustment_visible: false,
                    nhwrsf_noise_floor: 0.0,
                },
                tuning_mode: TuningMode::Auto,
                capture_state: CaptureState::Off,
            },
        }
    }
}

impl TunerApp {
    /// Creates the app and kicks off noise-floor calibration via `Task::perform`.
    /// Audio processing starts only after `CalibrationComplete` arrives.
    pub fn new() -> (Self, iced::Task<Message>) {
        let app = Self::default();
        let progress = Arc::clone(&app.calibration_progress);
        let calibration_task = iced::Task::perform(
            async move {
                calibration::calibrate_noise_floor(
                    AudioSource::Default,
                    calibration::DEFAULT_NOISE_MULTIPLIER,
                    calibration::DEFAULT_CALIBRATION_FRAMES,
                    progress,
                )
            },
            |result| Message::CalibrationComplete(result.map_err(|e| e.to_string())),
        );
        (app, calibration_task)
    }
    /// Starts the dedicated audio processing thread via [`audio::spawn_analysis_thread()`].
    ///
    /// All audio thread boilerplate (CPAL setup, ring buffer polling, analysis loop)
    /// is handled by the `tuner-core` host extension. This method simply calls it
    /// and stores the returned [`HostHandle`].
    #[allow(unreachable_code)]
    fn start_audio_processing(&mut self) {
        // Prevent headless tests from hanging indefinitely while trying to initialize physical audio hardware
        #[cfg(test)]
        {
            eprintln!("[AUDIO-THREAD] Disabled for unit testing.");
            return;
        }

        match audio::spawn_analysis_thread(AudioSource::Default) {
            Ok(handle) => {
                eprintln!("[AUDIO] Hardware stream active.");

                // Write the current threshold to the new pipeline's config
                // (since AudioPipeline::new() creates fresh defaults)
                if let Ok(mut config) = handle.pipeline_handle.config.lock() {
                    config.silence_threshold =
                        self.display_data.settings_data.current_silence_threshold;
                }

                self.pipeline_handle = handle.pipeline_handle.clone();
                self.analysis_receiver = Some(handle.analysis_rx.clone());
                self.host_handle = Some(handle);
                self.display_data.audio_worker_active = true;
            }
            Err(e) => {
                eprintln!("[AUDIO ERROR] Could not start hardware: {}", e);
            }
        }
    }

    /// Handles application state updates based on incoming messages.
    ///
    /// This function processes all user interactions and system events,
    /// updating the application state accordingly. It handles:
    /// - Piano key selections and tuning mode changes
    /// - Tool visibility toggles
    /// - Audio analysis data processing
    /// - Application exit requests
    pub fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::Exit => {
                eprintln!("[MAIN] Window close requested - starting cleanup...");
                // Properly shutdown audio host to prevent CPAL/ALSA segmentation faults.
                // HostHandle::stop() signals the analysis thread and waits for it to join.
                if let Some(mut handle) = self.host_handle.take() {
                    eprintln!("[MAIN] Shutting down audio host...");
                    handle.stop();
                    eprintln!("[MAIN] Audio host stopped.");
                }
                // Clear channels to prevent segfault
                eprintln!("[MAIN] Clearing analysis channels...");
                self.analysis_receiver = None;
                eprintln!("[MAIN] Cleanup completed - forcing clean exit");
                // Force clean exit to avoid segfault
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
                self.display_data
                    .settings_data
                    .noise_floor_adjustment_visible = !self
                    .display_data
                    .settings_data
                    .noise_floor_adjustment_visible;
            }
            Message::SilenceThresholdChanged(value) => {
                // Write to shared config so the audio thread picks it up immediately
                if let Ok(mut config) = self.pipeline_handle.config.lock() {
                    config.silence_threshold = value;
                }
                // Update local display data for immediate UI feedback
                self.display_data.settings_data.current_silence_threshold = value;
            }
            Message::CalibrationComplete(result) => {
                self.display_data.is_calibrating = false;
                match result {
                    Ok(cal) => {
                        eprintln!(
                            "[MAIN] Calibration complete. RMS baseline: {:.6}, threshold: {:.6}, N_max NHWRSF: {:.4}",
                            cal.rms_baseline, cal.rms_threshold, cal.noise_floor_peak
                        );
                        // Write calibrated values to shared config
                        if let Ok(mut config) = self.pipeline_handle.config.lock() {
                            config.silence_threshold = cal.rms_threshold;
                        }
                        // Update display data
                        self.display_data.settings_data.current_silence_threshold =
                            cal.rms_threshold;
                        self.display_data.settings_data.nhwrsf_noise_floor = cal.noise_floor_peak;
                        self.display_data.settings_data.calibration_complete = true;

                        // Now start the audio processing thread
                        self.start_audio_processing();
                    }
                    Err(e) => {
                        eprintln!(
                            "[MAIN] Calibration failed: {}. Starting audio with defaults.",
                            e
                        );
                        // Start audio anyway with default threshold
                        self.display_data.settings_data.calibration_complete = true;
                        self.start_audio_processing();
                    }
                }
            }
            Message::RecalibrateNoiseFloor => {
                eprintln!("[MAIN] Recalibration requested — stopping audio thread...");
                // Stop the audio host
                if let Some(mut handle) = self.host_handle.take() {
                    handle.stop();
                }
                self.analysis_receiver = None;
                self.display_data.audio_worker_active = false;
                self.display_data.settings_data.calibration_complete = false;

                // Set calibrating state and reset progress counter
                self.display_data.is_calibrating = true;
                self.display_data.calibration_progress = 0;
                self.calibration_progress.store(0, Ordering::Relaxed);

                let progress = Arc::clone(&self.calibration_progress);

                // Kick off recalibration
                return iced::Task::perform(
                    async move {
                        calibration::calibrate_noise_floor(
                            AudioSource::Default,
                            calibration::DEFAULT_NOISE_MULTIPLIER,
                            calibration::DEFAULT_CALIBRATION_FRAMES,
                            progress,
                        )
                    },
                    |result| Message::CalibrationComplete(result.map_err(|e| e.to_string())),
                );
            }
            Message::Tick => {
                // Poll calibration progress (atomic, lock-free)
                if self.display_data.is_calibrating {
                    self.display_data.calibration_progress =
                        self.calibration_progress.load(Ordering::Relaxed);
                }

                // Continuous update - poll for audio data
                if let Some(receiver) = &self.analysis_receiver {
                    // --- REFACTORED: Delegate result processing ---
                    // Collect all results first to avoid borrowing conflicts
                    let mut results = Vec::new();
                    while let Ok(result) = receiver.try_recv() {
                        results.push(result);
                    }
                    // Process all collected results
                    for result in results {
                        self.process_analysis_result(result);
                    }
                    // ---------------------------------------------
                }

                // Poll shared state for settings view (only when visible)
                if self.display_data.settings_view_visible
                    && self
                        .display_data
                        .settings_data
                        .noise_floor_adjustment_visible
                {
                    if let Ok(runtime) = self.pipeline_handle.runtime.lock() {
                        let history = &mut self.display_data.settings_data.rms_history;
                        history.push_back(runtime.current_rms_ema);
                        if history.len() > ENVELOPE_HISTORY_LENGTH {
                            history.pop_front();
                        }
                    }
                    if let Ok(config) = self.pipeline_handle.config.lock() {
                        self.display_data.settings_data.current_silence_threshold =
                            config.silence_threshold;
                    }
                }

                // State reset after capture processing
                if self.display_data.capture_state == CaptureState::Done {
                    eprintln!("[MAIN] Capture complete. Resetting state to Armed.");
                    self.display_data.capture_state = CaptureState::Armed;
                }
            }
        }
        iced::Task::none()
    }

    // --- PLANNED DEPRECATION: Helper function to process analysis results ---
    /// Processes a single AnalysisResult received from the audio thread.
    ///
    /// This function runs on the GUI thread and updates the application state
    /// based on the new analysis data. It handles:
    /// - Updating the stability buffer for capture
    /// - Triggering the capture process when stable
    /// - Updating the cent smoothing buffer
    /// - Storing the latest analysis result
    fn process_analysis_result(&mut self, result: AnalysisResult) {
        // --- Stability-Gated Capture Logic ---
        if self.display_data.capture_state == CaptureState::Capturing {
            self.stability_buffer.push_back(result.clone()); // Clone for stability check

            if self.stability_buffer.len() > STABILITY_TARGET {
                self.stability_buffer.pop_front();
            }

            if self.stability_buffer.len() == STABILITY_TARGET {
                if check_stability(&self.stability_buffer) {
                    eprintln!("[MAIN] STABILITY DETECTED! Capturing...");
                    self.display_data.capture_state = CaptureState::Done;
                    // Convert stability buffer to Vec and process it
                    let stability_data: Vec<AnalysisResult> =
                        self.stability_buffer.drain(..).collect();
                    // Call the processing function with the stability buffer using default operation
                    if let Some(measurement) = capture_processing::process(
                        stability_data,
                        ProcessingOperation::BestConfidence,
                    ) {
                        // Store the measurement in the profile
                        self.inharmonicity_profile
                            .measurements
                            .insert(measurement.key_index, measurement);
                    }
                    // Initialize the "Done" timer for visual feedback
                    initialize_done_timer();
                }
            }
        }
        // --- End Capture Logic ---

        // --- Smoothing Buffer Logic ---
        let cents_for_smoothing = match self.display_data.tuning_mode {
            TuningMode::Auto => result.cents_deviation,
            TuningMode::Manual { target_freq, .. } => result
                .detected_frequency
                .map(|freq| tuning::calculate_cents_deviation(freq, target_freq)),
        };
        if let Some(cents) = cents_for_smoothing {
            self.display_data.smoothing_buffer.push(cents);
            if self.display_data.smoothing_buffer.len() > SMOOTHING_FACTOR {
                self.display_data.smoothing_buffer.remove(0);
            }
        } else {
            self.display_data.smoothing_buffer.clear();
        }

        // --- Store Last Analysis ---
        self.display_data.last_analysis = Some(result); // Move the original result
    }
    // ----------------------------------------------------------------

    /// Renders the main application interface.
    ///
    /// Delegates all UI rendering to the main_display module,
    /// keeping this function focused on application logic only.
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
    ///
    /// Currently returns the built-in dark theme for a professional appearance.
    /// This can be extended to support dynamic theme switching in the future.
    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

/// Checks if all AnalysisResult frames in the buffer are "stable."
///
/// Stability is defined as:
/// 1. The buffer is not empty.
/// 2. All frames have a `note_name` that is `Some` and is the *same* note.
/// 3. All frames have a `confidence` that is `Some` and is above the `STABILITY_CONFIDENCE_THRESHOLD`.
fn check_stability(buffer: &VecDeque<AnalysisResult>) -> bool {
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
