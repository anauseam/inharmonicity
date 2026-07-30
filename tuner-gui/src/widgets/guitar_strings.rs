//! # Guitar String Select (debug)
//!
//! A dead-simple six-button surface for standard-tuning guitar strings
//! (EADGBE), offered as an alternative to the 88-key [`piano_keyboard`] for
//! quick strobe/manual testing on a non-piano source. It owns no DSP state:
//! each button publishes the *same* [`crate::Message::KeySelected`] with the
//! string's 0–87 key index, so the manual-mode contract is byte-identical to a
//! piano-key click. Nothing here measures inharmonicity.
//!
//! [`piano_keyboard`]: super::piano_keyboard

use iced::widget::{button, column, row, text};
use iced::{Alignment, Background, Color, Element, Length};

use crate::Message;

/// 0–87 key indices of the six open strings in standard tuning, low→high:
/// E2, A2, D3, G3, B3, E4 (key = MIDI − 21; low E2 = MIDI 40 = key 19).
pub const GUITAR_STRING_KEYS: [u8; 6] = [19, 24, 29, 34, 38, 43];

/// String ordinals paired 1:1 with [`GUITAR_STRING_KEYS`] (6th = low E2).
const STRING_ORDINALS: [&str; 6] = ["6th", "5th", "4th", "3rd", "2nd", "1st"];

/// Selected/detected highlight colors, mirroring [`super::piano_keyboard`] for
/// cross-surface consistency (red = selected target, green = live-detected).
const SELECTED: Color = Color::from_rgb(1.0, 0.2, 0.2);
const DETECTED: Color = Color::from_rgb(0.204, 0.859, 0.596);

/// Builds the six-string select row. `detected`/`selected` are 0–87 key
/// indices (as passed to [`super::piano_keyboard`]): a string highlights red
/// when it is the selected target, green when live-detected, selection taking
/// precedence. A `selected` key outside the six strings simply highlights
/// nothing — the current target isn't a standard open string.
pub fn view(detected: Option<u8>, selected: Option<u8>) -> Element<'static, Message> {
    let mut strings = row![].spacing(10).align_y(Alignment::Center);

    for (&key, ordinal) in GUITAR_STRING_KEYS.iter().zip(STRING_ORDINALS) {
        let note = tuner_core::models::find_nearest_note_by_index(key).0;
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
