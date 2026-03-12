//! # Background Worker (Thread 3) — Heavy Offline DSP
//!
//! The "Heavy Lifter" of the pipeline. A single dedicated background thread
//! that receives filled 1.5-second audio captures from the Gatekeeper (via the
//! [`AudioPool`](crate::pipeline::AudioPool)) and performs computationally
//! expensive offline DSP, such as extracting up to 32 partials and calculating
//! the inharmonicity constant ($B$).
//!
//! ## Why a Single Thread?
//!
//! Captures are infrequent (one stable note at a time, triggered by the Gatekeeper's
//! State 4 RELEASE). The MAT / ICF algorithms are fast enough to complete well before
//! the next capture could arrive, so a single dedicated thread avoids the overhead
//! of a full thread pool.
//!
//! ## Wireframe Status
//!
//! This module is currently a **wireframe**. The `WorkerManager` struct holds an
//! `Arc<AudioPool>` reference and exposes a `start_workers()` stub. When implemented:
//!
//! 1. Spawn a single background thread that blocks on a crossbeam receiver
//! 2. Receive a `Box<[f32; 66150]>` buffer (1.5s at 44.1kHz) from the Gatekeeper
//! 3. Run MAT (professional) or ICF (educational) to compute the $B$ coefficient
//! 4. Send the result to the UI thread
//! 5. Recycle the buffer back to the `AudioPool`

use crate::pipeline::AudioPool;
use std::sync::Arc;

/// Manages the lifecycle of the background worker thread.
///
/// The `WorkerManager` owns an `Arc<AudioPool>` so it can return processed buffers
/// back to the pool after the heavy DSP is complete. Currently a wireframe.
pub struct WorkerManager {
    /// Shared reference to the lock-free object pool for buffer recycling.
    #[allow(dead_code)] // Wireframe — will be used when worker thread is implemented
    audio_pool: Arc<AudioPool>,
}

impl WorkerManager {
    /// Creates a new `WorkerManager` bound to the given `AudioPool`.
    ///
    /// # Arguments
    /// * `audio_pool` — Shared reference to the lock-free object pool.
    pub fn new(audio_pool: Arc<AudioPool>) -> Self {
        Self { audio_pool }
    }

    /// Spawns the background worker thread.
    ///
    /// **Wireframe** — currently a no-op. When implemented, this will:
    ///
    /// 1. Spawn a single `std::thread` that blocks on a crossbeam receiver
    /// 2. On receipt of a buffer: run MAT or ICF → send B coefficient → recycle buffer
    pub fn start_workers(&self) {
        // Wireframe — implementation pending
    }
}
