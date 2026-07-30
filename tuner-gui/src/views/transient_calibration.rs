use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Alignment, Element, Fill, Length};

use crate::app::AppDisplayData;
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use crate::widgets::seismograph::SeismographViewer;

pub fn process_telemetry_tick(
    settings: &mut crate::app::TransientSettings,
    flux: f32,
    current_threshold: f32,
) {
    if settings.is_frozen {
        return;
    }

    settings.history.push_back(flux);
    if settings.history.len() > ENVELOPE_HISTORY_LENGTH {
        settings.history.pop_front();
    }

    if let Some(mut countdown) = settings.freeze_countdown {
        if countdown == 0 {
            settings.is_frozen = true;
            settings.freeze_countdown = None;
        } else {
            countdown -= 1;
            settings.freeze_countdown = Some(countdown);
        }
    } else if flux >= current_threshold {
        settings.freeze_countdown = Some(90);
    }
}

pub fn create_transient_calibration_panel(
    data: &AppDisplayData,
) -> Element<'static, crate::Message> {
    let current_val = data.settings_data.transient.current_threshold;

    // Draw the active threshold line on the scope
    let hist: Vec<f32> = data
        .settings_data
        .transient
        .history
        .iter()
        .copied()
        .collect();
    let seismograph = container(SeismographViewer::new(hist, current_val).view())
        .width(Fill)
        .height(Fill);

    let mut status_color = iced::Color::from_rgb8(0x2E, 0xCC, 0x71); // Green (Ready)
    let status_text = if data.settings_data.transient.is_frozen {
        status_color = iced::Color::from_rgb8(0xF3, 0x9C, 0x12); // Orange (Frozen)
        "FROZEN: Tuning mode active. Transient captured."
    } else if data.settings_data.transient.freeze_countdown.is_some() {
        status_color = iced::Color::from_rgb8(0x34, 0x98, 0xDB); // Blue (Capturing)
        "CAPTURING: Please hold..."
    } else {
        "READY: Play your softest note..."
    };

    let controls = column![
        text("Tune Threshold").size(20),
        text("Adjust the cut-off to reject false triggers while retaining valid soft strikes.")
            .size(16),
        Space::new().height(10),
        text(status_text)
            .size(16)
            .style(move |_theme| iced::widget::text::Style {
                color: Some(status_color),
            }),
        Space::new().height(20),
        row![
            text("0.0").size(14),
            slider(
                0.0..=2.0_f32,
                current_val,
                crate::Message::NhwrsfThresholdChanged
            )
            .step(0.001_f32)
            .width(Fill),
            text("2.0").size(14),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        text(format!("Current Threshold: {:.5}", current_val)).size(16),
        Space::new().height(20),
        row![
            button(text("Reset Scope").size(16))
                .on_press(crate::Message::ResetTransientScope)
                .padding([8, 16]),
            button(text("Done").size(16))
                .on_press(crate::Message::ToggleTransientCalibration)
                .padding([8, 16]),
        ]
        .spacing(15)
    ]
    .spacing(10);

    container(
        column![
            text("Live Scope: Transient Calibration").size(24),
            Space::new().height(20),
            seismograph,
            Space::new().height(20),
            controls,
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(500.0))
    .into()
}
