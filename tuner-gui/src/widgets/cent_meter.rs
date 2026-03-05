//! # Cent Meter Widget
//!
//! This module provides a visual cent deviation meter for piano tuning.
//! It displays the tuning accuracy with color-coded feedback and a
//! needle indicator showing how far off the current pitch is from the target.
//!
//! ## Features
//! - Real-time cent deviation display
//! - Color-coded accuracy zones (green/yellow/red)
//! - Smooth needle animation
//! - Professional tuning meter appearance

use iced::widget::canvas::{self, Canvas, Geometry, Path, Stroke};
use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

/// Maximum cent deviation range for the meter display.
/// The meter shows deviations from -50 to +50 cents.
const METER_RANGE: f32 = 50.0;

/// Cent meter widget for displaying tuning accuracy.
///
/// This widget provides a visual representation of how far the current
/// pitch deviates from the target note, with color-coded feedback
/// for different accuracy levels.
pub struct CentMeter {
    /// Current cent deviation (None if no pitch detected)
    cents: Option<f32>,
    cache: canvas::Cache,
}

impl CentMeter {
    /// Creates a new cent meter widget.
    ///
    /// # Arguments
    /// * `cents` - Current cent deviation (None if no pitch detected)
    pub fn new(cents: Option<f32>) -> Self {
        Self {
            cents,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element for the cent meter.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self)
            .width(iced::Length::Fill)
            .height(iced::Length::Fixed(80.0))
            .into()
    }
}

impl<Message> canvas::Program<Message> for CentMeter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            // Draw meter background
            let background = Path::rectangle(Point::ORIGIN, bounds.size());
            frame.fill(&background, Color::from_rgb8(0x40, 0x40, 0x40));

            // Draw center line
            let center_x = bounds.width / 2.0;
            let center_line = Path::line(
                Point::new(center_x, 0.0),
                Point::new(center_x, bounds.height),
            );
            frame.stroke(
                &center_line,
                Stroke::default().with_width(2.0).with_color(Color::WHITE),
            );

            // Draw needle
            if let Some(c) = self.cents {
                let clamped_cents = c.clamp(-METER_RANGE, METER_RANGE);
                let needle_pos = (clamped_cents + METER_RANGE) / (2.0 * METER_RANGE) * bounds.width;

                let color = if c.abs() < 5.0 {
                    Color::from_rgb8(0x34, 0xDB, 0x98) // Green
                } else if c.abs() < 20.0 {
                    Color::from_rgb8(0xFF, 0xC3, 0x00) // Yellow
                } else {
                    Color::from_rgb8(0xFF, 0x33, 0x33) // Red
                };

                let needle = Path::rectangle(
                    Point::new(needle_pos - 2.0, 0.0),
                    Size::new(4.0, bounds.height),
                );
                frame.fill(&needle, color);
            }
        });

        vec![geometry]
    }
}

/// A "rich" cent meter that includes text readouts for the note name,
/// frequency, and confidence above the animated cent meter bar.
pub struct CentMeterDisplay {
    cents: Option<f32>,
    note_name: String,
    freq_text: String,
    confidence_text: String,
}

impl CentMeterDisplay {
    /// Creates a new rich cent meter display.
    pub fn new(
        cents: Option<f32>,
        note_name: String,
        freq_text: String,
        confidence_text: String,
    ) -> Self {
        Self {
            cents,
            note_name,
            freq_text,
            confidence_text,
        }
    }

    /// Creates the view element for the rich cent meter display.
    pub fn view(self) -> Element<'static, crate::Message> {
        let content = column![
            row![
                text("Note").size(14),
                Space::new().width(Length::Fill),
                text("Confidence").size(14),
            ],
            Space::new().height(5),
            row![
                text(self.note_name).size(24),
                Space::new().width(10),
                text(self.freq_text).size(24),
                Space::new().width(Length::Fill),
                container(text(self.confidence_text).size(16)).padding([4, 8]),
            ]
            .align_y(Alignment::Center),
            Space::new().height(10),
            CentMeter::new(self.cents).view(),
        ]
        .spacing(5);

        content.into()
    }
}
