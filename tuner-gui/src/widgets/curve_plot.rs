//! # Tuning-Curve Plot Widget
//!
//! Renders a [`tuner_core::models::TuningCurve`]'s d(m) — cents deviation
//! from equal temperament per key — across the 88-key compass (strobe design
//! note §10). One rendering, two scales:
//!
//! - **Full**: axes, octave gridlines, cent labels, measured-key dots, and a
//!   cursor readout (nearest key + its target deviation). The live main-view
//!   plot and the gallery detail view.
//! - **Sparkline**: line + zero line only — the §9 gallery thumbnails. All
//!   thumbnails share one y-range (passed by the gallery) so curve *shapes*
//!   compare honestly; a per-thumbnail scale would make every engine look
//!   alike.
//!
//! Colors are the dataviz reference palette's dark-mode steps, validated
//! against the dark chart surface (series slot 1 passes the lightness band,
//! chroma floor, and ≥3:1 contrast checks): one series per plot, so the panel
//! title carries identity and no legend is drawn.

use iced::advanced::text::Alignment;
use iced::alignment::Vertical;
use iced::widget::canvas::{self, Canvas, Path, Stroke};
use iced::{Color, Element, Fill, Point, Rectangle, Renderer, Theme, mouse};
use tuner_core::models;

/// Dark chart surface (reference palette `--surface-1`, dark). Shared with
/// the gallery (§9) so plot cards and thumbnails sit on one surface system.
pub const SURFACE: Color = Color::from_rgb8(0x1a, 0x1a, 0x19);
/// Series color — categorical slot 1, dark step. Doubles as the gallery's
/// selected-card accent so "selected" and "the drawn curve" share identity.
pub const SERIES: Color = Color::from_rgb8(0x39, 0x87, 0xe5);
/// Recessive gridlines / card borders.
pub const GRID: Color = Color::from_rgb8(0x38, 0x38, 0x35);
/// Zero line — one step above the grid, still recessive.
pub const ZERO_LINE: Color = Color::from_rgb8(0x52, 0x51, 0x4e);
/// Secondary ink for axis labels and readouts.
pub const INK_SECONDARY: Color = Color::from_rgb8(0xc3, 0xc2, 0xb7);

/// Rendering scale of a [`CurvePlot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlotMode {
    /// Axes + gridlines + labels + measured dots + hover readout.
    Full,
    /// Line and zero line only (gallery thumbnail).
    Sparkline,
}

/// Canvas program plotting one tuning curve.
pub struct CurvePlot {
    /// d(m) in cents per key (0 = A0 … 87 = C8).
    cents: [f32; 88],
    /// Keys with a trusted measurement (drawn as dots in [`PlotMode::Full`]).
    measured: [bool; 88],
    mode: PlotMode,
    /// Fixed y-range in cents; `None` auto-ranges from this curve's data.
    /// The gallery passes a shared range across all thumbnails.
    y_range: Option<(f32, f32)>,
    cache: canvas::Cache,
}

impl CurvePlot {
    /// Builds a plot from a curve's cents array and per-key measured flags.
    pub fn new(
        cents: [f32; 88],
        measured: [bool; 88],
        mode: PlotMode,
        y_range: Option<(f32, f32)>,
    ) -> Self {
        Self {
            cents,
            measured,
            mode,
            y_range,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element; the caller sizes it via its container.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
    }

    /// The y-range actually drawn: fixed if given, else auto from the finite
    /// data — always spanning zero, padded, and never tighter than ±5 ¢ so a
    /// flat prior-only curve still renders with headroom.
    fn resolved_y_range(&self) -> (f32, f32) {
        if let Some(r) = self.y_range {
            return r;
        }
        auto_y_range(&self.cents)
    }
}

/// Shared auto-range rule (also used by the gallery to build the common
/// thumbnail range from several curves' data).
pub fn auto_y_range(cents: &[f32]) -> (f32, f32) {
    let mut lo = 0.0f32;
    let mut hi = 0.0f32;
    for &c in cents.iter().filter(|c| c.is_finite()) {
        lo = lo.min(c);
        hi = hi.max(c);
    }
    let pad = ((hi - lo) * 0.1).max(1.0);
    ((lo - pad).min(-5.0), (hi + pad).max(5.0))
}

/// Gridline/label step giving a handful of horizontal lines for a range.
/// At most ~4 lines: the full plot lives in a ~240 px panel, so a denser
/// grid collides its own 11 px labels.
fn cent_step(span: f32) -> f32 {
    for step in [1.0, 2.0, 5.0, 10.0, 20.0, 50.0] {
        if span / step <= 4.0 {
            return step;
        }
    }
    100.0
}

impl<Message> canvas::Program<Message> for CurvePlot {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry> {
        // The cursor readout draws inside the cached closure: widgets are
        // rebuilt (fresh cache) every Tick in this codebase, so per-instance
        // caching never holds a stale cursor across frames.
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            let (full, w, h) = (self.mode == PlotMode::Full, bounds.width, bounds.height);
            // Plot-area margins: room for cent labels left, key labels below.
            let (left, right, top, bottom) = if full {
                (40.0, 10.0, 8.0, 20.0)
            } else {
                (3.0, 3.0, 3.0, 3.0)
            };
            let plot_w = (w - left - right).max(1.0);
            let plot_h = (h - top - bottom).max(1.0);

            let (y_lo, y_hi) = self.resolved_y_range();
            let x_of = |key: f32| left + key / 87.0 * plot_w;
            let y_of = |cents: f32| top + (y_hi - cents) / (y_hi - y_lo) * plot_h;

            frame.fill(&Path::rectangle(Point::ORIGIN, frame.size()), SURFACE);

            if full {
                // Horizontal cent gridlines + labels.
                let step = cent_step(y_hi - y_lo);
                let mut line = (y_lo / step).ceil() * step;
                while line <= y_hi {
                    let y = y_of(line);
                    if line != 0.0 {
                        frame.stroke(
                            &Path::line(Point::new(left, y), Point::new(w - right, y)),
                            Stroke::default().with_width(1.0).with_color(GRID),
                        );
                    }
                    frame.fill_text(canvas::Text {
                        content: if line == 0.0 {
                            "0".to_string()
                        } else {
                            format!("{line:+.0}")
                        },
                        position: Point::new(left - 5.0, y),
                        color: INK_SECONDARY,
                        size: 11.0.into(),
                        align_x: Alignment::Right,
                        align_y: Vertical::Center,
                        ..canvas::Text::default()
                    });
                    line += step;
                }

                // Vertical octave gridlines (every A) + key labels.
                for octave in 0..8 {
                    let key = octave * 12;
                    let x = x_of(key as f32);
                    if key != 0 {
                        frame.stroke(
                            &Path::line(Point::new(x, top), Point::new(x, h - bottom)),
                            Stroke::default().with_width(1.0).with_color(GRID),
                        );
                    }
                    frame.fill_text(canvas::Text {
                        content: format!("A{octave}"),
                        position: Point::new(x, h - bottom + 3.0),
                        color: INK_SECONDARY,
                        size: 11.0.into(),
                        align_x: Alignment::Center,
                        align_y: Vertical::Top,
                        ..canvas::Text::default()
                    });
                }
            }

            // Zero line (drawn over the grid, under the series).
            if (y_lo..=y_hi).contains(&0.0) {
                let y = y_of(0.0);
                frame.stroke(
                    &Path::line(Point::new(left, y), Point::new(w - right, y)),
                    Stroke::default().with_width(1.0).with_color(ZERO_LINE),
                );
            }

            // The curve. Lyon asserts on non-finite path coordinates and a
            // canvas must never panic the app: skip bad points, breaking the
            // line so nothing interpolates across them.
            let series = Path::new(|b| {
                let mut pen_down = false;
                for (k, &c) in self.cents.iter().enumerate() {
                    if !c.is_finite() {
                        pen_down = false;
                        continue;
                    }
                    let p = Point::new(x_of(k as f32), y_of(c.clamp(y_lo, y_hi)));
                    if pen_down {
                        b.line_to(p);
                    } else {
                        b.move_to(p);
                        pen_down = true;
                    }
                }
            });
            let width = if full { 2.0 } else { 1.5 };
            frame.stroke(
                &series,
                Stroke::default().with_width(width).with_color(SERIES),
            );

            if full {
                // Measured-key dots — same series hue: they are the same
                // entity (the curve), accented where a measurement anchors it.
                for (k, &c) in self.cents.iter().enumerate() {
                    if self.measured[k] && c.is_finite() {
                        let p = Point::new(x_of(k as f32), y_of(c.clamp(y_lo, y_hi)));
                        frame.fill(&Path::circle(p, 3.5), SERIES);
                        frame.fill(&Path::circle(p, 1.5), SURFACE);
                    }
                }

                // Cursor readout: nearest key + its target deviation.
                if let Some(pos) = cursor.position_in(bounds)
                    && (left..=w - right).contains(&pos.x)
                {
                    let key = (((pos.x - left) / plot_w * 87.0).round() as usize).min(87);
                    let c = self.cents[key];
                    if c.is_finite() {
                        let x = x_of(key as f32);
                        frame.stroke(
                            &Path::line(Point::new(x, top), Point::new(x, h - bottom)),
                            Stroke::default().with_width(1.0).with_color(ZERO_LINE),
                        );
                        let p = Point::new(x, y_of(c.clamp(y_lo, y_hi)));
                        frame.fill(&Path::circle(p, 4.0), SERIES);
                        let (name, _) = models::find_nearest_note_by_index(key as u8);
                        frame.fill_text(canvas::Text {
                            content: format!("{name} · {c:+.1} ¢"),
                            position: Point::new(w - right - 4.0, top + 2.0),
                            color: INK_SECONDARY,
                            size: 12.0.into(),
                            align_x: Alignment::Right,
                            align_y: Vertical::Top,
                            ..canvas::Text::default()
                        });
                    }
                }
            }
        });

        vec![geometry]
    }
}
