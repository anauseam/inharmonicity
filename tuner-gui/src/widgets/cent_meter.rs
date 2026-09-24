//! # Cent meter
//!
//! A needle on a ±50 ¢ bar, coloured by its distance from the target — green
//! inside 5 ¢, yellow inside 20 ¢, red beyond, grey while the reading is stale —
//! under the note name, frequency and tracking status.

use iced::widget::canvas::{self, Canvas, Geometry, Path, Stroke};
use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Point, Rectangle, Renderer, Size, Theme, mouse};

/// Half-width of the meter, in cents.
const METER_RANGE: f32 = 50.0;

/// The needle bar.
pub struct CentMeter {
    cents: Option<f32>,
    /// Audio without a locked note.
    is_stale: bool,
    cache: canvas::Cache,
}

impl CentMeter {
    /// A meter showing `cents`, grey while `is_stale`.
    pub fn new(cents: Option<f32>, is_stale: bool) -> Self {
        Self {
            cents,
            is_stale,
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
            let background = Path::rectangle(Point::ORIGIN, bounds.size());
            frame.fill(&background, Color::from_rgb8(0x40, 0x40, 0x40));

            let center_x = bounds.width / 2.0;
            let center_line = Path::line(
                Point::new(center_x, 0.0),
                Point::new(center_x, bounds.height),
            );
            frame.stroke(
                &center_line,
                Stroke::default().with_width(2.0).with_color(Color::WHITE),
            );

            // Finite only: `clamp` passes NaN through, and lyon asserts on
            // non-finite path coordinates.
            if let Some(c) = self.cents.filter(|c| c.is_finite()) {
                let clamped_cents = c.clamp(-METER_RANGE, METER_RANGE);
                let needle_pos = (clamped_cents + METER_RANGE) / (2.0 * METER_RANGE) * bounds.width;

                let color = if self.is_stale {
                    Color::from_rgb8(0x80, 0x80, 0x80)
                } else if c.abs() < 5.0 {
                    Color::from_rgb8(0x34, 0xDB, 0x98)
                } else if c.abs() < 20.0 {
                    Color::from_rgb8(0xFF, 0xC3, 0x00)
                } else {
                    Color::from_rgb8(0xFF, 0x33, 0x33)
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

/// The meter under the note name, frequency and tracking status.
pub struct CentMeterDisplay {
    cents: Option<f32>,
    note_name: String,
    freq_text: String,
    status_text: String,
    is_stale: bool,
}

impl CentMeterDisplay {
    pub fn new(
        cents: Option<f32>,
        note_name: String,
        freq_text: String,
        status_text: String,
        is_stale: bool,
    ) -> Self {
        Self {
            cents,
            note_name,
            freq_text,
            status_text,
            is_stale,
        }
    }

    pub fn view(self) -> Element<'static, crate::Message> {
        let text_color = if self.is_stale {
            Color::from_rgb8(0xAA, 0xAA, 0xAA)
        } else {
            Color::WHITE
        };

        let status_color = if self.status_text == "Dropped" {
            Color::from_rgb8(0xFF, 0x88, 0x00)
        } else if self.status_text == "Tracking" {
            Color::from_rgb8(0x34, 0xDB, 0x98)
        } else {
            text_color
        };

        let content = column![
            row![
                text("Note").size(14).color(text_color),
                Space::new().width(Length::Fill),
                text("Partial 1").size(14).color(text_color),
            ],
            Space::new().height(5),
            row![
                text(self.note_name).size(24).color(text_color),
                Space::new().width(10),
                text(self.freq_text).size(24).color(text_color),
                Space::new().width(Length::Fill),
                container(text(self.status_text).size(16).color(status_color)).padding([4, 8]),
            ]
            .align_y(Alignment::Center),
            Space::new().height(10),
            CentMeter::new(self.cents, self.is_stale).view(),
        ]
        .spacing(5);

        content.into()
    }
}
