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

mod widgets;

use crossbeam_channel::{Receiver, Sender};
use cpal::traits::StreamTrait;
use iced::widget::{
    button, column, container, row, text, horizontal_space, Space
};
use iced::{
    self, Alignment, Element, Length, Theme, Subscription
};
use std::collections::VecDeque;
use std::thread::{self, JoinHandle};
use tuner_core::{audio, fft, pitch, tuning, AnalysisResult};

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
    let result = iced::application("Inharmonicity", update, view)
        .subscription(subscription)
        .theme(theme)
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
    
    // Settings menu items (placeholder for future implementation)
    PartialMeasurement,        // Partial measurement settings
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
                                            cents_deviation: None,
                                            note_name: None,
                                            spectrogram_data: vec![],
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
}

/// Handles application state updates based on incoming messages.
/// 
/// This function processes all user interactions and system events,
/// updating the application state accordingly. It handles:
/// - Piano key selections and tuning mode changes
/// - Tool visibility toggles
/// - Audio analysis data processing
/// - Application exit requests
fn update(app: &mut TunerApp, message: Message) {
    eprintln!("[UPDATE] Received message: {:?}", message);
    
    match message {
        Message::Exit => {
            eprintln!("[MAIN] Window close requested - starting cleanup...");
            // Properly shutdown audio worker
            if let Some(mut worker) = app.audio_worker.take() {
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
            app.analysis_receiver = None;
            app.analysis_sender = None;
            eprintln!("[MAIN] Cleanup completed - forcing clean exit");
            // Force clean exit to avoid segfault
            std::process::exit(0);
        }
        Message::KeySelected(key_index) => {
            // Check if the same key is already selected - if so, switch to auto mode
            if let TuningMode::Manual { key_index: current_key, .. } = &app.tuning_mode {
                if *current_key == key_index {
                    // Same key clicked again - switch to auto mode
                    app.tuning_mode = TuningMode::Auto;
                    app.smoothing_buffer.clear();
                    return;
                }
            }
            
            // Different key or not in manual mode - switch to manual mode with new key
            let (note_name, target_freq) = tuning::find_nearest_note_by_index(key_index);
            app.tuning_mode = TuningMode::Manual {
                key_index,
                note_name,
                target_freq,
            };
            app.smoothing_buffer.clear();
        }
        Message::SwitchToAutoMode => {
            app.tuning_mode = TuningMode::Auto;
            app.smoothing_buffer.clear();
        }
        Message::PartialMeasurement => {
            // Placeholder for partial measurement functionality
        }
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
            eprintln!("[MAIN] Toggling spectrogram visibility: {} -> {}", app.spectrogram_visible, !app.spectrogram_visible);
            app.spectrogram_visible = !app.spectrogram_visible;
        }
        Message::ToggleCentMeter => {
            eprintln!("[MAIN] Toggling cent meter visibility: {} -> {}", app.cent_meter_visible, !app.cent_meter_visible);
            app.cent_meter_visible = !app.cent_meter_visible;
        }
        Message::ToggleKeySelect => {
            eprintln!("[MAIN] Toggling key select visibility: {} -> {}", app.key_select_visible, !app.key_select_visible);
            app.key_select_visible = !app.key_select_visible;
        }
        Message::TogglePartials => {
            eprintln!("[MAIN] Toggling partials visibility: {} -> {}", app.partials_visible, !app.partials_visible);
            app.partials_visible = !app.partials_visible;
        }
        Message::Tick => {
            // Continuous update - poll for audio data
            if let Some(receiver) = &app.analysis_receiver {
                while let Ok(result) = receiver.try_recv() {
            let cents_for_smoothing = match app.tuning_mode {
                TuningMode::Auto => result.cents_deviation,
                TuningMode::Manual { target_freq, .. } => result
                    .detected_frequency
                    .map(|freq| tuning::calculate_cents_deviation(freq, target_freq)),
            };
            if let Some(cents) = cents_for_smoothing {
                app.smoothing_buffer.push_back(cents);
                if app.smoothing_buffer.len() > SMOOTHING_FACTOR {
                    app.smoothing_buffer.pop_front();
                }
            } else {
                app.smoothing_buffer.clear();
            }
            app.last_analysis = Some(result);
                }
            }
        }
    }
}

/// Renders the main application interface.
/// 
/// Creates the complete GUI layout including:
/// - Title and main content area
/// - Spectrogram and cent meter panels (top row)
/// - Piano keyboard and partials panels (bottom row)
/// - Settings sidebar with tool controls
/// 
/// The layout is responsive and adapts to tool visibility states.
fn view(app: &TunerApp) -> Element<'_, Message> {
    eprintln!("[VIEW] Rendering GUI with state: spectrogram={}, cent_meter={}, key_select={}, partials={}", 
            app.spectrogram_visible, app.cent_meter_visible, app.key_select_visible, app.partials_visible);
    
    if app.audio_worker.is_none() && app.analysis_sender.is_none() {
        return container(text("Shutting down...").size(40))
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
    }

    // Title at the top
    let title = text("Inharmonicity").size(28);

    // --- SPECTROGRAM PANEL (Larger) ---
    let spectrogram_data = app.last_analysis.as_ref()
        .map(|a| a.spectrogram_data.clone())
        .unwrap_or_default();
    
    let spectrogram_panel = if app.spectrogram_visible {
        let spectrogram_content = container(
            widgets::spectrogram::Spectrogram::new(spectrogram_data).view()
        )
        .width(Length::Fill)
        .height(Length::Fill);
        
        Some(container(
            column![
                text("Spectrogram").size(18),
                Space::with_height(10),
                spectrogram_content
            ]
            .spacing(5)
            .padding(15)
        )
        .width(Length::Fill)
        .height(Length::Fixed(250.0)))
    } else {
        None
    };

    // --- CENT METER PANEL (Smaller) ---
    let smoothed_cents = if app.smoothing_buffer.is_empty() { 
        None 
    } else { 
        Some(app.smoothing_buffer.iter().sum::<f32>() / app.smoothing_buffer.len() as f32) 
    };
    
    let (note_name, freq_text, confidence) = if let Some(analysis) = &app.last_analysis {
        let current_freq = analysis.detected_frequency.unwrap_or(0.0);
        let note_text = match &app.tuning_mode {
            TuningMode::Auto => analysis.note_name.clone().unwrap_or_else(|| "--".to_string()),
            TuningMode::Manual { note_name, .. } => note_name.clone(),
        };
        (note_text, format!("{:.2} Hz", current_freq), "81%".to_string())
    } else { 
        ("--".to_string(), "0.00 Hz".to_string(), "0%".to_string()) 
    };

    let cent_meter_panel = if app.cent_meter_visible {
        let cent_meter_content = column![
            row![
                text("Note").size(14),
                horizontal_space(),
                text("Confidence").size(14),
            ],
            Space::with_height(5),
            row![
                text(note_name).size(24),
                Space::with_width(10),
                text(freq_text).size(24),
                horizontal_space(),
                container(text(confidence).size(16))
                    .padding([4, 8]),
            ]
            .align_y(Alignment::Center),
            Space::with_height(10),
            widgets::cent_meter::CentMeter::new(smoothed_cents).view()
        ]
        .spacing(5);
        
        Some(container(
            column![
                text("Cent Meter").size(18),
                Space::with_height(10),
                cent_meter_content
            ]
            .spacing(5)
            .padding(15)
        )
        .width(Length::Fill)
        .height(Length::Fixed(180.0)))
    } else {
        None
    };

    // --- PARTIALS PANEL (Smaller) ---
    let partials_panel = if app.partials_visible {
        let partials_content = column![
            partial_row("Partial 1", "21"),
            partial_row("Partial 2", "42"),
            partial_row("Partial 3", "84"),
            partial_row("Partial 4", "130"),
            partial_row("Partial 5", "240"),
        ]
        .spacing(8);
        
        Some(container(
            column![
                text("Partials").size(18),
                Space::with_height(10),
                partials_content
            ]
            .spacing(5)
            .padding(15)
        )
        .width(Length::Fill)
        .height(Length::Fixed(180.0)))
    } else {
        None
    };

    // --- KEYBOARD PANEL (Larger) ---
    let (detected_key, selected_key) = if let Some(analysis) = &app.last_analysis {
        let detected_key_index = analysis.note_name.as_ref()
            .map(|name| tuning::get_key_index_from_name(name));
        match &app.tuning_mode {
            TuningMode::Auto => (detected_key_index, None),
            TuningMode::Manual { key_index, .. } => (detected_key_index, Some(*key_index)),
        }
    } else { 
        (None, None) 
    };
    
    let piano_keyboard = widgets::piano_keyboard::PianoKeyboard::new(detected_key, selected_key);

    let keyboard_panel = if app.key_select_visible {
        let keyboard_content = container(piano_keyboard.view())
            .width(Length::Fill)
            .height(Length::Fill);
        
        Some(container(
            column![
                text("KEYBOARD Key Select").size(18),
                Space::with_height(10),
                keyboard_content
            ]
            .spacing(5)
            .padding(15)
        )
        .width(Length::Fill)
        .height(Length::Fixed(200.0)))
    } else {
        None
    };

    // --- RIGHT SIDEBAR ---
    let sidebar = container(
        column![
        make_settings_section("Tools", vec![
                ("Spectrogram", Some(Message::ToggleSpectrogram)), // Working tool - toggle visibility
                ("Centmeter", Some(Message::ToggleCentMeter)),   // Working tool - toggle visibility
                ("Key select", Some(Message::ToggleKeySelect)),  // Working tool - toggle visibility
                ("Partials", Some(Message::TogglePartials)),     // Working tool - toggle visibility
                ("Partial Measurement", Some(Message::PartialMeasurement)), // Non-working - grayed out
            ]),
            Space::with_height(20),
        make_settings_section("Systemic change", vec![
                ("Tempermant", Some(Message::Temperament)), // Non-working - grayed out
                ("Tuning Standard", Some(Message::TuningStandard)), // Non-working - grayed out
                ("Inharmonic curve adjustment", Some(Message::InharmonicCurve)), // Non-working - grayed out
            ]),
            Space::with_height(20),
        make_settings_section("Program", vec![
                ("Sample Buffer adjustment", Some(Message::SampleBuffer)), // Non-working - grayed out
                ("Tuning Profile", Some(Message::TuningProfile)), // Non-working - grayed out
            ]),
        ]
        .spacing(10)
        .padding(15)
    )
    .width(Length::Fixed(250.0))
    .height(Length::Fill)
    ;

    // Build top row dynamically based on visibility
    let top_row: Element<Message> = match (spectrogram_panel, cent_meter_panel) {
        (Some(spectrogram), Some(cent_meter)) => {
            row![
                spectrogram,
                Space::with_width(10),
                cent_meter,
            ]
            .align_y(Alignment::Start)
            .into()
        }
        (Some(spectrogram), None) => {
            row![spectrogram].align_y(Alignment::Start).into()
        }
        (None, Some(cent_meter)) => {
            row![cent_meter].align_y(Alignment::Start).into()
        }
        (None, None) => {
            Space::with_height(0).into()
        }
    };
    
    // Build bottom row dynamically based on visibility
    let bottom_row: Element<Message> = match (keyboard_panel, partials_panel) {
        (Some(keyboard), Some(partials)) => {
            row![
                keyboard,
                Space::with_width(10),
                partials,
            ]
            .align_y(Alignment::Start)
            .into()
        }
        (Some(keyboard), None) => {
            row![keyboard].align_y(Alignment::Start).into()
        }
        (None, Some(partials)) => {
            row![partials].align_y(Alignment::Start).into()
        }
        (None, None) => {
            Space::with_height(0).into()
        }
    };
    
    // Main layout - matching the reference image structure
    let main_content = row![
        column![
            title,
            Space::with_height(20),
            top_row,
            Space::with_height(10),
            bottom_row,
        ]
        .width(Length::Fill)
        .spacing(10),
        Space::with_width(10),
        sidebar,
    ]
    .align_y(Alignment::Start)
    .padding(20);

    container(main_content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}


fn partial_row(name: &'static str, value: &'static str) -> Element<'static, Message> {
    row![
        container(Space::new(Length::Fixed(12.0), Length::Fixed(12.0)))
            .width(Length::Fixed(12.0))
            .height(Length::Fixed(12.0)),
        Space::with_width(8),
        text(name).size(14),
        horizontal_space(),
        text(value).size(14),
    ]
    .align_y(Alignment::Center)
        .into()
}

fn make_settings_section<'a>(
    title: &'a str,
    items: Vec<(&'a str, Option<Message>)>
) -> Element<'a, Message> {
    let title_widget = text(title).size(18);
    
    let items_widget = items.into_iter().fold(
        column![].spacing(8),
        |col, (label, msg)| {
            let button = if msg.is_some() {
                // Non-working tools - grayed out
                button(text(label).size(14).width(Length::Fill))
                    .padding([6, 10])
                    .on_press(msg.unwrap())
            } else {
                // Working tools - normal appearance
                button(text(label).size(14).width(Length::Fill))
                    .padding([6, 10])
            };
            
                col.push(button)
        }
    );

    let section = column![
        title_widget,
        Space::with_height(10),
        items_widget
    ]
    .spacing(5);

    section.into()
}

fn perform_analysis(audio_frame: &[f32], sample_rate: u32) -> AnalysisResult {
    let complex_spectrum = fft::perform_fft(audio_frame);
    let spectrogram_data = fft::spectrum_to_magnitudes(&complex_spectrum);
    let detected_frequency = pitch::detect_pitch_yin(audio_frame, sample_rate, AMPLITUDE_THRESHOLD)
        .and_then(|rf| pitch::refine_from_spectrum(&spectrogram_data, rf, sample_rate));

    let (cents_deviation, note_name) = if let Some(freq) = detected_frequency {
        let (name, target_freq) = tuning::find_nearest_note(freq);
        let deviation = tuning::calculate_cents_deviation(freq, target_freq);
        (Some(deviation), Some(name))
    } else {
        (None, None)
    };

    AnalysisResult {
        detected_frequency,
        cents_deviation,
        note_name,
        spectrogram_data,
    }
}

/// Creates a subscription for continuous application updates.
/// 
/// Returns a timer subscription that fires every 16ms (60 FPS) to ensure
/// smooth real-time audio visualization and responsive UI updates.
fn subscription(_app: &TunerApp) -> Subscription<Message> {
    iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick)
}

/// Returns the application theme.
/// 
/// Currently returns the built-in dark theme for a professional appearance.
/// This can be extended to support dynamic theme switching in the future.
fn theme(_app: &TunerApp) -> Theme {
    Theme::Dark
}


