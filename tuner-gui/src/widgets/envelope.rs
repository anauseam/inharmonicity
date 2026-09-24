//! # Envelope viewer
//!
//! A scrolling trace of the smoothed RMS against the silence threshold, on a
//! fixed axis matching the threshold slider's range.

use iced::widget::canvas::{self, Canvas, Geometry, Text, path};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, alignment, mouse};

/// Samples of history a scope shows, one per tick: ≈ 2 s.
pub const ENVELOPE_HISTORY_LENGTH: usize = 120;

/// The RMS trace's colour.
const TRACE: Color = Color::from_rgb8(0x2E, 0xCC, 0x71);

/// The threshold line's colour, before its transparency.
const THRESHOLD: Color = Color::from_rgb8(0xE7, 0x4C, 0x3C);

pub struct EnvelopeViewer {
    /// Newest last.
    rms_history: Vec<f32>,
    silence_threshold: f32,
    cache: canvas::Cache,
}

impl EnvelopeViewer {
    /// Creates an Envelope Viewer over `rms_history` (smoothed RMS, newest last), with
    /// the current `silence_threshold` drawn against it.
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

            let bg = canvas::Path::rectangle(Point::ORIGIN, bounds.size());
            frame.fill(&bg, Color::from_rgb8(0x1A, 0x1A, 0x2E));

            if self.rms_history.is_empty() {
                return;
            }

            // A fixed axis, the slider's range, so moving the slider moves only
            // the threshold line.
            let y_max = 0.5_f32;

            let len = self.rms_history.len();
            let x_step = bounds.width / (ENVELOPE_HISTORY_LENGTH as f32 - 1.0).max(1.0);

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

            frame.stroke(
                &envelope_path,
                canvas::Stroke::default().with_color(TRACE).with_width(2.0),
            );

            let threshold_normalized = (self.silence_threshold / y_max).clamp(0.0, 1.0);
            let threshold_y = bounds.height - (threshold_normalized * bounds.height);

            let threshold_line = canvas::Path::line(
                Point::new(0.0, threshold_y),
                Point::new(bounds.width, threshold_y),
            );

            frame.stroke(
                &threshold_line,
                canvas::Stroke::default()
                    .with_color(Color {
                        a: 0.8,
                        ..THRESHOLD
                    })
                    .with_width(1.5),
            );

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

            if let Some(&latest_rms) = self.rms_history.last() {
                let rms_text = Text {
                    content: format!("RMS {:.4}", latest_rms),
                    position: Point::new(bounds.width - 5.0, 5.0),
                    color: TRACE,
                    align_x: alignment::Horizontal::Right.into(),
                    align_y: alignment::Vertical::Top,
                    ..Default::default()
                };
                frame.fill_text(rms_text);
            }

            let threshold_text = Text {
                content: format!("Threshold: {:.4}", self.silence_threshold),
                position: Point::new(5.0, threshold_y - 5.0),
                color: Color {
                    a: 0.9,
                    ..THRESHOLD
                },
                align_x: alignment::Horizontal::Left.into(),
                align_y: alignment::Vertical::Bottom,
                ..Default::default()
            };
            frame.fill_text(threshold_text);
        });

        vec![geometry]
    }
}
