use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};

use crate::utils::view_utils::{ButtonConfig, ButtonType, make_sidebar_section};

const SYSTEMIC_CONFIG: [ButtonConfig; 3] = [
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

const PROGRAM_CONFIG: [ButtonConfig; 1] = [
    ButtonConfig {
        label: "Sample Buffer adjustment",
        message: None,
        button_type: ButtonType::Disabled,
    },
];

/// Static settings sidebar configuration
const SETTINGS_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 2] = [
    ("Systemic change", SYSTEMIC_CONFIG.as_slice()),
    ("Program", PROGRAM_CONFIG.as_slice()),
];

pub fn create_settings_view(data: &crate::app::AppDisplayData) -> Element<'static, crate::Message> {
    // Show shutdown message if audio worker is not active
    if !data.audio_worker_active {
        return container(text("Shutting down...").size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let title = text("Settings").size(28);
    let placeholder = text("Settings controls will go here.").size(18);

    let main_panel = container(
        column![title, Space::new().height(20), placeholder]
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
                    0.5, 1.0, 0.83,
                ))), // Aquamarine
                text_color: iced::Color::BLACK,
                ..button::Style::default()
            }
        })
        .on_press(crate::Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in SETTINGS_SIDEBAR_CONFIG {
        let in_measurement_mode = data.capture_state != crate::app::CaptureState::Off;
        sections = sections.push(make_sidebar_section(title, buttons, in_measurement_mode));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}
