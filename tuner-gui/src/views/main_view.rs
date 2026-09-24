//! # Main screen
//!
//! The tuning screen: the sidebar, and the panels in two columns, one for what
//! is being tuned and one for the live loop.

use crate::Message;
use crate::advisory;
use crate::app::strobe::{BAND_READABLE_HZ, UNISON_SPAN_LADDER};
use crate::app::{AppDisplayData, Instrument, TuningMode};
use crate::calibration::CALIBRATION_FRAMES;
use crate::views::curve_select;
use crate::views::sidebar::{self, ButtonConfig, ButtonType};
use crate::widgets::curve_plot::{CurvePlot, INK_SECONDARY, PlotMode, SUSPECT};
use crate::widgets::strobe_display::StrobeDisplay;
use crate::widgets::unison_display::{self, UnisonDisplay};
use crate::widgets::{cent_meter, guitar_strings, piano_keyboard, spectrum_plot};
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::models::{self, ReferenceMode};
use tuner_core::pipeline::CaptureState;
use tuner_core::strobe::MAX_STROBE_REFS;
use tuner_core::strobe::unison::UnisonVerdict;
use tuner_core::worker::CurveBundle;

const TOOLS_CONFIG: [ButtonConfig; 6] = [
    ButtonConfig {
        label: "Spectrum",
        message: Some(Message::ToggleSpectrum),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Cent Meter",
        message: Some(Message::ToggleCentMeter),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Key select",
        message: Some(Message::ToggleKeySelect),
        button_type: ButtonType::Standard,
    },
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
    ButtonConfig {
        label: "Measurement Mode",
        message: Some(Message::ToggleMeasurementMode),
        button_type: ButtonType::MeasurementMode,
    },
];

const PROGRAM_CONFIG: [ButtonConfig; 1] = [ButtonConfig {
    label: "Save Profile",
    message: Some(Message::SaveProfile),
    button_type: ButtonType::Standard,
}];

/// Width of the live-loop column, fixed so a marker's position means the same
/// at any window width.
const LIVE_COLUMN_WIDTH: f32 = 360.0;

/// Which unison panel: the displayed partial magnified, or every partial stacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UnisonPanel {
    Displayed,
    AllPartials,
}

/// Side of the square strobe band.
const STROBE_BAND: f32 = 150.0;

/// Height of the strobe panel: title, band, the two readout lines beneath it,
/// and room for the flag advisory and the curve-lock footer.
const STROBE_PANEL_HEIGHT: f32 = 300.0;

/// Row height of the magnified panel. Magnifying separates markers but adds no
/// resolution: both panels share one cents axis.
const UNISON_MAGNIFIED_ROW: f32 = 48.0;

/// Tools entries that exist only while unison assist is on.
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

const MAIN_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 2] = [
    ("Tools", TOOLS_CONFIG.as_slice()),
    ("Program", PROGRAM_CONFIG.as_slice()),
];

pub fn view(
    data: &AppDisplayData,
    capture_message: Message,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    // Calibration advances only on audio frames, so without an audio host it
    // would otherwise read "Calibrating…" forever.
    let rms = &data.settings_data.rms;
    if !data.audio_worker_active || rms.is_calibrating() {
        let message = if data.audio_worker_active {
            format!(
                "Calibrating… {}/{}",
                rms.calibration_progress().unwrap_or(0),
                CALIBRATION_FRAMES
            )
        } else {
            "No audio input".to_string()
        };
        return container(text(message).size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let widget_area = create_widget_area(data, curve_bundle);

    let sidebar = create_sidebar(
        data.measurement_mode_active,
        data.capture_state,
        data.undo_target_note.clone(),
        capture_message,
        data.reference_mode,
        data.unison_assist,
        SessionStatus {
            // Manual only: the declaration names strings of the key the operator
            // picked.
            strings: (data.string_isolation
                && matches!(data.tuning_mode, TuningMode::Manual { .. }))
            .then_some((data.sounding_strings, data.strings_touched)),
            extended_capture: data
                .extended_capture
                .then_some((data.extended_capture_secs, data.capture_progress_secs)),
            curve_recomputing: data.curve_recomputing,
        },
    );

    // Height is filled, not shrunk: the live-loop column scrolls, and a
    // scrollable in a shrink-height parent has no bound to scroll within.
    let main_content = row![sidebar, Space::new().width(10), widget_area,]
        .align_y(Alignment::Start)
        .height(Fill)
        .padding(20);

    let base = container(main_content).width(Fill).height(Fill);

    // Re-lock shifts every strobe target, so it asks first.
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

/// The two panel columns: context on the left (what is being tuned, and the
/// plan), and the live loop on the right (the strobe and the unison panels). A
/// hidden panel takes no space, and an empty column gives its width to the other.
pub fn create_widget_area(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    // The open instrument is always named: with resume-at-launch and autosave,
    // capturing a second instrument would otherwise fold into the previous
    // one's file unannounced.
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

    let mut context = column![].width(Fill).spacing(10);
    let mut context_panels = 0;
    // The spectrum and the cent meter share a row: both show what the engine hears.
    let top: Vec<_> = [create_spectrum_panel(data), create_cent_meter_panel(data)]
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
        // The strobe stays above the scroll: it is read without looking away
        // from the string, so it must not move.
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

    // Pushed last, not wrapped: it must not move the panels above it, and an
    // empty placeholder would still take a row of spacing.
    let mut content = column![title, Space::new().height(20), columns]
        .width(Fill)
        .height(Fill)
        .spacing(10);

    if let Some(notice) = create_auto_mode_notice(data) {
        content = content.push(notice);
    }

    content.into()
}

/// The Auto-mode notice. Nothing else shows that an Auto capture is untrusted:
/// it succeeds and lands in the profile, but the curve does not move.
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

/// Puts an element on the plots' horizontal span, past the label gutter and
/// inside the right margin, so a panel's text lines up with its plot.
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

/// The strobe band on the unison plots' horizontal span, centred where they draw
/// the target line, so the two stay aligned at any width.
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

/// What a live-loop panel says in Auto mode: it reads a nominated key's
/// targets, which Auto has not got.
fn auto_mode_note(instrument: Instrument, panel: &str) -> String {
    let select = match instrument {
        Instrument::Piano => "a key on the Keyboard Key Select panel",
        Instrument::Guitar => "a string on the Guitar String Select panel",
    };
    format!("Off in Auto mode — the {panel} needs a nominated key. Click {select}.")
}

/// The strobe panel. In Auto mode it says how to pick a key rather than hiding,
/// since there is no target without one.
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
        // Frozen, as the band shows a partial it cannot read.
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

    // The band's fine read while it is valid, else the coarse read, and with
    // neither "listening…" rather than a stale number.
    let (cents, coarse) = if !s.gated && !s.out_of_range && s.band_cents.is_some() {
        (s.band_cents, false)
    } else {
        (s.coarse_cents, true)
    };
    // The coarse read names its partial, which can differ from the title's. The
    // cents agree either way, but the reader should not have to assume so.
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

    // The strobe never silently chases a moving curve, nor silently ignores a
    // newer one.
    let muted = iced::Color::from_rgb8(0xc3, 0xc2, 0xb7);
    let amber = iced::Color::from_rgb8(0xd9, 0x92, 0x26);
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

    // Dropping is not offered here: it means choosing among a key's repeats,
    // which needs the inspector's list.
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

/// One of the two unison panels: the note's strings as markers on a cents axis.
///
/// Each panel carries what keeps its reading honest:
/// - its resolution, since strings closer than it resolve as one line, which
///   reads as clean exactly when a tuner decides they are done;
/// - the pair beats in Hz (the magnified panel), the rate a tuner counts by ear;
/// - the discriminator's verdict (the stack), shown rather than filtering: a
///   second line is not proof of a second string.
///
/// Markers are withheld while the band is out of range, where the lines alias.
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
        // The empty grid is the panel's nothing-to-show state.
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

    let resolution = rows
        .iter()
        .map(|r| r.resolution_cents)
        .fold(f32::NAN, f32::max);
    let muted = iced::Color::from_rgb8(0xc3, 0xc2, 0xb7);
    let amber = iced::Color::from_rgb8(0xd9, 0x92, 0x26);
    let out_of_range = format!(
        "Out of range — bring the string inside ±{BAND_READABLE_HZ:.0} Hz of target first."
    );

    // The axis and its row slots are drawn in every state; only the markers are
    // withheld, with the reason.
    let (drawn, blocked) = if data.strobe.out_of_range {
        (
            rows.iter()
                .map(|r| unison_display::UnisonRow { count: 0, ..*r })
                .collect(),
            Some(out_of_range.as_str()),
        )
    } else if rows.is_empty() {
        (
            empty_rows(which),
            Some("Listening… strike the note and let it ring."),
        )
    } else if rows.iter().all(|r| r.count == 0) {
        // Nothing resolved on any row: a decayed note, not a clean one.
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

    // The limit is a beat rate: `2/T` is the slowest beat the record can show,
    // and a beat is what a tuner hears.
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
    // Coarser than the finest axis step, the panel cannot help finish a unison,
    // so the figure is flagged.
    let resolution_color = if resolution.is_finite() && resolution > UNISON_SPAN_LADDER[0] {
        amber
    } else {
        muted
    };
    // One line is a clean unison or a beat too slow to see. The strobe can tell
    // them apart, one sounding string at a time.
    let handoff = (magnified && blocked.is_none() && strings <= 1).then_some(
        "Slower beats are beyond this display — listen for them, or mute two \
         strings and tune each one on the strobe.",
    );

    // Shown, never used to filter: a line the discriminator cannot attribute is
    // still one the tuner should see.
    // report 0013
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

    // The readout takes a share of the row, so a long message wraps beside the
    // figure. While blocked there is no figure: nothing on screen has that
    // resolution.
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

/// Height reserved for a unison panel's text: a two-line readout, plus a verdict
/// or a two-line handoff. Reserved, so a line appearing moves nothing.
const UNISON_FOOTER_HEIGHT: f32 = 62.0;

/// A unison panel's height, which does not depend on what is resolved.
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

/// A unison canvas's height: one row slot per reference the bank can target, so
/// the panel never resizes.
fn unison_body_height(which: UnisonPanel) -> f32 {
    match which {
        UnisonPanel::Displayed => UNISON_MAGNIFIED_ROW + unison_display::ROW_CHROME,
        UnisonPanel::AllPartials => {
            MAX_STROBE_REFS as f32 * unison_display::ROW_HEIGHT + unison_display::ROW_CHROME
        }
    }
}

/// The live tuning-curve plot of the selected engine.
fn create_curve_plot_panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Option<Element<'static, Message>> {
    if !data.curve_plot_visible {
        return None;
    }

    let engine = data.selected_engine;
    // The plot shows which keys want attention, so clicking it selects a key.
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
                    curve_select::engine_label(engine)
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
        // The empty grid, while the first bundle computes. Non-finite cents draw
        // nothing, so no key reads as measured at 0 ¢.
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

fn create_spectrum_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.spectrum_visible {
        return None;
    }

    let spectrum_data: Vec<f32> = data
        .last_frame
        .as_ref()
        .map(|f| f.magnitudes[..f.magnitude_len].to_vec())
        .unwrap_or_default();

    let spectrum_content: Element<'static, Message> =
        container(spectrum_plot::SpectrumPlot::new(spectrum_data).view())
            .width(Fill)
            .height(Fill)
            .into();

    let panel = container(
        column![
            text("Spectrum").size(18),
            Space::new().height(10),
            spectrum_content
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(250.0));

    Some(panel.into())
}

/// The cent meter: the engine's detected note, its frequency and its deviation.
/// It is Auto mode's only deviation readout, since the strobe needs a nominated
/// key.
fn create_cent_meter_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.cent_meter_visible {
        return None;
    }

    let smoothed_cents = if data.smoothing_buffer.is_empty() {
        data.last_cents
    } else {
        let sum: f32 = data.smoothing_buffer.iter().sum();
        Some(sum / data.smoothing_buffer.len() as f32)
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

/// The note picker: the piano keyboard, or the guitar strings.
fn create_keyboard_panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Option<Element<'static, Message>> {
    if !data.key_select_visible {
        return None;
    }

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

/// The string declaration the next capture carries: how many strings the key
/// has (`Total`), and which of them are unmuted (`Sounding`).
fn strings_section(strings: models::SoundingStrings, touched: bool) -> Element<'static, Message> {
    let on_key_row = (1..=models::MAX_STRINGS_PER_KEY as u8).fold(
        row![text("Total").size(13).width(Length::Fixed(70.0))].spacing(4),
        |r, n| {
            r.push(string_chip(
                n,
                // An untouched control declares nothing, so it highlights
                // nothing: the opening 3 is not a count the operator chose.
                (touched || strings.sounding_count() > 0) && strings.on_key == n,
                iced::Color::from_rgb(0.25, 0.28, 0.36),
                Some(Message::SetSoundingStrings(strings.with_on_key(n))),
            ))
        },
    );

    // A single-strung key's declaration is its count, so the row is inert.
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
        // Warned: the count alone declares nothing, so a capture armed now
        // records no string state.
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

/// What the sidebar reports under the capture button while measuring.
struct SessionStatus {
    /// The string declaration and whether the operator has touched it; `None`
    /// while String Isolation is off.
    strings: Option<(models::SoundingStrings, bool)>,
    /// An extended take's (target, elapsed) seconds; `None` at the shipped length.
    extended_capture: Option<(f32, f32)>,
    /// The curve is recomputing, so captures queue behind it.
    curve_recomputing: bool,
}

/// The reference toggle's text, naming the mode in force.
fn reference_label(mode: ReferenceMode) -> &'static str {
    match mode {
        ReferenceMode::Curve => "Ref: Curve",
        ReferenceMode::Et => "Ref: ET",
    }
}

/// The sidebar: Settings, the tool sections, the reference mode and, while
/// measuring, the capture controls.
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

    let settings_button = button(text("Settings").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| button::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.427, 0.298, 0.612,
            ))),
            text_color: iced::Color::WHITE,
            ..button::Style::default()
        })
        .on_press(Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    for (title, buttons) in MAIN_SIDEBAR_CONFIG {
        let mut entries: Vec<&ButtonConfig> = buttons.iter().collect();
        if title == "Tools" && unison_assist {
            entries.extend(UNISON_TOOLS_CONFIG.iter());
        }
        sections = sections.push(sidebar::section(title, entries, measurement_mode_active));
    }

    // Its own section: it is not a panel to show or hide, and the reference
    // pitch and temperament will join it.
    sections = sections.push(
        column![
            text("Reference").size(18),
            Space::new().height(10),
            button(text(reference_label(reference_mode)).size(14).width(Fill))
                .padding([6, 10])
                .on_press(Message::SetReferenceMode(reference_mode.toggled())),
        ]
        .spacing(5),
    );

    if measurement_mode_active {
        sections = sections.push(text("Measurement session").size(18));
        if let Some((strings, touched)) = strings {
            sections = sections.push(strings_section(strings, touched));
        }
        let recording = capture_state == CaptureState::Recording;
        // Only an extended take offers Stop: at 1.5 s it would only cancel the
        // capture just armed, by a double click.
        let abortable = recording && extended_capture.is_some();
        sections = sections.push(sidebar::capture_button(
            capture_state,
            if abortable {
                Message::AbortCapture
            } else {
                capture_message
            },
            abortable,
        ));
        if curve_recomputing {
            sections = sections.push(
                text("Recomputing curve — captures queue behind it")
                    .size(11)
                    .color(iced::Color::from_rgb(0.55, 0.55, 0.62)),
            );
        }
        // An extended record runs past the decay, so "Capturing…" alone cannot
        // be told from a hang.
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
        sections = sections.push(sidebar::undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}
