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
//! The same renderer draws both unison panels, differing only in the rows they
//! are handed: the displayed partial n\* alone, magnified, and every partial
//! stacked beneath it. Same axis, same scale, same marker style, so the eye
//! re-learns nothing moving between them. The stack is where the
//! discriminator's own evidence is legible — a unison's markers sit at the same
//! cents on every row, and a false beat's do not.
//!
//! The widget is a stateless renderer; `app.rs` converts the core's Hz offsets
//! to cents and decides which rows exist.
//!
//! **Labels stay out of the canvas.** Canvas text is shaped on every frame it
//! is drawn on, while a text widget is re-shaped only when its content changes
//! — and these change only when the key does. Drawing them in the canvas costs
//! better than half the frame rate of a debug build (measured;
//! `layout-by-task-design.md` D9). The canvas draws what moves.

use iced::alignment::{Horizontal, Vertical};
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::widget::{Space, column, container, row, text};
use iced::{Color, Element, Fill, Length, Point, Rectangle, Renderer, Theme, mouse};

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
    /// The same `2/T` in Hz, which is a **beat rate**: two strings this far
    /// apart beat once per `1/resolution_hz` seconds. It is the slowest beat the
    /// record can show, and unlike the cents figure it is the same at every key
    /// — the panel's limit is fixed in Hz while a unison is judged in cents.
    pub resolution_hz: f32,
    /// This partial's target frequency (Hz) — the reference the row's cents are
    /// measured against. With the markers being signed offsets from it, label
    /// plus marker is the measured partial frequency.
    pub ref_hz: f32,
    /// Strobe-bank amplitude of this reference, relative to the strongest
    /// reference this hop. Dims the row: a partial that has decayed out of the
    /// mix is still drawn, but reads as weak rather than as absent.
    pub level: f32,
    /// The bank's D3 amplitude gate — this reference is below the floor and its
    /// lines are held, not fresh.
    pub gated: bool,
}

/// Height of one partial's row, in pixels. Fixed, so a row appearing or
/// vanishing moves nothing else.
pub const ROW_HEIGHT: f32 = 18.0;

/// Fill of the per-row blind zone — the `2/T` band the record cannot separate
/// inside. Dimmer than [`GRID`] so it reads as absence rather than as content:
/// nothing is drawn there because nothing there is measurable.
const BLIND_ZONE: Color = Color::from_rgba8(0x38, 0x38, 0x35, 0.55);

/// Vertical chrome above and below the rows — the axis-label strip and a
/// margin. The strip is a row of text widgets above the canvas, so this is what
/// the caller adds to `rows × row height` to size the whole display.
pub const ROW_CHROME: f32 = 22.0;

/// Height of the axis-label strip, inside [`ROW_CHROME`].
const AXIS_STRIP: f32 = 14.0;

/// Label size for the gutter and the axis strip.
const LABEL_SIZE: f32 = 10.0;

/// Right margin of the plot area, so a marker at the axis end is not drawn on
/// the frame edge where its position stops being readable.
///
/// Public with [`GUTTER`]: the two together define where the plot area sits,
/// and the strobe centres its band on the same span so its centre and the
/// zero line are the same x.
pub const PLOT_RIGHT_MARGIN: f32 = 10.0;

/// Floor under [`UnisonRow::level`]'s dimming. A weak partial fades; it never
/// fades to nothing, because a row that vanished would be read as a partial the
/// bank is not targeting rather than as one that has decayed.
const MIN_LEVEL: f32 = 0.35;

/// Left gutter, wide enough for the longest row label at the label size — see
/// [`row_label`], which is kept short so this stays narrow. The label carries
/// the reference frequency, which is what makes the stacked layout a partials
/// list as well as a unison display.
///
/// Public because the panels' text and the strobe's band are laid out against
/// it: everything in the live loop occupies the same span as the plot.
pub const GUTTER: f32 = 52.0;

/// A row's gutter label: its partial number and that partial's target.
///
/// The target drops its decimal above 1 kHz, which keeps the longest label
/// short and is the more honest figure — 0.1 Hz at 4.9 kHz is 0.035 ¢, finer
/// than anything the display can resolve.
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
    /// Builds the display. `span_cents` is the **half**-width of the axis, held
    /// by the caller across hops; see the module note on why it is not derived
    /// from the data here.
    pub fn new(rows: Vec<UnisonRow>, span_cents: f32) -> Self {
        Self {
            rows,
            span_cents,
            row_height: ROW_HEIGHT,
            cache: canvas::Cache::default(),
        }
    }

    /// Draws the rows at `height` px each instead of [`ROW_HEIGHT`]. The cents
    /// axis is unchanged — magnifying a row buys marker separation, never
    /// resolution — so a magnified row and its counterpart in the stack are the
    /// same measurement at two sizes.
    pub fn row_height(mut self, height: f32) -> Self {
        self.row_height = height;
        self
    }

    /// Creates the view element; the caller sizes it via its container.
    ///
    /// The labels live here, in the widget tree, and only the moving parts go
    /// to the canvas — see the module note.
    pub fn view(self) -> Element<'static, crate::Message> {
        let half = self.span_cents.max(0.1);
        let row_height = self.row_height;

        // One label per row slot, bottom-aligned on the row's own axis line.
        let mut gutter = column![Space::new().height(AXIS_STRIP)];
        for r in &self.rows {
            let label = row_label(r.partial, r.ref_hz);
            gutter = gutter.push(
                container(
                    text(label)
                        .size(LABEL_SIZE)
                        .color(fade(INK_SECONDARY, dim(r.level, r.gated))),
                )
                .width(Fill)
                .height(Length::Fixed(row_height))
                .align_x(Horizontal::Right)
                .align_y(Vertical::Bottom)
                .padding([0.0, 6.0]),
            );
        }

        // The ends and the target, over the plot area the canvas draws into.
        let axis = row![
            text(format!("{:+.1}", -half))
                .size(LABEL_SIZE)
                .color(INK_SECONDARY),
            Space::new().width(Fill),
            text("0 ¢").size(LABEL_SIZE).color(INK_SECONDARY),
            Space::new().width(Fill),
            text(format!("{half:+.1}"))
                .size(LABEL_SIZE)
                .color(INK_SECONDARY),
        ]
        .height(Length::Fixed(AXIS_STRIP));

        // The axis strip is inset by the canvas's own right margin, so its end
        // labels sit over the ends of the plot rather than past them.
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

                let level = dim(row.level, row.gated);

                // The blind zone, centred on the target: two lines closer than
                // this merge into one, so anything the panel could tell you
                // about the interior is not there to be told. Filled rather
                // than stroked — a marker outside it was measured, a gap inside
                // it was not.
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
