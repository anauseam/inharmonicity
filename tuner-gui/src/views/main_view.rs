//! # Main Display Module
//!
//! This module contains the main display components and layout logic
//! for the Inharmonicity piano tuning application.

use crate::Message;
use crate::app::{AppDisplayData, Instrument, ReferenceMode, TuningMode};
use crate::calibration::CALIBRATION_FRAMES;
use crate::utils::view_utils::{
    ButtonConfig, ButtonType, make_capture_button, make_sidebar_section, make_undo_button,
};
use crate::widgets::curve_plot::{CurvePlot, PlotMode};
use crate::widgets::strobe_display::StrobeDisplay;
use crate::widgets::{cent_meter, guitar_strings, partials_display, piano_keyboard, spectrogram};
use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::models::{self, InharmonicityProfile};
use tuner_core::pipeline::CaptureState;
use tuner_core::worker::CurveBundle;

const TOOLS_CONFIG: [ButtonConfig; 7] = [
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
    ButtonConfig {
        label: "Partials",
        message: Some(Message::TogglePartials),
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

const PROGRAM_CONFIG: [ButtonConfig; 2] = [
    ButtonConfig {
        label: "Save Profile",
        message: Some(Message::SaveProfile),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Load Profile",
        message: Some(Message::LoadProfile),
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
        data.capture_state.clone(),
        data.undo_target_note.clone(),
        capture_message,
        data.reference_mode,
    );

    // Assemble the final layout
    let main_content = row![sidebar, Space::new().width(10), widget_area,]
        .align_y(Alignment::Start)
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

/// Creates the isolated widget area layout (spectrogram, cent meter, keyboard,
/// partials, live curve plot). This can be used independently of the settings
/// sidebar.
pub fn create_widget_area(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    let title = text("Inharmonicity").size(28);

    // Build UI panels using dedicated helper methods
    let spectrogram_panel = create_spectrogram_panel(data);
    let cent_meter_panel = create_cent_meter_panel(data);
    let keyboard_panel = create_keyboard_panel(data);
    let partials_panel = create_partials_panel(data);
    let curve_plot_panel = create_curve_plot_panel(data, curve_bundle);
    let strobe_panel = create_strobe_panel(data);
    let auto_mode_notice = create_auto_mode_notice(data);
    // let inharmonicity_graph_panel = create_inharmonicity_graph_panel(data, profile);

    // A helper function to safely embed optional widgets into a row/column layout.
    // If an optional panel (e.g., Spectrogram) is turned off and returns `None`,
    // this cleanly substitutes it with an invisible `Space` widget, preventing
    // missing elements from breaking the UI layout.
    fn wrap_panel(p: Option<Element<'static, Message>>) -> Element<'static, Message> {
        p.unwrap_or_else(|| Space::new().into())
    }

    let top_row = row![
        wrap_panel(spectrogram_panel),
        Space::new().width(10),
        wrap_panel(cent_meter_panel)
    ]
    .width(Fill)
    .align_y(Alignment::Start);

    let bottom_row = row![
        wrap_panel(keyboard_panel),
        Space::new().width(10),
        wrap_panel(partials_panel)
    ]
    .width(Fill)
    .align_y(Alignment::Start);

    // let inharmonicity_graph_row = row![
    //     wrap_panel(inharmonicity_graph_panel),
    // ]
    // .width(Length::Fill)
    // .align_y(Alignment::Start);

    let curve_row = row![
        wrap_panel(curve_plot_panel),
        Space::new().width(10),
        wrap_panel(strobe_panel)
    ]
    .width(Fill)
    .align_y(Alignment::Start);

    // The notice sits last and is pushed rather than wrapped: appearing and
    // disappearing must not move the panels above it, and an empty placeholder
    // would still take a row of the column's spacing.
    let mut content = column![
        title,
        Space::new().height(20),
        top_row,
        Space::new().height(10),
        bottom_row,
        Space::new().height(10),
        curve_row,
    ]
    .width(Fill)
    .spacing(10);

    if let Some(notice) = auto_mode_notice {
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

/// Creates the strobe panel (design §5). The strobe needs a named key — a
/// target does not exist without one — so in Auto mode the panel shows a
/// how-to-enter-manual-mode hint instead of hiding entirely. In Manual mode:
/// one band for the selected key's displayed partial n*, frozen/dimmed when
/// the partial is below the tracker's gate (D3), with the D4 coarse readout
/// computed against the **target** (R13), not ET-nearest.
///
/// The target it reads against follows the app-level [`ReferenceMode`] (the
/// sidebar toggle), shared with the cent meter.
fn create_strobe_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.strobe_visible {
        return None;
    }

    let TuningMode::Manual { note_name, .. } = &data.tuning_mode else {
        let panel = container(
            column![
                text("Strobe").size(18),
                Space::new().height(10),
                text(match data.instrument {
                    Instrument::Piano =>
                        "Manual mode required — click a key on the on-screen \
                         Keyboard Key Select panel to choose the note to strobe.",
                    Instrument::Guitar =>
                        "Manual mode required — click a string on the on-screen \
                         Guitar String Select panel to choose the note to strobe.",
                })
                .size(14)
                .color(iced::Color::from_rgb8(0xc3, 0xc2, 0xb7)),
            ]
            .width(Fill)
            .spacing(5)
            .padding(15),
        )
        .width(Length::Fixed(360.0))
        .height(Length::Fixed(240.0));
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

    let band = container(StrobeDisplay::new(s.beat_phase, s.gated).view())
        .width(Length::Fixed(150.0))
        .height(Length::Fixed(150.0));

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

    let panel = container(
        column![
            row![text(title).size(18), Space::new().width(Fill)].align_y(Alignment::Center),
            Space::new().height(10),
            row![
                band,
                Space::new().width(15),
                column![
                    text(readout).size(20),
                    Space::new().height(6),
                    text(target).size(13),
                ],
            ]
            .align_y(Alignment::Center),
            Space::new().height(8),
            lock_footer,
        ]
        .width(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Length::Fixed(360.0))
    .height(Length::Fixed(270.0));

    Some(panel.into())
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
    let (title_text, content): (String, Element<'static, Message>) = match curve_bundle {
        Some(bundle) => {
            let curve = engine.resolve(bundle);
            let mut measured = [false; 88];
            for (m, f) in measured.iter_mut().zip(curve.flags.iter()) {
                *m = f.measured;
            }
            let measured_count = measured.iter().filter(|&&m| m).count();
            (
                format!(
                    "Tuning Curve — {} · {measured_count}/88 measured",
                    engine.label()
                ),
                container(CurvePlot::new(curve.cents, measured, PlotMode::Full, None).view())
                    .width(Fill)
                    .height(Fill)
                    .into(),
            )
        }
        None => (
            "Tuning Curve".to_string(),
            text("Computing…").size(16).into(),
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

/// Creates the cent meter panel
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

    let (note_name, freq_text, confidence) = {
        let freq_str = if let Some(freq) = data.last_frequency {
            format!("{:.2} Hz", freq)
        } else {
            "--".to_string()
        };

        let note_text = match &data.tuning_mode {
            TuningMode::Auto => data
                .last_note_index
                .map(|idx| {
                    let (name, _) = models::find_nearest_note_by_index(idx);
                    name
                })
                .unwrap_or_else(|| "--".to_string()),
            TuningMode::Manual { note_name, .. } => note_name.clone(),
        };
        let status_text = if data.last_note_index.is_some() && !data.is_stale {
            if data.last_frequency.is_some() {
                "Tracking".to_string()
            } else {
                "Dropped".to_string()
            }
        } else {
            "--".to_string()
        };

        (note_text, freq_str, status_text)
    };

    let cent_meter_content: Element<'static, Message> = container(
        cent_meter::CentMeterDisplay::new(
            smoothed_cents,
            note_name,
            freq_text,
            confidence,
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
    .height(Length::Fixed(200.0));

    Some(panel.into())
}

/// Creates the note-select panel — the 88-key piano keyboard, or the six-button
/// guitar-string picker when the instrument toggle is set to Guitar (debug).
/// Both surfaces publish the same `KeySelected(key_index)`; only the picker
/// differs.
fn create_keyboard_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.key_select_visible {
        return None;
    }

    // Detected key index — directly from NoteEvent (no String→index lookup)
    let detected_key_index = data.last_note_index;

    let selected_key_index = match &data.tuning_mode {
        TuningMode::Manual { key_index, .. } => Some(*key_index),
        TuningMode::Auto => detected_key_index,
    };

    let (title, select_content): (&str, Element<'static, Message>) = match data.instrument {
        Instrument::Piano => (
            "Keyboard Key Select",
            piano_keyboard::PianoKeyboard::new(detected_key_index, selected_key_index).view(),
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

/// Creates the partials display panel
fn create_partials_panel(data: &AppDisplayData) -> Option<Element<'static, Message>> {
    if !data.partials_visible {
        return None;
    }

    // Partials have been removed from the pipeline output.
    // This panel is kept for future use but shows no data currently.
    let partials_data: Vec<f32> = Vec::new();

    let partials_content: Element<'static, Message> =
        partials_display::PartialsDisplay::new(partials_data).view();

    let panel = container(
        column![
            text("Partials").size(18),
            Space::new().height(10),
            partials_content
        ]
        .width(Fill)
        .height(Fill)
        .spacing(5)
        .padding(15),
    )
    .width(Fill)
    .height(Length::Fixed(180.0));

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
) -> Element<'static, Message> {
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

    // Reference-mode toggle: app-level, because every readout is measured
    // against it — the strobe band, its cents readout, and the cent meter.
    sections = sections.push(
        button(text(reference_mode.label()).size(14).width(Fill))
            .padding([8, 15])
            .on_press(Message::SetReferenceMode(reference_mode.toggled())),
    );
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in MAIN_SIDEBAR_CONFIG {
        sections = sections.push(make_sidebar_section(
            title,
            buttons,
            measurement_mode_active,
        ));
    }

    // Add capture button if in measurement mode
    if measurement_mode_active {
        sections = sections.push(make_capture_button(capture_state, capture_message));
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
