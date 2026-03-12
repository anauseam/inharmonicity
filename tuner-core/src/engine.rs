//! # Engine (Thread 2) — Fundamental Frequency Detection
//!
//! The "Brains" of the pipeline. Pulls audio samples from the Elastic Ring Buffer
//! and executes the Scout, Bass, and Treble algorithms against thread-local
//! [`ProcessingFrame`](crate::pipeline::ProcessingFrame) scratch buffers.
//!
//! ## Wireframe Status
//!
//! This module is currently a **wireframe**. The `Engine` struct owns the
//! `ProcessingFrame` and exposes a `process_tick()` stub. As the migration
//! progresses, the following will be implemented:
//!
//! 1. Pop samples from the Elastic Ring Buffer into `frame.audio_buffer`
//! 2. Run the Scout Engine (FFT) to determine rough frequency neighborhood
//! 3. Route to Bass Engine (pYIN, 8192 samples) or Treble Engine (QIFFT / DPLL, 2048 samples)
//! 4. Pass the resulting F0 + confidence to the Gatekeeper

use crate::pipeline::ProcessingFrame;

/// The Fundamental Frequency ($f_0$) Engine.
///
/// Owns the thread-local [`ProcessingFrame`] scratch memory and will eventually
/// execute the Scout → Router → Bass/Treble detection chain. Currently a wireframe
/// with a no-op `process_tick()`.
pub struct Engine {
    /// Thread 2's pre-allocated working memory — audio, time-domain, and frequency-domain buffers.
    pub frame: ProcessingFrame,
}

impl Engine {
    /// Creates a new Engine with zeroed-out scratch buffers.
    ///
    /// This should be called **once** during thread initialization to avoid
    /// runtime heap allocation on the audio hot-path.
    pub fn new() -> Self {
        Self {
            frame: ProcessingFrame::new(),
        }
    }

    /// Executes the primary DSP detection loop for a single frame.
    ///
    /// **Wireframe** — currently a no-op. When implemented:
    ///
    /// 1. Pop samples from Elastic Ring Buffer into `self.frame.audio_buffer`
    /// 2. Execute Scout Engine to route to Bass or Treble using `self.frame.frequency_buffer`
    /// 3. Execute Bass Engine (pYIN) using `self.frame.time_buffer`
    ///    **OR** Treble Engine (QIFFT / DPLL)
    /// 4. Send resulting F0 / confidence to the Gatekeeper
    pub fn process_tick(&mut self) {
        // Wireframe — implementation pending
    }
}

impl Default for Engine {
    fn default() -> Self {
        Self::new()
    }
}
