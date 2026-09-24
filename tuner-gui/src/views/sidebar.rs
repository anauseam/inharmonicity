//! # Sidebar — the control column a screen builds
//!
//! A titled group of buttons, the large capture button measurement mode shows,
//! and the undo button beneath it.

use iced::widget::{Space, button, column, text};
use iced::{Element, Fill};
use tuner_core::pipeline::CaptureState;

use crate::Message;

/// One button of a sidebar group.
#[derive(Debug, Clone)]
pub(crate) struct ButtonConfig {
    pub label: &'static str,
    pub message: Option<Message>,
    pub button_type: ButtonType,
}

/// How a sidebar button is styled.
#[derive(Debug, Clone)]
pub(crate) enum ButtonType {
    Standard,
    /// Red while measurement mode is on.
    MeasurementMode,
    /// Greyed and inert.
    Disabled,
}

/// One button, styled by its [`ButtonType`].
fn entry(config: &ButtonConfig, in_measurement_mode: bool) -> Element<'static, Message> {
    let mut btn = button(text(config.label).size(14).width(Fill)).padding([6, 10]);

    match config.button_type {
        ButtonType::Standard => {}
        ButtonType::MeasurementMode => {
            if in_measurement_mode {
                btn = btn.style(|_theme, _status| button::Style {
                    background: Some(iced::Background::Color(iced::Color::from_rgb(
                        0.8, 0.2, 0.2,
                    ))),
                    text_color: iced::Color::WHITE,
                    ..button::Style::default()
                });
            }
        }
        ButtonType::Disabled => {
            btn = btn.style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.3, 0.3, 0.3,
                ))),
                text_color: iced::Color::from_rgb(0.6, 0.6, 0.6),
                ..button::Style::default()
            });
        }
    }

    if let Some(message) = &config.message {
        btn.on_press(message.clone()).into()
    } else {
        btn.into()
    }
}

/// A titled group of buttons, which may be assembled per frame.
pub(crate) fn section<'a>(
    title: &'static str,
    buttons: impl IntoIterator<Item = &'a ButtonConfig>,
    in_measurement_mode: bool,
) -> Element<'static, Message> {
    let title_widget = text(title).size(18);

    let items_widget = buttons
        .into_iter()
        .fold(column![].spacing(8), |col, config| {
            col.push(entry(config, in_measurement_mode))
        });

    column![title_widget, Space::new().height(10), items_widget]
        .spacing(5)
        .into()
}

/// The large capture button, labelled and coloured by `capture_state`. While
/// `abortable`, a recording offers Stop instead.
pub(crate) fn capture_button(
    capture_state: CaptureState,
    capture_message: Message,
    abortable: bool,
) -> Element<'static, Message> {
    let (text_label, color) = match capture_state {
        CaptureState::Idle => ("Ready", iced::Color::from_rgb(0.3, 0.4, 0.6)),
        CaptureState::Armed => ("Armed", iced::Color::from_rgb(0.8, 0.6, 0.2)),
        // An extended record runs to its full length whatever the note does.
        CaptureState::Recording if abortable => {
            ("Stop — drop take", iced::Color::from_rgb(0.8, 0.2, 0.2))
        }
        CaptureState::Recording => ("Capturing...", iced::Color::from_rgb(0.8, 0.2, 0.2)),
        CaptureState::Processing => ("Processing...", iced::Color::from_rgb(0.2, 0.8, 0.2)),
    };

    button(text(text_label).size(18).width(Fill))
        .padding([12, 20])
        .style(move |_theme, _status| button::Style {
            background: Some(iced::Background::Color(color)),
            text_color: iced::Color::WHITE,
            ..button::Style::default()
        })
        .on_press(capture_message)
        .into()
}

/// The undo button, which reverts the last capture.
pub(crate) fn undo_button(note_name: String) -> Element<'static, Message> {
    button(
        text(format!("Undo Capture ({})", note_name))
            .size(16)
            .width(Fill),
    )
    .padding([10, 15])
    .style(|_theme, _status| button::Style {
        background: Some(iced::Background::Color(iced::Color::from_rgb(
            0.8, 0.4, 0.2,
        ))),
        text_color: iced::Color::WHITE,
        ..button::Style::default()
    })
    .on_press(Message::UndoLastCapture)
    .into()
}
