//! # Spectrogram Widget
//! 
//! This module provides a real-time frequency spectrum visualization
//! for piano tuning applications. It displays the magnitude spectrum
//! as a bar chart showing the frequency content of the audio signal.
//! 
//! ## Features
//! - Real-time frequency spectrum display
//! - Logarithmic magnitude scaling
//! - Smooth bar chart visualization
//! - Optimized for piano frequency range

use iced::widget::canvas::{self, Geometry, Path};
use iced::widget::container;
use iced::{mouse, Color, Element, Point, Rectangle, Renderer, Size, Theme};

/// Small epsilon value to prevent log(0) errors in magnitude calculations.
const EPSILON: f32 = 1e-12;

/// Spectrogram widget for displaying frequency spectrum data.
/// 
/// This widget visualizes the frequency content of audio signals
/// as a bar chart, with each bar representing the magnitude
/// of a frequency bin from the FFT analysis.
pub struct Spectrogram {
    /// Magnitude spectrum data from FFT analysis
    data: Vec<f32>,
}

impl Spectrogram {
    pub fn new(data: Vec<f32>) -> Self {
        Self { data }
    }

    pub fn view(self) -> Element<'static, super::super::Message> {
        container(
            canvas::Canvas::new(self)
                .width(iced::Length::Fill)
                .height(iced::Length::Fill),
        )
        .into()
    }
}

impl<Message> canvas::Program<Message> for Spectrogram {
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

        if !bounds.width.is_finite() || !bounds.height.is_finite() || self.data.is_empty() {
            return vec![frame.into_geometry()];
        }

        let max_magnitude = self.data.iter().fold(0.0f32, |max, &val| val.max(max));
        if max_magnitude <= 0.0 {
            return vec![frame.into_geometry()];
        }

        // FIX: Add EPSILON to prevent log(-inf)
        let log_max = (max_magnitude + EPSILON).ln();

        let bar_width = (bounds.width / self.data.len() as f32).max(1.0);

        for (i, &magnitude) in self.data.iter().enumerate() {
            // FIX: Add EPSILON here as well
            let log_magnitude = (magnitude + EPSILON).ln();
            let height = (log_magnitude / log_max * bounds.height).max(0.0);

            // The existing check is good, it will catch any remaining NaN/inf issues.
            if height.is_finite() && height > 0.0 {
                let bar = Path::rectangle(
                    Point::new(i as f32 * bar_width, bounds.height - height),
                    Size::new(bar_width, height),
                );
                frame.fill(&bar, Color::from_rgb8(0x34, 0x98, 0xDB));
            }
        }

        vec![frame.into_geometry()]
    }
}