//! # Profile library browser
//!
//! The saved profiles, and the identity form of the one open.

use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Border, Element, Fill, Length};

use crate::Message;
use crate::app::AppDisplayData;
use crate::library::{ProfileEntry, ProfileSort};
use crate::widgets::curve_plot;
use tuner_core::models::InstrumentKind;

/// Which text field of [`InstrumentIdentity`](tuner_core::models::InstrumentIdentity)
/// an edit targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityField {
    Name,
    Make,
    Model,
    Serial,
    Form,
    Owner,
    Notes,
}

/// Instrument families offered in the picker.
const KINDS: [InstrumentKind; 4] = [
    InstrumentKind::Piano,
    InstrumentKind::Guitar,
    InstrumentKind::Bass,
    InstrumentKind::Harp,
];

/// One labelled identity field.
fn identity_row(
    label: &'static str,
    field: IdentityField,
    value: String,
    placeholder: &'static str,
) -> Element<'static, Message> {
    row![
        text(label).size(13).width(Length::Fixed(90.0)),
        text_input(placeholder, &value)
            .on_input(move |v| Message::IdentityFieldChanged(field, v))
            .size(13)
            .width(Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The identity form of the open instrument, not of a selected row. Never
/// required: a profile autosaves from its first capture and is named later.
fn identity_panel(data: &AppDisplayData) -> Element<'static, Message> {
    let id = &data.open_identity;
    let form = column![
        // Not "Open instrument": every row below has an Open button.
        text("Instrument details").size(16),
        Space::new().height(6),
        identity_row(
            "Name",
            IdentityField::Name,
            id.name.clone(),
            "Untitled instrument"
        ),
        row![
            text("Family").size(13).width(Length::Fixed(90.0)),
            pick_list(KINDS, Some(id.kind.clone()), |k| {
                Message::InstrumentKindChanged(k)
            })
            .text_size(13)
            .width(Fill),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
        identity_row(
            "Make",
            IdentityField::Make,
            id.make.clone().unwrap_or_default(),
            "Manufacturer"
        ),
        identity_row(
            "Model",
            IdentityField::Model,
            id.model.clone().unwrap_or_default(),
            "Model"
        ),
        identity_row(
            "Serial",
            IdentityField::Serial,
            id.serial.clone().unwrap_or_default(),
            "Serial number"
        ),
        identity_row(
            "Form",
            IdentityField::Form,
            id.form.clone().unwrap_or_default(),
            "Grand, upright, dreadnought…"
        ),
        identity_row(
            "Owner",
            IdentityField::Owner,
            id.owner.clone().unwrap_or_default(),
            "Owner"
        ),
        identity_row(
            "Notes",
            IdentityField::Notes,
            id.notes.clone().unwrap_or_default(),
            "Anything worth remembering"
        ),
        // Not editable: the capture dumps are filed under it.
        row![
            text("Identity").size(13).width(Length::Fixed(90.0)),
            text(if id.id.is_empty() {
                "—".to_string()
            } else {
                id.id.clone()
            })
            .size(11)
            .font(iced::Font::MONOSPACE)
            .color(curve_plot::INK_SECONDARY),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    ]
    .spacing(6);

    // Boxed: the record being written to is a separate subject from the list.
    container(form)
        .padding(12)
        .width(Fill)
        .style(|_theme| container::Style {
            border: Border {
                color: curve_plot::GRID,
                width: 1.0,
                radius: 6.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// One row of the saved-profile list.
fn entry_row(entry: &ProfileEntry, is_open: bool) -> Element<'static, Message> {
    let mut subtitle = Vec::new();
    if let Some(make) = &entry.make {
        subtitle.push(make.clone());
    }
    if let Some(model) = &entry.model {
        subtitle.push(model.clone());
    }
    if let Some(serial) = &entry.serial {
        subtitle.push(format!("#{serial}"));
    }
    let units = match entry.kind {
        InstrumentKind::Piano => "keys",
        _ => "strings",
    };
    subtitle.push(format!("{} {units}", entry.measured_count));

    let path = entry.path.clone();
    let open = button(text(if is_open { "Open ✓" } else { "Open" }).size(13))
        .padding([4, 10])
        .on_press_maybe((!is_open).then(|| Message::OpenProfile(path.clone())));

    let duplicate = button(text("Duplicate").size(13))
        .padding([4, 10])
        .on_press(Message::DuplicateProfile(path.clone()));

    let delete = button(text("Delete").size(13))
        .padding([4, 10])
        .on_press_maybe((!is_open).then_some(Message::DeleteProfile(path)));

    container(
        row![
            column![
                text(entry.name.clone()).size(15),
                text(subtitle.join(" · ")).size(12),
            ]
            .spacing(2)
            .width(Fill),
            open,
            duplicate,
            delete,
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(8)
    .width(Fill)
    .into()
}

/// The full library panel: identity of the open instrument above, the saved
/// list below.
pub fn panel(data: &AppDisplayData) -> Element<'static, Message> {
    let controls = row![
        text_input("Search name, make, model, serial…", &data.library_search)
            .on_input(Message::LibrarySearchChanged)
            .size(13)
            .width(Fill),
        pick_list(ProfileSort::ALL, Some(data.library_sort), |s| {
            Message::LibrarySortChanged(s)
        })
        .text_size(13),
        button(text("New instrument").size(13))
            .padding([4, 10])
            .on_press(Message::NewProfile),
        button(text("Close").size(13))
            .padding([4, 10])
            .on_press(Message::ToggleLibrary),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut list = column![].spacing(4);
    let mut shown = 0usize;
    for entry in &data.library_entries {
        if !entry.matches(&data.library_search) {
            continue;
        }
        shown += 1;
        let is_open = data.open_profile_path.as_ref() == Some(&entry.path);
        list = list.push(entry_row(entry, is_open));
    }
    if shown == 0 {
        list = list.push(
            text(if data.library_search.is_empty() {
                "No instruments yet — captures on this one are saved automatically."
            } else {
                "No instrument matches that search."
            })
            .size(13),
        );
    }

    // "All", not "Saved": every instrument is saved.
    let total = data.library_entries.len();
    let heading = if shown == total {
        format!("All instruments · {total}")
    } else {
        format!("All instruments · {shown} of {total}")
    };

    container(
        column![
            identity_panel(data),
            Space::new().height(14),
            text(heading).size(16),
            Space::new().height(6),
            controls,
            Space::new().height(6),
            scrollable(list).height(Fill),
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
