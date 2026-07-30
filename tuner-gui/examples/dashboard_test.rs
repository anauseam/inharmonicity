//! Standalone GUI test for the combined Widget Area layout.
//!
//! This example demonstrates how to extract just the data visualization
//! components (Spectrogram, Cent Meter, Keyboard, Partials) from the Main View
//! without including the settings sidebar.

use iced::widget::container;
use iced::{Element, Length, Subscription, Theme};

mod shared;
use tuner_core::pipeline::CaptureState;
use tuner_core::{FrameOutput, models};
use tuner_gui::app::{AppDisplayData, Instrument, TuningMode};
use tuner_gui::views::main_view::create_widget_area;

pub fn main() -> iced::Result {
    iced::application(
        DashboardTest::new,
        DashboardTest::update,
        DashboardTest::view,
    )
    .title("Widget Area Dashboard Test")
    .theme(DashboardTest::theme)
    .subscription(DashboardTest::subscription)
    .run()
}

struct DashboardTest {
    display_data: AppDisplayData,
    audio_receiver: Option<tuner_core::audio::HostHandle>,
}

#[derive(Debug, Clone)]
enum LocalMessage {
    Tick,
    // We must define these even if we don't handle them because
    // create_widget_area expects Elements that emit tuner_gui::Message.
    // For a pure isolated test, we map the inner messages to a NoOp.
    #[allow(dead_code)]
    IgnoreWidgetMessage(tuner_gui::Message),
}

impl DashboardTest {
    fn new() -> (Self, iced::Task<LocalMessage>) {
        // Create an audio feed using our shared utility
        let audio_receiver = shared::start_audio_feed();

        let display_data = AppDisplayData {
            audio_worker_active: true,
            last_frame: None,
            last_note_index: None,
            last_frequency: None,
            last_confidence: None,
            last_cents: None,
            smoothing_buffer: Vec::new(),
            is_calibrating: false,
            calibration_progress: 0,
            calibration_total: 100,
            spectrogram_visible: true,
            cent_meter_visible: true,
            key_select_visible: true,
            partials_visible: true,
            curve_plot_visible: true,
            strobe_visible: true,
            settings_view_visible: false,
            curve_select_visible: false,
            curve_detail: None,
            selected_engine: tuner_gui::app::EngineChoice::MultiBalanced,
            strobe: tuner_gui::app::StrobeState::default(),
            reference_mode: Default::default(),
            strobe_lock_view: None,
            relock_confirm_open: false,
            settings_data: tuner_gui::app::SettingsDisplayData {
                rms: tuner_gui::app::NoiseFloorSettings {
                    history: std::collections::VecDeque::new(),
                    current_threshold: 0.005,
                    calibration_complete: true,
                    visible: false,
                    active_calibration: None,
                },
                transient: tuner_gui::app::TransientSettings {
                    noise_floor_baseline: 0.0,
                    visible: false,
                    is_frozen: false,
                    freeze_countdown: None,
                    history: std::collections::VecDeque::new(),
                    current_threshold: 0.05,
                },
                ninos: tuner_gui::app::NinosSettings {
                    visible: false,
                    history: std::collections::VecDeque::new(),
                    current_threshold: 10.0,
                },
            },
            instrument_select_visible: false,
            tuning_mode: TuningMode::Auto,
            instrument: Instrument::Piano,
            measurement_mode_active: false,
            capture_state: CaptureState::Idle,
            undo_target_note: None,
            is_stale: false,
        };

        (
            Self {
                display_data,
                audio_receiver: Some(audio_receiver),
            },
            iced::Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> iced::Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                // Poll the audio receiver 60 times a second
                if let Some(host) = &mut self.audio_receiver
                    && let Some(ref mut rx) = host.frame_rx
                    && rx.update()
                {
                    let result = rx.read().clone();
                    if let Some(cents) = et_cents(&result) {
                        self.display_data.smoothing_buffer.push(cents);
                        if self.display_data.smoothing_buffer.len() > 5 {
                            self.display_data.smoothing_buffer.remove(0);
                        }
                    } else {
                        self.display_data.smoothing_buffer.clear();
                    }
                    // Store the latest analysis
                    self.display_data.last_frame = Some(result.clone());
                    self.display_data.last_note_index = result.note_index;
                    self.display_data.last_frequency = result.detected_frequency;
                    self.display_data.last_confidence = result.confidence;
                    self.display_data.last_cents = et_cents(&result);
                }
            }
            LocalMessage::IgnoreWidgetMessage(_msg) => {
                // In a real isolated test, we might handle KeySelected here.
                // For now, we just ignore clicks inside the dashboard sandbox.
            }
        }
        iced::Task::none()
    }

    fn view(&self) -> Element<'_, LocalMessage> {
        // Create the widget area using the extracted layout function.
        // It returns an Element<'static, tuner_gui::Message>, so we MUST map
        // those messages to our LocalMessage enum.
        // No worker in this sandbox — the curve panel renders its
        // "Computing…" placeholder.
        let content =
            create_widget_area(&self.display_data, None).map(LocalMessage::IgnoreWidgetMessage);

        container(content)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(20)
            .into()
    }

    fn subscription(&self) -> Subscription<LocalMessage> {
        // Just emit a Tick every 16ms. We'll poll the real channel in the `update` loop.
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| LocalMessage::Tick)
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

/// Cents from the nearest equal-temperament note — the deviation this demo
/// displays. Computed here because the core ships the measured frequency and
/// leaves the choice of reference to the consumer.
fn et_cents(frame: &FrameOutput) -> Option<f32> {
    let f = frame
        .detected_frequency
        .filter(|v| v.is_finite() && *v > 0.0)?;
    let key = frame.note_index?;
    Some(models::calculate_cents_deviation(
        f,
        models::NOTES[key as usize].frequency,
    ))
}
