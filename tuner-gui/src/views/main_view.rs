//! # Main Display Module
//!
//! This module contains the main display components and layout logic
//! for the Inharmonicity piano tuning application.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use std::time::{Duration, Instant};

/// Local timer state for managing "Done" button display
use std::sync::Mutex;
use std::sync::OnceLock;

use crate::widgets::{cent_meter, partials_display, piano_keyboard, spectrogram};

static CAPTURE_DONE_TIMER: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Initializes the "Done" timer when capture completes.
/// This should be called when the capture state changes to Done.
pub fn initialize_done_timer() {
    let timer_guard = CAPTURE_DONE_TIMER.get_or_init(|| Mutex::new(None));
    let mut timer = timer_guard.lock().unwrap();
    *timer = Some(Instant::now());
}

/// Configuration for a single button in the settings sidebar
#[derive(Debug, Clone)]
struct ButtonConfig {
    label: &'static str,
    message: Option<crate::Message>,
    button_type: ButtonType,
}

/// Different types of buttons with their styling requirements
#[derive(Debug, Clone)]
enum ButtonType {
    /// Standard button with no special styling
    Standard,
    /// Measurement mode button that changes color when active
    MeasurementMode,
    /// Disabled button (no interaction)
    Disabled,
}

/// Static settings configuration - no need for a function
const SETTINGS_CONFIG: &[(&str, &[ButtonConfig])] = &[
    (
        "Tools",
        &[
            ButtonConfig {
                label: "Spectrogram",
                message: Some(crate::Message::ToggleSpectrogram),
                button_type: ButtonType::Standard,
            },
            ButtonConfig {
                label: "Centmeter",
                message: Some(crate::Message::ToggleCentMeter),
                button_type: ButtonType::Standard,
            },
            ButtonConfig {
                label: "Key select",
                message: Some(crate::Message::ToggleKeySelect),
                button_type: ButtonType::Standard,
            },
            ButtonConfig {
                label: "Partials",
                message: Some(crate::Message::TogglePartials),
                button_type: ButtonType::Standard,
            },
            // ButtonConfig {
            //     label: "Inharmonicity Graph",
            //     message: Some(crate::Message::ToggleInharmonicityGraph),
            //     button_type: ButtonType::Standard,
            // },
            ButtonConfig {
                label: "Measurement Mode",
                message: Some(crate::Message::ToggleMeasurementMode),
                button_type: ButtonType::MeasurementMode,
            },
        ],
    ),
    (
        "Systemic change",
        &[
            ButtonConfig {
                label: "Temperament",
                message: None,
                button_type: ButtonType::Disabled,
            },
            ButtonConfig {
                label: "Tuning Standard",
                message: None,
                button_type: ButtonType::Disabled,
            },
            ButtonConfig {
                label: "Inharmonic curve adjustment",
                message: None,
                button_type: ButtonType::Disabled,
            },
        ],
    ),
    (
        "Program",
        &[
            ButtonConfig {
                label: "Sample Buffer adjustment",
                message: None,
                button_type: ButtonType::Disabled,
            },
            ButtonConfig {
                label: "Save Profile",
                message: Some(crate::Message::SaveProfile),
                button_type: ButtonType::Standard,
            },
            ButtonConfig {
                label: "Load Profile",
                message: Some(crate::Message::LoadProfile),
                button_type: ButtonType::Standard,
            },
        ],
    ),
];

/// Creates the complete main application view
pub fn create_main_view(
    data: &crate::app::AppDisplayData,
    _profile: &tuner_core::inharmonicity::InharmonicityProfile,
    capture_message: crate::Message,
) -> Element<'static, crate::Message> {
    // Show shutdown message if audio worker is not active
    if !data.audio_worker_active {
        return container(text("Shutting down...").size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let widget_area = create_widget_area(data);

    // Create sidebar
    let sidebar = create_sidebar(data.capture_state.clone(), capture_message);

    // Assemble the final layout
    let main_content = row![sidebar, Space::new().width(10), widget_area,]
        .align_y(Alignment::Start)
        .padding(20);

    container(main_content).width(Fill).height(Fill).into()
}

/// Creates the isolated widget area layout (spectrogram, cent meter, keyboard, partials).
/// This can be used independently of the settings sidebar.
pub fn create_widget_area(data: &crate::app::AppDisplayData) -> Element<'static, crate::Message> {
    let title = text("Inharmonicity").size(28);

    // Build UI panels using dedicated helper methods
    let spectrogram_panel = create_spectrogram_panel(data);
    let cent_meter_panel = create_cent_meter_panel(data);
    let keyboard_panel = create_keyboard_panel(data);
    let partials_panel = create_partials_panel(data);
    // let inharmonicity_graph_panel = create_inharmonicity_graph_panel(data, profile);

    // A helper function to safely embed optional widgets into a row/column layout.
    // If an optional panel (e.g., Spectrogram) is turned off and returns `None`,
    // this cleanly substitutes it with an invisible `Space` widget, preventing
    // missing elements from breaking the UI layout.
    fn wrap_panel(p: Option<Element<'static, crate::Message>>) -> Element<'static, crate::Message> {
        p.unwrap_or_else(|| Space::new().into())
    }

    let top_row = row![
        wrap_panel(spectrogram_panel),
        Space::new().width(10),
        wrap_panel(cent_meter_panel)
    ]
    .width(Fill)
    .align_y(Alignment::Start);

    let bottom_row = row![
        wrap_panel(keyboard_panel),
        Space::new().width(10),
        wrap_panel(partials_panel)
    ]
    .width(Fill)
    .align_y(Alignment::Start);

    // let inharmonicity_graph_row = row![
    //     wrap_panel(inharmonicity_graph_panel),
    // ]
    // .width(Length::Fill)
    // .align_y(Alignment::Start);

    column![
        title,
        Space::new().height(20),
        top_row,
        Space::new().height(10),
        bottom_row,
    ]
    .width(Fill)
    .spacing(10)
    .into()
}

/// Creates the spectrogram panel widget.
fn create_spectrogram_panel(
    data: &crate::app::AppDisplayData,
) -> Option<Element<'static, crate::Message>> {
    if !data.spectrogram_visible {
        return None;
    }

    let spectrogram_data = data
        .last_analysis
        .as_ref()
        .map(|a| a.spectrogram_data.clone())
        .unwrap_or_default();

    let spectrogram_content: Element<'static, crate::Message> =
        container(spectrogram::Spectrogram::new(spectrogram_data).view())
            .width(Fill)
            .height(Fill)
            .into();

    let panel = container(
        column![
            text("Spectrogram").size(18),
            Space::new().height(10),
            spectrogram_content
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(250.0));

    Some(panel.into())
}

/// Creates the cent meter panel
fn create_cent_meter_panel(
    data: &crate::app::AppDisplayData,
) -> Option<Element<'static, crate::Message>> {
    if !data.cent_meter_visible {
        return None;
    }

    // Calculate smoothed cent deviation
    let smoothed_cents = if data.smoothing_buffer.is_empty() {
        data.last_analysis
            .as_ref()
            .and_then(|analysis| analysis.cents_deviation)
    } else {
        let sum: f32 = data.smoothing_buffer.iter().sum();
        let count = data.smoothing_buffer.len() as f32;
        if count > 0.0 { Some(sum / count) } else { None }
    };

    let (note_name, freq_text, confidence, _target_freq_text) = if let Some(analysis) =
        &data.last_analysis
    {
        let current_freq = analysis.detected_frequency.unwrap_or(0.0);
        let note_text = match &data.tuning_mode {
            crate::app::TuningMode::Auto => analysis
                .note_name
                .clone()
                .unwrap_or_else(|| "--".to_string()),
            crate::app::TuningMode::Manual { note_name, .. } => note_name.clone(),
        };
        let target_freq_text = match &data.tuning_mode {
            crate::app::TuningMode::Auto => String::from("Auto"),
            crate::app::TuningMode::Manual { target_freq, .. } => format!("{:.1} Hz", target_freq),
        };
        // Convert the confidence value (0.0-1.0) to a percentage string.
        let confidence_text = analysis
            .confidence
            .map(|c| format!("{:.0}%", c * 100.0))
            .unwrap_or_else(|| "0%".to_string());

        (
            note_text,
            format!("{:.2} Hz", current_freq),
            confidence_text,
            target_freq_text,
        )
    } else {
        (
            "--".to_string(),
            "0.00 Hz".to_string(),
            "0%".to_string(),
            "Auto".to_string(),
        )
    };

    let cent_meter_content: Element<'static, crate::Message> = container(
        cent_meter::CentMeterDisplay::new(smoothed_cents, note_name, freq_text, confidence).view(),
    )
    .width(Fill)
    .height(Fill)
    .into();

    let panel = container(
        column![
            text("Cent Meter").size(18),
            Space::new().height(10),
            cent_meter_content
        ]
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(200.0));

    Some(panel.into())
}

/// Creates the piano keyboard panel
fn create_keyboard_panel(
    data: &crate::app::AppDisplayData,
) -> Option<Element<'static, crate::Message>> {
    if !data.key_select_visible {
        return None;
    }

    // Determine detected and selected key indices
    let detected_key_index = data
        .last_analysis
        .as_ref()
        .and_then(|analysis| analysis.note_name.as_ref())
        .and_then(|name| {
            Some(tuner_core::algorithms::tuning::get_key_index_from_name(
                name,
            ))
        });

    let selected_key_index = match &data.tuning_mode {
        crate::app::TuningMode::Manual { key_index, .. } => Some(*key_index),
        crate::app::TuningMode::Auto => detected_key_index,
    };

    let piano_keyboard = piano_keyboard::PianoKeyboard::new(detected_key_index, selected_key_index);

    let keyboard_content: Element<'static, crate::Message> = piano_keyboard.view();

    let panel = container(
        column![
            text("Keyboard Key Select").size(18),
            Space::new().height(10),
            keyboard_content
        ]
        .width(Fill)
        .height(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(200.0));

    Some(panel.into())
}

/// Creates the partials display panel
fn create_partials_panel(
    data: &crate::app::AppDisplayData,
) -> Option<Element<'static, crate::Message>> {
    if !data.partials_visible {
        return None;
    }

    let partials_data = data
        .last_analysis
        .as_ref()
        .map(|a| a.partials.clone())
        .unwrap_or_default();

    let partials_content: Element<'static, crate::Message> =
        partials_display::PartialsDisplay::new(partials_data).view();

    let panel = container(
        column![
            text("Partials").size(18),
            Space::new().height(10),
            partials_content
        ]
        .width(Fill)
        .height(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(180.0));

    Some(panel.into())
}

// /// Creates the inharmonicity graph panel
// fn create_inharmonicity_graph_panel(
//     data: &crate::AppDisplayData,
//     // --- MODIFIED: Accept profile as a reference ---
//     profile: &tuner_core::inharmonicity::InharmonicityProfile,
// ) -> Option<Element<'static, crate::Message>> {
//     if !data.inharmonicity_graph_visible {
//         return None;
//     }
//
//     // --- MODIFIED: Use the passed profile reference ---
//     let graph_content = inharmonicity_graph::InharmonicityGraph::new(profile).view();
//
//     let panel = container(
//         column![
//             text("Inharmonicity 'B' Values").size(18),
//             Space::new().height(10),
//             graph_content
//         ]
//         .spacing(5)
//         .padding(15),
//     )
//     .width(Length::Fill)
//     .height(Length::Fixed(250.0)); // Graph panel is a bit taller
//
//     Some(panel.into())
// }

/// Creates the settings sidebar widget.
///
/// Builds the right-side settings panel containing all application controls
/// organized into logical sections (Tools, Systemic change, Program). The sidebar
/// includes tool visibility toggles, measurement mode controls, and profile
/// management buttons. When in measurement mode, it also displays a large
/// capture button for recording partial measurements.
///
/// # Arguments
/// * `capture_state` - Current capture state (Off, Armed, Done)
/// * `capture_message` - Message to send when capture button is pressed
///
/// # Returns
/// * `Element` - Complete sidebar widget with all controls and sections
fn create_sidebar(
    capture_state: crate::app::CaptureState,
    capture_message: crate::Message,
) -> Element<'static, crate::Message> {
    let mut sections = column![].spacing(10);

    // Add all settings sections
    for (title, buttons) in SETTINGS_CONFIG {
        let in_measurement_mode = capture_state != crate::app::CaptureState::Off;
        sections = sections.push(make_settings_section(title, buttons, in_measurement_mode));
    }

    // Add capture button if in measurement mode
    if capture_state != crate::app::CaptureState::Off {
        sections = sections.push(make_capture_button(capture_state, capture_message));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}

/// Creates a button based on configuration and application state.
///
/// Generates a styled button widget based on the provided configuration.
/// Applies different visual styles based on button type (Standard, MeasurementMode, Disabled)
/// and current application state. Measurement mode buttons change color when active,
/// while disabled buttons are grayed out and non-interactive.
///
/// # Arguments
/// * `config` - Button configuration containing label, message, and type
/// * `in_measurement_mode` - Whether the application is in measurement mode
///
/// # Returns
/// * `Element` - Styled button widget with appropriate message handler
fn make_button(
    config: &ButtonConfig,
    in_measurement_mode: bool,
) -> Element<'static, crate::Message> {
    let mut button = button(text(config.label).size(14).width(Fill)).padding([6, 10]);

    // Apply styling based on button type and state
    match config.button_type {
        ButtonType::Standard => {
            // No special styling needed
        }
        ButtonType::MeasurementMode => {
            if in_measurement_mode {
                button = button.style(|_theme, _status| {
                    use iced::widget::button;
                    button::Style {
                        background: Some(iced::Background::Color(iced::Color::from_rgb(
                            0.8, 0.2, 0.2,
                        ))), // Red background
                        text_color: iced::Color::WHITE,
                        ..button::Style::default()
                    }
                });
            }
        }
        ButtonType::Disabled => {
            button = button.style(|_theme, _status| {
                use iced::widget::button;
                button::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(
                        0.3, 0.3, 0.3,
                    ))), // Gray background
                    text_color: iced::Color::from_rgb(0.6, 0.6, 0.6), // Gray text
                    ..button::Style::default()
                }
            });
        }
    }

    // Add message handler if available
    if let Some(message) = &config.message {
        button.on_press(message.clone()).into()
    } else {
        button.into()
    }
}

/// Creates a large Capture button for measurement mode.
///
/// Generates a special large capture button that appears only in measurement mode.
/// The button changes appearance based on its state:
/// - Off: Gray button with "Off" text
/// - Armed: Gold button with "Capture" text  
/// - Done: Green button with "Done" text (shows for 3 seconds)
/// This provides clear visual feedback for the measurement process.
///
/// # Arguments
/// * `capture_state` - Current capture state (Off, Armed, Done)
/// * `capture_message` - Message to send when the button is pressed
///
/// # Returns
/// * `Element` - Large, prominently styled capture button
fn make_capture_button(
    capture_state: crate::app::CaptureState,
    capture_message: crate::Message,
) -> Element<'static, crate::Message> {
    // Handle timer logic for "Done" state display
    let should_show_done = {
        let timer_guard = CAPTURE_DONE_TIMER.get_or_init(|| Mutex::new(None));
        let mut timer = timer_guard.lock().unwrap();

        // Check if we should show "Done" based on timer
        if let Some(start_time) = *timer {
            let elapsed = start_time.elapsed();
            if elapsed < Duration::from_secs(1) {
                // Still within 1 second - show "Done"
                true
            } else {
                // Timer expired - clear timer and don't show "Done"
                *timer = None;
                false
            }
        } else {
            // No timer set - don't show "Done"
            false
        }
    };

    let (text_label, color, _pulsing) = if should_show_done {
        ("Done", iced::Color::from_rgb(0.2, 0.8, 0.2), false) // Green
    } else {
        // Show normal button behavior based on actual state
        match capture_state {
            // When off, display "Off" and slightly dim the button.
            crate::app::CaptureState::Off => ("Off", iced::Color::from_rgb(0.5, 0.5, 0.5), false),
            // When armed, display Armed and use a neutral or slightly distinct color...
            crate::app::CaptureState::Armed => {
                ("Armed", iced::Color::from_rgb(0.8, 0.6, 0.2), false)
            } // Orange-ish
            // When actively capturing (waiting for stability), indicate progress.
            crate::app::CaptureState::Capturing => {
                ("Capturing...", iced::Color::from_rgb(0.8, 0.2, 0.2), true)
            } // Red and pulsing
            // When done, show confirmed status. (A timer elsewhere resets this to Armed).
            crate::app::CaptureState::Done => {
                ("Done!", iced::Color::from_rgb(0.2, 0.8, 0.2), false)
            } // Green
        }
    };

    button(text(text_label).size(18).width(Fill))
        .padding([12, 20])
        .style(move |_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(color)),
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(capture_message)
        .into()
}

/// Creates a settings section with title and buttons.
///
/// Builds a grouped section of the settings sidebar with a title and
/// a vertical list of buttons. Each section represents a logical grouping
/// of related controls (e.g., "Tools", "Systemic change", "Program").
/// The buttons within each section are styled according to their type
/// and the current application state.
///
/// # Arguments
/// * `title` - Section title (e.g., "Tools", "Program")
/// * `buttons` - Array of button configurations for this section
/// * `in_measurement_mode` - Whether the application is in measurement mode
///
/// # Returns
/// * `Element` - Complete settings section with title and button list
fn make_settings_section(
    title: &'static str,
    buttons: &[ButtonConfig],
    in_measurement_mode: bool,
) -> Element<'static, crate::Message> {
    let title_widget = text(title).size(18);

    let items_widget = buttons.iter().fold(column![].spacing(8), |col, config| {
        col.push(make_button(config, in_measurement_mode))
    });

    column![title_widget, Space::new().height(10), items_widget]
        .spacing(5)
        .into()
}
