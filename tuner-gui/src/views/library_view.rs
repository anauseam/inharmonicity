//! # Profile library browser
//!
//! The instrument picker: the list of saved profiles, plus the identity of the
//! one currently open. Renders `AppDisplayData`'s library state; every action
//! is a `Message` handled in `app.rs`, so this file holds no policy.
//!
//! Search covers the serial number because that is the only field that
//! identifies an instrument unambiguously. The shape — a browsable list rather
//! than an OS file picker — is argued in
//! `docs/design/session-persistence-and-profile-library.md` §5.

use iced::widget::{
    Space, button, column, container, pick_list, row, scrollable, text, text_input,
};
use iced::{Alignment, Border, Element, Fill, Length};

use crate::app::{AppDisplayData, IdentityField};
use crate::library::{ProfileEntry, ProfileSort};
use crate::widgets::curve_plot;
use tuner_core::models::InstrumentKind;

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
) -> Element<'static, crate::Message> {
    row![
        text(label).size(13).width(Length::Fixed(90.0)),
        text_input(placeholder, &value)
            .on_input(move |v| crate::Message::IdentityFieldChanged(field, v))
            .size(13)
            .width(Fill),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

/// The identity form for the open instrument — the detail half of the panel's
/// list–detail pair, following what is *open* rather than what is selected.
///
/// Editable after the fact and never required: a profile exists and auto-saves
/// from its first capture, and is named later. Its whole job is to make "is
/// this the instrument in front of me?" answerable before autosave writes
/// another instrument's measurements into this file.
fn identity_panel(data: &AppDisplayData) -> Element<'static, crate::Message> {
    let id = &data.open_identity;
    let form = column![
        // Not "Open instrument": every row of the list below carries an *Open*
        // button, so the word reads as the action there rather than as the
        // state here.
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
                crate::Message::InstrumentKindChanged(k)
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
    ]
    .spacing(6);

    // Boxed, because this panel is two subjects rather than one: the record
    // being written to, and the collection it belongs to.
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
fn entry_row(entry: &ProfileEntry, is_open: bool) -> Element<'static, crate::Message> {
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
    subtitle.push(format!(
        "{} {}",
        entry.measured_count,
        entry.kind.unit_plural()
    ));

    let path = entry.path.clone();
    let open = button(text(if is_open { "Open ✓" } else { "Open" }).size(13))
        .padding([4, 10])
        .on_press_maybe((!is_open).then(|| crate::Message::OpenProfile(path.clone())));

    let duplicate = button(text("Duplicate").size(13))
        .padding([4, 10])
        .on_press(crate::Message::DuplicateProfile(path.clone()));

    // Deleting the instrument being tuned would leave autosave writing to a
    // file that no longer exists, so the open row cannot offer it.
    let delete = button(text("Delete").size(13))
        .padding([4, 10])
        .on_press_maybe((!is_open).then_some(crate::Message::DeleteProfile(path)));

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
pub fn create_library_panel(data: &AppDisplayData) -> Element<'static, crate::Message> {
    let controls = row![
        text_input("Search name, make, model, serial…", &data.library_search)
            .on_input(crate::Message::LibrarySearchChanged)
            .size(13)
            .width(Fill),
        pick_list(ProfileSort::ALL, Some(data.library_sort), |s| {
            crate::Message::LibrarySortChanged(s)
        })
        .text_size(13),
        button(text("New instrument").size(13))
            .padding([4, 10])
            .on_press(crate::Message::NewProfile),
        button(text("Close").size(13))
            .padding([4, 10])
            .on_press(crate::Message::ToggleLibrary),
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

    // "All", not "Saved": everything is saved, always, so the word no longer
    // distinguishes one instrument from another.
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
    // Bounded, like every other settings panel: the parent column is Shrink,
    // so a `Fill` height here collapses the panel to nothing.
    .height(Length::Fixed(620.0))
    .into()
}
