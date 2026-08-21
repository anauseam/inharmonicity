//! # Main Display Module
//!
//! This module contains the main display components and layout logic
//! for the Inharmonicity piano tuning application.

use crate::Message;
use crate::advisory;
use crate::app::{AppDisplayData, Instrument, ReferenceMode, TuningMode, UNISON_SPAN_LADDER};
use crate::calibration::CALIBRATION_FRAMES;
use crate::utils::view_utils::{
    ButtonConfig, ButtonType, make_capture_button, make_sidebar_section, make_undo_button,
};
use crate::views::curve_select;
use crate::widgets::curve_plot::{CurvePlot, INK_SECONDARY, PlotMode, SUSPECT};
use crate::widgets::strobe_display::StrobeDisplay;
use crate::widgets::unison_display::{self, UnisonDisplay};
use crate::widgets::{cent_meter, guitar_strings, piano_keyboard, spectrogram};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::models::{self, InharmonicityProfile};
use tuner_core::pipeline::CaptureState;
use tuner_core::strobe::MAX_STROBE_REFS;
use tuner_core::strobe::unison::UnisonVerdict;
use tuner_core::worker::CurveBundle;

const TOOLS_CONFIG: [ButtonConfig; 6] = [
    ButtonConfig {
        label: "Spectrogram",
        message: Some(Message::ToggleSpectrogram),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Centmeter",
        message: Some(Message::ToggleCentMeter),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Key select",
        message: Some(Message::ToggleKeySelect),
        button_type: ButtonType::Standard,
    },
    // The live tuning-curve plot (strobe design §10): watch the curve form
    // while capturing.
    ButtonConfig {
        label: "Curve Plot",
        message: Some(Message::ToggleCurvePlot),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Strobe",
        message: Some(Message::ToggleStrobe),
        button_type: ButtonType::Standard,
    },
    // ButtonConfig {
    //     label: "Inharmonicity Graph",
    //     message: Some(Message::ToggleInharmonicityGraph),
    //     button_type: ButtonType::Standard,
    // },
    ButtonConfig {
        label: "Measurement Mode",
        message: Some(Message::ToggleMeasurementMode),
        button_type: ButtonType::MeasurementMode,
    },
];

const PROGRAM_CONFIG: [ButtonConfig; 1] = [ButtonConfig {
    // Captures auto-save; this is an explicit flush, kept because a
    // "did that save?" affordance is worth more than the button costs.
    label: "Save Profile",
    message: Some(Message::SaveProfile),
    button_type: ButtonType::Standard,
}];

/// Width of the live-loop column. Fixed, because the strobe and the unison
/// axes are read at a glance and a readout that changes width with the window
/// changes what a marker's position means.
const LIVE_COLUMN_WIDTH: f32 = 360.0;

/// Which rows a unison panel draws — the displayed partial magnified, or every
/// partial stacked. The same measurement at two sizes; the panels differ in the
/// rows they are handed and in which numbers they print beside them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnisonPanel {
    /// The strobe band's own partial n*, drawn large.
    Displayed,
    /// Every reference in the bank, stacked — the discriminator's evidence.
    AllPartials,
}

/// Side of the square strobe band.
const STROBE_BAND: f32 = 150.0;

/// Height of the strobe panel: title, band, the two readout lines beneath it,
/// and room for the flag advisory and the curve-lock footer.
const STROBE_PANEL_HEIGHT: f32 = 300.0;

/// Row height of the magnified single-partial panel, against
/// [`unison_display::ROW_HEIGHT`] in the stack. Magnification buys marker
/// separation, not resolution: both panels share one cents axis.
const UNISON_MAGNIFIED_ROW: f32 = 48.0;

/// Tools entries present only while unison assist is enabled in Settings ▸
/// Advanced. Each panel toggles on its own.
const UNISON_TOOLS_CONFIG: [ButtonConfig; 2] = [
    ButtonConfig {
        label: "Unison (partial)",
        message: Some(Message::ToggleUnisonDisplayed),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Unison (all)",
        message: Some(Message::ToggleUnisonAll),
        button_type: ButtonType::Standard,
    },
];

/// Static main sidebar configuration
const MAIN_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 2] = [
    ("Tools", TOOLS_CONFIG.as_slice()),
    ("Program", PROGRAM_CONFIG.as_slice()),
];

/// Creates the complete main application view
pub fn create_main_view(
    data: &AppDisplayData,
    _profile: &InharmonicityProfile,
    capture_message: Message,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    // Show calibrating/shutdown message if audio worker is not active or calibrating
    if !data.audio_worker_active || data.is_calibrating {
        let message = if data.is_calibrating {
            format!(
                "Calibrating… {}/{}",
                data.calibration_progress, CALIBRATION_FRAMES
            )
        } else {
            "Shutting down...".to_string()
        };
        return container(text(message).size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let widget_area = create_widget_area(data, curve_bundle);

    // Create sidebar
    let sidebar = create_sidebar(
        data.measurement_mode_active,
        data.capture_state,
        data.undo_target_note.clone(),
        capture_message,
        data.reference_mode,
        data.unison_assist,
        SessionStatus {
            // Manual only: the declaration names strings of a key the operator
            // named, and Auto latches the key by discovery instead.
            strings: (data.string_isolation
                && matches!(data.tuning_mode, TuningMode::Manual { .. }))
            .then_some((data.sounding_strings, data.strings_touched)),
            extended_capture: data
                .extended_capture
                .then_some((data.extended_capture_secs, data.capture_progress_secs)),
            curve_recomputing: data.curve_recomputing,
        },
    );

    // Assemble the final layout
    // Height is filled, not shrunk: the live-loop column scrolls, and a
    // scrollable in a shrink-height parent has no bound to scroll within.
    let main_content = row![sidebar, Space::new().width(10), widget_area,]
        .align_y(Alignment::Start)
        .height(Fill)
        .padding(20);

    let base = container(main_content).width(Fill).height(Fill);

    // Re-lock confirm modal (design §8): a scrim + card stacked over the view.
    // Re-lock shifts every strobe target, so it is guarded rather than instant.
    if data.relock_confirm_open {
        let scrim = container(Space::new())
            .width(Fill)
            .height(Fill)
            .style(|_| container::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgba(
                    0.0, 0.0, 0.0, 0.6,
                ))),
                ..container::Style::default()
            });
        let card = container(
            column![
                text("Re-lock to the latest curve?").size(18),
                Space::new().height(8),
                text(
                    "This shifts all strobe targets — keys already tuned to the \
                      current lock will read off relative to the new one."
                )
                .size(13)
                .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7)),
                Space::new().height(16),
                row![
                    Space::new().width(Fill),
                    button(text("Cancel").size(14))
                        .padding([6, 14])
                        .on_press(Message::CancelRelock),
                    Space::new().width(10),
                    button(text("Re-lock").size(14))
                        .padding([6, 14])
                        .on_press(Message::ConfirmRelock),
                ]
                .align_y(Alignment::Center),
            ]
            .width(Length::Fixed(420.0))
            .padding(24),
        )
        .style(|_| container::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb8(
                0x2a, 0x2a, 0x28,
            ))),
            border: iced::Border {
                color: iced::Color::from_rgb8(0x55, 0x54, 0x50),
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        });
        let overlay = container(card)
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill);
        iced::widget::stack![base, scrim, overlay].into()
    } else {
        base.into()
    }
}

/// Creates the widget area — two columns split by task
/// ([`docs/design/layout-by-task-design.md`](../../../docs/design/layout-by-task-design.md)).
///
/// **Left, context:** what am I tuning and what is the plan — the spectrogram,
/// the note-select surface, the tuning curve, and the engine's own state.
/// **Right, the live loop:** what the eye is on while the hand is on the lever
/// — the strobe, then the same string's unison at two magnifications.
///
/// A hidden panel is not pushed, so it costs no space and its neighbours do not
/// move; a column with nothing in it is omitted and the other takes the width.
pub fn create_widget_area(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    // The open instrument is named beside the title on every frame. This is
    // what makes resume-at-launch plus autosave safe: arriving at a second
    // instrument and capturing would otherwise fold its measurements into the
    // previous one's file with nothing on screen having said so. Managing
    // instruments is a settings task; *knowing which one is open* is not.
    let instrument = if data.open_identity.name.is_empty() {
        "Untitled instrument".to_string()
    } else {
        data.open_identity.name.clone()
    };
    let title = row![
        text("Inharmonicity").size(28),
        Space::new().width(12),
        text(instrument)
            .size(14)
            .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7)),
    ]
    .align_y(Alignment::Center);

    // Left column — context, in the order a session reads it: what the
    // instrument is doing, which key I am on, where that key sits in the plan.
    let mut context = column![].width(Fill).spacing(10);
    let mut context_panels = 0;
    // The spectrogram and the cent meter share the top row: both answer what the
    // engine is hearing.
    let top: Vec<_> = [
        create_spectrogram_panel(data),
        create_cent_meter_panel(data),
    ]
    .into_iter()
    .flatten()
    .collect();
    context_panels += top.len();
    if !top.is_empty() {
        let mut top_row = row![].width(Fill).spacing(10).align_y(Alignment::Start);
        for panel in top {
            top_row = top_row.push(panel);
        }
        context = context.push(top_row);
    }
    for panel in [
        create_keyboard_panel(data, curve_bundle),
        create_curve_plot_panel(data, curve_bundle),
    ]
    .into_iter()
    .flatten()
    {
        context = context.push(panel);
        context_panels += 1;
    }

    // Right column — the live loop, in descending magnification of one
    // question: is this string where I want it.
    let strobe_panel = create_strobe_panel(data, curve_bundle);
    let mut live = column![].spacing(10);
    let mut live_panels = 0;
    for panel in [
        create_unison_panel(data, UnisonPanel::Displayed),
        create_unison_panel(data, UnisonPanel::AllPartials),
    ]
    .into_iter()
    .flatten()
    {
        live = live.push(panel);
        live_panels += 1;
    }

    let mut columns = row![].height(Fill).align_y(Alignment::Start);
    if context_panels > 0 {
        columns = columns.push(scrollable(context).height(Fill));
    }
    if strobe_panel.is_some() || live_panels > 0 {
        // The strobe is pinned above the scroll. It is read without looking away
        // from the string, so it is the one panel that must not move when the
        // column below it does.
        let mut right = column![]
            .spacing(10)
            .width(Length::Fixed(LIVE_COLUMN_WIDTH));
        if let Some(panel) = strobe_panel {
            right = right.push(panel);
        }
        if live_panels > 0 {
            right = right.push(scrollable(live).height(Fill));
        }
        columns = columns.push(Space::new().width(10));
        columns = columns.push(right);
    }

    // The notice sits last and is pushed rather than wrapped: appearing and
    // disappearing must not move the panels above it, and an empty placeholder
    // would still take a row of the column's spacing.
    let mut content = column![title, Space::new().height(20), columns]
        .width(Fill)
        .height(Fill)
        .spacing(10);

    if let Some(notice) = create_auto_mode_notice(data) {
        content = content.push(notice);
    }

    content.into()
}

/// Creates the Auto-mode notice, shown only while no key is selected.
///
/// Auto-mode captures are recorded untrusted and never reach the tuning curve
/// (ADR 0006 Corrections item 3), which is invisible from the UI otherwise: the
/// capture succeeds, the measurement lands in the profile, and the curve simply
/// does not move. Mode is also implicit here — selecting a key *is* entering
/// Manual mode — so the notice names the surface that switches it.
fn create_auto_mode_notice(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !matches!(data.tuning_mode, TuningMode::Auto) {
        return None;
    }

    let select_hint = match data.instrument {
        Instrument::Piano => "Select a key on the Keyboard Key Select panel",
        Instrument::Guitar => "Select a string on the Guitar String Select panel",
    };

    Some(
        text(format!(
            "Auto mode — captures are excluded from the tuning curve. \
             {select_hint} to tune or measure it."
        ))
        .size(14)
        .color(iced::Color::from_rgb8(0xd9, 0x92, 0x26))
        .into(),
    )
}

/// Puts an element on the same horizontal span as the plots — indented past the
/// label gutter and inset by the plot's right margin.
///
/// A live-loop panel's text is read against its plot, so it starts and ends
/// where the plot does.
fn on_plot_span(content: Element<'static, Message>) -> Element<'static, Message> {
    container(content)
        .width(Fill)
        .padding(iced::Padding {
            top: 0.0,
            right: unison_display::PLOT_RIGHT_MARGIN,
            bottom: 0.0,
            left: unison_display::GUTTER,
        })
        .into()
}

/// The strobe band in the same horizontal span the unison plots use: a gutter
/// of [`unison_display::GUTTER`], then the plot area, with the band **centred**
/// in it.
///
/// Centred, because the line the panels share is the target: the unison plots
/// draw it at the midpoint of their plot area (`x_of(0)`) and the band is the
/// whole of it. Composed from the two gutter constants rather than an offset
/// fixed here, so band centre and zero line stay the same x at any width.
fn strobe_row(band: Element<'static, Message>, partial: Option<u8>) -> Element<'static, Message> {
    let label: Element<'static, Message> = match partial {
        Some(n) => text(format!("n{n}"))
            .size(10)
            .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7))
            .into(),
        None => Space::new().into(),
    };
    row![
        container(label)
            .width(Length::Fixed(unison_display::GUTTER))
            .height(Length::Fixed(STROBE_BAND))
            .align_x(iced::alignment::Horizontal::Right)
            .align_y(iced::alignment::Vertical::Bottom)
            .padding([0.0, 6.0]),
        container(
            container(band)
                .width(Length::Fixed(STROBE_BAND))
                .height(Length::Fixed(STROBE_BAND))
        )
        .width(Fill)
        .center_x(Fill),
        Space::new().width(Length::Fixed(unison_display::PLOT_RIGHT_MARGIN)),
    ]
    .into()
}

/// What a live-loop panel says in Auto mode: it is off, and why, and the one
/// surface that turns it on. The strobe and both unison panels read against a
/// nominated key's targets, which Auto has not got — the engine identifies the
/// note it hears, but nothing has said which note is being *tuned*.
fn auto_mode_note(instrument: Instrument, panel: &str) -> String {
    let select = match instrument {
        Instrument::Piano => "a key on the Keyboard Key Select panel",
        Instrument::Guitar => "a string on the Guitar String Select panel",
    };
    format!("Off in Auto mode — the {panel} needs a nominated key. Click {select}.")
}

/// Creates the strobe panel (design §5). The strobe needs a named key — a
/// target does not exist without one — so in Auto mode the panel shows a
/// how-to-enter-manual-mode hint instead of hiding entirely. In Manual mode:
/// one band for the selected key's displayed partial n*, frozen/dimmed when
/// the partial is below the tracker's gate (D3), with the D4 coarse readout
/// computed against the **target** (R13), not ET-nearest.
///
/// The target it reads against follows the app-level [`ReferenceMode`] (the
/// sidebar toggle), shared with the cent meter.
fn create_strobe_panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Option<Element<'static, Message>> {
    if !data.strobe_visible {
        return None;
    }

    let TuningMode::Manual {
        note_name,
        key_index,
    } = &data.tuning_mode
    else {
        // Frozen: the state the band already carries for a partial it cannot
        // read.
        let panel = container(
            column![
                on_plot_span(text("Strobe").size(18).into()),
                Space::new().height(10),
                strobe_row(StrobeDisplay::new(0.0, true).view(), None),
                Space::new().height(8),
                on_plot_span(
                    text(auto_mode_note(data.instrument, "strobe"))
                        .size(13)
                        .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7))
                        .into()
                ),
            ]
            .width(Fill)
            .spacing(5)
            .padding(15),
        )
        .width(Fill)
        .height(Length::Fixed(STROBE_PANEL_HEIGHT));
        return Some(panel.into());
    };
    let s = &data.strobe;

    // Readout regime (design D4). The band-slope is the *fine* read — the
    // strobe's own rotation rate, phase-integrated so it is ~100× steadier than
    // the instantaneous estimate — but it is only valid inside the band's
    // readable range; past that it aliases (measured: exact to the edge, garbage
    // beyond). The coarse spectral read (`coarse_cents`) covers everything else:
    // out of range, band gated, or fit window still filling. With neither,
    // "listening…" — never a stale number.
    //
    // `out_of_range` is decided in `app.rs`, where the hop cadence and the
    // debounce state live: the range test compares a noisy estimate against a
    // fixed boundary, so undebounced it flips source hop-to-hop while a string
    // sits at the edge.
    let (cents, coarse) = if !s.gated && !s.out_of_range && s.band_cents.is_some() {
        (s.band_cents, false)
    } else {
        (s.coarse_cents, true)
    };
    // The coarse read names its partial because it is not always the one in the
    // panel title: it follows its own fixed rule, so in the bass the band can be
    // on the 6th partial while the number came from the 4th. The cents value is
    // the same either way — a partial's deviation from its own target equals the
    // string's, exactly — but the reader should not have to assume that.
    let readout = match cents {
        Some(c) if coarse => format!("{c:+.1} ¢ (coarse · partial {})", s.coarse_n),
        Some(c) => format!("{c:+.1} ¢ vs target"),
        None => "listening…".to_string(),
    };
    let target = match (s.ref_hz, data.reference_mode) {
        (Some(r), ReferenceMode::Et) => format!("ET target {r:.2} Hz"),
        (Some(r), ReferenceMode::Curve) => format!("curve target {r:.2} Hz"),
        (None, _) => "no target".to_string(),
    };
    let title = if data.reference_mode == ReferenceMode::Et {
        format!("Strobe (ET) — {note_name} · fundamental")
    } else {
        format!("Strobe — {note_name} · partial {}", s.n_star)
    };

    let band = strobe_row(
        StrobeDisplay::new(s.beat_phase, s.gated).view(),
        Some(s.n_star),
    );

    // Curve-lock footer (design §8) — curve mode only; ET mode has no curve to
    // lock. Frozen targets are shown with their generation; when the live curve
    // has advanced (a recapture/undo/load), a Re-lock offer appears (R6). The
    // strobe never silently chases the moving curve, and never silently ignores
    // a newer one.
    let muted = iced::Color::from_rgb8(0xc3, 0xc2, 0xb7);
    let amber = iced::Color::from_rgb8(0xd9, 0x92, 0x26);
    // `strobe_lock_view` is already `None` in ET mode (it bypasses the curve),
    // so matching on it alone is enough.
    let lock_footer: Element<'static, Message> = match data.strobe_lock_view {
        Some(v) if v.newer => row![
            text(format!(
                "Locked · gen {} · newer curve available",
                v.generation
            ))
            .size(12)
            .color(amber),
            Space::new().width(10),
            button(text("Re-lock").size(12))
                .padding([3, 8])
                .on_press(Message::RequestRelock),
        ]
        .align_y(Alignment::Center)
        .into(),
        Some(v) => text(format!("Curve locked · gen {}", v.generation))
            .size(12)
            .color(muted)
            .into(),
        None => Space::new().into(),
    };

    // §5.6: a red ✗ with the reason when the curve doubts this key's
    // measurement, and its two remedies. Dropping is not offered here — it
    // means choosing between a key's repeats, which needs the inspector's list.
    let key = *key_index;
    let flagged: Element<'static, Message> = curve_bundle
        .and_then(|b| advisory::suspect(&b.curve(data.selected_engine).flags[key as usize]))
        .map_or_else(
            || Space::new().into(),
            |a| {
                column![
                    text(format!("✗ {}", a.reason)).size(12).color(SUSPECT),
                    row![
                        button(text("Re-measure").size(12))
                            .padding([3, 8])
                            .on_press(Message::RemeasureKey(key)),
                        Space::new().width(8),
                        button(text("Review measurements").size(12))
                            .padding([3, 8])
                            .on_press(Message::ReviewKey(key)),
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(5)
                .into()
            },
        );

    let panel = container(
        column![
            on_plot_span(text(title).size(18).into()),
            Space::new().height(10),
            band,
            Space::new().height(8),
            on_plot_span(text(readout).size(20).into()),
            on_plot_span(text(target).size(13).into()),
            Space::new().height(8),
            on_plot_span(flagged),
            on_plot_span(lock_footer),
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(STROBE_PANEL_HEIGHT));

    Some(panel.into())
}

/// Creates one of the two unison panels (ADR 0012): the selected note's
/// individual strings, resolved as separate spectral lines and drawn as markers
/// on a cents axis. [`UnisonPanel::Displayed`] magnifies the strobe band's own
/// partial; [`UnisonPanel::AllPartials`] stacks every reference beneath it.
///
/// Three things the pair must always carry, and each is measured rather than
/// stylistic:
///
/// - **the current resolution.** Until the DSP-side record is long enough two
///   separated strings resolve as one line, which reads as "clean" exactly when
///   a tuner is deciding they are done. "Clean to ±3 ¢" is honest; bare "clean"
///   is not. Each panel states the resolution of *its own* rows, so where the
///   two disagree the disagreement is on screen.
/// - **the pair beats, in Hz.** Positions are cents and rates are Hz, the
///   convention the rest of the readout uses, and the beat is what a tuner
///   counts by ear. Printed by the magnified panel, whose partial the beats are
///   computed for.
/// - **the discriminator's verdict**, visible rather than silently filtering.
///   A second line is not proof of a second string: one string beating with
///   itself looks identical, and in the bass it is measurably not a second
///   string (ADR 0013 §4). Printed by the stack, which is the evidence it is
///   drawn from — the test is precisely that the split is constant *across*
///   partials.
///
/// Gated on the strobe's own debounced `out_of_range` flag: past ±21.5 Hz the
/// baseband folds, so the lines would be real content at fictitious places.
fn create_unison_panel(
    data: &AppDisplayData,
    which: UnisonPanel,
) -> Option<Element<'static, Message>> {
    let magnified = which == UnisonPanel::Displayed;
    let visible = if magnified {
        data.unison_displayed_visible
    } else {
        data.unison_all_visible
    };
    if !data.unison_assist || !visible {
        return None;
    }
    let title_for = |note: &str| {
        if magnified {
            format!("Unison — {note} · partial {}", data.strobe.n_star)
        } else {
            format!("Unison — {note} · all partials")
        }
    };
    let TuningMode::Manual { note_name, .. } = &data.tuning_mode else {
        // Row slots are fixed, so the empty grid is the panel's own
        // nothing-to-show state. No key is nominated, so no row has a target.
        let display = unison_canvas(
            empty_rows(which),
            UNISON_SPAN_LADDER[UNISON_SPAN_LADDER.len() - 1],
            magnified,
        );
        return Some(
            container(
                column![
                    on_plot_span(text(title_for("—")).size(18).into()),
                    Space::new().height(8),
                    container(display.view())
                        .width(Fill)
                        .height(Length::Fixed(unison_body_height(which))),
                    Space::new().height(6),
                    on_plot_span(
                        container(
                            text(auto_mode_note(data.instrument, "unison display"))
                                .size(12)
                                .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7)),
                        )
                        .width(Fill)
                        .height(Length::Fixed(UNISON_FOOTER_HEIGHT))
                        .into()
                    ),
                ]
                .width(Fill)
                .spacing(4)
                .padding(PANEL_PADDING),
            )
            .width(Fill)
            .height(Length::Fixed(unison_panel_height(which)))
            .into(),
        );
    };

    let u = &data.unison;
    let rows: Vec<_> = match (magnified, u.displayed) {
        (true, Some(i)) => vec![u.rows[i]],
        (true, None) => Vec::new(),
        (false, _) => u.rows.clone(),
    };
    let body_height = unison_body_height(which);

    // The resolution the reading is worth, from the rows on screen.
    let resolution = rows
        .iter()
        .map(|r| r.resolution_cents)
        .fold(f32::NAN, f32::max);
    let muted = iced::Color::from_rgb8(0xc3, 0xc2, 0xb7);
    let amber = iced::Color::from_rgb8(0xd9, 0x92, 0x26);

    // Why the panel is showing no markers, when it is showing none. The axis and
    // its row slots stay drawn in every state; only the markers are withheld.
    let (drawn, blocked) = if data.strobe.out_of_range {
        // The band's own verdict, reused: beyond it the lines alias, so the
        // slots are drawn and the markers withheld.
        (
            rows.iter()
                .map(|r| unison_display::UnisonRow { count: 0, ..*r })
                .collect(),
            Some("Out of range — bring the string inside ±21.5 Hz of target first."),
        )
    } else if rows.is_empty() {
        (
            empty_rows(which),
            Some("Listening… strike the note and let it ring."),
        )
    } else if rows.iter().all(|r| r.count == 0) {
        // Targeted, but nothing resolved on any of them — a decayed note, not a
        // clean one.
        (
            rows.clone(),
            Some("Listening… strike the note and let it ring."),
        )
    } else {
        (rows.clone(), None)
    };

    let body: Element<'static, Message> =
        container(unison_canvas(drawn, data.unison.span_cents, magnified).view())
            .width(Fill)
            .height(Length::Fixed(body_height))
            .into();

    // How many strings, how far apart, and how fast they beat. The limit is
    // stated as a beat rate because that is the quantity a tuner hears, and
    // because `2/T` in Hz *is* the slowest beat the record can show.
    let strings = rows.iter().map(|r| r.count).max().unwrap_or(0);
    let slowest_visible_beat = rows
        .iter()
        .map(|r| r.resolution_hz)
        .fold(f32::NAN, f32::max);
    let beat_limit = |hz: f32| {
        if hz.is_finite() && hz > 0.0 {
            format!("no beat above {hz:.1} Hz")
        } else {
            "nothing resolved yet".to_string()
        }
    };
    let readout = match (magnified, strings, u.beats_hz.first()) {
        (_, 0, _) => "—".to_string(),
        (true, 1, _) => format!("one line · {}", beat_limit(slowest_visible_beat)),
        (true, n, Some(beat)) => format!("{n} lines · beat {beat:.2} Hz"),
        (true, n, None) => format!("{n} lines"),
        (false, _, _) => {
            let resolved = rows.iter().filter(|r| r.count > 1).count();
            format!("{resolved} of {} partials split", rows.len())
        }
    };
    let readout = blocked.map_or(readout, str::to_string);
    let resolution_note = if resolution.is_finite() && resolution > 0.0 {
        format!("resolved to ±{resolution:.1} ¢")
    } else {
        "resolution unknown".to_string()
    };
    // Past the finest step the readouts display (`UNISON_SPAN_LADDER[0]`, "a
    // unison being finished"), the panel cannot contribute at the scale the
    // rest of the app works at — so the figure is flagged rather than left to
    // be read as if it were fine.
    let resolution_color = if resolution.is_finite() && resolution > UNISON_SPAN_LADDER[0] {
        amber
    } else {
        muted
    };
    // The handoff: one line means either a clean unison or a beat too slow to
    // see, and the panel cannot tell them apart. The strobe can — on one
    // sounding string it reads far finer than this ever will.
    let handoff = (magnified && blocked.is_none() && strings <= 1).then_some(
        "Slower beats are beyond this display — listen for them, or mute two \
         strings and tune each one on the strobe.",
    );

    // Shown, never used to filter: a second line the discriminator cannot
    // attribute is still a line the tuner should see. `Undetermined` states what
    // is known of it — a second line is not a second string (ADR 0013 §4).
    let lines_here = rows.iter().any(|r| r.count >= 2);
    let verdict: Option<(&str, iced::Color)> =
        (!magnified && blocked.is_none()).then_some(match u.verdict {
            UnisonVerdict::Unison if lines_here => ("✓ consistent with a unison", muted),
            UnisonVerdict::FalseBeat if lines_here => ("✗ false beat — one string, not two", amber),
            _ if lines_here => (
                "undetermined — a second line is not proof of a second string; one \
             string can split this way",
                muted,
            ),
            _ => ("verdict undetermined — too few partials resolved", muted),
        });

    // The readout takes a share of the row, not its natural width, so a long
    // message wraps inside it rather than over the figure beside it. While
    // blocked there is no figure: the resolution of a reading the panel is not
    // showing is not a fact about anything on screen.
    let mut footer = column![
        row![
            text(readout).size(15).width(Fill),
            Space::new().width(8),
            match blocked {
                Some(_) => Element::from(Space::new()),
                None => text(resolution_note)
                    .size(12)
                    .color(resolution_color)
                    .into(),
            },
        ]
        .align_y(Alignment::Center),
    ]
    .width(Fill)
    .spacing(4);

    if let Some((verdict_text, verdict_color)) = verdict {
        footer = footer.push(text(verdict_text).size(12).color(verdict_color));
    }
    if let Some(t) = handoff {
        footer = footer.push(text(t).size(11).color(muted));
    }

    let panel_content = column![
        on_plot_span(text(title_for(note_name)).size(18).into()),
        Space::new().height(8),
        body,
        Space::new().height(6),
        // Fixed, for the same reason the row slots are: a line appearing must
        // not move the axis above it, and a panel that resizes re-lays out the
        // column it sits in.
        on_plot_span(
            container(footer)
                .width(Fill)
                .height(Length::Fixed(UNISON_FOOTER_HEIGHT))
                .into()
        ),
    ]
    .width(Fill)
    .spacing(4)
    .padding(PANEL_PADDING);

    Some(
        container(panel_content)
            .width(Fill)
            .height(Length::Fixed(unison_panel_height(which)))
            .into(),
    )
}

/// Padding inside a panel, between its border and its content.
const PANEL_PADDING: f32 = 15.0;

/// Height reserved for a unison panel's text, below the plot.
///
/// Sized for the longest footer either panel produces: a readout that wraps to
/// two lines (the out-of-range message is the longest) plus a verdict or a
/// two-line handoff. Reserved rather than grown, so a line appearing moves
/// nothing.
const UNISON_FOOTER_HEIGHT: f32 = 62.0;

/// Overall height of a unison panel — its padding, title row, the plot, and the
/// text slot beneath it, none of which depend on what is currently resolved.
fn unison_panel_height(which: UnisonPanel) -> f32 {
    /// Title row at 18 px, the 8 px and 6 px gaps around the plot, and the
    /// column's own spacing between the four items.
    const TITLE_AND_GAPS: f32 = 56.0;
    2.0 * PANEL_PADDING + TITLE_AND_GAPS + unison_body_height(which) + UNISON_FOOTER_HEIGHT
}

/// One empty row slot per reference the panel can draw — what the axis looks
/// like with nothing resolved on it.
fn empty_rows(which: UnisonPanel) -> Vec<unison_display::UnisonRow> {
    let slots = match which {
        UnisonPanel::Displayed => 1,
        UnisonPanel::AllPartials => MAX_STROBE_REFS,
    };
    (0..slots)
        .map(|i| unison_display::UnisonRow {
            partial: i as u8 + 1,
            ..unison_display::UnisonRow::default()
        })
        .collect()
}

/// The unison canvas at the size this panel draws it.
fn unison_canvas(
    rows: Vec<unison_display::UnisonRow>,
    span_cents: f32,
    magnified: bool,
) -> UnisonDisplay {
    let display = UnisonDisplay::new(rows, span_cents);
    if magnified {
        display.row_height(UNISON_MAGNIFIED_ROW)
    } else {
        display
    }
}

/// Height of a unison canvas, in pixels — one fixed row slot per reference the
/// bank can target, so the panel never resizes as partials come and go. Rows
/// past the key's own reference count are simply not drawn.
fn unison_body_height(which: UnisonPanel) -> f32 {
    match which {
        UnisonPanel::Displayed => UNISON_MAGNIFIED_ROW + unison_display::ROW_CHROME,
        UnisonPanel::AllPartials => {
            MAX_STROBE_REFS as f32 * unison_display::ROW_HEIGHT + unison_display::ROW_CHROME
        }
    }
}

/// Creates the live tuning-curve plot panel (strobe design §10): the selected
/// engine's d(m) from the freshest bundle, updating as captures land. Shows
/// the prior-only curve at launch and a computing note until the first bundle
/// arrives.
fn create_curve_plot_panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Option<Element<'static, Message>> {
    if !data.curve_plot_visible {
        return None;
    }

    let engine = data.selected_engine;
    // Clicking the plot selects that key, exactly as clicking the keyboard
    // does — the plot is the surface that shows *which* keys want attention.
    let selected_key = match &data.tuning_mode {
        TuningMode::Manual { key_index, .. } => Some(*key_index),
        TuningMode::Auto => None,
    };
    let (title_text, content): (String, Element<'static, Message>) = match curve_bundle {
        Some(bundle) => {
            let curve = bundle.curve(engine);
            let (cents, measured, suspect) = curve_select::plot_inputs(curve);
            let measured_count = measured.iter().filter(|&&m| m).count();
            let flagged = suspect.iter().filter(|&&s| s).count();
            let flagged_note = if flagged > 0 {
                format!(" · {flagged} flagged")
            } else {
                String::new()
            };
            (
                format!(
                    "Tuning Curve — {} · {measured_count}/88 measured{flagged_note}",
                    engine.label()
                ),
                container(
                    CurvePlot::new(cents, measured, suspect, PlotMode::Full, None)
                        .selected(selected_key)
                        .on_select(Message::KeySelected)
                        .view(),
                )
                .width(Fill)
                .height(Fill)
                .into(),
            )
        }
        // The grid draws with no series on it while the first bundle computes.
        // Non-finite cents draw nothing, so no key reads as measured at 0 ¢.
        None => (
            "Tuning Curve — computing…".to_string(),
            container(
                CurvePlot::new(
                    [f32::NAN; 88],
                    [false; 88],
                    [false; 88],
                    PlotMode::Full,
                    None,
                )
                .selected(selected_key)
                .on_select(Message::KeySelected)
                .view(),
            )
            .width(Fill)
            .height(Fill)
            .into(),
        ),
    };

    let panel = container(
        column![text(title_text).size(18), Space::new().height(10), content]
            .width(Fill)
            .spacing(5)
            .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(240.0));

    Some(panel.into())
}

/// Creates the spectrogram panel widget.
fn create_spectrogram_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.spectrogram_visible {
        return None;
    }

    let spectrogram_data: Vec<f32> = data
        .last_frame
        .as_ref()
        .map(|f| f.magnitudes[..f.magnitude_len].to_vec())
        .unwrap_or_default();

    let spectrogram_content: Element<'static, Message> =
        container(spectrogram::Spectrogram::new(spectrogram_data).view())
            .width(Fill)
            .height(Fill)
            .into();

    let panel = container(
        column![
            text("Spectrogram").size(18),
            Space::new().height(10),
            spectrogram_content
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(250.0));

    Some(panel.into())
}

/// Creates the cent meter — the engine's own readout: detected note, its
/// frequency, whether it is still tracking, and the deviation as a needle.
///
/// It is the only deviation readout **Auto mode** has, the strobe needing a
/// nominated key. That is also its sunset condition: when Auto mode can strobe,
/// the panel goes (`layout-by-task-design.md` D4).
fn create_cent_meter_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.cent_meter_visible {
        return None;
    }

    // Calculate smoothed cent deviation
    let smoothed_cents = if data.smoothing_buffer.is_empty() {
        data.last_cents
    } else {
        let sum: f32 = data.smoothing_buffer.iter().sum();
        let count = data.smoothing_buffer.len() as f32;
        if count > 0.0 { Some(sum / count) } else { None }
    };

    let note_name = match &data.tuning_mode {
        TuningMode::Auto => data
            .last_note_index
            .map(|idx| models::find_nearest_note_by_index(idx).0)
            .unwrap_or_else(|| "--".to_string()),
        TuningMode::Manual { note_name, .. } => note_name.clone(),
    };
    let freq_text = data
        .last_frequency
        .map_or_else(|| "--".to_string(), |f| format!("{f:.2} Hz"));
    let status_text = if data.last_note_index.is_some() && !data.is_stale {
        if data.last_frequency.is_some() {
            "Tracking".to_string()
        } else {
            "Dropped".to_string()
        }
    } else {
        "--".to_string()
    };

    let cent_meter_content: Element<'static, Message> = container(
        cent_meter::CentMeterDisplay::new(
            smoothed_cents,
            note_name,
            freq_text,
            status_text,
            data.is_stale,
        )
        .view(),
    )
    .width(Fill)
    .height(Fill)
    .into();

    let panel = container(
        column![
            text("Cent Meter").size(18),
            Space::new().height(10),
            cent_meter_content
        ]
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(250.0));

    Some(panel.into())
}

/// Creates the note-select panel — the 88-key piano keyboard, or the six-button
/// guitar-string picker when the instrument toggle is set to Guitar (debug).
/// Both surfaces publish the same `KeySelected(key_index)`; only the picker
/// differs.
fn create_keyboard_panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Option<Element<'static, Message>> {
    if !data.key_select_visible {
        return None;
    }

    // Detected key index — directly from NoteEvent (no String→index lookup)
    let detected_key_index = data.last_note_index;

    let selected_key_index = match &data.tuning_mode {
        TuningMode::Manual { key_index, .. } => Some(*key_index),
        TuningMode::Auto => detected_key_index,
    };

    // Suspect marks come from the engine on display, so the ✗ on a key and the
    // ✗ on the plot are always the same verdict.
    let suspect = curve_bundle.map_or([false; 88], |b| {
        advisory::suspect_keys(&b.curve(data.selected_engine).flags)
    });

    let (title, select_content): (&str, Element<'static, Message>) = match data.instrument {
        Instrument::Piano => (
            "Keyboard Key Select",
            piano_keyboard::PianoKeyboard::new(detected_key_index, selected_key_index, suspect)
                .view(),
        ),
        Instrument::Guitar => (
            "Guitar String Select",
            guitar_strings::view(detected_key_index, selected_key_index),
        ),
    };

    let panel = container(
        column![
            text(title).size(18),
            Space::new().height(10),
            select_content
        ]
        .width(Fill)
        .height(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(200.0));

    Some(panel.into())
}

// /// Creates the inharmonicity graph panel
// fn create_inharmonicity_graph_panel(
//     data: &crate::AppDisplayData,
//     // --- MODIFIED: Accept profile as a reference ---
//     profile: &tuner_core::rigaud::InharmonicityProfile,
// ) -> Option<Element<'static, Message>> {
//     if !data.inharmonicity_graph_visible {
//         return None;
//     }
//
//     // --- MODIFIED: Use the passed profile reference ---
//     let graph_content = inharmonicity_graph::InharmonicityGraph::new(profile).view();
//
//     let panel = container(
//         column![
//             text("Inharmonicity 'B' Values").size(18),
//             Space::new().height(10),
//             graph_content
//         ]
//         .spacing(5)
//         .padding(15),
//     )
//     .width(Length::Fill)
//     .height(Length::Fixed(250.0)); // Graph panel is a bit taller
//
//     Some(panel.into())
// }

/// One string chip: lit when set, inert when the key has no such string.
fn string_chip(
    label: u8,
    lit: bool,
    lit_color: iced::Color,
    on_press: Option<Message>,
) -> Element<'static, Message> {
    let enabled = on_press.is_some();
    let mut chip = button(text(label.to_string()).size(13))
        .padding([4, 12])
        .style(move |_theme, _status| button::Style {
            background: Some(iced::Background::Color(if lit {
                lit_color
            } else {
                iced::Color::from_rgb(0.22, 0.22, 0.21)
            })),
            text_color: if enabled {
                iced::Color::WHITE
            } else {
                iced::Color::from_rgb(0.45, 0.45, 0.45)
            },
            border: iced::Border {
                radius: 4.0.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        });
    if let Some(message) = on_press {
        chip = chip.on_press(message);
    }
    chip.into()
}

/// The string declaration the next capture will carry: how many strings the
/// key is strung with (`Total`), and which of them are unmuted (`Sounding`).
///
/// Shown only when string isolation is switched on in Settings. The
/// declaration is stamped onto the capture as it dispatches, so it applies to
/// the *next* capture and must be set before arming.
fn strings_section(strings: models::SoundingStrings, touched: bool) -> Element<'static, Message> {
    let on_key_row = (1..=models::MAX_STRINGS_PER_KEY as u8).fold(
        row![text("Total").size(13).width(Length::Fixed(70.0))].spacing(4),
        |r, n| {
            r.push(string_chip(
                n,
                // An untouched control declares nothing, so it highlights
                // nothing — the opening 3 is a starting point, not a count the
                // operator chose.
                (touched || strings.sounding_count() > 0) && strings.on_key == n,
                iced::Color::from_rgb(0.25, 0.28, 0.36),
                Some(Message::SetSoundingStrings(strings.with_on_key(n))),
            ))
        },
    );

    // A single-strung key's declaration is its count, so the whole row is
    // inert there rather than offering a choice that does not exist.
    let sounding_row = (0..models::MAX_STRINGS_PER_KEY).fold(
        row![text("Sounding").size(13).width(Length::Fixed(70.0))].spacing(4),
        |r, i| {
            let selectable = strings.on_key > 1 && i < strings.on_key as usize;
            r.push(string_chip(
                i as u8 + 1,
                strings.sounding[i],
                iced::Color::from_rgb(0.24, 0.47, 0.30),
                selectable.then(|| Message::SetSoundingStrings(strings.toggled(i))),
            ))
        },
    );

    column![
        text("Strings").size(14),
        text("on this key, then which sound")
            .size(11)
            .color(INK_SECONDARY),
        Space::new().height(6),
        on_key_row.align_y(Alignment::Center),
        sounding_row.align_y(Alignment::Center),
        // Warned rather than merely stated: the count alone declares nothing,
        // so a capture armed in this state records no string state at all.
        text(strings.to_string())
            .size(11)
            .color(if strings.sounding_count() == 0 {
                SUSPECT
            } else {
                INK_SECONDARY
            }),
    ]
    .spacing(5)
    .into()
}

/// What the sidebar reports about a measurement session in progress.
///
/// Grouped because they share one condition — measurement mode — and one place
/// on screen, under the capture button.
struct SessionStatus {
    /// The string declaration and whether the operator has touched it; `None`
    /// while String Isolation is off.
    strings: Option<(models::SoundingStrings, bool)>,
    /// An extended take's (target, elapsed) seconds; `None` at the shipped length.
    extended_capture: Option<(f32, f32)>,
    /// The curve is recomputing, so captures queue behind it.
    curve_recomputing: bool,
}

/// Creates the settings sidebar widget.
///
/// Builds the right-side settings panel containing all application controls
/// organized into logical sections (Tools, Systemic change, Program). The sidebar
/// includes tool visibility toggles, measurement mode controls, and profile
/// management buttons. When in measurement mode, it also displays a large
/// capture button for recording partial measurements.
///
/// # Arguments
/// * `capture_state` - Current capture state (Off, Armed, Done)
/// * `capture_message` - Message to send when capture button is pressed
///
/// # Returns
/// * `Element` - Complete sidebar widget with all controls and sections
fn create_sidebar(
    measurement_mode_active: bool,
    capture_state: CaptureState,
    undo_target_note: Option<String>,
    capture_message: Message,
    reference_mode: ReferenceMode,
    unison_assist: bool,
    session: SessionStatus,
) -> Element<'static, Message> {
    let SessionStatus {
        strings,
        extended_capture,
        curve_recomputing,
    } = session;
    let mut sections = column![].spacing(10);

    // Add Settings button at the top
    let settings_button = button(text("Settings").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    //0.38, 0.294, 0.502,
                    0.427, 0.298, 0.612,
                ))), // purple
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in MAIN_SIDEBAR_CONFIG {
        let mut entries: Vec<&ButtonConfig> = buttons.iter().collect();
        if title == "Tools" && unison_assist {
            entries.extend(UNISON_TOOLS_CONFIG.iter());
        }
        sections = sections.push(make_sidebar_section(
            title,
            entries,
            measurement_mode_active,
        ));
    }

    // Reference: what every readout is measured against — the strobe band, its
    // cents readout, and the cent meter alike. Its own section because the
    // reference *pitch* and the temperament belong beside the mode when they
    // are built (TODO.md), and because it is not a tool that can be shown or
    // hidden like the entries above.
    sections = sections.push(
        column![
            text("Reference").size(18),
            Space::new().height(10),
            button(text(reference_mode.label()).size(14).width(Fill))
                .padding([6, 10])
                .on_press(Message::SetReferenceMode(reference_mode.toggled())),
        ]
        .spacing(5),
    );

    // The string declaration, the capture control, and what the session is
    // doing: one heading, under the one condition that governs them all.
    if measurement_mode_active {
        sections = sections.push(text("Measurement session").size(18));
        if let Some((strings, touched)) = strings {
            sections = sections.push(strings_section(strings, touched));
        }
        let recording = capture_state == CaptureState::Recording;
        // Only an extended take offers Stop: at 1.5 s the button would be a
        // way to cancel the capture you just armed, by double-clicking it.
        let abortable = recording && extended_capture.is_some();
        sections = sections.push(make_capture_button(
            capture_state,
            if abortable {
                Message::AbortCapture
            } else {
                capture_message
            },
            abortable,
        ));
        // An extended record runs past the decay, so the button's "Capturing…"
        // alone cannot be told from a hang: say how far along it is.
        if curve_recomputing {
            sections = sections.push(
                text("Recomputing curve — captures queue behind it")
                    .size(11)
                    .color(iced::Color::from_rgb(0.55, 0.55, 0.62)),
            );
        }
        if let Some((target_secs, progress_secs)) = extended_capture {
            let label = if recording {
                format!("Extended · {progress_secs:.1} / {target_secs:.1} s")
            } else {
                format!("Extended · {target_secs:.1} s")
            };
            sections = sections.push(
                text(label)
                    .size(12)
                    .color(iced::Color::from_rgb(0.8, 0.6, 0.2)),
            );
        }
    }

    if let Some(note_name) = undo_target_note {
        sections = sections.push(Space::new().height(20));
        sections = sections.push(make_undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}

// End of file
