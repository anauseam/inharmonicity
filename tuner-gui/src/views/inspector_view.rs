//! # Per-key measurement inspector
//!
//! Every retained measurement of one key — epoch, provenance, partial count and
//! B — with the curve's verdict on that key and the two remedies: drop one
//! entry, or re-measure. The review surface autosave assumes, since no
//! automatic acceptance gate exists
//! (`docs/design/session-persistence-and-profile-library.md` §4, §5.2).
//!
//! Renders `AppDisplayData`'s mirrored rows; every action is a `Message`
//! handled in `app.rs`, so this file holds no policy.

use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::models::{self, CurveKeyFlags};
use tuner_core::worker::CurveBundle;

use crate::Message;
use crate::advisory::{self, Severity};
use crate::app::{AppDisplayData, InspectorRow};
use crate::views::curve_select;
use crate::widgets::curve_plot::{CurvePlot, INK_SECONDARY, PlotMode, SUSPECT};

/// One measurement row: when, how, and what it measured, plus its drop button.
fn entry_row(key: u8, e: &InspectorRow) -> Element<'static, Message> {
    let when = if e.epoch.is_empty() {
        "—".to_string()
    } else {
        e.epoch.clone()
    };
    // Provenance is the load-bearing column: an auto-mode entry is retained but
    // never feeds the curve, so "why did dropping it change nothing?" has to be
    // answerable from the row itself.
    let provenance = if e.trusted { "manual" } else { "auto" };
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
    if e.is_active {
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
pub fn create_inspector_panel(
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

    // The curve itself is the key picker: it shows at a glance which keys are
    // measured (dots), which are doubted (✗) and which are gaps, so choosing
    // one to review is the same act as reading the curve. Selecting a key on
    // the main-view keyboard moves it too, so the panel follows the key being
    // tuned.
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
            // Collapsed to the entry in use: the app already resolves which
            // measurement a key presents, so the history is an override, not a
            // question the user is asked on arrival.
            let mut rows = column![].spacing(4);
            for e in &data.inspector_rows {
                if data.inspector_expanded || e.is_active {
                    rows = rows.push(entry_row(key, e));
                }
            }
            if data.inspector_rows.is_empty() {
                rows = rows.push(text("This key has no retained measurements.").size(13));
            }
            let earlier = data.inspector_rows.len().saturating_sub(1);
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
            text(format!("{} · click a key to review it", engine.label()))
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
    // Bounded, like every other settings panel: the parent column is Shrink,
    // so a `Fill` height here collapses the panel to nothing.
    .height(Length::Fixed(620.0))
    .into()
}
