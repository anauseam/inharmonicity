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

use crossbeam_channel::{Receiver, Sender, bounded};
use crossbeam_queue::ArrayQueue;
use realfft::RealToComplex;
use rustfft::num_complex::Complex;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use crate::FrameOutput;
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE, WINDOW_SIZE};
use crate::cola::CircularFifo;

use crate::engine::Engine;
use crate::gatekeeper::Gatekeeper;

// ─── Memory Infrastructure ───────────────────────────────────────────────────

// Buffer capacity for non-causal pre-roll.
// 32,768 samples provides ~743ms of history at 44.1kHz, safely accommodating
// the 15-frame (15,360 sample) pre-roll requirement.
const ONSET_HISTORY_SAMPLES: usize = 32768;

/// Payload dispatched from the pipeline to the Worker thread.
pub struct CapturePayload {
    /// 1.5 seconds of high-resolution overlap-added buffer content.
    pub stable_buffer: Box<[f32; 66150]>,
    /// Number of valid samples written to the stable buffer.
    pub stable_sample_count: usize,
    /// Purely diagnostic buffer containing the full acoustic event (pre-roll, strike, and decay).
    /// This is written to disk for analysis tooling (`diagnose_engine.rs`) and is NEVER
    /// fed back into the Engine or MAT algorithms.
    pub full_event_buffer: Option<Box<[f32; 66150]>>,
    /// Number of valid samples written to the full event buffer.
    pub full_event_sample_count: usize,
    /// The target note index the UI requested, or 255 for Auto.
    pub target_note: u8,
    /// Fixed sampling rate of the pipeline.
    pub sample_rate: u32,
    /// Calibrated noise floor for the capture.
    pub noise_floor: f32,
    /// Highly accurate unified Goertzel seed for MAT (None if tracking failed)
    pub measured_f0: Option<f32>,
}

/// Capture lifecycle state, communicated via AtomicU8.
///
/// Uses a baton-pass pattern — three threads each own a distinct transition:
///   - **GUI thread** writes `Idle → Armed` (arming) and `Armed → Idle` (cancel).
///   - **DSP pipeline** writes `Armed → Recording` and `Recording → Processing`.
///   - **Worker thread** writes `Processing → Idle` (completion).
///
/// Full lifecycle: Idle → Armed → Recording → Processing → Idle
#[derive(Debug, Clone, PartialEq, Eq)]
#[repr(u8)]
pub enum CaptureState {
    Idle = 0,
    Armed = 1,
    Recording = 2,
    Processing = 3,
}

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

    /// Pre-allocated 1024-bin magnitude scratch buffer for Stage 1 (and the GUI spectrogram).
    pub treble_magnitude_buffer: Box<[f32]>,

    /// Pre-allocated 4096-bin magnitude scratch buffer strictly for Stage 2 (Bass localization)
    pub bass_magnitude_buffer: Box<[f32]>,
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
            treble_magnitude_buffer: vec![0.0; WINDOW_SIZE / 2].into_boxed_slice(),
            bass_magnitude_buffer: vec![0.0; BASS_WINDOW_SIZE / 2].into_boxed_slice(),
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
/// Each field is an individual [`AtomicU32`] or [`AtomicU8`] — wait-free reads with zero
/// risk of priority inversion or lock contention. Replaces the former
/// `Arc<Mutex<ConfigState>>`.
pub struct ConfigAtomics {
    /// Minimum RMS amplitude required to exit the `Silence` state.
    pub silence_threshold: AtomicU32,
    /// NHWRSF threshold required to declare a new transient note event.
    pub nhwrsf_threshold: AtomicU32,
    /// NINOS2 threshold required to declare a stable harmonic sustain.
    pub ninos2_stability_threshold: AtomicU32,
    /// Pre-calculated base inharmonicity metric. `NaN` = `None`.
    pub inharmonicity_b: AtomicU32,
    /// GUI → Pipeline: Unison target selection. Indicates the 0-87 key index
    /// the user currently has selected in the UI. 255 represents 'Auto'.
    pub target_note: AtomicU8,
}

/// Audio-thread-owned runtime observations. Framework consumers read only.
///
/// Updated by the pipeline after each frame. Framework consumers can poll these
/// atomics from multiple independent threads simultaneously without breaking the
/// SPSC constraint of the primary `FrameOutput` triple buffer.
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
    /// UI → DSP: configuration parameters (silence threshold, target key, etc.).
    pub config: ConfigAtomics,
    /// DSP → UI/Consumers: runtime observations (RMS, NHWRSF).
    pub runtime: RuntimeAtomics,
    /// UI → DSP: shutdown signal. The audio thread checks this every loop iteration.
    pub shutdown: AtomicBool,
    /// Bidirectional capture lifecycle state.
    /// GUI writes `Armed`, Pipeline writes `Recording`/`Processing`, Worker writes `Idle`.
    pub capture_state: AtomicU8,
}

impl Default for PipelineAtomics {
    fn default() -> Self {
        Self {
            config: ConfigAtomics {
                silence_threshold: AtomicU32::new(0.005_f32.to_bits()),
                nhwrsf_threshold: AtomicU32::new(0.9_f32.to_bits()),
                ninos2_stability_threshold: AtomicU32::new(10.0_f32.to_bits()),
                inharmonicity_b: AtomicU32::new(f32::NAN.to_bits()),
                target_note: AtomicU8::new(255), // Default to Auto
            },
            runtime: RuntimeAtomics {
                current_rms_ema: AtomicU32::new(0.0_f32.to_bits()),
                current_nhwrsf: AtomicU32::new(0.0_f32.to_bits()),
            },
            shutdown: AtomicBool::new(false),
            capture_state: AtomicU8::new(CaptureState::Idle as u8),
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
    fft_bass_instance: Arc<dyn RealToComplex<f32>>,
    processing_frame: ProcessingFrame,

    // Worker Thread Dispatch
    pub capture_tx: Sender<CapturePayload>,

    // Capture Accumulation State
    capture_buffer: Option<Box<[f32; 66150]>>,
    capture_count: usize,
    /// Parallel accumulator for the diagnostic `full_event_buffer`.
    full_event_buffer: Option<Box<[f32; 66150]>>,
    full_event_count: usize,
    /// Continuous circular history of the raw audio stream.
    /// Maintained strictly to provide non-causal pre-roll for diagnostic captures.
    history_buffer: Box<[f32; ONSET_HISTORY_SAMPLES]>,
    history_idx: usize,
    /// Latches 'true' when a new onset is detected while Armed.
    /// Reset to 'false' upon capture start or Silence.
    capture_onset_pending: bool,
    /// Latched fundamental frequency for Auto-Mode dispatch validation
    latched_auto_key: Option<u8>,
    /// Last measured physical frequency from the Engine
    pub last_measured_f0: Option<f32>,
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
    /// Receiver for worker thread results.
    pub result_rx: Receiver<crate::models::KeyMeasurement>,
}

impl Default for PipelineHandle {
    fn default() -> Self {
        let (_, dummy_rx) = crossbeam_channel::bounded(0);
        Self {
            atomics: Arc::new(PipelineAtomics::default()),
            result_rx: dummy_rx,
        }
    }
}

impl std::fmt::Debug for PipelineHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PipelineHandle")
            .field(
                "silence_threshold",
                &crate::pipeline::load_f32(&self.atomics.config.silence_threshold),
            )
            .field(
                "rms_ema",
                &crate::pipeline::load_f32(&self.atomics.runtime.current_rms_ema),
            )
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
        let audio_pool = Arc::new(ArrayQueue::new(8));
        // Pre-fill pool
        for _ in 0..8 {
            let _ = audio_pool.push(Box::new([0.0; 66150]));
        }

        let atomics = Arc::new(PipelineAtomics::default());

        let gatekeeper = Gatekeeper::new(Arc::clone(&audio_pool));
        let engine = Engine::new(44100);

        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft_instance = planner.plan_fft_forward(WINDOW_SIZE);
        let fft_bass_instance = planner.plan_fft_forward(BASS_WINDOW_SIZE);

        let (capture_tx, capture_rx) = bounded(2);
        let (result_tx, result_rx) = bounded(4);

        crate::worker::WorkerManager::new(
            Arc::clone(&audio_pool),
            Arc::clone(&atomics),
            capture_rx,
            result_tx,
        )
        .start_workers();

        let pipeline = Self {
            gatekeeper,
            engine,
            atomics: Arc::clone(&atomics),
            audio_pool,
            cola: CircularFifo::new(BASS_WINDOW_SIZE),
            fft_instance,
            fft_bass_instance,
            processing_frame: ProcessingFrame::new(),
            capture_tx,
            capture_buffer: None,
            capture_count: 0,
            full_event_buffer: None,
            full_event_count: 0,
            history_buffer: vec![0.0f32; ONSET_HISTORY_SAMPLES]
                .into_boxed_slice()
                .try_into()
                .unwrap(),
            history_idx: 0,
            capture_onset_pending: false,
            latched_auto_key: None,
            last_measured_f0: None,
        };

        let handle = PipelineHandle { atomics, result_rx };

        (pipeline, handle)
    }

    /// Pushes new raw audio samples directly into the internal COLA FIFO.
    ///
    /// Returns `Some` containing the DSP results IF a full hop boundary was reached.
    /// The returned `FrameOutput` is a fixed-size struct ready for the triple buffer.
    pub fn push_audio(&mut self, samples: &[f32]) -> Option<FrameOutput> {
        self.cola.push_samples(samples);

        if self.cola.is_hop_ready(HOP_SIZE) {
            self.process_cola_hop()
        } else {
            None
        }
    }

    /// Internal helper that processes a single hop of audio data pulled from the COLA.
    fn process_cola_hop(&mut self) -> Option<FrameOutput> {
        // ─── Step 1: COLA & Windowing ───

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

        crate::algorithms::spectral::perform_fft(
            &self.processing_frame.audio_buffer[..BASS_WINDOW_SIZE],
            &mut self.processing_frame.time_buffer[..BASS_WINDOW_SIZE],
            &mut self.processing_frame.bass_frequency_buffer[..],
            &self.fft_bass_instance,
            BASS_WINDOW_SIZE,
        );

        self.cola.acknowledge_hop(HOP_SIZE);

        // --- Synchronous History Accumulator ---
        // Perfectly aligned with the DSP clock to prevent OS buffer chunk misalignment
        // Placed AFTER read_window() so the audio_buffer contains the freshest data
        let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
        let new_samples = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];
        for &s in new_samples {
            self.history_buffer[self.history_idx] = s;
            self.history_idx = (self.history_idx + 1) % self.history_buffer.len();
        }

        // ─── Step 2: Read Shared Atomics ───

        self.gatekeeper.config.silence_threshold = load_f32(&self.atomics.config.silence_threshold);
        self.gatekeeper.config.nhwrsf_threshold = load_f32(&self.atomics.config.nhwrsf_threshold);
        self.gatekeeper.config.ninos2_stability_threshold =
            load_f32(&self.atomics.config.ninos2_stability_threshold);
        self.engine.noise_floor = load_f32(&self.atomics.config.silence_threshold);

        let target_note = match self.atomics.config.target_note.load(Ordering::Relaxed) {
            255 => None,
            val if val < 88 => Some(val), // Bounds Safety
            _ => None,
        };

        // ─── Step 3: Signal Gating (Gatekeeper) ───

        // Pure DSP — Gatekeeper evaluates signal stability and returns result
        let gate_result = self.gatekeeper.process_frame(&self.processing_frame);

        // Sync runtime observations to shared atomics for framework consumers
        store_f32(&self.atomics.runtime.current_rms_ema, gate_result.rms_ema);
        store_f32(&self.atomics.runtime.current_nhwrsf, gate_result.nhwrsf);

        // ─── Step 4: Treble Magnitude Extraction ───

        let mag_count = WINDOW_SIZE / 2;
        crate::algorithms::spectral::spectrum_to_magnitudes(
            &self.processing_frame.frequency_buffer[..],
            WINDOW_SIZE,
            &mut self.processing_frame.treble_magnitude_buffer[..mag_count],
        );

        let mag_count_bass = BASS_WINDOW_SIZE / 2;
        crate::algorithms::spectral::spectrum_to_magnitudes(
            &self.processing_frame.bass_frequency_buffer[..],
            BASS_WINDOW_SIZE,
            &mut self.processing_frame.bass_magnitude_buffer[..mag_count_bass],
        );

        // ─── Step 5: Pitch Detection (Engine) ───

        let is_silence = gate_result.state == crate::gatekeeper::SignalState::Silence;
        if gate_result.is_new_onset {
            self.last_measured_f0 = None;
        }

        let pitch_result = self.engine.process(
            &mut self.processing_frame,
            is_silence,
            gate_result.is_new_onset,
            gate_result.is_transient_bypass,
            target_note,
        );

        // ─── Step 6: Capture Accumulation & Worker Dispatch ───

        let current_capture_state = self.atomics.capture_state.load(Ordering::Relaxed);

        if gate_result.state == crate::gatekeeper::SignalState::Silence {
            self.capture_onset_pending = false;
            // Proactively recover diagnostic buffer on false transients
            if current_capture_state == CaptureState::Armed as u8 {
                if let Some(dbuf) = self.full_event_buffer.take() {
                    let _ = self.audio_pool.push(dbuf);
                }
                self.full_event_count = 0;
            }
        }

        // ─── MUST Split the original else-if chain into two if blocks here ───

        if current_capture_state == CaptureState::Armed as u8 {
            if gate_result.is_new_onset {
                self.capture_onset_pending = true;
                // Prevent memory leak if an old diagnostic buffer was abandoned (e.g. decayed to silence)
                if let Some(old_buf) = self.full_event_buffer.take() {
                    let _ = self.audio_pool.push(old_buf);
                }
                self.full_event_count = 0; // Unconditionally reset stale state

                // Grab non-causal pre-roll from history for the diagnostic buffer
                if let Some(mut buf) = self.audio_pool.pop() {
                    let pre_roll_samples = 15 * HOP_SIZE; // 15360 samples (~348ms)
                    let hist_len = self.history_buffer.len();
                    for i in 0..pre_roll_samples {
                        let idx = (self.history_idx + hist_len - pre_roll_samples - HOP_SIZE + i)
                            % hist_len;
                        buf[i] = self.history_buffer[idx];
                    }
                    self.full_event_buffer = Some(buf);
                    self.full_event_count = pre_roll_samples;
                }
            }

            if self.capture_onset_pending
                && gate_result.state == crate::gatekeeper::SignalState::Stable
                && let Some(buf) = self.audio_pool.pop()
            {
                self.capture_onset_pending = false;
                self.capture_buffer = Some(buf);
                self.capture_count = 0;
                self.atomics
                    .capture_state
                    .store(CaptureState::Recording as u8, Ordering::Relaxed);
            }
        }

        // --- Diagnostic Accumulator ---
        // Accumulate the full event buffer globally. This runs unconditionally
        // AFTER the initialization block to ensure the very first frame of the onset is captured seamlessly.
        // This audio is solely for CLI diagnostics and is isolated from the live Engine.
        if let Some(mut buf) = self.full_event_buffer.take() {
            let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
            let src_slice = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];
            let remaining = 66150 - self.full_event_count;
            let to_copy = src_slice.len().min(remaining);
            buf[self.full_event_count..self.full_event_count + to_copy]
                .copy_from_slice(&src_slice[..to_copy]);
            self.full_event_count += to_copy;
            self.full_event_buffer = Some(buf);
        }

        if current_capture_state == CaptureState::Recording as u8 {
            // ── Latch ──
            if let Some(ref result) = pitch_result {
                self.latched_auto_key = Some(result.key_index);
                self.last_measured_f0 = result.measured_f0;
            }

            if let Some(mut buf) = self.capture_buffer.take() {
                let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
                let src_slice = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];

                let remaining = 66150 - self.capture_count;
                let to_copy = src_slice.len().min(remaining);

                buf[self.capture_count..self.capture_count + to_copy]
                    .copy_from_slice(&src_slice[..to_copy]);

                self.capture_count += to_copy;

                let done = self.capture_count == 66150;
                let decayed = gate_result.state == crate::gatekeeper::SignalState::Silence;

                if done || decayed {
                    let target_note = self.atomics.config.target_note.load(Ordering::Relaxed);

                    // ── Dispatch Gate ──
                    let dispatch_note = if target_note == 255 {
                        self.latched_auto_key
                    } else {
                        Some(target_note)
                    };

                    if let Some(note_to_send) = dispatch_note {
                        let payload = CapturePayload {
                            stable_buffer: buf,
                            stable_sample_count: self.capture_count,
                            full_event_buffer: self.full_event_buffer.take(),
                            full_event_sample_count: self.full_event_count,
                            target_note: note_to_send,
                            sample_rate: 44100,
                            noise_floor: load_f32(&self.atomics.config.silence_threshold),
                            measured_f0: self.last_measured_f0,
                        };
                        self.full_event_count = 0; // Clear state after dispatch

                        // Safely dispatch and recover buffers if the worker is backed up
                        // Fixes pre-existing bricked-state bug when try_send fails
                        match self.capture_tx.try_send(payload) {
                            Ok(()) => {
                                self.atomics
                                    .capture_state
                                    .store(CaptureState::Processing as u8, Ordering::Relaxed);
                            }
                            Err(e) => {
                                let dropped = e.into_inner();
                                let _ = self.audio_pool.push(dropped.stable_buffer);
                                if let Some(dbuf) = dropped.full_event_buffer {
                                    let _ = self.audio_pool.push(dbuf);
                                }
                                self.atomics
                                    .capture_state
                                    .store(CaptureState::Armed as u8, Ordering::Relaxed);
                            }
                        }
                    } else {
                        // Garbage detected (No Lock). Recycle buffer and reset to Armed.
                        let _ = self.audio_pool.push(buf);
                        if let Some(dbuf) = self.full_event_buffer.take() {
                            let _ = self.audio_pool.push(dbuf);
                        }
                        self.full_event_count = 0; // Clear state on garbage
                        self.atomics
                            .capture_state
                            .store(CaptureState::Armed as u8, Ordering::Relaxed);
                    }
                    self.latched_auto_key = None;
                } else {
                    self.capture_buffer = Some(buf);
                }
            }
        }

        // ─── Step 7: Triple Buffer Telemetry Assembly ───

        // Build fixed-size FrameOutput — zero heap allocations
        let mut frame_output = FrameOutput::default();
        frame_output.magnitudes[..mag_count]
            .copy_from_slice(&self.processing_frame.treble_magnitude_buffer[..mag_count]);
        frame_output.magnitude_len = mag_count;

        // Map gate telemetry (is_new_onset intentionally dropped — internal to DSP)
        frame_output.rms_ema = gate_result.rms_ema;
        frame_output.nhwrsf = gate_result.nhwrsf;
        frame_output.ninos2 = gate_result.ninos2_ema;
        frame_output.is_silence = is_silence;

        if let Some(result) = pitch_result {
            frame_output.detected_frequency = result.measured_f0;
            frame_output.confidence = None;
            frame_output.note_index = Some(result.key_index);
            frame_output.cents_deviation = result.cents_deviation;

            // Populate strobe arrays (limit to 12 for GUI rendering)
            let strobe_count = result.partial_count.min(12);
            frame_output.partial_freqs[..strobe_count]
                .copy_from_slice(&result.partial_freqs[..strobe_count]);
            frame_output.partial_ns[..strobe_count]
                .copy_from_slice(&result.partial_ns[..strobe_count]);
            frame_output.partial_count = strobe_count;
        }

        Some(frame_output)
    }
}
