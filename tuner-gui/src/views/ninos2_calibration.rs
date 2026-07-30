use iced::widget::{Space, button, column, container, row, slider, text};
use iced::{Alignment, Element, Fill, Length};

use crate::app::AppDisplayData;
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use crate::widgets::seismograph::SeismographViewer;

pub fn process_telemetry_tick(settings: &mut crate::app::NinosSettings, ninos2: f32) {
    settings.history.push_back(ninos2);
    if settings.history.len() > ENVELOPE_HISTORY_LENGTH {
        settings.history.pop_front();
    }
}

pub fn create_ninos2_calibration_panel(data: &AppDisplayData) -> Element<'static, crate::Message> {
    let current_val = data.settings_data.ninos.current_threshold;

    let hist: Vec<f32> = data.settings_data.ninos.history.iter().copied().collect();

    let seismograph_viewer = SeismographViewer::new(hist, current_val);

    let seismograph = container(seismograph_viewer.view())
        .width(Fill)
        .height(Fill);

    let controls = column![
        text("NINOS2 Threshold").size(20),
        text("NINOS2 measures tonality, not volume. Broadband noise (fans, hiss) sits around 1–5. Tonal signals (piano, speech, AC hum) sit much higher.")
            .size(16),
        text("Ensure your ambient room noise sits below the red threshold line (Default: 10.0). Do not test this by talking or humming!")
            .size(16),
        text("Warning: Raising this threshold too high may cause complex or rapidly decaying notes to drop out.")
            .size(16)
            .style(move |_theme| iced::widget::text::Style {
                color: Some(iced::Color::from_rgb8(0xE7, 0x4C, 0x3C)),
            }),
        Space::new().height(20),
        row![
            text("5.0").size(14),
            slider(
                5.0..=15.0_f32,
                current_val,
                crate::Message::NinosThresholdChanged
            )
            .step(0.1_f32)
            .width(Fill),
            text("15.0").size(14),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
        text(format!("Current Threshold: {:.1}", current_val)).size(16),
        Space::new().height(20),
        row![
            button(text("Reset Scope").size(16))
                .on_press(crate::Message::ResetNinosScope)
                .padding([8, 16]),
            button(text("Done").size(16))
                .on_press(crate::Message::ToggleNinosCalibration)
                .padding([8, 16]),
        ]
        .spacing(15)
    ]
    .spacing(10);

    container(
        column![
            text("Live Scope: NINOS2 Stability Calibration").size(24),
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
    .height(Length::Fixed(550.0))
    .into()
}
