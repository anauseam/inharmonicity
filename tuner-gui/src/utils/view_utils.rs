use iced::widget::{Space, button, column, text};
use iced::{Element, Fill};

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

static CAPTURE_DONE_TIMER: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

/// Initializes the "Done" timer when capture completes.
/// This should be called when the capture state changes to Done.
pub fn initialize_done_timer() {
    let timer_guard = CAPTURE_DONE_TIMER.get_or_init(|| Mutex::new(None));
    let mut timer = timer_guard.lock().unwrap();
    *timer = Some(Instant::now());
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
pub fn make_capture_button(
    capture_state: tuner_core::pipeline::CaptureState,
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
            // When idle, display "Ready".
            tuner_core::pipeline::CaptureState::Idle => ("Ready", iced::Color::from_rgb(0.3, 0.4, 0.6), false),
            // When armed, display Armed and use a neutral or slightly distinct color...
            tuner_core::pipeline::CaptureState::Armed => {
                ("Armed", iced::Color::from_rgb(0.8, 0.6, 0.2), false)
            } // Orange-ish
            // When actively capturing (waiting for stability), indicate progress.
            tuner_core::pipeline::CaptureState::Recording => {
                ("Capturing...", iced::Color::from_rgb(0.8, 0.2, 0.2), true)
            } // Red and pulsing
            // When done, show confirmed status. (A timer elsewhere resets this to Armed).
            tuner_core::pipeline::CaptureState::Processing => {
                ("Processing...", iced::Color::from_rgb(0.2, 0.8, 0.2), false)
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

/// Creates a large Undo button that matches the capture button styling.
/// Used to revert the last captured profile entry.
pub fn make_undo_button(note_name: String) -> Element<'static, crate::Message> {
    button(text(format!("Undo Capture ({})", note_name)).size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(0.8, 0.4, 0.2))), // Orange
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(crate::Message::UndoLastCapture)
        .into()
}

/// Configuration for a single button in the settings sidebar
#[derive(Debug, Clone)]
pub struct ButtonConfig {
    pub label: &'static str,
    pub message: Option<crate::Message>,
    pub button_type: ButtonType,
}

/// Different types of buttons with their styling requirements
#[derive(Debug, Clone)]
pub enum ButtonType {
    /// Standard button with no special styling
    Standard,
    /// Measurement mode button that changes color when active
    MeasurementMode,
    /// Disabled button (no interaction)
    Disabled,
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
pub fn make_button(
    config: &ButtonConfig,
    in_measurement_mode: bool,
) -> Element<'static, crate::Message> {
    let mut btn = button(text(config.label).size(14).width(Fill)).padding([6, 10]);

    // Apply styling based on button type and state
    match config.button_type {
        ButtonType::Standard => {
            // No special styling needed
        }
        ButtonType::MeasurementMode => {
            if in_measurement_mode {
                btn = btn.style(|_theme, _status| {
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
            btn = btn.style(|_theme, _status| {
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
        btn.on_press(message.clone()).into()
    } else {
        btn.into()
    }
}

/// Creates a settings/sidebar section with title and buttons.
///
/// Builds a grouped section of the sidebar with a title and
/// a vertical list of buttons. Each section represents a logical grouping
/// of related controls (e.g., "Tools", "Systemic change", "Program").
///
/// # Arguments
/// * `title` - Section title (e.g., "Tools", "Program")
/// * `buttons` - Array of button configurations for this section
/// * `in_measurement_mode` - Whether the application is in measurement mode
///
/// # Returns
/// * `Element` - Complete settings section with title and button list
pub fn make_sidebar_section(
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
