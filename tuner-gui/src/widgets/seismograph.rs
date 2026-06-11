use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use iced::widget::canvas::{self, Canvas, Geometry, Text, path};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, alignment, mouse};

struct Annotation {
    value: f32,
    color: Color,
    label: String,
}

pub struct SeismographViewer {
    history: Vec<f32>,
    current_threshold: f32,
    annotations: Vec<Annotation>,
    cache: canvas::Cache,
}

impl SeismographViewer {
    pub fn new(history: Vec<f32>, current_threshold: f32) -> Self {
        Self {
            history,
            current_threshold,
            annotations: Vec::new(),
            cache: canvas::Cache::default(),
        }
    }

    pub fn with_annotation(mut self, value: f32, color: Color, label: String) -> Self {
        self.annotations.push(Annotation {
            value,
            color,
            label,
        });
        self
    }

    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
    }
}

impl<Message> canvas::Program<Message> for SeismographViewer {
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
            frame.fill(&bg, Color::from_rgb8(0x0A, 0x1A, 0x2A));

            if self.history.is_empty() {
                return;
            }

            let mut max_val = self
                .history
                .iter()
                .copied()
                .fold(self.current_threshold * 1.5, f32::max)
                .max(1.0);

            for ann in &self.annotations {
                if ann.value * 1.5 > max_val {
                    max_val = ann.value * 1.5;
                }
            }

            let y_scale = max_val * 1.1;

            let len = self.history.len();
            let x_step = bounds.width / (ENVELOPE_HISTORY_LENGTH as f32 - 1.0).max(1.0);
            let mut builder = path::Builder::new();
            let x_offset = (ENVELOPE_HISTORY_LENGTH - len) as f32 * x_step;

            for (i, &v) in self.history.iter().enumerate() {
                let x = x_offset + i as f32 * x_step;
                let normalized = (v / y_scale).clamp(0.0, 1.0);
                let y = bounds.height - (normalized * bounds.height);
                if i == 0 {
                    builder.move_to(Point::new(x, y));
                } else {
                    builder.line_to(Point::new(x, y));
                }
            }

            let p = builder.build();
            frame.stroke(
                &p,
                canvas::Stroke::default()
                    .with_color(Color::from_rgb8(0x34, 0x98, 0xDB))
                    .with_width(2.0),
            );

            // Draw current threshold
            let threshold_n = (self.current_threshold / y_scale).clamp(0.0, 1.0);
            let th_y = bounds.height - (threshold_n * bounds.height);
            let t_line = canvas::Path::line(Point::new(0.0, th_y), Point::new(bounds.width, th_y));
            frame.stroke(
                &t_line,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba8(0xF3, 0x9C, 0x12, 0.8))
                    .with_width(1.5),
            );

            let th_text = Text {
                content: format!("Threshold: {:.2}", self.current_threshold),
                position: Point::new(5.0, th_y - 5.0),
                color: Color::from_rgba8(0xF3, 0x9C, 0x12, 0.9),
                align_x: alignment::Horizontal::Left.into(),
                align_y: alignment::Vertical::Bottom,
                ..Default::default()
            };
            frame.fill_text(th_text);

            // Draw annotations
            for ann in &self.annotations {
                let ann_n = (ann.value / y_scale).clamp(0.0, 1.0);
                let ann_y = bounds.height - (ann_n * bounds.height);
                let a_line =
                    canvas::Path::line(Point::new(0.0, ann_y), Point::new(bounds.width, ann_y));

                frame.stroke(
                    &a_line,
                    canvas::Stroke::default()
                        .with_color(Color {
                            a: 0.8,
                            ..ann.color
                        })
                        .with_width(1.5),
                );

                let a_text = Text {
                    content: format!("{}: {:.2}", ann.label, ann.value),
                    position: Point::new(bounds.width - 5.0, ann_y - 5.0),
                    color: ann.color,
                    align_x: alignment::Horizontal::Right.into(),
                    align_y: alignment::Vertical::Bottom,
                    ..Default::default()
                };
                frame.fill_text(a_text);
            }
        });

        vec![geometry]
    }
}
