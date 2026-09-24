//! # Settings screen
//!
//! The settings sidebar and the panel each of its entries opens: the three
//! calibrations, the instrument library, the curve gallery and the inspector,
//! and the switches for the measurement-session surfaces.

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::worker::CurveBundle;

use crate::Message;
use crate::app::{AppDisplayData, Instrument};
use crate::views::sidebar::{self, ButtonConfig, ButtonType};
use crate::views::{
    curve_select, inspector_view, library_view, rms_calibration, sustain_calibration,
    transient_calibration,
};

const TONAL_CONFIG: [ButtonConfig; 4] = [
    ButtonConfig {
        label: "Temperament",
        message: None,
        button_type: ButtonType::Disabled,
    },
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
        label: "Sustain Stability Calibration",
        message: Some(Message::ToggleSustainCalibration),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Silence Threshold Calibration",
        message: Some(Message::ToggleNoiseFloorAdjustment),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Sample Buffer Adjustment",
        message: None,
        button_type: ButtonType::Disabled,
    },
];

// Surfaces an ordinary tuning session never touches.
const ADVANCED_CONFIG: [ButtonConfig; 4] = [
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
    ButtonConfig {
        label: "Unison Assist",
        message: Some(Message::ToggleUnisonAssistPanel),
        button_type: ButtonType::Standard,
    },
    ButtonConfig {
        label: "Capture Duration",
        message: Some(Message::ToggleExtendedCapturePanel),
        button_type: ButtonType::Standard,
    },
];

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

const SETTINGS_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 4] = [
    ("Instrument", LIBRARY_CONFIG.as_slice()),
    ("Tonal adjustments", TONAL_CONFIG.as_slice()),
    ("Program adjustments", PROGRAM_CONFIG.as_slice()),
    ("Advanced", ADVANCED_CONFIG.as_slice()),
];

pub fn view(
    data: &AppDisplayData,
    curve_bundle: Option<&CurveBundle>,
) -> Element<'static, Message> {
    let title = text("Settings").size(28);

    let main_panel_content: Element<'static, Message> = if data.library_visible {
        library_view::panel(data)
    } else if data.inspector_visible {
        inspector_view::panel(data, curve_bundle)
    } else if data.curve_select_visible {
        curve_select::panel(curve_bundle, data.selected_engine, data.curve_detail)
    } else if data.settings_data.rms.visible {
        rms_calibration::panel(data)
    } else if data.settings_data.transient.visible {
        transient_calibration::panel(data)
    } else if data.settings_data.sustain.visible {
        sustain_calibration::panel(data)
    } else if data.instrument_select_visible {
        create_instrument_select_panel(data.instrument)
    } else if data.string_isolation_visible {
        create_string_isolation_panel(data.string_isolation)
    } else if data.unison_assist_visible {
        create_unison_assist_panel(data.unison_assist)
    } else if data.extended_capture_visible {
        create_capture_duration_panel(data.extended_capture, data.extended_capture_secs)
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

/// The Unison Assist panel: what the unison panels are, and their switch.
fn create_unison_assist_panel(enabled: bool) -> Element<'static, Message> {
    column![
        text("Unison Assist").size(20),
        Space::new().height(8),
        text(
            "Resolves the sounding note into its individual strings and draws \
             them as markers on the same cents axis the strobe reads against — \
             one panel magnifying the strobe's own partial, one stacking every \
             partial the bank targets."
        )
        .size(13),
        Space::new().height(8),
        text(
            "It answers a narrow question — is this unison set — and it answers \
             it only above its own resolution floor, which it states on every \
             reading. Below that floor two strings resolve as one line, so a \
             clean-looking panel is not proof of a clean unison. Leave it off \
             for ordinary tuning."
        )
        .size(13),
        Space::new().height(16),
        row![
            enable_segment("Off", false, !enabled),
            Space::new().width(8),
            enable_segment("On", true, enabled),
        ],
    ]
    .spacing(4)
    .into()
}

/// One side of an on/off pair, lit when it is the current setting.
fn enable_segment(
    label: &'static str,
    target: bool,
    active: bool,
) -> iced::widget::Button<'static, Message> {
    highlight(
        button(text(label).size(16))
            .padding([8, 24])
            .on_press(Message::SetUnisonAssist(target)),
        active,
    )
}

/// Lights a segment button when it is the current setting, in the purple of
/// the active Settings button.
fn highlight(
    btn: iced::widget::Button<'static, Message>,
    active: bool,
) -> iced::widget::Button<'static, Message> {
    if active {
        btn.style(|_theme, _status| button::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.325, 0.278, 0.388,
            ))),
            text_color: iced::Color::WHITE,
            ..button::Style::default()
        })
    } else {
        btn
    }
}

/// The string-isolation panel: what the per-capture string declaration is, and
/// its switch.
fn create_string_isolation_panel(enabled: bool) -> Element<'static, Message> {
    fn segment(
        label: &'static str,
        target: bool,
        active: bool,
    ) -> iced::widget::Button<'static, Message> {
        highlight(
            button(text(label).size(16))
                .padding([8, 24])
                .on_press(Message::SetStringIsolation(target)),
            active,
        )
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

/// The capture-duration panel: whether a capture records past the shipped
/// 1.5 s, and for how long.
fn create_capture_duration_panel(enabled: bool, secs: f32) -> Element<'static, Message> {
    let mode = |label: &'static str, target: bool| {
        highlight(
            button(text(label).size(16))
                .padding([8, 24])
                .on_press(Message::SetExtendedCapture(target)),
            enabled == target,
        )
    };

    let mut lengths = row![].spacing(10);
    for choice in [2.0f32, 3.0, 5.0] {
        let mut btn = highlight(
            button(text(format!("{choice:.0} s")).size(16)).padding([8, 20]),
            enabled && (secs - choice).abs() < 0.05,
        );
        if enabled {
            btn = btn.on_press(Message::SetExtendedCaptureSecs(choice));
        }
        lengths = lengths.push(btn);
    }

    column![
        text("Capture Duration").size(20),
        Space::new().height(8),
        text(
            "A capture normally records 1.5 s and stops early if the note decays. \
             Extended captures record the full length instead, past the point the \
             Gatekeeper calls silence — the tail a decay fit needs, and the audio \
             the offline harnesses read."
        )
        .size(13),
        Space::new().height(8),
        text(
            "Only the stored audio grows. Every measurement is still made from the \
             first 1.5 s, so extended captures stay comparable with the existing \
             capture sets. Leave this off for tuning: a long record runs for its \
             whole length whatever happens, so anything else played into it is \
             part of the recording."
        )
        .size(13),
        Space::new().height(16),
        row![mode("1.5 s (default)", false), mode("Extended", true)].spacing(10),
        Space::new().height(12),
        lengths,
    ]
    .spacing(6)
    .into()
}

/// The note-picker panel: the piano keyboard or the guitar strings.
fn create_instrument_select_panel(instrument: Instrument) -> Element<'static, Message> {
    fn segment(
        label: &'static str,
        target: Instrument,
        active: bool,
    ) -> iced::widget::Button<'static, Message> {
        highlight(
            button(text(label).size(16))
                .padding([8, 24])
                .on_press(Message::SetInstrument(target)),
            active,
        )
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
             six standard-tuning string buttons (EADGBE); Piano shows the 88-key \
             keyboard. The strobe reference does not change with it: set it from the \
             Reference control. No inharmonicity is measured for guitar."
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

    // The main view's Settings button, named for where it goes rather than
    // where it is, or it would read as this view's label.
    let settings_button = button(text("← Back to Tuner").size(16).width(Fill))
        .padding([10, 15])
        .style(|_theme, _status| button::Style {
            background: Some(iced::Background::Color(iced::Color::from_rgb(
                0.325, 0.278, 0.388,
            ))),
            text_color: iced::Color::WHITE,
            ..button::Style::default()
        })
        .on_press(Message::ToggleSettingsView);

    sections = sections.push(settings_button);
    sections = sections.push(Space::new().height(10));

    for (title, buttons) in SETTINGS_SIDEBAR_CONFIG {
        sections = sections.push(sidebar::section(
            title,
            buttons,
            data.measurement_mode_active,
        ));
    }

    if data.measurement_mode_active {
        // Never abortable: this copy is a shortcut back to capturing, not the
        // control a take is watched on.
        sections = sections.push(sidebar::capture_button(
            data.capture_state,
            Message::CaptureButtonClicked,
            false,
        ));
    }

    if let Some(note_name) = data.undo_target_note.clone() {
        sections = sections.push(Space::new().height(20));
        sections = sections.push(sidebar::undo_button(note_name));
    }

    container(sections.padding(15))
        .width(Length::Fixed(250.0))
        .height(Fill)
        .into()
}
