//! # Curve gallery
//!
//! The tuning-curve engines side by side, one section per class (a)–(d), and
//! one engine's detail view. Choosing an engine sets which curve the plot and
//! the strobe read.

use crate::Message;
use crate::advisory;
use crate::widgets::curve_plot::{self, CurvePlot, PlotMode};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Border, Color, Element, Length};
use tuner_core::models::{EngineChoice, TuningCurve};
use tuner_core::worker::CurveBundle;

/// Muted ink for deferred/disabled affordances (house Disabled grey).
const INK_DISABLED: Color = Color::from_rgb(0.6, 0.6, 0.6);

/// The engines offered as tuning targets. (b) and (c) are computed, but
/// withheld while their validity is open.
// the tuning-curve grounding analysis
const GALLERY_ENGINES: [EngineChoice; 3] = [
    EngineChoice::RigaudPure,
    EngineChoice::MultiBalanced,
    EngineChoice::MultiPureTwelfths,
];

/// Whether the gallery offers `choice` as a tuning target.
fn is_offered(choice: EngineChoice) -> bool {
    GALLERY_ENGINES.contains(&choice)
}

/// A curve engine's full name, as a title shows it.
pub(crate) fn engine_label(engine: EngineChoice) -> &'static str {
    match engine {
        EngineChoice::RigaudPure => "(a) Rigaud prior",
        EngineChoice::PerKeySmoothed => "(b) Per-key + Whittaker",
        EngineChoice::GiordanoMean => "(c) Giordano · ρ Mean",
        EngineChoice::MultiBalanced => "(d) Multi-interval · Balanced",
        EngineChoice::MultiPureTwelfths => "(d) Multi-interval · Pure 12ths",
    }
}

/// A curve engine's name without its class, where a section header already
/// names the class.
fn engine_short_label(engine: EngineChoice) -> &'static str {
    match engine {
        EngineChoice::RigaudPure => "Rigaud prior",
        EngineChoice::PerKeySmoothed => "Per-key smoothed",
        EngineChoice::GiordanoMean => "ρ Mean",
        EngineChoice::MultiBalanced => "Balanced",
        EngineChoice::MultiPureTwelfths => "Pure 12ths",
    }
}

/// The gallery, or the detail view if one is open.
pub fn panel(
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

/// The four-section gallery.
fn create_gallery(bundle: &CurveBundle, selected: EngineChoice) -> Element<'static, Message> {
    let range = shared_thumb_range(bundle);

    let section = |title: &'static str, items: Element<'static, Message>| {
        column![text(title).size(16), items].spacing(8)
    };

    column![
        text("Curve Select").size(22),
        text("Click a curve to inspect it; the chosen curve drives the live plot and the strobe.")
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

/// One engine's detail view.
fn create_detail(
    bundle: &CurveBundle,
    choice: EngineChoice,
    selected: EngineChoice,
) -> Element<'static, Message> {
    let curve = bundle.curve(choice);
    let (cents, measured, suspect) = plot_inputs(curve);

    let measured_count = measured.iter().filter(|&&m| m).count();
    let flagged_count = suspect.iter().filter(|&&s| s).count();

    let back = button(text("← Gallery").size(14))
        .padding([6, 10])
        .on_press(Message::CurveDetailClosed);

    let plot = container(CurvePlot::new(cents, measured, suspect, PlotMode::Full, None).view())
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

    // Disabled slots for curve metrics and auralization playback, both in TODO.md.
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
        row![
            back,
            Space::new().width(14),
            text(engine_label(choice)).size(22)
        ]
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

/// A clickable sparkline card, or a greyed card for an engine not offered.
/// `wide` is the card of a section holding one engine.
fn thumb(
    bundle: &CurveBundle,
    choice: EngineChoice,
    selected: EngineChoice,
    y_range: (f32, f32),
    wide: bool,
) -> Element<'static, Message> {
    if !is_offered(choice) {
        return placeholder_thumb("withheld", engine_short_label(choice), wide);
    }

    let (cents, measured, suspect) = plot_inputs(bundle.curve(choice));
    let (w, h) = card_size(wide);

    let plot = container(
        CurvePlot::new(cents, measured, suspect, PlotMode::Sparkline, Some(y_range)).view(),
    )
    .width(Length::Fixed(w))
    .height(Length::Fixed(h));

    let name = if selected == choice {
        format!("{} ✓", engine_short_label(choice))
    } else {
        engine_short_label(choice).to_string()
    };

    let is_selected = selected == choice;
    button(
        column![plot, text(name).size(13)]
            .spacing(4)
            .align_x(Alignment::Center),
    )
    .padding(6)
    .style(move |_theme, _status| button::Style {
        // The card sits on the plot's own surface, and a selected card takes
        // the curve's colour, so card and curve read as one object.
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

/// Plot size of a gallery card: the plain wide card, or a thumbnail.
fn card_size(wide: bool) -> (f32, f32) {
    if wide { (330.0, 64.0) } else { (155.0, 54.0) }
}

/// A greyed card for a curve the gallery shows but does not offer: `status` is
/// `"deferred"` for a preset not computed, `"withheld"` for an engine that is.
fn placeholder_thumb(
    status: &'static str,
    name: &'static str,
    wide: bool,
) -> Element<'static, Message> {
    let (w, h) = card_size(wide);
    container(
        column![
            container(text(status).size(12).color(INK_DISABLED))
                .width(Length::Fixed(w))
                .height(Length::Fixed(h))
                .center_x(Length::Fixed(w))
                .center_y(Length::Fixed(h)),
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

/// The (c) ρ Low/High slots: presets with no computation path yet.
fn deferred_thumb(name: &'static str) -> Element<'static, Message> {
    placeholder_thumb("deferred", name, false)
}

/// Cents, measured flags and suspect marks of a curve, in the plot widget's
/// input form.
pub fn plot_inputs(curve: &TuningCurve) -> ([f32; 88], [bool; 88], [bool; 88]) {
    let mut measured = [false; 88];
    for (m, f) in measured.iter_mut().zip(curve.flags.iter()) {
        *m = f.measured;
    }
    (curve.cents, measured, advisory::suspect_keys(&curve.flags))
}

/// One y-range across every offered engine, so the thumbnails compare shapes on
/// one scale. Withheld cards plot nothing, so they do not widen it.
fn shared_thumb_range(bundle: &CurveBundle) -> (f32, f32) {
    let mut lo = f32::MAX;
    let mut hi = f32::MIN;
    for choice in GALLERY_ENGINES {
        let (l, h) = curve_plot::auto_y_range(&bundle.curve(choice).cents);
        lo = lo.min(l);
        hi = hi.max(h);
    }
    (lo, hi)
}
