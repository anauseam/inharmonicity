//! # Audio Processing Pipeline
//!
//! This module defines the lock-free memory structures and object pools required
//! for real-time audio processing. It eliminates dynamic heap allocations on the
//! audio thread during continuous processing.

use crossbeam_queue::ArrayQueue;
use rustfft::num_complex::Complex;

/// A lock-free Object Pool for 2-second audio captures.
///
/// Thread 2 (The Brains) borrows a pre-allocated array from this pool
/// when the Gatekeeper triggers. Once filled with 1.5 seconds of audio (66,150 samples),
/// the array is dispatched to Thread 3 (Background Worker) for heavy DSP
/// (like inharmonicity mapping). Thread 3 returns the array to the pool when finished.
pub type AudioPool = ArrayQueue<Box<[f32; 66150]>>;

/// Thread-Local Scratch Buffers for Thread 2 (The F0 Engine).
///
/// This structure holds statically-sized, pre-allocated working arrays.
/// It is meant to be owned by Thread 2 and reused every frame to perform
/// continuous fundamental frequency ($f_0$) detection without ever calling
/// `Vec::new()` or `Vec::push()`.
pub struct ProcessingFrame {
    /// Holds the raw linear audio samples popped from the Elastic Ring Buffer.
    /// Needs to be up to 8192 samples to support the Bass Engine.
    pub audio_buffer: [f32; 8192],

    /// A generic time-domain working space (e.g., for the YIN difference function).
    /// Size matches the audio_buffer (8192) to accommodate the Bass Engine.
    pub time_buffer: [f32; 8192],

    /// A frequency-domain working space for in-place FFT operations.
    /// The Scout and Treble Engines use 2048-sample windows.
    pub frequency_buffer: [Complex<f32>; 2048],
}

impl ProcessingFrame {
    /// Instantiates a new ProcessingFrame, zeroing out all internal arrays.
    /// This should be called **once** during application startup/thread initialization.
    pub fn new() -> Self {
        Self {
            audio_buffer: [0.0; 8192],
            time_buffer: [0.0; 8192],
            frequency_buffer: [Complex { re: 0.0, im: 0.0 }; 2048],
        }
    }
}
