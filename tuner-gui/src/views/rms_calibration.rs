use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Alignment, Element, Fill, Length};

use crate::app::AppDisplayData;
use crate::widgets::envelope;

pub fn create_rms_calibration_panel(data: &AppDisplayData) -> Element<'static, crate::Message> {
    let rms_data: Vec<f32> = data.settings_data.rms.history.iter().copied().collect();
    let threshold = data.settings_data.rms.current_threshold;

    let envelope_content: Element<'static, crate::Message> =
        container(envelope::EnvelopeViewer::new(rms_data, threshold).view())
            .width(Fill)
            .height(Fill)
            .into();

    let slider_min = 0.001_f32;
    let slider_max = 0.5_f32;
    let slider_step = 0.0001_f32;
    let calibration_complete = data.settings_data.rms.calibration_complete;

    let controls: Element<'static, crate::Message> = if calibration_complete {
        column![
            row![
                text("Silence Threshold: ").size(14),
                slider(slider_min..=slider_max, threshold, crate::Message::SilenceThresholdChanged)
                    .step(slider_step)
                    .width(Fill),
                text(format!("{:.5}", threshold)).size(14),
            ]
            .spacing(10)
            .align_y(Alignment::Center),
            Space::new().height(5),
            button(text("Recalibrate Noise Floor").size(14))
                .on_press(crate::Message::RecalibrateNoiseFloor)
                .padding([8, 16]),
        ]
        .spacing(5)
        .into()
    } else {
        text("Calibrating…").size(16).into()
    };

    container(
        column![
            text("Noise Floor Adjustment").size(18),
            Space::new().height(10),
            envelope_content,
            Space::new().height(10),
            controls,
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(340.0))
    .into()
}
