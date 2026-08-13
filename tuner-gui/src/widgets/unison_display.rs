//! # Unison Display Widget
//!
//! The note's individual strings as markers on a **cents** axis centred on the
//! curve target — one marker per spectral line resolved by
//! `tuner_core::strobe::unison`, its height the line's relative amplitude. A
//! tuner watches the markers converge; the beat rate they imply is the numeric
//! readout the panel prints beside this.
//!
//! Cents for positions, Hz for rates, is the existing convention: the core ships
//! signed Hz offsets and the frontend owns the reference they are shown against.
//!
//! **The resolution bar is not decoration.** Until the DSP-side record is long
//! enough, two separated strings resolve as *one* line, which reads as "clean" at
//! exactly the moment a tuner decides they are finished. The bar under the axis
//! is the width of the smallest gap this record can see, so "one marker" is read
//! as "one marker, to ±this" and not as "done".
//!
//! **Nothing here rescales itself.** The row slots are fixed and the cents axis
//! is a span the caller chose and holds, because an instrument whose scale moves
//! while you read it cannot be read: a marker that shifts because the axis
//! changed is indistinguishable from a string that moved. Rows a reference has
//! not resolved are drawn empty rather than omitted, so a partial coming and
//! going does not reflow the ones around it.
//!
//! Two layouts, because which one reads better is an open question the panel's
//! toggle exists to answer:
//!
//! - [`UnisonMode::Displayed`] — the displayed partial n\* alone, the same
//!   partial the strobe band shows.
//! - [`UnisonMode::AllPartials`] — every partial that resolved anything, stacked.
//!   Wider, but it shows the discriminator's own evidence: a unison's markers
//!   sit at the same cents on every row, and a false beat's do not.
//!
//! The widget is a stateless renderer; `app.rs` converts the core's Hz offsets
//! to cents and decides which rows exist.

use iced::advanced::text::Alignment;
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, mouse};

use tuner_core::algorithms::peaks::MAX_UNISON_LINES;

use crate::widgets::curve_plot::{GRID, INK_SECONDARY, SERIES, SURFACE, ZERO_LINE};

/// Marker colour for a line the estimator does not stand behind on its own: the
/// weakest of three, whose reported position is the one measured to sit nearest
/// the resolution limit (ADR 0012 §8). Drawn, because hiding it would hide a
/// real string; drawn differently, because it may also be the shoulder of a pair
/// the record cannot separate.
const PROVISIONAL: Color = Color::from_rgb8(0xd9, 0x92, 0x26);

/// One partial's resolved lines, already in cents against that partial's target.
#[derive(Debug, Clone, Copy, Default)]
pub struct UnisonRow {
    /// Partial number n, for the row label.
    pub partial: u8,
    /// Valid entries of [`Self::cents`] / [`Self::amplitude`].
    pub count: u8,
    /// Signed offset from the target, in cents, strongest line first.
    pub cents: [f32; MAX_UNISON_LINES],
    /// Magnitude relative to the strongest line of this partial.
    pub amplitude: [f32; MAX_UNISON_LINES],
    /// `2/T` at this partial, in cents — the smallest gap this record resolves.
    pub resolution_cents: f32,
}

/// Which partials the widget draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnisonMode {
    /// The displayed partial n\* alone.
    Displayed,
    /// Every partial that resolved a line, stacked.
    AllPartials,
}

/// Height of one partial's row, in pixels. Fixed, so a row appearing or
/// vanishing moves nothing else.
pub const ROW_HEIGHT: f32 = 18.0;

/// Vertical chrome above and below the rows — the axis labels and a margin.
pub const ROW_CHROME: f32 = 22.0;

/// Canvas program drawing one unison panel.
pub struct UnisonDisplay {
    rows: Vec<UnisonRow>,
    mode: UnisonMode,
    span_cents: f32,
    cache: canvas::Cache,
}

impl UnisonDisplay {
    /// Builds the display. `span_cents` is the **half**-width of the axis, held
    /// by the caller across hops; see the module note on why it is not derived
    /// from the data here.
    pub fn new(rows: Vec<UnisonRow>, mode: UnisonMode, span_cents: f32) -> Self {
        Self {
            rows,
            mode,
            span_cents,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element; the caller sizes it via its container.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
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

            let (left, right, top) = (30.0f32, 10.0f32, 14.0f32);
            let plot_w = (bounds.width - left - right).max(1.0);
            let half = self.span_cents.max(0.1);
            let x_of = |cents: f32| left + (cents / half * 0.5 + 0.5).clamp(0.0, 1.0) * plot_w;

            // Axis labels: the ends and the target.
            for (cents, label) in [
                (-half, format!("{:+.1}", -half)),
                (0.0, "0 ¢".to_string()),
                (half, format!("{half:+.1}")),
            ] {
                frame.fill_text(canvas::Text {
                    content: label,
                    position: Point::new(x_of(cents), 1.0),
                    color: INK_SECONDARY,
                    size: 10.0.into(),
                    align_x: Alignment::Center,
                    align_y: Vertical::Top,
                    ..canvas::Text::default()
                });
            }

            for (index, row) in self.rows.iter().enumerate() {
                let base = top + (index as f32 + 1.0) * ROW_HEIGHT - 2.0;
                let head = base - (ROW_HEIGHT - 6.0);
                if base > bounds.height {
                    break;
                }

                // The target line, which is what the markers converge onto.
                frame.stroke(
                    &Path::line(Point::new(x_of(0.0), head), Point::new(x_of(0.0), base)),
                    Stroke::default().with_width(1.0).with_color(ZERO_LINE),
                );
                // The axis this row's markers stand on.
                frame.stroke(
                    &Path::line(Point::new(left, base), Point::new(left + plot_w, base)),
                    Stroke::default().with_width(1.0).with_color(GRID),
                );

                if self.mode == UnisonMode::AllPartials {
                    frame.fill_text(canvas::Text {
                        content: format!("n{}", row.partial),
                        position: Point::new(left - 4.0, base - 1.0),
                        color: INK_SECONDARY,
                        size: 10.0.into(),
                        align_x: Alignment::Right,
                        align_y: Vertical::Bottom,
                        ..canvas::Text::default()
                    });
                }

                // The resolution bar, centred on the target: the width of the
                // smallest gap this record can separate.
                if row.resolution_cents > 0.0 {
                    let y = base + 3.0;
                    let (x0, x1) = (
                        x_of(-row.resolution_cents / 2.0),
                        x_of(row.resolution_cents / 2.0),
                    );
                    frame.stroke(
                        &Path::line(Point::new(x0, y), Point::new(x1, y)),
                        Stroke::default().with_width(2.0).with_color(GRID),
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
