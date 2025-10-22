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

mod ui;

use crossbeam_channel::{Receiver, Sender};
use cpal::traits::StreamTrait;
use iced::{
    self, Element, Theme, Subscription
};
use std::collections::VecDeque;
use std::thread::{self, JoinHandle};
use tuner_core::{
    audio, fft, pitch, tuning, AnalysisResult,
    inharmonicity::InharmonicityProfile,
    capture_processing::{self, ProcessingOperation}
};
use ui::main_display::create_main_view;

// Audio processing constants
const SMOOTHING_FACTOR: usize = 5;  // Number of samples for cent smoothing
const AMPLITUDE_THRESHOLD: f32 = 0.01;  // Minimum amplitude for pitch detection
const STABILITY_TARGET: usize = 20; // Number of stable frames required for capture
const STABILITY_CONFIDENCE_THRESHOLD: f32 = 0.9; // Confidence threshold for stability


/// Main entry point for the Inharmonicity application.
/// 
/// Initializes the Iced GUI application with dark theme, real-time audio processing,
/// and continuous updates for smooth visualization.
pub fn main() -> iced::Result {
    eprintln!("[MAIN] Starting Inharmonicity application...");
    eprintln!("[MAIN] Initializing GUI framework...");
    let result = iced::application("Inharmonicity", TunerApp::update, TunerApp::view)
        .subscription(TunerApp::subscription)
        .theme(TunerApp::theme)
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
    KeySelected(u8),           // User selected a piano key (0-87)
    SwitchToAutoMode,          // Switch from manual to automatic pitch detection
    
    // --- Messages for Inharmonicity Measurement & Profile ---
    ToggleMeasurementMode,     // Toggle the partial measurement mode
    CaptureButtonClicked,      // Capture button was clicked (behavior depends on current state)
    SaveProfile,               // Save the current inharmonicity profile
    LoadProfile,               // Load an inharmonicity profile from file
    // ----------------------------------------------
    
    // Settings menu items (placeholder for future implementation)
    Temperament,              // Temperament selection
    TuningStandard,           // Tuning standard (A440, etc.)
    InharmonicCurve,          // Inharmonicity curve adjustment
    SampleBuffer,             // Sample buffer size adjustment
    TuningProfile,            // Tuning profile management
    
    // Application control
    Exit,                     // Application exit request
    
    // Working tool visibility toggles
    ToggleSpectrogram,        // Show/hide spectrogram panel
    ToggleCentMeter,         // Show/hide cent meter panel
    ToggleKeySelect,         // Show/hide piano keyboard
    TogglePartials,          // Show/hide partials panel
    
    // Continuous update message
    Tick,                     // Timer tick for real-time updates
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
        key_index: u8,        // Piano key index (0-87)
        note_name: String,    // Note name (e.g., "A4", "C#3")
        target_freq: f32,     // Target frequency in Hz
    },
}

/// State for the stability-gated capture system.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureState {
    Off,        // Not capturing
    Armed,      // Ready to capture (button shows "Off")
    Capturing,  // Actively capturing (button shows "Capturing")
    Done,       // Capture is complete, data is being processed
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
    
    // UI visibility states
    pub spectrogram_visible: bool,
    pub cent_meter_visible: bool,
    pub key_select_visible: bool,
    pub partials_visible: bool,
    
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
struct TunerApp {
    // Audio processing components
    audio_worker: Option<AudioWorker>,                    // Audio thread management
    analysis_receiver: Option<Receiver<AnalysisResult>>,  // Channel to receive analysis results
    analysis_sender: Option<Sender<AnalysisResult>>,      // Channel to send analysis results
    
    // --- New Inharmonicity State ---
    stability_buffer: VecDeque<AnalysisResult>, // Buffer for checking note stability
    inharmonicity_profile: InharmonicityProfile,
    // ---------------------------------
    
    // Single source of truth for all display data
    display_data: AppDisplayData,
}

/// Audio worker thread management structure.
/// 
/// Handles the dedicated audio processing thread and provides
/// a way to shut it down gracefully.
#[derive(Debug)]
struct AudioWorker {
    shutdown_tx: Sender<()>,              // Channel to send shutdown signal
    thread_handle: Option<JoinHandle<()>>, // Handle to the audio thread
}


impl Default for TunerApp {
    /// Creates a new TunerApp instance with default settings.
    /// 
    /// Initializes the application with:
    /// - Crossbeam channels for audio data communication
    /// - All tool panels visible by default
    /// - Automatic tuning mode
    /// - Audio processing thread started
    fn default() -> Self {
        eprintln!("[MAIN] Creating TunerApp...");
        let (analysis_tx, analysis_rx) = crossbeam_channel::unbounded();
        let mut app = Self {
            audio_worker: None,
            analysis_receiver: Some(analysis_rx),
            analysis_sender: Some(analysis_tx),
            // --- Initialize new state ---
            stability_buffer: VecDeque::with_capacity(STABILITY_TARGET),
            inharmonicity_profile: InharmonicityProfile::default(),
            // ----------------------------
            // Initialize display data
            display_data: AppDisplayData {
                audio_worker_active: false, // Will be set to true after audio starts
                last_analysis: None,
                smoothing_buffer: Vec::new(),
                spectrogram_visible: true,
                cent_meter_visible: true,
                key_select_visible: true,
                partials_visible: true,
                tuning_mode: TuningMode::Auto,
                capture_state: CaptureState::Off,
            },
        };
        
        eprintln!("[MAIN] Starting audio processing...");
        app.start_audio_processing();
        eprintln!("[MAIN] TunerApp created successfully with audio enabled");
        app
    }
}

impl TunerApp {
    /// Starts the dedicated audio processing thread.
    /// 
    /// This function:
    /// 1. Creates crossbeam channels for audio data communication
    /// 2. Spawns a dedicated thread for audio capture and analysis
    /// 3. Sets up the audio worker for graceful shutdown
    /// 
    /// The audio thread runs independently and sends analysis results
    /// back to the GUI thread via the analysis channel.
    fn start_audio_processing(&mut self) {
        if let Some(analysis_tx) = self.analysis_sender.take() {
            let (shutdown_tx, shutdown_rx) = crossbeam_channel::bounded(1);
            let thread_handle = thread::spawn(move || {
            eprintln!("[AUDIO-THREAD] Starting audio thread...");
                let (raw_audio_tx, raw_audio_rx) = crossbeam_channel::unbounded::<Vec<f32>>();
            
            eprintln!("[AUDIO-THREAD] Attempting to start audio capture...");
                let (stream, sample_rate) = match audio::start_audio_capture(raw_audio_tx) {
                Ok(tuple) => {
                    eprintln!("[AUDIO-THREAD] Audio capture started successfully");
                    tuple
                },
                    Err(e) => {
                        eprintln!("[AUDIO-THREAD] Fatal Error starting audio: {}", e);
                        return;
                    }
                };
            
            eprintln!("[AUDIO-THREAD] Entering audio processing loop...");
            // Add a small delay to let GUI initialize
            std::thread::sleep(std::time::Duration::from_millis(100));
            
                loop {
                    crossbeam_channel::select! {
                        recv(raw_audio_rx) -> msg => match msg {
                            Ok(audio_frame) => {
                            // Add error handling for analysis
                            let result = match std::panic::catch_unwind(|| {
                                perform_analysis(&audio_frame, sample_rate)
                            }) {
                                Ok(result) => result,
                                Err(_) => {
                                    eprintln!("[AUDIO-THREAD] Analysis panicked, using default result");
                                    AnalysisResult {
                                        detected_frequency: None,
                                        confidence: None,
                                        cents_deviation: None,
                                        note_name: None,
                                        spectrogram_data: vec![],
                                        partials: vec![],
                                    }
                                }
                            };
                            
                            if analysis_tx.send(result).is_err() { 
                                eprintln!("[AUDIO-THREAD] Failed to send analysis result");
                                break; 
                            }
                        },
                        Err(_) => {
                            eprintln!("[AUDIO-THREAD] Audio channel closed");
                            break;
                        },
                    },
                    recv(shutdown_rx) -> _ => {
                        eprintln!("[AUDIO-THREAD] Received shutdown signal");
                        break;
                    },
                }
            }
            
            eprintln!("[AUDIO-THREAD] Stopping stream and exiting...");
            // Properly stop the stream before dropping it
            if let Err(e) = stream.pause() {
                eprintln!("[AUDIO-THREAD] Error pausing stream: {}", e);
            }
            // Give the stream a moment to fully stop
            std::thread::sleep(std::time::Duration::from_millis(50));
            drop(stream);
            eprintln!("[AUDIO-THREAD] Audio thread finished");
        });
        self.audio_worker = Some(AudioWorker {
                shutdown_tx,
                thread_handle: Some(thread_handle),
            });
        // Update the display data to reflect that audio is active
        self.display_data.audio_worker_active = true;
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
    fn update(
        &mut self,
        message: Message
    ) {
        eprintln!("[UPDATE] Received message: {:?}", message);
        
        match message {
            Message::Exit => {
                eprintln!("[MAIN] Window close requested - starting cleanup...");
                // Properly shutdown audio worker
                if let Some(mut worker) = self.audio_worker.take() {
                    eprintln!("[MAIN] Shutting down audio worker...");
                    let _ = worker.shutdown_tx.send(());
                    if let Some(handle) = worker.thread_handle.take() {
                        eprintln!("[MAIN] Waiting for audio thread to finish...");
                        // Use detach to avoid hanging on problematic thread join
                        handle.thread().unpark();
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        eprintln!("[MAIN] Audio thread detached - continuing cleanup");
                    }
                }
                // Clear channels to prevent segfault
                eprintln!("[MAIN] Clearing analysis channels...");
                self.analysis_receiver = None;
                self.analysis_sender = None;
                eprintln!("[MAIN] Cleanup completed - forcing clean exit");
                // Force clean exit to avoid segfault
                std::process::exit(0);
            }
            Message::KeySelected(key_index) => {
                // Check if the same key is already selected - if so, switch to auto mode
                if let TuningMode::Manual { key_index: current_key, .. } = &self.display_data.tuning_mode {
                    if *current_key == key_index {
                        // Same key clicked again - switch to auto mode
                        self.display_data.tuning_mode = TuningMode::Auto;
                        self.display_data.smoothing_buffer.clear();
                        return;
                    }
                }
                
                // Different key or not in manual mode - switch to manual mode with new key
                let (note_name, target_freq) = tuning::find_nearest_note_by_index(key_index);
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
                        CaptureState::Armed  // Start in Armed state (ready to capture)
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
            Message::LoadProfile => {
                match load_profile("tuning_profile.json") {
                    Ok(profile) => {
                        self.inharmonicity_profile = profile;
                        eprintln!("[MAIN] Tuning profile loaded successfully.");
                    }
                    Err(e) => eprintln!("[MAIN] Error loading profile: {}", e),
                }
            }
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
                eprintln!("[MAIN] Toggling spectrogram visibility: {} -> {}", self.display_data.spectrogram_visible, !self.display_data.spectrogram_visible);
                self.display_data.spectrogram_visible = !self.display_data.spectrogram_visible;
            }
            Message::ToggleCentMeter => {
                eprintln!("[MAIN] Toggling cent meter visibility: {} -> {}", self.display_data.cent_meter_visible, !self.display_data.cent_meter_visible);
                self.display_data.cent_meter_visible = !self.display_data.cent_meter_visible;
            }
            Message::ToggleKeySelect => {
                eprintln!("[MAIN] Toggling key select visibility: {} -> {}", self.display_data.key_select_visible, !self.display_data.key_select_visible);
                self.display_data.key_select_visible = !self.display_data.key_select_visible;
            }
            Message::TogglePartials => {
                eprintln!("[MAIN] Toggling partials visibility: {} -> {}", self.display_data.partials_visible, !self.display_data.partials_visible);
                self.display_data.partials_visible = !self.display_data.partials_visible;
            }
            Message::Tick => {
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

                // --- State reset after capture processing ---
                if self.display_data.capture_state == CaptureState::Done {
                    // Reset state after capture is processed
                    eprintln!("[MAIN] Capture complete. Resetting state to Armed.");
                    self.display_data.capture_state = CaptureState::Armed;
                }
            }
        }
    }

    // --- ADDED: New helper function to process analysis results ---
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
                    let stability_data: Vec<AnalysisResult> = self.stability_buffer.drain(..).collect();
                    // Call the processing function with the stability buffer using default operation
                    if let Some(measurement) = capture_processing::process(stability_data, ProcessingOperation::BestConfidence) {
                        // Store the measurement in the profile
                        self.inharmonicity_profile
                            .measurements
                            .insert(measurement.key_index, measurement);
                    }
                    // Initialize the "Done" timer for visual feedback
                    ui::main_display::initialize_done_timer();
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
    fn view(&self) -> Element<'_, Message> {
        create_main_view(
            &self.display_data, 
            Message::CaptureButtonClicked
        )
    }
    
    /// Creates a subscription for continuous application updates.
    /// 
    /// Returns a timer subscription that fires every 16ms (60 FPS) to ensure
    /// smooth real-time audio visualization and responsive UI updates.
    fn subscription(&self) -> Subscription<Message> {
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
    }

    /// Returns the application theme.
    /// 
    /// Currently returns the built-in dark theme for a professional appearance.
    /// This can be extended to support dynamic theme switching in the future.
    fn theme(&self) -> Theme {
        Theme::Dark
    }
}



/// Performs a full analysis on a single frame of audio data.
/// 
/// This function processes raw audio data through the complete analysis pipeline:
/// 1. Performs FFT to get frequency spectrum
/// 2. Detects fundamental frequency using PYIN algorithm
/// 3. Refines frequency detection using spectrum analysis
/// 4. Finds nearest musical note and calculates cents deviation
/// 5. Identifies harmonic partials for inharmonicity analysis
/// 
/// # Arguments
/// * `audio_frame` - Raw audio samples (typically 2048 samples)
/// * `sample_rate` - Sample rate in Hz (typically 44100 or 48000)
/// 
/// # Returns
/// * `AnalysisResult` - Complete analysis including frequency, confidence, 
///   cents deviation, note name, spectrogram data, and detected partials
fn perform_analysis(
    audio_frame: &[f32],
    sample_rate: u32
) -> AnalysisResult {
    let complex_spectrum = fft::perform_fft(audio_frame);
    let spectrogram_data = fft::spectrum_to_magnitudes(&complex_spectrum);
    
    // --- Unpack the frequency and confidence ---
    let (detected_frequency, confidence) = 
        if let Some((freq, conf)) = pitch::detect_pitch_pyin(audio_frame, sample_rate, AMPLITUDE_THRESHOLD) {
            let refined_freq = pitch::refine_from_spectrum(&spectrogram_data, freq, sample_rate);
            (refined_freq, Some(conf))
        } else {
            (None, None)
        };

    let (cents_deviation, note_name) = if let Some(freq) = detected_frequency {
        let (name, target_freq) = tuning::find_nearest_note(freq);
        let deviation = tuning::calculate_cents_deviation(freq, target_freq);
        (Some(deviation), Some(name))
    } else {
        (None, None)
    };
    
    let partials = if let Some(fundamental) = detected_frequency {
        // Search for up to 7 partials
        pitch::find_partials(&spectrogram_data, fundamental, sample_rate, 7)
    } else {
        vec![] // No fundamental, no partials
    };

    AnalysisResult {
        detected_frequency,
        confidence,
        cents_deviation,
        note_name,
        spectrogram_data,
        partials,
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

use std::fs::File;
use std::io::{Read, Write};
use serde_json;

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