use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use iced::widget::canvas::{self, Canvas, Geometry, Text, path};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, alignment, mouse};

pub struct SeismographViewer {
    history: Vec<f32>,
    noise_ceiling: f32, // target threshold N_max
    cache: canvas::Cache,
}

impl SeismographViewer {
    pub fn new(history: Vec<f32>, noise_ceiling: f32) -> Self {
        Self {
            history,
            noise_ceiling,
            cache: canvas::Cache::default(),
        }
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

            let max_val = self
                .history
                .iter()
                .copied()
                .fold(self.noise_ceiling * 1.5, f32::max)
                .max(1.0);
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

            // Draw noise ceiling
            let threshold_n = (self.noise_ceiling / y_scale).clamp(0.0, 1.0);
            let th_y = bounds.height - (threshold_n * bounds.height);
            let t_line = canvas::Path::line(Point::new(0.0, th_y), Point::new(bounds.width, th_y));
            frame.stroke(
                &t_line,
                canvas::Stroke::default()
                    .with_color(Color::from_rgba8(0xF3, 0x9C, 0x12, 0.8))
                    .with_width(1.5),
            );

            let th_text = Text {
                content: format!("N_max: {:.2}", self.noise_ceiling),
                position: Point::new(5.0, th_y - 5.0),
                color: Color::from_rgba8(0xF3, 0x9C, 0x12, 0.9),
                align_x: alignment::Horizontal::Left.into(),
                align_y: alignment::Vertical::Bottom,
                ..Default::default()
            };
            frame.fill_text(th_text);
        });

        vec![geometry]
    }
}
