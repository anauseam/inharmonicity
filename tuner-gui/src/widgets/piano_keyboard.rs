//! # Piano keyboard
//!
//! An 88-key keyboard: the detected key lit green, the selected key red, and a
//! ✗ on each key whose measurement the curve doubts. A click publishes
//! `Message::KeySelected`.

use iced::widget::canvas::{self, Canvas, Event, Fill, Geometry, Path, Stroke};
use iced::{Color, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::widgets::curve_plot::SUSPECT;

/// Number of white keys on an 88-key piano.
const WHITE_KEY_COUNT: usize = 52;
/// Total number of keys on an 88-key piano.
const TOTAL_KEY_COUNT: usize = 88;

/// Which keys of an octave are black, from A, since key 0 is A0.
const IS_BLACK: [bool; 12] = [
    false, true, false, false, true, false, true, false, false, true, false, true,
];

/// Fill of the selected key.
pub const SELECTED: Color = Color::from_rgb8(0xFF, 0x33, 0x33);

/// Fill of the detected key.
pub const DETECTED: Color = Color::from_rgb8(0x34, 0xDB, 0x98);

#[derive(Debug, Clone)]
pub struct PianoKeyboard {
    detected_key_index: Option<u8>,
    selected_key_index: Option<u8>,
    /// Keys whose measurement the curve doubts, marked ✗.
    suspect: [bool; 88],
}

impl PianoKeyboard {
    /// A keyboard lighting the detected and selected keys (0–87), with `suspect`
    /// keys marked.
    pub fn new(
        detected_key_index: Option<u8>,
        selected_key_index: Option<u8>,
        suspect: [bool; 88],
    ) -> Self {
        Self {
            detected_key_index,
            selected_key_index,
            suspect,
        }
    }

    /// Determines which piano key (0–87) a click at `pos` lands on, testing black keys
    /// first since they are drawn on top; `None` when it misses every key.
    fn key_index_from_pos(&self, bounds: Size, pos: Point) -> Option<u8> {
        let white_key_width = bounds.width / WHITE_KEY_COUNT as f32;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = bounds.height * 0.6;

        let mut white_key_idx: f32 = 0.0;
        for i in 0..TOTAL_KEY_COUNT {
            let note_in_octave = i % 12;
            if IS_BLACK[note_in_octave] {
                let key_x = (white_key_idx - 0.5) * white_key_width; // Center on the line
                let black_key_rect = Rectangle {
                    x: key_x,
                    y: 0.0,
                    width: black_key_width,
                    height: black_key_height,
                };
                if black_key_rect.contains(pos) {
                    return Some(i as u8);
                }
            } else {
                white_key_idx += 1.0;
            }
        }

        let clicked_white_key = (pos.x / white_key_width).floor() as usize;
        let mut current_white_key_idx = 0;
        for i in 0..TOTAL_KEY_COUNT {
            let note_in_octave = i % 12;
            if !IS_BLACK[note_in_octave] {
                if current_white_key_idx == clicked_white_key {
                    return Some(i as u8);
                }
                current_white_key_idx += 1;
            }
        }
        None
    }

    /// Creates the view element.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self)
            .width(iced::Length::Fill)
            .height(iced::Length::Fixed(120.0))
            .into()
    }
}

impl<Message> canvas::Program<Message> for PianoKeyboard
where
    Message: From<crate::Message>,
{
    type State = ();

    fn update(
        &self,
        _state: &mut Self::State,
        event: &Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<canvas::Action<Message>> {
        if let Some(position) = cursor.position_in(bounds)
            && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && let Some(key_index) = self.key_index_from_pos(bounds.size(), position)
        {
            return Some(canvas::Action::publish(
                crate::Message::KeySelected(key_index).into(),
            ));
        }
        None
    }

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());

        /// A ✗ centred on `(cx, cy)`, white on a lit key, where red would vanish
        /// against the red selection.
        fn suspect_mark(frame: &mut canvas::Frame, cx: f32, cy: f32, arm: f32, highlighted: bool) {
            let cross = Path::new(|b| {
                b.move_to(Point::new(cx - arm, cy - arm));
                b.line_to(Point::new(cx + arm, cy + arm));
                b.move_to(Point::new(cx + arm, cy - arm));
                b.line_to(Point::new(cx - arm, cy + arm));
            });
            let color = if highlighted { Color::WHITE } else { SUSPECT };
            frame.stroke(&cross, Stroke::default().with_width(1.6).with_color(color));
        }

        let white_key_width = bounds.width / WHITE_KEY_COUNT as f32;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = bounds.height * 0.6;

        // Draw white keys
        let mut white_key_x = 0.0;
        for i in 0..TOTAL_KEY_COUNT {
            let note_in_octave = i % 12;
            if !IS_BLACK[note_in_octave] {
                let is_detected = self.detected_key_index == Some(i as u8);
                let is_selected = self.selected_key_index == Some(i as u8);

                let color = match (is_selected, is_detected) {
                    (true, _) => SELECTED,
                    (false, true) => DETECTED,
                    _ => Color::WHITE,
                };

                frame.fill_rectangle(
                    Point::new(white_key_x, 0.0),
                    Size::new(white_key_width, bounds.height),
                    Fill::from(color),
                );
                frame.stroke(
                    &Path::rectangle(
                        Point::new(white_key_x, 0.0),
                        Size::new(white_key_width, bounds.height),
                    ),
                    Stroke::default().with_color(Color::BLACK),
                );
                // Below the black keys, where a white key is unobstructed.
                if self.suspect[i] {
                    suspect_mark(
                        &mut frame,
                        white_key_x + white_key_width * 0.5,
                        bounds.height * 0.82,
                        white_key_width * 0.25,
                        is_selected || is_detected,
                    );
                }
                white_key_x += white_key_width;
            }
        }

        // Draw black keys
        let mut white_key_idx: f32 = 0.0;
        for i in 0..TOTAL_KEY_COUNT {
            let note_in_octave = i % 12;
            if IS_BLACK[note_in_octave] {
                let key_x = (white_key_idx - 0.5) * white_key_width;
                let is_detected = self.detected_key_index == Some(i as u8);
                let is_selected = self.selected_key_index == Some(i as u8);

                let color = match (is_selected, is_detected) {
                    (true, _) => SELECTED,
                    (false, true) => DETECTED,
                    _ => Color::BLACK,
                };

                frame.fill_rectangle(
                    Point::new(key_x, 0.0),
                    Size::new(black_key_width, black_key_height),
                    Fill::from(color),
                );
                // Inside the black key: below it is the white key's territory.
                if self.suspect[i] {
                    suspect_mark(
                        &mut frame,
                        key_x + black_key_width * 0.5,
                        black_key_height * 0.8,
                        black_key_width * 0.3,
                        is_selected || is_detected,
                    );
                }
            } else {
                white_key_idx += 1.0;
            }
        }

        vec![frame.into_geometry()]
    }
}
