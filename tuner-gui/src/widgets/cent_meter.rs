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

use iced::widget::canvas::{self, Geometry, Path, Stroke};
use iced::widget::container;
use iced::{mouse, Color, Element, Point, Rectangle, Renderer, Size, Theme};

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
}

impl CentMeter {
    /// Creates a new cent meter widget.
    /// 
    /// # Arguments
    /// * `cents` - Current cent deviation (None if no pitch detected)
    pub fn new(cents: Option<f32>) -> Self {
        Self { cents }
    }

    /// Creates the view element for the cent meter.
    /// 
    /// This method consumes the CentMeter instance to create an Iced Element
    /// that can be embedded in the GUI layout.
    pub fn view(self) -> Element<'static, super::super::Message> {
        container(
            canvas::Canvas::new(self)
                .width(iced::Length::Fill)
                .height(iced::Length::Fixed(80.0)),
        )
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
        let mut frame = canvas::Frame::new(renderer, bounds.size());

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
            Stroke::default()
                .with_width(2.0)
                .with_color(Color::WHITE),
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

            let needle =
                Path::rectangle(Point::new(needle_pos - 2.0, 0.0), Size::new(4.0, bounds.height));
            frame.fill(&needle, color);
        }

        vec![frame.into_geometry()]
    }
}
