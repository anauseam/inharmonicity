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
use iced::widget::canvas::{self, Canvas, Geometry};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Size, Theme, mouse};

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
    cache: canvas::Cache,
}

impl Spectrogram {
    /// Creates a new spectrogram widget.
    ///
    /// # Arguments
    /// * `data` - Magnitude spectrum data from FFT analysis
    pub fn new(data: Vec<f32>) -> Self {
        Self {
            data,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element for the spectrogram.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
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
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            if !bounds.width.is_finite() || !bounds.height.is_finite() || self.data.is_empty() {
                return;
            }

            let max_magnitude = self.data.iter().fold(EPSILON, |max, &val| val.max(max));

            // Use a 60 dB dynamic range for the visualization
            let min_db = -60.0f32;
            let len_f32 = self.data.len() as f32;
            // Ensure that bars are at least 1 pixel wide to avoid subpixel rendering issues
            let bar_width = (bounds.width / len_f32).max(1.0);

            for (i, &magnitude) in self.data.iter().enumerate() {
                let safe_mag = magnitude.max(EPSILON);
                let db = 20.0 * (safe_mag / max_magnitude).log10();
                let normalized = ((db - min_db) / (-min_db)).clamp(0.0, 1.0);
                let height = normalized * bounds.height;

                if height > 0.0 {
                    let x = (i as f32 / len_f32) * bounds.width;
                    let rect = canvas::Path::rectangle(
                        Point::new(x, bounds.height - height),
                        Size::new(bar_width, height),
                    );
                    frame.fill(&rect, Color::from_rgb8(0x34, 0x98, 0xDB));
                }
            }
        });

        vec![geometry]
    }
}
