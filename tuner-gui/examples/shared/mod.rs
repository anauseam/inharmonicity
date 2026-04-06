//! Shared testing utilities for visual examples
use tuner_core::audio::{AudioSource, HostHandle, spawn_analysis_thread};

/// Starts a background audio thread that captures microphone input,
/// processes it through the entire FFT and pitch detection pipeline,
/// and returns a HostHandle yielding complete `FrameOutput` frames via triple buffer.
pub fn start_audio_feed() -> HostHandle {
    match spawn_analysis_thread(AudioSource::Default) {
        Ok(handle) => handle,
        Err(e) => {
            panic!("Failed to start audio feed for test: {}", e);
            #[allow(unreachable_code)]
            tuner_core::audio::spawn_analysis_thread(AudioSource::Default).unwrap()
        }
    }
}
