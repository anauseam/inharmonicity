//! # Background Workers (Thread 3)
//!
//! The "Heavy Lifters" of the pipeline. A thread pool that receives filled 2-second
//! audio captures from the Gatekeeper and performs intense offline DSP, such as
//! extracting up to 32 partials and calculating the $B$ inharmonicity constant.

use crate::pipeline::AudioPool;
use std::sync::Arc;

/// Wireframe for the Background Worker Pool manager
pub struct WorkerManager {
    audio_pool: Arc<AudioPool>,
}

impl WorkerManager {
    pub fn new(audio_pool: Arc<AudioPool>) -> Self {
        Self { audio_pool }
    }

    /// Spawns the background threads to wait for payloads from the Gatekeeper
    pub fn start_workers(&self) {
        // Spawn threads that block on a crossbeam receiver waiting for Box<[f32; 88200]>
        // Process the buffer (Partials + B constant)
        // Return the buffer to the audio_pool
    }
}
