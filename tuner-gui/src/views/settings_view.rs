use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};

use crate::utils::view_utils::{ButtonConfig, ButtonType, make_sidebar_section};


const TONAL_CONFIG: [ButtonConfig; 3] = [
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
];

const PROGRAM_CONFIG: [ButtonConfig; 3] = [
    ButtonConfig {
        label: "Transient Threshold Calibration",
        message: Some(crate::Message::ToggleTransientCalibration),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Noise Floor Adjustment",
        message: Some(crate::Message::ToggleNoiseFloorAdjustment),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Sample Buffer Adjustment",
        message: None,
        button_type: ButtonType::Disabled,
    },
];

/// Static settings sidebar configuration
const SETTINGS_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 2] = [
    ("Tonal adjustments", TONAL_CONFIG.as_slice()),
    ("Program adjustments", PROGRAM_CONFIG.as_slice()),
];

pub fn create_settings_view(data: &crate::app::AppDisplayData) -> Element<'static, crate::Message> {
    // Show calibrating/shutdown message if audio worker is not active AND not in settings recalibration
    if !data.audio_worker_active && !data.is_calibrating {
        return container(text("Shutting down...").size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let title = text("Settings").size(28);

    // Build main panel content based on which sub-view is active
    let main_panel_content: Element<'static, crate::Message> =
        if data.settings_data.rms.visible {
            crate::views::rms_calibration::create_rms_calibration_panel(data)
        } else if data.settings_data.transient.visible {
            crate::views::transient_calibration::create_transient_calibration_panel(data)
        } else {
            text("Select a setting to adjust.").size(18).into()
        };

    let main_panel = container(
        column![title, Space::new().height(20), main_panel_content]
            .width(Fill)
            .spacing(10),
    )
    .width(Fill)
    .height(Fill);

    let sidebar = create_settings_sidebar(data);

    let main_content = row![sidebar, Space::new().width(10), main_panel]
        .align_y(Alignment::Start)
        .padding(20);

    container(main_content).width(Fill).height(Fill).into()
}

fn create_settings_sidebar(data: &crate::app::AppDisplayData) -> Element<'static, crate::Message> {
    let mut sections = column![].spacing(10);

    // Add Settings button at the top
    let settings_button = button(text("Settings").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.325, 0.278, 0.388,
                ))), // purple
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(crate::Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in SETTINGS_SIDEBAR_CONFIG {
        sections = sections.push(make_sidebar_section(title, buttons, data.measurement_mode_active));
    }

    // Add capture button if in measurement mode
    if data.measurement_mode_active {
        sections = sections.push(
            crate::utils::view_utils::make_capture_button(
                data.capture_state.clone(),
                crate::Message::CaptureButtonClicked,
            ),
        );
    }

    // Show undo button if undo history exists
    if let Some(note_name) = data.undo_target_note.clone() {
        sections = sections.push(iced::widget::Space::new().height(20));
        sections = sections.push(crate::utils::view_utils::make_undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}
