use iced::{Element, Subscription, Task};
use tuner_gui::widgets::cent_meter::CentMeterDisplay;

// Import our shared testing utilities
mod shared;

pub fn main() -> iced::Result {
    iced::application(
        CentMeterViewer::new,
        CentMeterViewer::update,
        CentMeterViewer::view,
    )
    .subscription(CentMeterViewer::subscription)
    .title("Cent Meter Visual Test")
    .run()
}

#[derive(Debug, Clone)]
enum LocalMessage {
    Tick,
}

struct CentMeterViewer {
    last_analysis: Option<tuner_core::AnalysisResult>,
    smoothing_buffer: Vec<f32>,
    channel_rx: crossbeam_channel::Receiver<tuner_core::AnalysisResult>,
}

impl CentMeterViewer {
    fn new() -> (Self, Task<LocalMessage>) {
        // Just one line to get a live audio feed!
        let rx = shared::start_audio_feed();

        (
            Self {
                last_analysis: None,
                smoothing_buffer: Vec::new(),
                channel_rx: rx,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                // Drain the channel and take the latest frame
                while let Ok(result) = self.channel_rx.try_recv() {
                    if let Some(cents) = result.cents_deviation {
                        self.smoothing_buffer.push(cents);
                        if self.smoothing_buffer.len() > 5 {
                            self.smoothing_buffer.remove(0);
                        }
                    } else {
                        self.smoothing_buffer.clear();
                    }
                    self.last_analysis = Some(result);
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, LocalMessage> {
        let smoothed_cents = if self.smoothing_buffer.is_empty() {
            self.last_analysis.as_ref().and_then(|a| a.cents_deviation)
        } else {
            let sum: f32 = self.smoothing_buffer.iter().sum();
            Some(sum / self.smoothing_buffer.len() as f32)
        };

        let (note_name, freq_text, confidence_text) = if let Some(a) = &self.last_analysis {
            let current_freq = a.detected_frequency.unwrap_or(0.0);
            let note_text = a.note_name.clone().unwrap_or_else(|| "--".to_string());
            let conf_text = a
                .confidence
                .map(|c| format!("{:.0}%", c * 100.0))
                .unwrap_or_else(|| "0%".to_string());

            (note_text, format!("{:.2} Hz", current_freq), conf_text)
        } else {
            ("--".to_string(), "0.00 Hz".to_string(), "0%".to_string())
        };

        CentMeterDisplay::new(smoothed_cents, note_name, freq_text, confidence_text)
            .view()
            .map(|_message| LocalMessage::Tick)
    }

    fn subscription(&self) -> Subscription<LocalMessage> {
        // Run updates at 60 FPS
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| LocalMessage::Tick)
    }
}
