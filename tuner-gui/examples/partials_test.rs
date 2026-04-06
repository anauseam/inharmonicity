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
    host_handle: tuner_core::audio::HostHandle,
}

impl PartialsViewer {
    fn new() -> (Self, Task<LocalMessage>) {
        // Just one line to get a live audio feed!
        let rx = shared::start_audio_feed();

        (
            Self {
                partials: Vec::new(),
                host_handle: rx,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                // Partials are temporarily stubbed per the GUI refactor
                if let Some(ref mut rx) = self.host_handle.frame_rx {
                    if rx.update() {
                        let _result = rx.read().clone();
                        self.partials = vec![440.0, 880.0, 1320.0, 1760.0];
                    }
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
