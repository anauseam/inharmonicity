//! # Unison display
//!
//! A note's individual strings as markers on a cents axis centred on the
//! target: a row per partial, a marker per resolved line with its height the
//! line's relative amplitude, and a shaded band for the gap the record cannot
//! resolve.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::widget::text::Wrapping;
use iced::widget::{Space, column, container, row, text};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};

use tuner_core::algorithms::peaks::MAX_UNISON_LINES;

use crate::widgets::curve_plot::{GRID, INK_SECONDARY, SERIES, SURFACE, ZERO_LINE};

/// Marker colour for the weakest of three lines, which may be the shoulder of a
/// pair the record cannot separate. Drawn, since hiding it could hide a string.
const PROVISIONAL: Color = Color::from_rgb8(0xd9, 0x92, 0x26);

/// One partial's resolved lines, already in cents against that partial's target.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnisonRow {
    pub partial: u8,
    /// Valid entries of [`Self::cents`] / [`Self::amplitude`].
    pub count: u8,
    /// Signed offset from the target, in cents, strongest line first.
    pub cents: [f32; MAX_UNISON_LINES],
    /// Magnitude relative to the strongest line of this partial.
    pub amplitude: [f32; MAX_UNISON_LINES],
    /// `2/T` at this partial, in cents: the smallest gap this record resolves.
    pub resolution_cents: f32,
    /// The same `2/T` in Hz: the slowest beat the record can show, the same at
    /// every key.
    pub resolution_hz: f32,
    /// The partial's target (Hz), which the row's cents are measured against.
    pub ref_hz: f32,
    /// Bank amplitude relative to the strongest reference this hop; dims the row.
    pub level: f32,
    /// The bank's amplitude gate is shut: the lines are held, not fresh.
    pub gated: bool,
}

/// Height of one partial's row, in pixels. Fixed, so a row appearing or
/// vanishing moves nothing else.
pub const ROW_HEIGHT: f32 = 18.0;

/// Fill of the `2/T` band a row's record cannot separate inside, dimmer than
/// [`GRID`] so it reads as absence.
const BLIND_ZONE: Color = Color::from_rgba8(0x38, 0x38, 0x35, 0.55);

/// Height of the axis-label strip and margin, added to `rows × row height` to
/// size the display.
pub const ROW_CHROME: f32 = 22.0;

/// Height of the axis-label strip, inside [`ROW_CHROME`].
const AXIS_STRIP: f32 = 14.0;

/// Label size for the gutter and the axis strip.
const LABEL_SIZE: f32 = 10.0;

/// Right margin of the plot area, so a marker at the axis end is not drawn on
/// the frame edge.
pub const PLOT_RIGHT_MARGIN: f32 = 10.0;

/// Floor under [`UnisonRow::level`]'s dimming: a row that vanished would read as
/// untargeted rather than decayed.
const MIN_LEVEL: f32 = 0.35;

/// Left gutter, sized for the widest row label (`n12 19875`, nine characters at
/// ≈ 0.55 em) plus `LABEL_PAD`. A font-metric estimate: [`Wrapping::None`] on
/// the labels keeps one that outgrows it on its own line.
pub const GUTTER: f32 = 9.0 * 0.55 * LABEL_SIZE + LABEL_PAD;

/// Space between a row label and the plot it labels.
const LABEL_PAD: f32 = 6.0;

/// A row's gutter label: its partial number and target, without the decimal
/// above 1 kHz, where 0.1 Hz is finer than the display resolves.
fn row_label(partial: u8, ref_hz: f32) -> String {
    match ref_hz {
        f if f <= 0.0 => format!("n{partial}"),
        f if f < 1000.0 => format!("n{partial} {f:.1}"),
        f => format!("n{partial} {f:.0}"),
    }
}

/// How strongly a row is drawn: its bank-relative amplitude, floored, and held
/// at the floor while the gate has frozen it — a held row is not a fresh one.
fn dim(level: f32, gated: bool) -> f32 {
    if gated {
        MIN_LEVEL
    } else {
        level.clamp(MIN_LEVEL, 1.0)
    }
}

/// `color` at `level` of its opacity.
fn fade(color: Color, level: f32) -> Color {
    Color {
        a: color.a * level,
        ..color
    }
}

/// Canvas program drawing one unison panel.
pub struct UnisonDisplay {
    rows: Vec<UnisonRow>,
    span_cents: f32,
    row_height: f32,
    cache: canvas::Cache,
}

impl UnisonDisplay {
    /// Builds the display. `span_cents` is the axis half-width, which the caller
    /// holds across hops.
    pub fn new(rows: Vec<UnisonRow>, span_cents: f32) -> Self {
        Self {
            rows,
            span_cents,
            row_height: ROW_HEIGHT,
            cache: canvas::Cache::default(),
        }
    }

    /// Draws the rows at `height` px each instead of [`ROW_HEIGHT`], on the same
    /// cents axis.
    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    /// Creates the view element; the caller sizes it via its container.
    pub fn view(self) -> Element<'static, crate::Message> {
        let half = self.span_cents.max(0.1);
        let row_height = self.row_height;

        // Labels are text widgets, not canvas text: canvas text is reshaped on
        // every frame, which cost over half a debug build's frame rate.
        // One label per row slot, bottom-aligned on the row's own axis line.
        let mut gutter = column![Space::new().height(AXIS_STRIP)];
        for r in &self.rows {
            let label = row_label(r.partial, r.ref_hz);
            gutter = gutter.push(
                container(
                    text(label)
                        .size(LABEL_SIZE)
                        .color(fade(INK_SECONDARY, dim(r.level, r.gated)))
                        .wrapping(Wrapping::None),
                )
                .width(Fill)
                .height(Length::Fixed(row_height))
                .align_x(Horizontal::Right)
                .align_y(Vertical::Bottom)
                .padding(iced::Padding {
                    top: 0.0,
                    right: LABEL_PAD,
                    bottom: 0.0,
                    left: 0.0,
                }),
            );
        }

        // The ends and the target, over the plot area the canvas draws into.
        let tick = |label: String| {
            text(label)
                .size(LABEL_SIZE)
                .color(INK_SECONDARY)
                .wrapping(Wrapping::None)
        };
        let axis = row![
            tick(format!("{:+.1}", -half)),
            Space::new().width(Fill),
            tick("0 ¢".to_string()),
            Space::new().width(Fill),
            tick(format!("{half:+.1}")),
        ]
        .height(Length::Fixed(AXIS_STRIP));

        // Inset by the canvas's right margin, so the end labels sit over the
        // ends of the plot.
        let axis = container(axis).padding(iced::Padding {
            top: 0.0,
            right: PLOT_RIGHT_MARGIN,
            bottom: 0.0,
            left: 0.0,
        });

        row![
            container(gutter).width(Length::Fixed(GUTTER)),
            column![axis, Canvas::new(self).width(Fill).height(Fill)].width(Fill),
        ]
        .into()
    }
}

impl<Message> canvas::Program<Message> for UnisonDisplay {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill(&Path::rectangle(Point::ORIGIN, frame.size()), SURFACE);

            // The gutter and the axis strip are widgets beside and above this
            // canvas, so the plot area starts at its own left edge.
            let (left, right, top) = (0.0f32, PLOT_RIGHT_MARGIN, 0.0f32);
            let plot_w = (bounds.width - left - right).max(1.0);
            let half = self.span_cents.max(0.1);
            let x_of = |cents: f32| left + (cents / half * 0.5 + 0.5).clamp(0.0, 1.0) * plot_w;

            for (index, row) in self.rows.iter().enumerate() {
                let base = top + (index as f32 + 1.0) * self.row_height - 2.0;
                let head = base - (self.row_height - 6.0);
                if base > bounds.height {
                    break;
                }

                // The target line.
                frame.stroke(
                    &Path::line(Point::new(x_of(0.0), head), Point::new(x_of(0.0), base)),
                    Stroke::default().with_width(1.0).with_color(ZERO_LINE),
                );
                // The axis this row's markers stand on.
                frame.stroke(
                    &Path::line(Point::new(left, base), Point::new(left + plot_w, base)),
                    Stroke::default().with_width(1.0).with_color(GRID),
                );

                let level = dim(row.level, row.gated);

                // The blind zone, centred on the target: two lines closer than
                // this merge into one.
                if row.resolution_cents > 0.0 {
                    let (x0, x1) = (
                        x_of(-row.resolution_cents / 2.0),
                        x_of(row.resolution_cents / 2.0),
                    );
                    frame.fill(
                        &Path::rectangle(
                            Point::new(x0, base - self.row_height / 2.0 + 2.0),
                            iced::Size::new((x1 - x0).max(1.0), self.row_height - 4.0),
                        ),
                        BLIND_ZONE,
                    );
                }

                // One marker per line, height by relative amplitude. The weakest
                // of three is drawn provisional.
                for line in 0..row.count as usize {
                    let x = x_of(row.cents[line]);
                    let strength = row.amplitude[line].clamp(0.15, 1.0);
                    let color =
                        if row.count == MAX_UNISON_LINES as u8 && line == MAX_UNISON_LINES - 1 {
                            PROVISIONAL
                        } else {
                            SERIES
                        };
                    let color = fade(color, level);
                    frame.stroke(
                        &Path::line(
                            Point::new(x, base),
                            Point::new(x, base - (base - head) * strength),
                        ),
                        Stroke::default().with_width(3.0).with_color(color),
                    );
                }
            }
        });

        vec![geometry]
    }
}
