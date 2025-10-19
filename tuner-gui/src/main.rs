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
    inharmonicity::{InharmonicityProfile, KeyMeasurement, Partial}
};
use ui::main_display::{create_main_view, AppDisplayData};

// Audio processing constants
const SMOOTHING_FACTOR: usize = 5;  // Number of samples for cent smoothing
const AMPLITUDE_THRESHOLD: f32 = 0.01;  // Minimum amplitude for pitch detection



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
    CaptureMeasurement,        // Capture the current note's partials
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
enum TuningMode {
    /// Automatic pitch detection mode - detects any note being played
    Auto,
    /// Manual mode - user has selected a specific piano key to tune
    Manual {
        key_index: u8,        // Piano key index (0-87)
        note_name: String,    // Note name (e.g., "A4", "C#3")
        target_freq: f32,     // Target frequency in Hz
    },
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
    
    // Current analysis state
    last_analysis: Option<AnalysisResult>,                // Most recent audio analysis
    tuning_mode: TuningMode,                             // Current tuning mode
    smoothing_buffer: VecDeque<f32>,                      // Buffer for cent smoothing
    
    // --- New Inharmonicity State ---
    in_measurement_mode: bool,
    capture_button_clicked: bool,  // Track if capture button has been clicked
    inharmonicity_profile: InharmonicityProfile,
    // ---------------------------------
    
    // UI visibility states
    spectrogram_visible: bool,    // Show/hide spectrogram panel
    cent_meter_visible: bool,     // Show/hide cent meter panel
    key_select_visible: bool,     // Show/hide piano keyboard
    partials_visible: bool,       // Show/hide partials panel
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
            last_analysis: None,
            smoothing_buffer: VecDeque::with_capacity(SMOOTHING_FACTOR),
            tuning_mode: TuningMode::Auto,
            // --- Initialize new state ---
            in_measurement_mode: false,
            capture_button_clicked: false,
            inharmonicity_profile: InharmonicityProfile::default(),
            // ----------------------------
            // Initialize all tools as visible by default
            spectrogram_visible: true,
            cent_meter_visible: true,
            key_select_visible: true,
            partials_visible: true,
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
                if let TuningMode::Manual { key_index: current_key, .. } = &self.tuning_mode {
                    if *current_key == key_index {
                        // Same key clicked again - switch to auto mode
                        self.tuning_mode = TuningMode::Auto;
                        self.smoothing_buffer.clear();
                        return;
                    }
                }
                
                // Different key or not in manual mode - switch to manual mode with new key
                let (note_name, target_freq) = tuning::find_nearest_note_by_index(key_index);
                self.tuning_mode = TuningMode::Manual {
                    key_index,
                    note_name,
                    target_freq,
                };
                self.smoothing_buffer.clear();
            }
            Message::SwitchToAutoMode => {
                self.tuning_mode = TuningMode::Auto;
                self.smoothing_buffer.clear();
            }
            Message::ToggleMeasurementMode => {
                self.in_measurement_mode = !self.in_measurement_mode;
                self.capture_button_clicked = false; // Reset capture button state
                eprintln!("[MAIN] Measurement mode set to: {}", self.in_measurement_mode);
            }
            Message::CaptureMeasurement => {
                if !self.in_measurement_mode { return; }
                
                // Toggle capture button state
                self.capture_button_clicked = !self.capture_button_clicked;
                
                if self.capture_button_clicked {
                    // Only capture when button is being activated (not deactivated)
                    if let Some(analysis) = &self.last_analysis {
                        if let (Some(note_name), Some(freq)) = (&analysis.note_name, analysis.detected_frequency) {
                            let key_index = tuning::get_key_index_from_name(note_name);

                            // Create the fundamental partial (n=1)
                            let mut all_partials = vec![Partial { number: 1, frequency: freq }];
                            
                            // Create the overtone partials (n=2, 3, 4...)
                            let overtone_partials = analysis.partials.iter().enumerate()
                                .map(|(i, &freq)| Partial {
                                    number: (i + 2) as u32, // find_partials starts at the 2nd partial
                                    frequency: freq,
                                });
                            all_partials.extend(overtone_partials);
                            
                            let mut measurement = KeyMeasurement {
                                key_index,
                                partials: all_partials,
                                calculated_b: None,
                            };
                            
                            // Calculate the 'B' value immediately
                            measurement.calculate_b_value();
                            
                            eprintln!("[MAIN] Captured measurement for {}: B={:?}", note_name, measurement.calculated_b);
                            
                            // Store it in the profile
                            self.inharmonicity_profile.measurements.insert(key_index, measurement);
                        } else {
                            eprintln!("[MAIN] Capture failed: No stable note detected.");
                        }
                    }
                } else {
                    eprintln!("[MAIN] Capture ended.");
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
                eprintln!("[MAIN] Toggling spectrogram visibility: {} -> {}", self.spectrogram_visible, !self.spectrogram_visible);
                self.spectrogram_visible = !self.spectrogram_visible;
            }
            Message::ToggleCentMeter => {
                eprintln!("[MAIN] Toggling cent meter visibility: {} -> {}", self.cent_meter_visible, !self.cent_meter_visible);
                self.cent_meter_visible = !self.cent_meter_visible;
            }
            Message::ToggleKeySelect => {
                eprintln!("[MAIN] Toggling key select visibility: {} -> {}", self.key_select_visible, !self.key_select_visible);
                self.key_select_visible = !self.key_select_visible;
            }
            Message::TogglePartials => {
                eprintln!("[MAIN] Toggling partials visibility: {} -> {}", self.partials_visible, !self.partials_visible);
                self.partials_visible = !self.partials_visible;
            }
            Message::Tick => {
                // Continuous update - poll for audio data
                if let Some(receiver) = &self.analysis_receiver {
                    while let Ok(result) = receiver.try_recv() {
                        let cents_for_smoothing = match self.tuning_mode {
                            TuningMode::Auto => result.cents_deviation,
                            TuningMode::Manual { target_freq, .. } => result
                                .detected_frequency
                                .map(|freq| tuning::calculate_cents_deviation(freq, target_freq)),
                        };
                        if let Some(cents) = cents_for_smoothing {
                            self.smoothing_buffer.push_back(cents);
                            if self.smoothing_buffer.len() > SMOOTHING_FACTOR {
                                self.smoothing_buffer.pop_front();
                            }
                        } else {
                            self.smoothing_buffer.clear();
                        }
                        self.last_analysis = Some(result);
                    }
                }
            }
        }
    }

    /// Renders the main application interface.
    /// 
    /// Delegates all UI rendering to the main_display module,
    /// keeping this function focused on application logic only.
    fn view(&self) -> Element<'_, Message> {
        let display_data = AppDisplayData {
            audio_worker_active: self.audio_worker.is_some(),
            last_analysis: self.last_analysis.clone(),
            smoothing_buffer: self.smoothing_buffer.iter().cloned().collect(),
            spectrogram_visible: self.spectrogram_visible,
            cent_meter_visible: self.cent_meter_visible,
            key_select_visible: self.key_select_visible,
            partials_visible: self.partials_visible,
            tuning_mode: self.tuning_mode.clone(),
            in_measurement_mode: self.in_measurement_mode,
            capture_button_clicked: self.capture_button_clicked,
        };

        create_main_view(&display_data, &(), Message::CaptureMeasurement)
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