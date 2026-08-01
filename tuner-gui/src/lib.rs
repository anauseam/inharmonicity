//! # Inharmonicity - Professional Piano Tuning GUI
//!
//! This module contains the main GUI application for the Inharmonicity piano tuning software.

pub mod app;
pub mod calibration;
pub mod library;
pub mod session;
pub mod utils;
pub mod views;
pub mod widgets;

// Re-export only the necessary application entry points
pub use app::{Message, TunerApp};
