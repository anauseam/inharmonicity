//! Standalone GUI test for the combined Widget Area layout.
//!
//! This example demonstrates how to extract just the data visualization
//! components (Spectrogram, Cent Meter, Keyboard, Partials) from the Main View
//! without including the settings sidebar.

use iced::widget::container;
use iced::{Element, Length, Subscription, Theme};
use tuner_core::AnalysisResult;

mod shared;
use tuner_gui::app::{AppDisplayData, CaptureState, TuningMode};
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
    audio_receiver: Option<crossbeam_channel::Receiver<AnalysisResult>>,
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
            last_analysis: None,
            smoothing_buffer: Vec::new(),
            spectrogram_visible: true,
            cent_meter_visible: true,
            key_select_visible: true,
            partials_visible: true,
            settings_view_visible: false,
            settings_data: tuner_gui::app::SettingsDisplayData {
                rms_history: std::collections::VecDeque::new(),
                current_silence_threshold: 0.005,
                noise_floor_adjustment_visible: false,
            },
            tuning_mode: TuningMode::Auto,
            capture_state: CaptureState::Off,
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
                if let Some(rx) = &self.audio_receiver {
                    while let Ok(result) = rx.try_recv() {
                        // Update the smoothing buffer for the cent meter
                        if let Some(cents) = result.cents_deviation {
                            self.display_data.smoothing_buffer.push(cents);
                            if self.display_data.smoothing_buffer.len() > 5 {
                                self.display_data.smoothing_buffer.remove(0);
                            }
                        } else {
                            self.display_data.smoothing_buffer.clear();
                        }

                        // Store the latest analysis
                        self.display_data.last_analysis = Some(result);
                    }
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
        let content = create_widget_area(&self.display_data)
            .map(|msg| LocalMessage::IgnoreWidgetMessage(msg));

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
