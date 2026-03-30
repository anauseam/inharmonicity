//! # Audio Processing Pipeline
//!
//! This module defines the lock-free memory structures, shared state types,
//! and the `AudioPipeline` orchestrator for real-time audio processing.
//!
//! ## Architecture
//!
//! The pipeline follows the **Split / Handle pattern**:
//!
//! - [`AudioPipeline`] is moved to the audio thread. It owns the pure DSP
//!   components ([`Gatekeeper`]) and coordinates them, syncing observations
//!   to shared state after each frame.
//!
//! - [`PipelineHandle`] is kept by the frontend (GUI, WASM, etc.). It provides
//!   read/write access to the shared state via `Arc<Mutex<...>>`.
//!
//! ```text
//! AudioPipeline::new() -> (AudioPipeline, PipelineHandle)
//!       │                         │
//!       ▼                         ▼
//!   Audio Thread              GUI Thread
//! ```

use crossbeam_queue::ArrayQueue;
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::sync::{Arc, Mutex};

use crate::audio;
use crate::cola::CircularFifo;

use crate::engine::Engine;
use crate::gatekeeper::{Gatekeeper, SignalState};

// ─── Memory Infrastructure ───────────────────────────────────────────────────

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
pub(crate) struct ProcessingFrame {
    /// Holds the raw linear audio samples popped from the Elastic Ring Buffer.
    /// Needs to be up to 8192 samples to support the Bass Engine.
    pub audio_buffer: Box<[f32]>,

    /// A generic time-domain working space (e.g., for the YIN difference function).
    /// Size matches the audio_buffer (8192) to accommodate the Bass Engine.
    pub time_buffer: Box<[f32]>,

    /// A frequency-domain working space for in-place FFT operations.
    /// The Scout and Treble Engines use 2048-sample windows.
    pub frequency_buffer: Box<[Complex<f32>]>,
}

impl ProcessingFrame {
    /// Instantiates a new ProcessingFrame, zeroing out all internal arrays.
    /// This should be called **once** during application startup/thread initialization.
    pub fn new() -> Self {
        Self {
            audio_buffer: vec![0.0; 8192].into_boxed_slice(),
            time_buffer: vec![0.0; 8192].into_boxed_slice(),
            frequency_buffer: vec![Complex { re: 0.0, im: 0.0 }; 2048].into_boxed_slice(),
        }
    }
}

// ─── Shared State Types ──────────────────────────────────────────────────────

/// Startup/UI-editable configuration. The audio thread reads only.
///
/// These values are initialized at application startup (e.g., via noise floor
/// calibration) and can be adjusted by the user through the Settings UI.
/// The audio/processing thread reads them each frame to gate its behavior.
#[derive(Debug, Clone)]
pub struct ConfigState {
    /// Minimum RMS amplitude required to exit the `Silence` state.
    pub silence_threshold: f32,
    /// NHWRSF threshold required to declare a new transient note event.
    pub nhwrsf_threshold: f32,
}

impl Default for ConfigState {
    fn default() -> Self {
        Self {
            silence_threshold: 0.005,
            nhwrsf_threshold: 0.5,
        }
    }
}

/// Audio-thread-owned observations. The UI reads only.
///
/// These values are updated by the pipeline after each frame.
/// The UI polls them (e.g., at 60 FPS during `Message::Tick`) to drive
/// visualizations like the Envelope Viewer.
#[derive(Debug, Clone)]
pub struct RuntimeState {
    /// The current smoothed RMS amplitude (Exponential Moving Average).
    pub current_rms_ema: f32,
    /// The current signal flux
    pub current_nhwrsf: f32,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            current_rms_ema: 0.0,
            current_nhwrsf: 0.0,
        }
    }
}

/// Thread-safe handle to `ConfigState`. Frontend writes, audio thread reads.
pub type SharedConfigState = Arc<Mutex<ConfigState>>;

/// Thread-safe handle to `RuntimeState`. Audio thread writes, frontend reads.
pub type SharedRuntimeState = Arc<Mutex<RuntimeState>>;

// ─── AudioPipeline (Mediator) ────────────────────────────────────────────────

/// The orchestrator that coordinates all DSP components on the audio thread.
///
/// `AudioPipeline` owns the pure DSP components (like [`Gatekeeper`]) and
/// is the **only** thing that touches `Arc<Mutex<...>>` shared state.
/// After each frame, it reads the DSP components' public fields and syncs
/// the relevant observations to shared state for the frontend to poll.
///
/// Created via [`AudioPipeline::new()`], which returns both the pipeline
/// (moved to the audio thread) and a [`PipelineHandle`] (kept by the frontend).
pub struct AudioPipeline {
    /// The Gatekeeper — pure DSP, evaluates signal stability.
    pub gatekeeper: Gatekeeper,
    /// The Engine — F0 detection chain
    pub engine: Engine,

    // Shared state bridges (pipeline ↔ frontend)
    shared_config: SharedConfigState,
    shared_runtime: SharedRuntimeState,

    // Memory infrastructure
    #[allow(dead_code)] // To be utilized upon full implementation
    audio_pool: Arc<AudioPool>,

    // Internal COLA State
    cola: CircularFifo,
    fft_instance: Arc<dyn RealToComplex<f32>>,
    processing_frame: ProcessingFrame,
}

/// Frontend-side handle to the pipeline's shared state.
///
/// Returned by [`AudioPipeline::new()`] and kept by the frontend (GUI, WASM, etc.).
/// Provides `Arc<Mutex<...>>` handles for polling runtime observations and
/// editing configuration values.
#[derive(Debug)]
pub struct PipelineHandle {
    /// Shared configuration state — the frontend can **read and write** this
    /// (e.g., to adjust the silence threshold from the Settings UI).
    pub config: SharedConfigState,

    /// Shared runtime state — the frontend can **read** this
    /// (e.g., to poll the current RMS for the Envelope Viewer).
    pub runtime: SharedRuntimeState,
}

impl Default for PipelineHandle {
    fn default() -> Self {
        Self {
            config: Arc::new(Mutex::new(ConfigState::default())),
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
        }
    }
}

impl AudioPipeline {
    /// Creates a new AudioPipeline and its corresponding [`PipelineHandle`].
    ///
    /// This follows the **Split / Handle pattern**: the `AudioPipeline` is moved
    /// to the audio thread, and the `PipelineHandle` is kept by the frontend.
    ///
    /// # Returns
    /// A tuple of `(AudioPipeline, PipelineHandle)`.
    pub fn new() -> (Self, PipelineHandle) {
        let audio_pool = Arc::new(ArrayQueue::new(4));
        let shared_config = Arc::new(Mutex::new(ConfigState::default()));
        let shared_runtime = Arc::new(Mutex::new(RuntimeState::default()));

        let gatekeeper = Gatekeeper::new(Arc::clone(&audio_pool));
        let engine = Engine::new(44100);

        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft_instance = planner.plan_fft_forward(crate::audio::WINDOW_SIZE);

        let pipeline = Self {
            gatekeeper,
            engine,
            shared_config: Arc::clone(&shared_config),
            shared_runtime: Arc::clone(&shared_runtime),
            audio_pool,
            cola: CircularFifo::new(audio::WINDOW_SIZE * 4), // Capacity large enough to hold the 8192 points needed for bass F0
            fft_instance,
            processing_frame: ProcessingFrame::new(),
        };

        let handle = PipelineHandle {
            config: shared_config,
            runtime: shared_runtime,
        };

        (pipeline, handle)
    }

    /// Pushes new raw audio samples directly into the internal COLA FIFO.
    /// Returns `Some` containing the DSP results IF a full hop boundary was reached.
    pub fn push_audio(
        &mut self,
        samples: &[f32],
    ) -> Option<(Option<(f32, Option<f32>)>, std::vec::Vec<f32>)> {
        self.cola.push_samples(samples);

        if self.cola.is_hop_ready(crate::audio::HOP_SIZE) {
            self.consume_cola_hop()
        } else {
            None
        }
    }

    /// Internal helper that processes a single hop of audio data pulled from the COLA.
    fn consume_cola_hop(&mut self) -> Option<(Option<(f32, Option<f32>)>, std::vec::Vec<f32>)> {
        // Read next window of audio out of the sliding queue
        self.cola.read_window(
            crate::audio::WINDOW_SIZE,
            &mut self.processing_frame.audio_buffer[..crate::audio::WINDOW_SIZE],
        );

        // Populate the frame's frequency buffer in place
        crate::algorithms::spectral::perform_fft(
            &self.processing_frame.audio_buffer[..crate::audio::WINDOW_SIZE],
            &mut self.processing_frame.time_buffer,
            &mut self.processing_frame.frequency_buffer[..],
            &self.fft_instance,
            crate::audio::WINDOW_SIZE,
        );

        self.cola.acknowledge_hop(crate::audio::HOP_SIZE);

        // Run the rest of the DSP pipeline
        let engine_result = self.process_frame_internal();

        let spectrogram = crate::algorithms::spectral::spectrum_to_magnitudes(
            &self.processing_frame.frequency_buffer[..],
            crate::audio::WINDOW_SIZE,
        );

        Some((engine_result, spectrogram))
    }

    /// Internal method to run the DSP pipeline on the populated `processing_frame`.
    fn process_frame_internal(&mut self) -> Option<(f32, Option<f32>)> {
        // 1. Read GUI-set configs into the Gatekeeper
        if let Ok(config) = self.shared_config.try_lock() {
            self.gatekeeper.config.silence_threshold = config.silence_threshold;
            self.gatekeeper.config.nhwrsf_threshold = config.nhwrsf_threshold;
        }

        // 2. Pure DSP — Gatekeeper evaluates signal stability
        self.gatekeeper.process_frame(&self.processing_frame);

        // 3. Sync runtime observations to shared state for the frontend
        if let Ok(mut runtime) = self.shared_runtime.try_lock() {
            runtime.current_rms_ema = self.gatekeeper.current_rms_ema;
            runtime.current_nhwrsf = self.gatekeeper.current_nhwrsf;
        }

        // 4. Run the Engine to extract fundamental frequency
        let is_silence = self.gatekeeper.current_state == SignalState::Silence;
        let is_new_onset = self.gatekeeper.is_new_onset;

        self.engine
            .process(&mut self.processing_frame, is_silence, is_new_onset)
    }
}
