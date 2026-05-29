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

use iced::widget::canvas::{self, Canvas, Geometry, Text, path};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, alignment, mouse};

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

            // Fixed Y-axis: anchored at 0.5 to match the slider's absolute range.
            // Both RMS and threshold lines represent true values — moving the slider
            // visibly moves the red line independently of the green RMS line.
            let y_max = 0.5_f32;

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

            // Draw grid lines (horizontal)
            let grid_color = Color::from_rgba8(0x44, 0x44, 0x66, 0.3);
            for i in 1..4 {
                let y = bounds.height * (i as f32 / 4.0);
                let grid_line = canvas::Path::line(Point::new(0.0, y), Point::new(bounds.width, y));
                frame.stroke(
                    &grid_line,
                    canvas::Stroke::default()
                        .with_color(grid_color)
                        .with_width(1.0),
                );
            }

            // Draw grid lines (vertical)
            for i in 1..4 {
                let x = bounds.width * (i as f32 / 4.0);
                let grid_line =
                    canvas::Path::line(Point::new(x, 0.0), Point::new(x, bounds.height));
                frame.stroke(
                    &grid_line,
                    canvas::Stroke::default()
                        .with_color(grid_color)
                        .with_width(1.0),
                );
            }

            // Draw Text Labels
            let label_color = Color::from_rgba8(0xBD, 0xC3, 0xC7, 0.6);
            for i in 1..4 {
                let value = y_max * (1.0 - i as f32 / 4.0);
                let y = bounds.height * (i as f32 / 4.0);
                let label = Text {
                    content: format!("{:.3}", value),
                    position: Point::new(4.0, y + 2.0),
                    color: label_color,
                    align_x: alignment::Horizontal::Left.into(),
                    align_y: alignment::Vertical::Top,
                    size: iced::Pixels(10.0),
                    ..Default::default()
                };
                frame.fill_text(label);
            }

            // Current RMS value (latest sample)
            if let Some(&latest_rms) = self.rms_history.last() {
                let rms_text = Text {
                    content: format!("RMS {:.4}", latest_rms),
                    position: Point::new(bounds.width - 5.0, 5.0),
                    color: Color::from_rgb8(0x2E, 0xCC, 0x71), // Match envelope color
                    align_x: alignment::Horizontal::Right.into(),
                    align_y: alignment::Vertical::Top,
                    ..Default::default()
                };
                frame.fill_text(rms_text);
            }

            // Silence threshold value
            let threshold_text = Text {
                content: format!("Threshold: {:.4}", self.silence_threshold),
                position: Point::new(5.0, threshold_y - 5.0),
                color: Color::from_rgba8(0xE7, 0x4C, 0x3C, 0.9), // Match threshold color
                align_x: alignment::Horizontal::Left.into(),
                align_y: alignment::Vertical::Bottom,
                ..Default::default()
            };
            frame.fill_text(threshold_text);
        });

        vec![geometry]
    }
}
