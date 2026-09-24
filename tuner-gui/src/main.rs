//! # Inharmonicity GUI
//!
//! The iced frontend over `tuner-core`.

mod advisory;
mod app;
mod calibration;
mod library;
mod session;
mod views;
mod widgets;

// The views and widgets name the app's message type as `crate::Message`.
use app::Message;

fn main() -> iced::Result {
    app::run()
}
