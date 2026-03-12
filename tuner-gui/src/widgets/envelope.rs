//! # Envelope Viewer Widget
//!
//! A real-time time-domain visualization of the audio stream's RMS envelope,
//! primarily used for noise floor adjustment. It renders a scrolling line graph
//! of the smoothed RMS amplitude with an optional horizontal silence threshold line.
//!
//! ## Features
//! - Scrolling RMS envelope (connected line graph)
//! - Horizontal silence threshold marker
//! - Dynamic Y-axis scaling

use iced::widget::canvas::{self, Canvas, Geometry, path};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, mouse};

/// Maximum number of RMS history samples to display.
/// At 60 FPS this represents approximately 2 seconds of history.
/// Adjust this constant to change the visible time window.
pub const ENVELOPE_HISTORY_LENGTH: usize = 120;

/// Envelope Viewer widget for displaying the RMS amplitude envelope
/// and the silence threshold over time.
///
/// This widget visualizes the smoothed RMS output from the Gatekeeper's
/// EMA filter, allowing the user to see the noise floor of their audio
/// environment and adjust the silence threshold accordingly.
pub struct EnvelopeViewer {
    /// RMS history data (newest sample at the end)
    rms_history: Vec<f32>,
    /// Current silence threshold value
    silence_threshold: f32,
    cache: canvas::Cache,
}

impl EnvelopeViewer {
    /// Creates a new Envelope Viewer widget.
    ///
    /// # Arguments
    /// * `rms_history` - Slice of smoothed RMS values (newest at the end)
    /// * `silence_threshold` - The current silence threshold to draw
    pub fn new(rms_history: Vec<f32>, silence_threshold: f32) -> Self {
        Self {
            rms_history,
            silence_threshold,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element for the envelope viewer.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
    }
}

impl<Message> canvas::Program<Message> for EnvelopeViewer {
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
            if !bounds.width.is_finite() || !bounds.height.is_finite() {
                return;
            }

            // Draw background
            let bg = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
            frame.fill(&bg, Color::from_rgb8(0x1A, 0x1A, 0x2E));

            if self.rms_history.is_empty() {
                return;
            }

            // Dynamic Y-axis scaling: use the max of either the history peak
            // or the silence threshold (whichever is larger) to ensure the
            // threshold line is always visible. Add 20% headroom.
            let history_max = self
                .rms_history
                .iter()
                .fold(0.0f32, |max, &val| val.max(max));
            let y_max = history_max.max(self.silence_threshold) * 1.2;

            // Avoid division by zero for completely silent signals
            if y_max <= 0.0 {
                return;
            }

            let len = self.rms_history.len();
            let x_step = bounds.width / (ENVELOPE_HISTORY_LENGTH as f32 - 1.0).max(1.0);

            // Build the RMS envelope path as a connected line graph
            let mut builder = path::Builder::new();
            let x_offset = (ENVELOPE_HISTORY_LENGTH - len) as f32 * x_step;

            for (i, &rms) in self.rms_history.iter().enumerate() {
                let x = x_offset + i as f32 * x_step;
                let normalized = (rms / y_max).clamp(0.0, 1.0);
                let y = bounds.height - (normalized * bounds.height);

                if i == 0 {
                    builder.move_to(Point::new(x, y));
                } else {
                    builder.line_to(Point::new(x, y));
                }
            }

            let envelope_path = builder.build();

            // Draw the RMS envelope line
            frame.stroke(
                &envelope_path,
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(0x2E, 0xCC, 0x71)) // green
                    .with_width(2.0),
            );

            // Draw the silence threshold as a horizontal dashed line
            let threshold_normalized = (self.silence_threshold / y_max).clamp(0.0, 1.0);
            let threshold_y = bounds.height - (threshold_normalized * bounds.height);

            let threshold_line = canvas::Path::line(
                Point::new(0.0, threshold_y),
                Point::new(bounds.width, threshold_y),
            );

            frame.stroke(
                &threshold_line,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba8(0xE7, 0x4C, 0x3C, 0.8)) // red, slightly transparent
                    .with_width(1.5),
            );
        });

        vec![geometry]
    }
}
