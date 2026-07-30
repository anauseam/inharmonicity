use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Fill, Length};
use tuner_core::worker::CurveBundle;

use crate::Message;
use crate::app::{AppDisplayData, Instrument};
use crate::utils::view_utils::{
    ButtonConfig, ButtonType, make_capture_button, make_sidebar_section, make_undo_button,
};
use crate::views::{curve_select, ninos2_calibration, rms_calibration, transient_calibration};

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

// Debug-only surface: swaps the main-view note-select widget between the piano
// keyboard and a six-button guitar-string picker. Not a full instrument mode —
// no inharmonicity is measured for guitar (see `Instrument`).
const INSTRUMENT_CONFIG: [ButtonConfig; 1] = [ButtonConfig {
    label: "Instrument Select",
    message: Some(Message::ToggleInstrumentSelect),
    button_type: ButtonType::Standard,
}];

/// Static settings sidebar configuration
const SETTINGS_SIDEBAR_CONFIG: [(&str, &[ButtonConfig]); 3] = [
    ("Tonal adjustments", TONAL_CONFIG.as_slice()),
    ("Program adjustments", PROGRAM_CONFIG.as_slice()),
    ("Instrument (debug)", INSTRUMENT_CONFIG.as_slice()),
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
    let main_panel_content: Element<'static, Message> = if data.curve_select_visible {
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

    // Add Settings button at the top
    let settings_button = button(text("Settings").size(16).width(Fill))
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
