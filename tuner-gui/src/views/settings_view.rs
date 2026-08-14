use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::worker::CurveBundle;

use crate::Message;
use crate::app::{AppDisplayData, Instrument};
use crate::utils::view_utils::{
    ButtonConfig, ButtonType, make_capture_button, make_sidebar_section, make_undo_button,
};
use crate::views::{
    curve_select, inspector_view, library_view, ninos2_calibration, rms_calibration,
    transient_calibration,
};

const TONAL_CONFIG: [ButtonConfig; 4] = [
    ButtonConfig {
        label: "Temperament",
        message: None,
        button_type: ButtonType::Disabled,
    },
    // Reference-pitch / A440 view entry. Locked to A440 for now (curve d_g = 0);
    // the view that lets the user set a non-440 reference is deferred UX.
    ButtonConfig {
        label: "Tuning Standard",
        message: None,
        button_type: ButtonType::Disabled,
    },
    ButtonConfig {
        label: "Inharmonic curve adjustment",
        message: None,
        button_type: ButtonType::Disabled,
    },
    // The curve comparison/selection gallery (strobe design §9): every
    // engine is in the bundle already, this view renders and picks from it.
    ButtonConfig {
        label: "Curve Select",
        message: Some(Message::ToggleCurveSelect),
        button_type: ButtonType::Standard,
    },
];

const PROGRAM_CONFIG: [ButtonConfig; 4] = [
    ButtonConfig {
        label: "Transient Threshold Calibration",
        message: Some(Message::ToggleTransientCalibration),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "NINOS2 Stability Calibration",
        message: Some(Message::ToggleNinosCalibration),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Noise Floor Adjustment",
        message: Some(Message::ToggleNoiseFloorAdjustment),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Sample Buffer Adjustment",
        message: None,
        button_type: ButtonType::Disabled,
    },
];

// Surfaces an ordinary tuning session never touches. Both persist with the
// app rather than the open profile.
const ADVANCED_CONFIG: [ButtonConfig; 2] = [
    // Swaps the main-view note picker between the piano keyboard and a
    // six-button guitar-string picker. Not a full instrument mode — no
    // inharmonicity is measured for guitar (see `Instrument`).
    ButtonConfig {
        label: "Instrument Select",
        message: Some(Message::ToggleInstrumentSelect),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "String Isolation",
        message: Some(Message::ToggleStringIsolationPanel),
        button_type: ButtonType::Standard,
    },
];

// Which instrument is open, and every instrument previously measured; plus the
// per-key review surface autosave assumes (design note §4).
const LIBRARY_CONFIG: [ButtonConfig; 2] = [
    ButtonConfig {
        label: "Instrument Library",
        message: Some(Message::ToggleLibrary),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Measurement Inspector",
        message: Some(Message::ToggleInspector),
        button_type: ButtonType::Standard,
    },
];

/// Static settings sidebar configuration
const SETTINGS_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 4] = [
    ("Instrument", LIBRARY_CONFIG.as_slice()),
    ("Tonal adjustments", TONAL_CONFIG.as_slice()),
    ("Program adjustments", PROGRAM_CONFIG.as_slice()),
    ("Advanced", ADVANCED_CONFIG.as_slice()),
];

pub fn create_settings_view(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    // Show calibrating/shutdown message if audio worker is not active AND not in settings recalibration
    if !data.audio_worker_active && !data.is_calibrating {
        return container(text("Shutting down...").size(40))
            .width(Fill)
            .height(Fill)
            .center_x(Fill)
            .center_y(Fill)
            .into();
    }

    let title = text("Settings").size(28);

    // Build main panel content based on which sub-view is active
    let main_panel_content: Element<'static, Message> = if data.library_visible {
        library_view::create_library_panel(data)
    } else if data.inspector_visible {
        inspector_view::create_inspector_panel(data, curve_bundle)
    } else if data.curve_select_visible {
        curve_select::create_curve_select_panel(
            curve_bundle,
            data.selected_engine,
            data.curve_detail,
        )
    } else if data.settings_data.rms.visible {
        rms_calibration::create_rms_calibration_panel(data)
    } else if data.settings_data.transient.visible {
        transient_calibration::create_transient_calibration_panel(data)
    } else if data.settings_data.ninos.visible {
        ninos2_calibration::create_ninos2_calibration_panel(data)
    } else if data.instrument_select_visible {
        create_instrument_select_panel(data.instrument)
    } else if data.string_isolation_visible {
        create_string_isolation_panel(data.string_isolation)
    } else {
        text("Select a setting to adjust.").size(18).into()
    };

    let main_panel = container(
        column![title, Space::new().height(20), main_panel_content]
            .width(Fill)
            .spacing(10),
    )
    .width(Fill)
    .height(Fill);

    let sidebar = create_settings_sidebar(data);

    let main_content = row![sidebar, Space::new().width(10), main_panel]
        .align_y(Alignment::Start)
        .padding(20);

    container(main_content).width(Fill).height(Fill).into()
}

/// The string-isolation panel: what the per-capture string declaration is,
/// and the switch that offers it.
///
/// It is off by default and hidden while off, because a declaration is only
/// meaningful when strings are actually being damped one at a time — and a
/// stale one left standing would label ordinary captures with a mute pattern
/// that was not there.
fn create_string_isolation_panel(enabled: bool) -> Element<'static, Message> {
    fn segment(
        label: &'static str,
        target: bool,
        active: bool,
    ) -> iced::widget::Button<'static, Message> {
        let btn = button(text(label).size(16))
            .padding([8, 24])
            .on_press(Message::SetStringIsolation(target));
        if active {
            btn.style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.325, 0.278, 0.388,
                ))), // purple — matches the active Settings button
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            })
        } else {
            btn
        }
    }

    column![
        text("String Isolation").size(20),
        Space::new().height(8),
        text(
            "For measurement sessions where a note's strings are recorded one at a \
             time, the others damped with a mute. Turning this on adds a declaration \
             to the capture controls: how many strings the key is strung with, and \
             which of them are sounding. It is written into the capture's \
             analysis.json and shown on the measurement inspector's rows."
        )
        .size(13),
        Space::new().height(8),
        text(
            "A solo capture measures one string, not the note, so it is not \
             interchangeable with an ordinary capture. Leave this off for tuning: \
             while it is off, captures record no string state at all."
        )
        .size(13),
        Space::new().height(16),
        row![
            segment("Off", false, !enabled),
            segment("On", true, enabled)
        ]
        .spacing(10),
    ]
    .spacing(6)
    .into()
}

/// The instrument-select debug panel: a two-button segmented toggle between the
/// piano keyboard and the guitar-string picker, with the active surface
/// highlighted. Switching is otherwise handled in `App::set_instrument` (strobe
/// reference + manual-target coupling live there, not here).
fn create_instrument_select_panel(instrument: Instrument) -> Element<'static, Message> {
    fn segment(
        label: &'static str,
        target: Instrument,
        active: bool,
    ) -> iced::widget::Button<'static, Message> {
        let btn = button(text(label).size(16))
            .padding([8, 24])
            .on_press(Message::SetInstrument(target));
        if active {
            btn.style(|_theme, _status| button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.325, 0.278, 0.388,
                ))), // purple — matches the active Settings button
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            })
        } else {
            btn
        }
    }

    let toggle = row![
        segment("Piano", Instrument::Piano, instrument == Instrument::Piano),
        segment(
            "Guitar",
            Instrument::Guitar,
            instrument == Instrument::Guitar
        ),
    ]
    .spacing(10);

    column![
        text("Instrument Select").size(20),
        Space::new().height(8),
        text(
            "Debug convenience — swaps the main-view note picker only. Guitar shows \
             six standard-tuning string buttons (EADGBE) and sets the strobe reference \
             to equal temperament; Piano restores the 88-key keyboard and the tuning \
             curve. No inharmonicity is measured for guitar."
        )
        .size(13),
        Space::new().height(16),
        toggle,
    ]
    .spacing(6)
    .into()
}

fn create_settings_sidebar(data: &AppDisplayData) -> Element<'static, Message> {
    let mut sections = column![].spacing(10);

    // The same control as the main view's "Settings", so it names where it
    // goes rather than where it is — otherwise it reads as the label of the
    // view you are already in.
    let settings_button = button(text("← Back to Tuner").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| {
            use iced::widget::button;
            button::Style {
                background: Some(iced::Background::Color(iced::Color::from_rgb(
                    0.325, 0.278, 0.388,
                ))), // purple
                text_color: iced::Color::WHITE,
                ..button::Style::default()
            }
        })
        .on_press(Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    // Add all settings sections
    for (title, buttons) in SETTINGS_SIDEBAR_CONFIG {
        sections = sections.push(make_sidebar_section(
            title,
            buttons,
            data.measurement_mode_active,
        ));
    }

    // Add capture button if in measurement mode
    if data.measurement_mode_active {
        sections = sections.push(make_capture_button(
            data.capture_state.clone(),
            Message::CaptureButtonClicked,
        ));
    }

    // Show undo button if undo history exists
    if let Some(note_name) = data.undo_target_note.clone() {
        sections = sections.push(iced::widget::Space::new().height(20));
        sections = sections.push(make_undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}
