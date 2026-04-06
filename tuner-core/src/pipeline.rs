//! # Audio Processing Pipeline
//!
//! This module defines the lock-free memory structures, shared state types,
//! and the `AudioPipeline` orchestrator for real-time continuous audio analysis.
//!
//! ## Architecture
//!
//! The pipeline follows the **Split / Handle pattern**:
//!
//! - [`AudioPipeline`] is moved to the audio thread. It owns and mediates all
//!   internal pure DSP components ([`Gatekeeper`], [`Engine`], and the COLA [`CircularFifo`]).
//!   It acts as a zero-allocation data sink via `push_audio()`, orchestrating overlapping
//!   FFT frames transparently.
//!
//! - [`PipelineHandle`] is kept by the frontend (GUI, WASM, etc.). It provides
//!   read/write access to the shared atomic state via `Arc<PipelineAtomics>`.
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
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::FrameOutput;
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
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
pub struct ProcessingFrame {
    /// Holds the raw linear audio samples popped from the Elastic Ring Buffer.
    /// Needs to be up to 8192 samples to support the Bass Engine.
    pub audio_buffer: Box<[f32]>,

    /// A generic time-domain working space (e.g., for the YIN difference function).
    /// Size matches the audio_buffer (8192) to accommodate the Bass Engine.
    pub time_buffer: Box<[f32]>,

    /// A frequency-domain working space for in-place FFT operations.
    /// The Scout and Treble Engines use 2048-sample windows.
    pub frequency_buffer: Box<[Complex<f32>]>,

    /// High-resolution frequency-domain buffer strictly for the 8192-point Bass TWM.
    pub bass_frequency_buffer: Box<[Complex<f32>]>,

    /// Pre-allocated magnitude scratch buffer for the Engine's TWM + XQIFFT chain.
    /// Sized to `BASS_WINDOW_SIZE / 2` (4096 elements) to cover both treble (1024) and bass (4096) paths.
    /// Zero heap allocations on the DSP hot path.
    pub magnitude_buffer: Box<[f32]>,
}

impl ProcessingFrame {
    /// Instantiates a new ProcessingFrame, zeroing out all internal arrays.
    /// This should be called **once** during application startup/thread initialization.
    pub fn new() -> Self {
        Self {
            audio_buffer: vec![0.0; BASS_WINDOW_SIZE].into_boxed_slice(),
            time_buffer: vec![0.0; BASS_WINDOW_SIZE].into_boxed_slice(),
            frequency_buffer: vec![Complex { re: 0.0, im: 0.0 }; WINDOW_SIZE].into_boxed_slice(),
            bass_frequency_buffer: vec![Complex { re: 0.0, im: 0.0 }; BASS_WINDOW_SIZE]
                .into_boxed_slice(),
            magnitude_buffer: vec![0.0; BASS_WINDOW_SIZE / 2].into_boxed_slice(),
        }
    }
}

// ─── Wait-Free Shared State (Atomics) ────────────────────────────────────────

/// Loads an `f32` from an [`AtomicU32`] using bit reinterpretation.
#[inline]
pub fn load_f32(atom: &AtomicU32) -> f32 {
    f32::from_bits(atom.load(Ordering::Relaxed))
}

/// Stores an `f32` into an [`AtomicU32`] using bit reinterpretation.
#[inline]
pub fn store_f32(atom: &AtomicU32, val: f32) {
    atom.store(val.to_bits(), Ordering::Relaxed);
}

/// Loads an `Option<f32>` from an [`AtomicU32`], treating `NaN` as `None`.
///
/// This sentinel works because `NaN` is never a meaningful value for the
/// parameters stored here (frequencies, thresholds, B coefficients).
#[inline]
pub fn load_option_f32(atom: &AtomicU32) -> Option<f32> {
    let val = f32::from_bits(atom.load(Ordering::Relaxed));
    if val.is_nan() { None } else { Some(val) }
}

/// Stores an `Option<f32>` into an [`AtomicU32`], encoding `None` as `NaN`.
#[inline]
pub fn store_option_f32(atom: &AtomicU32, val: Option<f32>) {
    let bits = match val {
        Some(v) => v.to_bits(),
        None => f32::NAN.to_bits(),
    };
    atom.store(bits, Ordering::Relaxed);
}

/// UI-editable configuration parameters. The audio thread reads only.
///
/// Each field is an individual [`AtomicU32`] — wait-free reads with zero
/// risk of priority inversion or lock contention. Replaces the former
/// `Arc<Mutex<ConfigState>>`.
pub struct ConfigAtomics {
    /// Minimum RMS amplitude required to exit the `Silence` state.
    pub silence_threshold: AtomicU32,
    /// NHWRSF threshold required to declare a new transient note event.
    pub nhwrsf_threshold: AtomicU32,
    /// Expected frequency hint provided by GUI keys. `NaN` = `None`.
    pub key_hint: AtomicU32,
    /// Pre-calculated base inharmonicity metric. `NaN` = `None`.
    pub inharmonicity_b: AtomicU32,
}

/// Audio-thread-owned runtime observations. The UI reads only.
///
/// Updated by the pipeline after each frame. The UI polls these atomics
/// (e.g., at 60 FPS during `Message::Tick`) to drive visualisations
/// like the Envelope Viewer.
pub struct RuntimeAtomics {
    /// The current smoothed RMS amplitude (Exponential Moving Average).
    pub current_rms_ema: AtomicU32,
    /// The current signal flux.
    pub current_nhwrsf: AtomicU32,
}

/// Combined wait-free shared state between the DSP thread and the GUI thread.
///
/// Shared via `Arc<PipelineAtomics>` — both threads get a cheap clone.
/// All operations are `Ordering::Relaxed` — sufficient for independent
/// scalar parameters that are not part of a happens-before chain.
pub struct PipelineAtomics {
    /// UI → DSP: configuration parameters (silence threshold, key hint, etc.).
    pub config: ConfigAtomics,
    /// DSP → UI: runtime observations (RMS, NHWRSF).
    pub runtime: RuntimeAtomics,
    /// UI → DSP: shutdown signal. The audio thread checks this every loop iteration.
    pub shutdown: AtomicBool,
}

impl Default for PipelineAtomics {
    fn default() -> Self {
        Self {
            config: ConfigAtomics {
                silence_threshold: AtomicU32::new(0.005_f32.to_bits()),
                nhwrsf_threshold: AtomicU32::new(0.9_f32.to_bits()),
                key_hint: AtomicU32::new(f32::NAN.to_bits()),
                inharmonicity_b: AtomicU32::new(f32::NAN.to_bits()),
            },
            runtime: RuntimeAtomics {
                current_rms_ema: AtomicU32::new(0.0_f32.to_bits()),
                current_nhwrsf: AtomicU32::new(0.0_f32.to_bits()),
            },
            shutdown: AtomicBool::new(false),
        }
    }
}

// ─── AudioPipeline (Mediator) ────────────────────────────────────────────────

/// The orchestrator that coordinates all DSP components on the audio thread.
///
/// `AudioPipeline` owns the pure DSP components (like [`Gatekeeper`]) and
/// reads/writes the shared [`PipelineAtomics`] for parameter and observation
/// exchange with the frontend.
///
/// Created via [`AudioPipeline::new()`], which returns both the pipeline
/// (moved to the audio thread) and a [`PipelineHandle`] (kept by the frontend).
pub struct AudioPipeline {
    /// The Gatekeeper — pure DSP, evaluates signal stability.
    pub gatekeeper: Gatekeeper,
    /// The Engine — F0 detection chain
    pub engine: Engine,

    // Wait-free shared state
    atomics: Arc<PipelineAtomics>,

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
/// Provides `Arc<PipelineAtomics>` for wait-free reads and writes.
#[derive(Clone)]
pub struct PipelineHandle {
    /// Shared atomic state — the frontend reads runtime observations and
    /// writes configuration parameters.
    pub atomics: Arc<PipelineAtomics>,
}

impl Default for PipelineHandle {
    fn default() -> Self {
        Self {
            atomics: Arc::new(PipelineAtomics::default()),
        }
    }
}

impl std::fmt::Debug for PipelineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineHandle")
            .field(
                "silence_threshold",
                &load_f32(&self.atomics.config.silence_threshold),
            )
            .field("rms_ema", &load_f32(&self.atomics.runtime.current_rms_ema))
            .finish()
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
        let atomics = Arc::new(PipelineAtomics::default());

        let gatekeeper = Gatekeeper::new(Arc::clone(&audio_pool));
        let engine = Engine::new(44100);

        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft_instance = planner.plan_fft_forward(WINDOW_SIZE);

        let pipeline = Self {
            gatekeeper,
            engine,
            atomics: Arc::clone(&atomics),
            audio_pool,
            cola: CircularFifo::new(BASS_WINDOW_SIZE),
            fft_instance,
            processing_frame: ProcessingFrame::new(),
        };

        let handle = PipelineHandle { atomics };

        (pipeline, handle)
    }

    /// Pushes new raw audio samples directly into the internal COLA FIFO.
    ///
    /// Returns `Some` containing the DSP results IF a full hop boundary was reached.
    /// The returned `FrameOutput` is a fixed-size struct ready for the triple buffer.
    pub fn push_audio(&mut self, samples: &[f32]) -> Option<FrameOutput> {
        self.cola.push_samples(samples);

        if self.cola.is_hop_ready(HOP_SIZE) {
            self.consume_cola_hop()
        } else {
            None
        }
    }

    /// Internal helper that processes a single hop of audio data pulled from the COLA.
    fn consume_cola_hop(&mut self) -> Option<FrameOutput> {
        // Read the FULL history of audio out of the sliding queue
        self.cola.read_window(
            BASS_WINDOW_SIZE,
            &mut self.processing_frame.audio_buffer[..BASS_WINDOW_SIZE],
        );

        // Populate the frame's generic frequency buffer in place
        // The newest WINDOW_SIZE samples are at the END of the buffer
        let newest_start = BASS_WINDOW_SIZE - WINDOW_SIZE;
        crate::algorithms::spectral::perform_fft(
            &self.processing_frame.audio_buffer[newest_start..BASS_WINDOW_SIZE],
            &mut self.processing_frame.time_buffer[..WINDOW_SIZE],
            &mut self.processing_frame.frequency_buffer[..],
            &self.fft_instance,
            WINDOW_SIZE,
        );

        self.cola.acknowledge_hop(HOP_SIZE);

        // Run the rest of the DSP pipeline
        let engine_result = self.process_frame_internal();

        // Build fixed-size FrameOutput — zero heap allocations
        let mut frame_output = FrameOutput::default();
        let mag_count = WINDOW_SIZE / 2;
        crate::algorithms::spectral::spectrum_to_magnitudes(
            &self.processing_frame.frequency_buffer[..],
            WINDOW_SIZE,
            &mut frame_output.magnitudes[..mag_count],
        );
        frame_output.magnitude_len = mag_count;
        frame_output.rms_ema = self.gatekeeper.current_rms_ema;
        frame_output.nhwrsf = self.gatekeeper.current_nhwrsf;

        if let Some((freq, conf)) = engine_result {
            let note_index = crate::models::find_nearest_note_index(freq);
            let (_, target_freq) = crate::models::find_nearest_note_by_index(note_index);
            let cents = crate::algorithms::tuning::calculate_cents_deviation(freq, target_freq);

            frame_output.detected_frequency = Some(freq);
            frame_output.confidence = conf;
            frame_output.note_index = Some(note_index);
            frame_output.cents_deviation = Some(cents);
        }

        Some(frame_output)
    }

    /// Internal method to run the DSP pipeline on the populated `processing_frame`.
    fn process_frame_internal(&mut self) -> Option<(f32, Option<f32>)> {
        // 1. Read GUI-set configs into the Gatekeeper and Engine (wait-free)
        self.gatekeeper.config.silence_threshold = load_f32(&self.atomics.config.silence_threshold);
        self.gatekeeper.config.nhwrsf_threshold = load_f32(&self.atomics.config.nhwrsf_threshold);
        self.engine.key_hint = load_option_f32(&self.atomics.config.key_hint);
        self.engine.inharmonicity_b = load_option_f32(&self.atomics.config.inharmonicity_b);

        // 2. Pure DSP — Gatekeeper evaluates signal stability
        self.gatekeeper.process_frame(&self.processing_frame);

        // 3. Sync runtime observations to shared atomics for the frontend
        store_f32(
            &self.atomics.runtime.current_rms_ema,
            self.gatekeeper.current_rms_ema,
        );
        store_f32(
            &self.atomics.runtime.current_nhwrsf,
            self.gatekeeper.current_nhwrsf,
        );

        // 4. Run the Engine to extract fundamental frequency
        let is_silence = self.gatekeeper.current_state == SignalState::Silence;
        let is_new_onset = self.gatekeeper.is_new_onset;

        self.engine
            .process(&mut self.processing_frame, is_silence, is_new_onset)
    }
}
