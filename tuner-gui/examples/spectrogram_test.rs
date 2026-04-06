use iced::{Element, Subscription, Task};
use tuner_gui::widgets::spectrogram::Spectrogram;

// Import our new shared testing utilities
mod shared;

pub fn main() -> iced::Result {
    iced::application(
        SpectrogramViewer::new,
        SpectrogramViewer::update,
        SpectrogramViewer::view,
    )
    .subscription(SpectrogramViewer::subscription)
    .title("Spectrogram Visual Test")
    .run()
}

#[derive(Debug, Clone)]
enum LocalMessage {
    Tick,
}

struct SpectrogramViewer {
    spectrum_data: Vec<f32>,
    host_handle: tuner_core::audio::HostHandle,
}

impl SpectrogramViewer {
    fn new() -> (Self, Task<LocalMessage>) {
        // Just one line to get a live audio feed!
        let rx = shared::start_audio_feed();

        (
            Self {
                // Initialize with an empty 1024-bin FFT
                spectrum_data: vec![0.0; 1024],
                host_handle: rx,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                if let Some(ref mut rx) = self.host_handle.frame_rx {
                    if rx.update() {
                        let result = rx.read();
                        self.spectrum_data = result.magnitudes[..result.magnitude_len].to_vec();
                    }
                }
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, LocalMessage> {
        // Map the real widget's generic Message return type to our internal LocalMessage
        Spectrogram::new(self.spectrum_data.clone())
            .view()
            .map(|_message| LocalMessage::Tick)
    }

    fn subscription(&self) -> Subscription<LocalMessage> {
        // Run updates at 60 FPS
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| LocalMessage::Tick)
    }
}
