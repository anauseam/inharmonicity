//! # Measurement inspector
//!
//! Every retained measurement of one key, the curve's verdict on it, and the two
//! remedies: drop an entry, or re-measure. Autosave keeps every capture and
//! nothing accepts one automatically, so this is where a bad one is caught.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::models::{self, CurveKeyFlags};
use tuner_core::worker::CurveBundle;

use crate::Message;
use crate::advisory::{self, Severity};
use crate::app::AppDisplayData;
use crate::views::curve_select;
use crate::widgets::curve_plot::{CurvePlot, INK_SECONDARY, PlotMode, SUSPECT};

/// One retained measurement of one key, as the inspector renders it.
#[derive(Debug, Clone)]
pub struct InspectorRow {
    /// Position in the key's measurement list.
    pub index: usize,
    /// The capture's timestamp, [`models::KeyMeasurement::last_captured`].
    pub epoch: String,
    pub manual: bool,
    /// How many partials the measurement kept.
    pub partials: usize,
    pub b: Option<f32>,
    pub sounding_strings: Option<models::SoundingStrings>,
    /// The entry [`models::InharmonicityProfile::active`] resolves to.
    pub in_use: bool,
    /// [`models::KeyMeasurement::is_trusted`]: a consumer may read the entry.
    pub trusted: bool,
}

/// One measurement row: when, how, and what it measured, plus its drop button.
fn entry_row(key: u8, e: &InspectorRow) -> Element<'static, Message> {
    let when = if e.epoch.is_empty() {
        "—".to_string()
    } else {
        e.epoch.clone()
    };
    // On every row: an auto entry never feeds the curve, so dropping it changes
    // nothing, and the row must say why.
    let provenance = if e.manual { "manual" } else { "auto" };
    let b = match e.b {
        Some(b) => format!("B = {b:.3e}"),
        None => "B —".to_string(),
    };

    let mut label = column![
        text(format!("{when} · {provenance}")).size(13),
        text(format!("{} partials · {b}", e.partials))
            .size(12)
            .color(INK_SECONDARY),
    ]
    .spacing(2)
    .width(Fill);
    // A solo capture measured one string, not the note.
    if let Some(strings) = e.sounding_strings {
        label = label.push(text(strings.to_string()).size(11).color(INK_SECONDARY));
    }
    if e.in_use {
        label = label.push(
            text("in use — the entry the curve and strobe read")
                .size(11)
                .color(INK_SECONDARY),
        );
    }

    container(
        row![
            label,
            button(text("Drop").size(13))
                .padding([4, 10])
                .on_press(Message::DropMeasurement(key, e.index)),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(8)
    .width(Fill)
    .into()
}

/// Plural suffix for a count.
fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}

/// The curve's verdict on this key — suspect lines in red, the rest muted.
fn advisory_lines(flags: &CurveKeyFlags) -> Element<'static, Message> {
    let mut col = column![].spacing(3);
    for a in advisory::advisories(flags) {
        let mut line = a.reason.to_string();
        if let Some(hint) = a.hint {
            line.push(' ');
            line.push_str(hint);
        }
        let (mark, color) = match a.severity {
            Severity::Suspect => ("✗ ", SUSPECT),
            Severity::Informational => ("· ", INK_SECONDARY),
        };
        col = col.push(text(format!("{mark}{line}")).size(12).color(color));
    }
    col.into()
}

/// The full inspector panel.
pub fn panel(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    let engine = data.selected_engine;
    let header = row![
        text("Measurement Inspector").size(20).width(Fill),
        button(text("Close").size(13))
            .padding([4, 10])
            .on_press(Message::ToggleInspector),
    ]
    .align_y(Alignment::Center);

    // The curve is the key picker: it shows which keys are measured, doubted or
    // missing.
    let picker: Element<'static, Message> = match curve_bundle {
        Some(bundle) => {
            let (cents, measured, suspect) = curve_select::plot_inputs(bundle.curve(engine));
            container(
                CurvePlot::new(cents, measured, suspect, PlotMode::Full, None)
                    .selected(data.inspector_key)
                    .on_select(Message::InspectKey)
                    .view(),
            )
            .width(Fill)
            .height(Length::Fixed(220.0))
            .into()
        }
        None => text("Computing the curve…").size(13).into(),
    };

    let body: Element<'static, Message> = match data.inspector_key {
        None => text("No measurements yet — capture a key to review it here.")
            .size(13)
            .into(),
        Some(key) => {
            let (name, _) = models::find_nearest_note_by_index(key);
            let flags = curve_bundle.map(|b| b.curve(data.selected_engine).flags[key as usize]);
            let advisories = match flags {
                Some(f) => advisory_lines(&f),
                None => Space::new().into(),
            };
            // Collapsed to the entry in use: the app already resolves which entry a
            // key presents, so the history is an override, not a question.
            let (used, unused): (Vec<_>, Vec<_>) =
                data.inspector_rows.iter().partition(|e| e.trusted);
            let mut rows = column![].spacing(4);
            for e in &used {
                if data.inspector_expanded || e.in_use {
                    rows = rows.push(entry_row(key, e));
                }
            }
            // "Nothing at all" and "nothing readable" are different answers,
            // and only the second needs explaining.
            if used.is_empty() {
                let line = if unused.is_empty() {
                    "This key has no retained measurements."
                } else {
                    "No measurement in use — this key falls back to the prior."
                };
                rows = rows.push(text(line).size(13));
            }
            let earlier = used.len().saturating_sub(1);
            if earlier > 0 {
                let label = if data.inspector_expanded {
                    "Hide earlier measurements".to_string()
                } else {
                    format!("{earlier} earlier measurement{}", plural(earlier))
                };
                rows = rows.push(
                    button(text(label).size(12))
                        .padding([3, 8])
                        .on_press(Message::ToggleInspectorHistory),
                );
            }
            // Retained but read by nothing: filed apart and closed, evidence to go
            // looking for rather than part of the review.
            if !unused.is_empty() {
                let label = if data.inspector_unused_expanded {
                    "Hide captures not in use".to_string()
                } else {
                    format!("Show {} not in use", unused.len())
                };
                rows = rows.push(
                    button(text(label).size(12))
                        .padding([3, 8])
                        .on_press(Message::ToggleInspectorUnused),
                );
                if data.inspector_unused_expanded {
                    for e in &unused {
                        rows = rows.push(entry_row(key, e));
                    }
                }
            }

            column![
                row![
                    text(format!("{name} — {} retained", data.inspector_rows.len())).size(16),
                    Space::new().width(Fill),
                    button(text("Re-measure this key").size(13))
                        .padding([4, 10])
                        .on_press(Message::RemeasureKey(key)),
                ]
                .align_y(Alignment::Center),
                advisories,
                Space::new().height(6),
                scrollable(rows).height(Fill),
                text("Dropping removes the measurement; the capture's audio stays on disk.")
                    .size(11)
                    .color(INK_SECONDARY),
            ]
            .spacing(6)
            .into()
        }
    };

    container(
        column![
            header,
            text(format!(
                "{} · click a key to review it",
                curve_select::engine_label(engine)
            ))
            .size(12)
            .color(INK_SECONDARY),
            picker,
            Space::new().height(12),
            body,
        ]
        .width(Fill)
        .spacing(4)
        .padding(15),
    )
    .width(Fill)
    // Bounded: the parent column is Shrink, so a `Fill` height collapses it.
    .height(Length::Fixed(620.0))
    .into()
}
