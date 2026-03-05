use iced::{Element, Subscription, Task};
use tuner_gui::widgets::partials_display::PartialsDisplay;

// Import our shared testing utilities
mod shared;

pub fn main() -> iced::Result {
    iced::application(
        PartialsViewer::new,
        PartialsViewer::update,
        PartialsViewer::view,
    )
    .subscription(PartialsViewer::subscription)
    .title("Partials Display Visual Test")
    .run()
}

#[derive(Debug, Clone)]
enum LocalMessage {
    Tick,
}

struct PartialsViewer {
    partials: Vec<f32>,
    channel_rx: crossbeam_channel::Receiver<tuner_core::AnalysisResult>,
}

impl PartialsViewer {
    fn new() -> (Self, Task<LocalMessage>) {
        // Just one line to get a live audio feed!
        let rx = shared::start_audio_feed();

        (
            Self {
                partials: Vec::new(),
                channel_rx: rx,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                // Drain the channel and take the latest frame's partials
                while let Ok(result) = self.channel_rx.try_recv() {
                    self.partials = result.partials;
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, LocalMessage> {
        // Map the real widget's generic Message return type to our internal LocalMessage
        PartialsDisplay::new(self.partials.clone())
            .view()
            .map(|_message| LocalMessage::Tick)
    }

    fn subscription(&self) -> Subscription<LocalMessage> {
        // Run updates at 60 FPS
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| LocalMessage::Tick)
    }
}
