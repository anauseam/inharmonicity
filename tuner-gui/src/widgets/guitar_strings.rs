//! # Guitar strings
//!
//! Six buttons for the open strings of a standard-tuned guitar, a debug
//! alternative to the piano keyboard. Each selects its string's 0–87 key index.

use iced::widget::{button, column, row, text};
use iced::{Alignment, Background, Color, Element, Length};
use tuner_core::models;

use crate::Message;
use crate::widgets::piano_keyboard::{DETECTED, SELECTED};

/// 0–87 key indices of the six open strings in standard tuning, low→high:
/// E2, A2, D3, G3, B3, E4 (key = MIDI − 21; low E2 = MIDI 40 = key 19).
pub const GUITAR_STRING_KEYS: [u8; 6] = [19, 24, 29, 34, 38, 43];

/// String ordinals paired 1:1 with [`GUITAR_STRING_KEYS`] (6th = low E2).
const STRING_ORDINALS: [&str; 6] = ["6th", "5th", "4th", "3rd", "2nd", "1st"];

/// The six string buttons, lit as the piano keyboard lights the `selected` and
/// `detected` keys (0–87); a key that is not an open string lights nothing.
pub fn view(detected: Option<u8>, selected: Option<u8>) -> Element<'static, Message> {
    let mut strings = row![].spacing(10).align_y(Alignment::Center);

    for (&key, ordinal) in GUITAR_STRING_KEYS.iter().zip(STRING_ORDINALS) {
        let note = models::find_nearest_note_by_index(key).0;
        let highlight = match (selected == Some(key), detected == Some(key)) {
            (true, _) => Some(SELECTED),
            (false, true) => Some(DETECTED),
            _ => None,
        };

        let label = column![text(note).size(22), text(ordinal).size(11)]
            .spacing(2)
            .align_x(Alignment::Center);

        let mut btn = button(label)
            .padding([12, 16])
            .width(Length::Fixed(66.0))
            .on_press(Message::KeySelected(key));
        if let Some(color) = highlight {
            btn = btn.style(move |_theme, _status| button::Style {
                background: Some(Background::Color(color)),
                text_color: Color::BLACK,
                ..button::Style::default()
            });
        }

        strings = strings.push(btn);
    }

    strings.into()
}
