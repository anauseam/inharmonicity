//! # Curve Selection & Comparison Gallery
//!
//! The master–detail gallery of strobe design note §9: four sections, one per
//! engine class (a)–(d), with sparkline thumbnails; clicking one opens the
//! detail view — full plot plus the deferred **metrics** and **listen** slots
//! (greyed shells until those efforts land). The (c) ρ Low/High presets are
//! greyed "deferred" placeholders until (c)'s Giordano calibration is factored
//! out of the per-preset path (§14 step 6).
//!
//! Per R7, the sub-class-less (a)/(b) render as plain wide cards, not
//! single-item thumbnail rows. Selection is **display-only** (D7): it sets
//! which curve the live plot (and later the strobe) shows — never a recompute.
//! All thumbnails share one y-range so the engines' shapes compare honestly.

use crate::Message;
use crate::app::EngineChoice;
use crate::widgets::curve_plot::{self, CurvePlot, PlotMode};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Border, Color, Element, Length};
use tuner_core::models::TuningCurve;
use tuner_core::worker::CurveBundle;

/// Muted ink for deferred/disabled affordances (house Disabled grey).
const INK_DISABLED: Color = Color::from_rgb(0.6, 0.6, 0.6);

/// Entry point for the settings main panel: gallery, or detail if one is open.
pub fn create_curve_select_panel(
    bundle: Option<&CurveBundle>,
    selected: EngineChoice,
    detail: Option<EngineChoice>,
) -> Element<'static, Message> {
    let Some(bundle) = bundle else {
        return text("Computing tuning curves…").size(18).into();
    };
    match detail {
        Some(choice) => create_detail(bundle, choice, selected),
        None => create_gallery(bundle, selected),
    }
}

/// The four-section gallery (§9).
fn create_gallery(bundle: &CurveBundle, selected: EngineChoice) -> Element<'static, Message> {
    let range = shared_thumb_range(bundle);

    let section = |title: &'static str, items: Element<'static, Message>| {
        column![text(title).size(16), items].spacing(8)
    };

    column![
        text("Curve Select").size(22),
        text("Click a curve to inspect it; the chosen curve drives the live plot.")
            .size(13)
            .color(curve_plot::INK_SECONDARY),
        Space::new().height(4),
        section(
            "(a) Rigaud prior",
            thumb(bundle, EngineChoice::RigaudPure, selected, range, true),
        ),
        section(
            "(b) Per-key + Whittaker",
            thumb(bundle, EngineChoice::PerKeySmoothed, selected, range, true),
        ),
        section(
            "(c) Giordano-calibrated octave type",
            row![
                deferred_thumb("ρ Low"),
                thumb(bundle, EngineChoice::GiordanoMean, selected, range, false),
                deferred_thumb("ρ High"),
            ]
            .spacing(10)
            .into(),
        ),
        section(
            "(d) Multi-interval least squares",
            row![
                thumb(bundle, EngineChoice::MultiBalanced, selected, range, false),
                thumb(
                    bundle,
                    EngineChoice::MultiPureTwelfths,
                    selected,
                    range,
                    false
                ),
            ]
            .spacing(10)
            .into(),
        ),
    ]
    .spacing(14)
    .into()
}

/// Detail view: full plot + display-selection + deferred metric/listen slots.
fn create_detail(
    bundle: &CurveBundle,
    choice: EngineChoice,
    selected: EngineChoice,
) -> Element<'static, Message> {
    let curve = choice.resolve(bundle);
    let (cents, measured) = plot_inputs(curve);

    let measured_count = measured.iter().filter(|&&m| m).count();
    // Advisory flags on *measured* keys only (§5.6's recapture set); on an
    // unmeasured key, prior fallback is expected, not a warning.
    let flagged_count = curve
        .flags
        .iter()
        .filter(|f| f.measured && (f.excluded || f.negative_stretch || f.giordano_excluded))
        .count();

    let back = button(text("← Gallery").size(14))
        .padding([6, 10])
        .on_press(Message::CurveDetailClosed);

    let plot = container(CurvePlot::new(cents, measured, PlotMode::Full, None).view())
        .width(Length::Fill)
        .height(Length::Fixed(320.0));

    let select_button = if selected == choice {
        button(text("✓ In use").size(14))
            .padding([8, 14])
            .style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.2, 0.35, 0.25))),
                text_color: Color::WHITE,
                ..button::Style::default()
            })
    } else {
        button(text("Use for display").size(14))
            .padding([8, 14])
            .on_press(Message::EngineSelected(choice))
    };

    // Deferred slots (§9): curve metrics (README No-ETA "Advanced mode") and
    // the in-app auralization playback (the seventh crossing). Shells only.
    let deferred_slot = |label: &'static str| {
        button(text(label).size(14).color(INK_DISABLED))
            .padding([8, 14])
            .style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(Color::from_rgb(0.3, 0.3, 0.3))),
                text_color: INK_DISABLED,
                ..button::Style::default()
            })
    };

    column![
        row![back, Space::new().width(14), text(choice.label()).size(22)]
            .align_y(Alignment::Center),
        plot,
        text(format!(
            "{measured_count} of 88 keys measured · {flagged_count} flagged for recapture"
        ))
        .size(13)
        .color(curve_plot::INK_SECONDARY),
        row![
            select_button,
            Space::new().width(10),
            deferred_slot("Metrics (planned)"),
            Space::new().width(10),
            deferred_slot("Listen (planned)"),
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(12)
    .into()
}

/// A clickable curve thumbnail (small sparkline card). `wide` renders the
/// R7 plain-card format used by the sub-class-less (a)/(b).
fn thumb(
    bundle: &CurveBundle,
    choice: EngineChoice,
    selected: EngineChoice,
    y_range: (f32, f32),
    wide: bool,
) -> Element<'static, Message> {
    let (cents, measured) = plot_inputs(choice.resolve(bundle));
    let (w, h) = if wide { (330.0, 64.0) } else { (155.0, 54.0) };

    let plot =
        container(CurvePlot::new(cents, measured, PlotMode::Sparkline, Some(y_range)).view())
            .width(Length::Fixed(w))
            .height(Length::Fixed(h));

    let name = if selected == choice {
        format!("{} ✓", choice.short_label())
    } else {
        choice.short_label().to_string()
    };

    let is_selected = selected == choice;
    button(
        column![plot, text(name).size(13)]
            .spacing(4)
            .align_x(Alignment::Center),
    )
    .padding(6)
    .style(move |_theme, _status| button::Style {
        background: Some(iced::Background::Color(curve_plot::SURFACE)),
        text_color: Color::WHITE,
        border: Border {
            color: if is_selected {
                curve_plot::SERIES
            } else {
                curve_plot::GRID
            },
            width: if is_selected { 2.0 } else { 1.0 },
            radius: 4.0.into(),
        },
        ..button::Style::default()
    })
    .on_press(Message::CurveDetailOpened(choice))
    .into()
}

/// Greyed placeholder for a preset that is not computed yet (§9: the (c)
/// ρ Low/High trio slots — the idiomatic missing-feature card).
fn deferred_thumb(name: &'static str) -> Element<'static, Message> {
    container(
        column![
            container(text("deferred").size(12).color(INK_DISABLED))
                .width(Length::Fixed(155.0))
                .height(Length::Fixed(54.0))
                .center_x(Length::Fixed(155.0))
                .center_y(Length::Fixed(54.0)),
            text(name).size(13).color(INK_DISABLED),
        ]
        .spacing(4)
        .align_x(Alignment::Center),
    )
    .padding(6)
    .style(|_theme| container::Style {
        background: Some(iced::Background::Color(Color::from_rgb(0.18, 0.18, 0.18))),
        border: Border {
            color: curve_plot::GRID,
            width: 1.0,
            radius: 4.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// Cents + measured flags of a curve, in the plot widget's input form.
fn plot_inputs(curve: &TuningCurve) -> ([f32; 88], [bool; 88]) {
    let mut measured = [false; 88];
    for (m, f) in measured.iter_mut().zip(curve.flags.iter()) {
        *m = f.measured;
    }
    (curve.cents, measured)
}

/// One y-range across every engine in the bundle, so the gallery's
/// thumbnails compare shapes on a common scale.
fn shared_thumb_range(bundle: &CurveBundle) -> (f32, f32) {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for choice in EngineChoice::ALL {
        let (l, h) = curve_plot::auto_y_range(&choice.resolve(bundle).cents);
        lo = lo.min(l);
        hi = hi.max(h);
    }
    (lo, hi)
}
