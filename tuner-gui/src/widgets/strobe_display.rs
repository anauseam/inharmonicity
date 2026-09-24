//! # Strobe display
//!
//! The strobe band: a segmented ring turned by the accumulated beat phase, one
//! pattern period per beat, so a string in tune stands still. While gated it
//! freezes and dims.

use iced::widget::canvas::{self, Canvas, Path, Stroke, path::Arc};
use iced::{Color, Element, Fill, Point, Radians, Rectangle, Renderer, Theme, mouse};

use crate::widgets::curve_plot::{GRID, SERIES, SURFACE};

/// Dark/light pattern periods around the ring.
// Kept small: an S-fold pattern aliases once S·Δf passes half the hop rate.
// Ours, set by eye.
const PATTERN_PERIODS: usize = 4;

/// Wedge fill while gated: visible, since a frozen band is the re-strike
/// signal, but not the live series colour.
const GATED_FILL: Color = Color::from_rgb8(0x6e, 0x6d, 0x66);

/// Canvas program drawing one strobe band.
pub struct StrobeDisplay {
    /// Accumulated beat phase in cycles, [0, 1).
    beat_phase: f32,
    /// Amplitude gate: `true` dims the band, which the caller freezes by holding
    /// the phase.
    gated: bool,
    cache: canvas::Cache,
}

impl StrobeDisplay {
    /// Builds the band from the current phase and gate state.
    pub fn new(beat_phase: f32, gated: bool) -> Self {
        Self {
            beat_phase,
            gated,
            cache: canvas::Cache::default(),
        }
    }

    /// Creates the view element; the caller sizes it via its container.
    pub fn view(self) -> Element<'static, crate::Message> {
        Canvas::new(self).width(Fill).height(Fill).into()
    }
}

impl<Message> canvas::Program<Message> for StrobeDisplay {
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

            let center = Point::new(bounds.width / 2.0, bounds.height / 2.0);
            let outer = (bounds.width.min(bounds.height) / 2.0 - 6.0).max(1.0);
            let inner = outer * 0.55;

            // One pattern period of rotation per beat cycle, so the
            // phase wrap at 1.0 lands exactly on the pattern's symmetry.
            let period = std::f32::consts::TAU / PATTERN_PERIODS as f32;
            let theta = self.beat_phase.rem_euclid(1.0) * period;
            let fill = if self.gated { GATED_FILL } else { SERIES };

            // Dark wedges: one per pattern period, each half a period wide.
            for k in 0..PATTERN_PERIODS {
                let start = theta + k as f32 * period;
                let end = start + period / 2.0;
                let wedge = Path::new(|b| {
                    b.move_to(center);
                    b.arc(Arc {
                        center,
                        radius: outer,
                        start_angle: Radians(start),
                        end_angle: Radians(end),
                    });
                    b.close();
                });
                frame.fill(&wedge, fill);
            }

            // Punch the ring's hole and outline it.
            frame.fill(&Path::circle(center, inner), SURFACE);
            let ring = Stroke::default().with_width(1.0).with_color(GRID);
            frame.stroke(&Path::circle(center, outer), ring);
            frame.stroke(&Path::circle(center, inner), ring);
        });

        vec![geometry]
    }
}
