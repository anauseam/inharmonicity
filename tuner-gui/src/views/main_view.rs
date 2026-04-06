//! # Main Display Module
//!
//! This module contains the main display components and layout logic
//! for the Inharmonicity piano tuning application.

use crate::utils::view_utils::{
    ButtonConfig, ButtonType, make_capture_button, make_undo_button, make_sidebar_section,
};
use crate::widgets::{cent_meter, partials_display, piano_keyboard, spectrogram};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};

const TOOLS_CONFIG: [ButtonConfig; 5] = [
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
];

const PROGRAM_CONFIG: [ButtonConfig; 2] = [
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
];

/// Static main sidebar configuration
const MAIN_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 2] = [
    ("Tools", TOOLS_CONFIG.as_slice()),
    ("Program", PROGRAM_CONFIG.as_slice()),
];

/// Creates the complete main application view
pub fn create_main_view(
    data: &crate::app::AppDisplayData,
    _profile: &tuner_core::models::InharmonicityProfile,
    capture_message: crate::Message,
) -> Element<'static, crate::Message> {
    // Show calibrating/shutdown message if audio worker is not active or calibrating
    if !data.audio_worker_active || data.is_calibrating {
        let message = if data.is_calibrating {
            format!(
                "Calibrating… {}/{}",
                data.calibration_progress, crate::calibration::CALIBRATION_FRAMES
            )
        } else {
            "Shutting down...".to_string()
        };
        return container(text(message).size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let widget_area = create_widget_area(data);

    // Create sidebar
    let sidebar = create_sidebar(
        data.measurement_mode_active,
        data.capture_state.clone(),
        data.undo_target_note.clone(),
        capture_message,
    );

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

    let spectrogram_data: Vec<f32> = data
        .last_frame
        .as_ref()
        .map(|f| f.magnitudes[..f.magnitude_len].to_vec())
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
        data.last_cents
    } else {
        let sum: f32 = data.smoothing_buffer.iter().sum();
        let count = data.smoothing_buffer.len() as f32;
        if count > 0.0 { Some(sum / count) } else { None }
    };

    let (note_name, freq_text, confidence, _target_freq_text) = {
        let current_freq = data.last_frequency.unwrap_or(0.0);
        let note_text = match &data.tuning_mode {
            crate::app::TuningMode::Auto => data
                .last_note_index
                .map(|idx| {
                    let (name, _) = tuner_core::models::find_nearest_note_by_index(idx);
                    name
                })
                .unwrap_or_else(|| "--".to_string()),
            crate::app::TuningMode::Manual { note_name, .. } => note_name.clone(),
        };
        let target_freq_text = match &data.tuning_mode {
            crate::app::TuningMode::Auto => String::from("Auto"),
            crate::app::TuningMode::Manual { target_freq, .. } => format!("{:.1} Hz", target_freq),
        };
        let confidence_text = data
            .last_confidence
            .map(|c| format!("{:.0}%", c * 100.0))
            .unwrap_or_else(|| "0%".to_string());

        (
            note_text,
            format!("{:.2} Hz", current_freq),
            confidence_text,
            target_freq_text,
        )
    };

    let cent_meter_content: Element<'static, crate::Message> = container(
        cent_meter::CentMeterDisplay::new(smoothed_cents, note_name, freq_text, confidence, data.is_stale).view(),
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

    // Detected key index — directly from NoteEvent (no String→index lookup)
    let detected_key_index = data.last_note_index;

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

    // Partials have been removed from the pipeline output.
    // This panel is kept for future use but shows no data currently.
    let partials_data: Vec<f32> = Vec::new();

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
    measurement_mode_active: bool,
    capture_state: tuner_core::pipeline::CaptureState,
    undo_target_note: Option<String>,
    capture_message: crate::Message,
) -> Element<'static, crate::Message> {
    let mut sections = column![].spacing(10);

    // Add Settings button at the top
    let settings_button = button(text("Settings").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    //0.38, 0.294, 0.502,
                    0.427, 0.298, 0.612,
                ))), // purple
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(crate::Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in MAIN_SIDEBAR_CONFIG {
        sections = sections.push(make_sidebar_section(title, buttons, measurement_mode_active));
    }

    // Add capture button if in measurement mode
    if measurement_mode_active {
        sections = sections.push(make_capture_button(capture_state, capture_message));
    }

    if let Some(note_name) = undo_target_note {
        sections = sections.push(Space::new().height(20));
        sections = sections.push(make_undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}

// End of file
