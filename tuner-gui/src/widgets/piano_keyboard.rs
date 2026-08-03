//! # Piano Keyboard Widget
//!
//! This module provides an interactive 88-key piano keyboard widget
//! for piano tuning applications. It displays a visual representation
//! of the piano keyboard with clickable keys and visual feedback
//! for detected and selected notes.
//!
//! ## Features
//! - 88-key piano keyboard visualization
//! - Interactive key selection
//! - Visual feedback for detected notes
//! - Professional piano appearance
//! - Click-to-select functionality

use iced::widget::canvas::{self, Canvas, Event, Fill, Geometry, Path, Stroke};
use iced::{Color, Element, Point, Rectangle, Renderer, Size, Theme, mouse};

use crate::widgets::curve_plot::SUSPECT;

/// Number of white keys on an 88-key piano.
const WHITE_KEY_COUNT: usize = 52;
/// Total number of keys on an 88-key piano.
const TOTAL_KEY_COUNT: usize = 88;

/// Pattern indicating which keys in an octave are black keys.
/// This array represents the pattern: C, C#, D, D#, E, F, F#, G, G#, A, A#, B
const IS_BLACK: [bool; 12] = [
    false, true, false, false, true, false, true, false, false, true, false, true,
];

/// Interactive piano keyboard widget for note selection and visualization.
///
/// This widget displays a full 88-key piano keyboard with visual feedback
/// for detected notes and user-selected keys. It supports click-to-select
/// functionality for manual tuning mode.
#[derive(Debug, Clone)]
pub struct PianoKeyboard {
    /// Currently detected key index (from audio analysis)
    detected_key_index: Option<u8>,
    /// User-selected key index (from mouse clicks)
    selected_key_index: Option<u8>,
    /// Keys whose measurement the curve doubts, from
    /// [`crate::advisory::suspect_keys`] — marked with a red ✗.
    suspect: [bool; 88],
}

impl PianoKeyboard {
    /// Creates a new piano keyboard widget.
    ///
    /// # Arguments
    /// * `detected_key_index` - Currently detected key from audio analysis (0-87)
    /// * `selected_key_index` - User-selected key from mouse clicks (0-87)
    /// * `suspect` - Per-key suspect-measurement marks
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

    /// Determines which piano key was clicked based on mouse position.
    ///
    /// Calculates the key index (0-87) from a mouse click position on the keyboard.
    /// Handles both white and black keys, with black keys taking priority since
    /// they are drawn on top. Returns the key index if a valid key was clicked.
    ///
    /// # Arguments
    /// * `bounds` - Size of the keyboard widget
    /// * `pos` - Mouse click position within the keyboard bounds
    ///
    /// # Returns
    /// * `Some(u8)` - Key index (0-87) if a key was clicked
    /// * `None` - If click was outside any key area
    fn key_index_from_pos(&self, bounds: Size, pos: Point) -> Option<u8> {
        let white_key_width = bounds.width / WHITE_KEY_COUNT as f32;
        let black_key_width = white_key_width * 0.6;
        let black_key_height = 120.0 * 0.6;

        // Check black keys first (they are on top)
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

        // Check white keys
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

    /// Creates the view element for the piano keyboard.
    ///
    /// This method consumes the PianoKeyboard instance to create an Iced Element
    /// that can be embedded in the GUI layout.
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
    ) -> Option<iced::widget::canvas::Action<Message>> {
        if let Some(position) = cursor.position_in(bounds)
            && let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event
            && let Some(key_index) = self.key_index_from_pos(bounds.size(), position)
        {
            return Some(iced::widget::canvas::Action::publish(
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

        /// A ✗ centred on `(cx, cy)`, on a key whose measurement the curve
        /// doubts. Drawn white over the saturated selected/detected fills —
        /// the red mark is invisible against the red selection, which is
        /// exactly the key the user just clicked.
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
                    (true, _) => Color::from_rgb8(0xFF, 0x33, 0x33), // Red (Selected)
                    (false, true) => Color::from_rgb8(0x34, 0xDB, 0x98), // Green (Detected)
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
                    (true, _) => Color::from_rgb8(0xFF, 0x33, 0x33), // Red
                    (false, true) => Color::from_rgb8(0x34, 0xDB, 0x98), // Green
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
