//! # Onset-threshold panel
//!
//! The live spectral-flux trace against the threshold a strike must clear,
//! with a slider over it and a freeze for reading one strike.

use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Alignment, Element, Fill, Length};

use crate::Message;
use crate::app::AppDisplayData;
use crate::widgets::seismograph::SeismographViewer;

pub fn panel(data: &AppDisplayData) -> Element<'static, Message> {
    let current_val = data.settings_data.transient.current_threshold;

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

    let mut status_color = iced::Color::from_rgb8(0x2E, 0xCC, 0x71);
    let status_text = if data.settings_data.transient.is_frozen {
        status_color = iced::Color::from_rgb8(0xF3, 0x9C, 0x12);
        "FROZEN: Tuning mode active. Transient captured."
    } else if data.settings_data.transient.freeze_countdown.is_some() {
        status_color = iced::Color::from_rgb8(0x34, 0x98, 0xDB);
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
            slider(0.0..=2.0_f32, current_val, Message::NhwrsfThresholdChanged)
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
                .on_press(Message::ResetTransientScope)
                .padding([8, 16]),
            button(text("Done").size(16))
                .on_press(Message::ToggleTransientCalibration)
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
