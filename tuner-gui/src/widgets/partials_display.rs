//! # Partials Display Widget
//!
//! A custom Iced widget to display the measured partials of a musical note.
//! It dynamically lists the frequency of each detected partial using a Canvas,
//! consistent with other widgets in the application.

use iced::widget::canvas::{self, Frame, Geometry, Text};
use iced::widget::container;
use iced::{Element, Point, Rectangle, Renderer, Theme};

/// Represents the state and view logic for the partials display panel.
pub struct PartialsDisplay {
    /// A vector containing the frequencies of the detected partials.
    partials: Vec<f32>,
}

impl PartialsDisplay {
    /// Creates a new `PartialsDisplay` widget.
    ///
    /// # Arguments
    /// * `partials` - A vector of f32 frequencies for each detected partial.
    pub fn new(partials: Vec<f32>) -> Self {
        Self { partials }
    }

    /// Renders the widget's view as a Canvas element.
    // This now correctly refers to the parent's message type, just like your other widgets.
    pub fn view(self) -> Element<'static, super::super::Message> {
        container(
            canvas::Canvas::new(self)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
        )
        .into()
    }
}

// The implementation is now correctly generic over `Message`.
impl<Message> canvas::Program<Message> for PartialsDisplay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let text_color = theme.palette().text;

        // Draw the title "Partials"
        let title_text = Text {
            content: "Partials".to_string(),
            position: Point::new(bounds.width / 2.0, 15.0),
            color: text_color,
            size: 18.0.into(),
            horizontal_alignment: iced::alignment::Horizontal::Center,
            vertical_alignment: iced::alignment::Vertical::Center,
            ..Text::default()
        };
        frame.fill_text(title_text);


        if self.partials.is_empty() {
            // Display a placeholder message if no partials are detected
            let placeholder = Text {
                content: "No partials detected".to_string(),
                position: frame.center(),
                color: text_color,
                size: 14.0.into(),
                horizontal_alignment: iced::alignment::Horizontal::Center,
                vertical_alignment: iced::alignment::Vertical::Center,
                ..Text::default()
            };
            frame.fill_text(placeholder);
        } else {
            // Define layout constants
            let start_y: f32 = 40.0;
            let line_height: f32 = 18.0;
            let padding: f32 = 15.0;

            // Draw each partial's information
            for (i, &freq) in self.partials.iter().enumerate().take(8) { // Limit to 8 to fit
                let y = start_y + (i as f32 * line_height);

                // Draw "Partial X" on the left
                let partial_label = Text {
                    content: format!("Partial {}", i + 1),
                    position: Point::new(padding, y),
                    color: text_color,
                    size: 14.0.into(),
                    horizontal_alignment: iced::alignment::Horizontal::Left,
                    vertical_alignment: iced::alignment::Vertical::Top,
                    ..Text::default()
                };
                frame.fill_text(partial_label);

                // Draw "XXX.XX Hz" on the right
                let freq_label = Text {
                    content: format!("{:.2} Hz", freq),
                    position: Point::new(bounds.width - padding, y),
                    color: text_color,
                    size: 14.0.into(),
                    horizontal_alignment: iced::alignment::Horizontal::Right,
                    vertical_alignment: iced::alignment::Vertical::Top,
                    ..Text::default()
                };
                frame.fill_text(freq_label);
            }
        }

        vec![frame.into_geometry()]
    }
}

