use iced::{Element, Subscription, Task};
use tuner_gui::widgets::piano_keyboard::PianoKeyboard;

// Import our shared testing utilities
mod shared;

pub fn main() -> iced::Result {
    iced::application(KeyboardView::new, KeyboardView::update, KeyboardView::view)
        .subscription(KeyboardView::subscription)
        .title("Piano Keyboard Visual Test")
        .run()
}

#[derive(Debug, Clone)]
enum LocalMessage {
    Tick,
    // When the user clicks a key on the widget
    KeyClicked(u8),
}

// Implement conversion from the crate's GUI message to our local message
// since the widget emits `crate::Message::KeySelected`
impl From<tuner_gui::Message> for LocalMessage {
    fn from(msg: tuner_gui::Message) -> Self {
        match msg {
            tuner_gui::Message::KeySelected(idx) => LocalMessage::KeyClicked(idx),
            _ => LocalMessage::Tick,
        }
    }
}

struct KeyboardView {
    detected_key_index: Option<u8>,
    selected_key_index: Option<u8>,
    channel_rx: crossbeam_channel::Receiver<tuner_core::AnalysisResult>,
}

impl KeyboardView {
    fn new() -> (Self, Task<LocalMessage>) {
        // Just one line to get a live audio feed!
        let rx = shared::start_audio_feed();

        (
            Self {
                detected_key_index: None,
                selected_key_index: None,
                channel_rx: rx,
            },
            Task::none(),
        )
    }

    fn update(&mut self, message: LocalMessage) -> Task<LocalMessage> {
        match message {
            LocalMessage::Tick => {
                // Drain the channel and take the latest frame's note name
                while let Ok(result) = self.channel_rx.try_recv() {
                    self.detected_key_index = result
                        .note_name
                        .map(|name| tuner_core::models::get_key_index_from_name(&name));
                }
            }
            LocalMessage::KeyClicked(idx) => {
                self.selected_key_index = Some(idx);
            }
        }
        Task::none()
    }

    fn view(&self) -> Element<'_, LocalMessage> {
        PianoKeyboard::new(self.detected_key_index, self.selected_key_index)
            .view()
            .map(|msg| LocalMessage::from(msg))
    }

    fn subscription(&self) -> Subscription<LocalMessage> {
        // Run updates at 60 FPS
        iced::time::every(std::time::Duration::from_millis(16)).map(|_| LocalMessage::Tick)
    }
}
