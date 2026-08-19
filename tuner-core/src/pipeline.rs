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
//! - [`PipelinePorts`] is kept by the frontend (GUI, WASM, etc.): the shareable
//!   [`PipelineHandle`] plus the Worker→UI and UI→DSP channel endpoints.
//!
//! ```text
//! AudioPipeline::new() -> (AudioPipeline, PipelinePorts)
//!       │                         │
//!       ▼                         ▼
//!   Audio Thread              GUI Thread
//! ```

use crossbeam_channel::{Receiver, Sender, bounded};
use crossbeam_queue::ArrayQueue;
use realfft::RealToComplex;
use ringbuf::{
    HeapCons, HeapProd, HeapRb,
    traits::{Consumer, Producer, Split},
};
use rustfft::num_complex::Complex;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use crate::FrameOutput;
use crate::audio::{BASS_WINDOW_SIZE, HOP_SIZE, SAMPLE_RATE, WINDOW_SIZE};
use crate::cola::CircularFifo;

use crate::algorithms::spectral;
use crate::engine::Engine;
use crate::gatekeeper::{Gatekeeper, SignalState};
use crate::models::{
    InharmonicityProfile, KeyMeasurement, KeyProfile, PROFILE_PATH, SoundingStrings,
    build_default_profiles,
};
use crate::strobe::{Strobe, StrobeRefUpdate};
use crate::worker::{WorkerJob, WorkerManager, WorkerOutput};

// ─── Memory Infrastructure ───────────────────────────────────────────────────

// Buffer capacity for non-causal pre-roll.
// 32,768 samples provides ~743ms of history at 44.1kHz, safely accommodating
// the 15-frame (15,360 sample) pre-roll requirement.
const ONSET_HISTORY_SAMPLES: usize = 32768;

/// Shipped fill target for a capture: 1.5 s.
///
/// **Do not move it without an ADR.** Every capture set the project measures
/// against was recorded at this length (`06-capture-sets.md`), so a changed
/// default makes new measurements incomparable with all of them. A measurement
/// session moves the runtime target ([`PipelineAtomics::capture_samples`])
/// instead, which does not touch what is analysed.
pub const CAPTURE_DEFAULT_SAMPLES: usize = 3 * SAMPLE_RATE as usize / 2;

/// How much of a record the Worker analyses, however long the record is.
///
/// Deliberately **not** defined in terms of [`CAPTURE_DEFAULT_SAMPLES`], though
/// it holds the same value: this one is pinned to the length the capture sets
/// were recorded at (`06-capture-sets.md`), so if the default fill ever moves,
/// the analysed span must stay put or every new measurement silently stops
/// being comparable with them.
pub const CAPTURE_ANALYSIS_SAMPLES: usize = 3 * SAMPLE_RATE as usize / 2;

/// Ceiling the pool allocates every buffer to, so raising the fill target never
/// allocates on the audio thread (`03-dsp-pipeline.md`).
///
/// 5 s. Ours, measured (`06-capture-sets.md`): a struck string's usable record
/// ends when it decays into the noise floor, and past that a longer window adds
/// noise-only samples, which sharpen no line — a decaying sinusoid's frequency
/// resolution saturates near its own decay constant. The cost is resident
/// whatever the knob says: 8 pool buffers × 5 s × 4 B ≈ 7 MB.
pub const CAPTURE_MAX_SAMPLES: usize = 5 * SAMPLE_RATE as usize;

/// Length of the diagnostic full-event record (pre-roll + attack + decay).
///
/// Fixed at 1.5 s and deliberately **not** moved by the fill-target knob: the
/// stable record is the one that grows, and between them they cover the event
/// from before the strike to the end of the decay.
const FULL_EVENT_SAMPLES: usize = CAPTURE_DEFAULT_SAMPLES;

/// Payload dispatched from the pipeline to the Worker thread.
pub struct CapturePayload {
    /// High-resolution overlap-added buffer content, [`CAPTURE_MAX_SAMPLES`]
    /// long and filled to `stable_sample_count`.
    pub stable_buffer: Box<[f32]>,
    /// Number of valid samples written to the stable buffer — the fill target
    /// that stood when recording began, or less if the note decayed first.
    /// Only the first [`CAPTURE_ANALYSIS_SAMPLES`] are measured; the rest is
    /// stored audio.
    pub stable_sample_count: usize,
    /// Purely diagnostic buffer containing the full acoustic event (pre-roll, strike, and decay).
    /// This is written to disk for analysis tooling (`diagnose_engine.rs`) and is NEVER
    /// fed back into the Engine or MAT algorithms.
    pub full_event_buffer: Option<Box<[f32]>>,
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
    /// Capture provenance: `true` when the key identity came from the
    /// auto-discovery latch rather than a user-named target. Recorded on
    /// [`crate::models::KeyMeasurement`] — the tuning curve trusts manual
    /// captures only (ADR 0006 item 3).
    pub captured_in_auto: bool,
    /// The operator's string declaration, decoded from
    /// [`PipelineAtomics::capture_strings`] as this payload was assembled;
    /// `None` when nothing was declared, which is the ordinary case.
    pub sounding_strings: Option<SoundingStrings>,
}

// ─── Profile Updates (Crossing #4: UI → DSP) ─────────────────────────────────
//
// SPSC `ringbuf` of heap-free `KeyProfileUpdate`s. The frontend pushes via
// [`ProfileSender`]; the pipeline drains the queue and swaps templates into
// `live_profiles` on a frame boundary. See
// docs/internals/02-cross-thread-communication.md §4.

/// **MEASURED-B DISCOVERY SEEDING — DISABLED PENDING FIX (flip to `true` to re-enable).**
///
/// When `true`, measured per-key inharmonicity `B` (from the persisted
/// [`InharmonicityProfile`]) seeds the live discovery templates — at startup and on
/// every capture/undo/load via [`ProfileSender`]. When `false`, the engine always
/// uses the Rigaud prior for discovery (the worker still measures `B` and the UI
/// still stores/persists/displays it — only the *discovery template* path is gated).
///
/// ## Why it's off
/// The synthetic oracle-B ablation predicted a bass false-lock collapse (27%→1.5%,
/// ADR 0006 §oracle-B). But validation on the one real instrument (2026-06-27,
/// `test_engine_all.py --profile tuning_profile.json` over the captures) showed the
/// **opposite**: applying the MAT-measured `B` to discovery was a net regression
/// (74→73/87), and the highest-ratio bass keys (3/16/17 at 18–25× the prior) *broke*.
/// Root cause (per `docs/adr/0006-...md` + `mobo-methodology.md` §8.2): on this
/// out-of-tune upright there is **no trusted `B` reference**, and MAT appears to
/// **over-estimate bass `B`**; the oracle was also asymmetric (only the true key got
/// perfect `B`, whereas here impostors are boosted too). The only clean wins were
/// *treble* keys where the prior over-estimates `B`.
///
/// The full pathway (conversion, crossing #4, GUI pushes) is built and tested; this
/// flag is the single switch. **Re-enable once a second, in-tune instrument
/// validates the measured values** (the standing gate in ADR 0006).
pub const APPLY_MEASURED_B_TO_DISCOVERY: bool = false;

/// Ring-buffer capacity for the profile-update channel.
///
/// One slot per piano key (88) — the upper bound for a single coherent profile
/// refresh ([`ProfileSender::update_all`]) pushed between two DSP hops. Per-capture
/// live updates arrive one at a time, seconds apart, so this never backs up in
/// normal use; sizing to a full refresh means even a whole-instrument profile load
/// is delivered without dropping a key.
pub const PROFILE_QUEUE_CAPACITY: usize = 88;

/// Ring-buffer capacity for the strobe-reference channel (UI → DSP, the
/// second crossing-#4 instance).
///
/// Updates arrive only on key change / re-lock — user-rate events, orders of
/// magnitude slower than the hop rate — and the pipeline drains to the
/// newest update each hop (stale reference sets are worthless). Two slots
/// absorb a same-tick change pair; on a full buffer the UI simply re-sends
/// next tick.
pub const STROBE_REF_QUEUE_CAPACITY: usize = 2;

/// Capacity of the capture-dispatch channel (DSP → Worker, crossing #5).
///
/// The real backpressure ceiling is the [`AudioPool`]
/// ([`AUDIO_POOL_CAPACITY`]); this channel is subordinate.
/// Captures are serialised at the source (a note is held ~1.5 s and the worker
/// finishes well within that), so at most one capture is genuinely in flight.
/// Two = one being processed + one just-completed slot before the `try_send`
/// backpressure path (`Recording → Armed` recovery) trips.
pub const CAPTURE_QUEUE_CAPACITY: usize = 2;

/// Capacity of the worker-result channel (Worker → UI, crossing #5).
///
/// The UI drains it every ~16 ms (60 FPS) and the worker `try_send`s (dropping
/// on full — a lost result is only a missed display update, recoverable by
/// re-capture). Four is generous slack over the ≤ 1 result the serialised
/// capture cadence can leave pending, so a drop is effectively impossible. The
/// channel is shared with the curve-bundle result ([`WorkerOutput`]), but
/// bundles are coalesced and infrequent (one per settled edit), so four still
/// holds for the combined traffic.
pub const WORKER_RESULT_QUEUE_CAPACITY: usize = 4;

/// Capacity of the worker-job channel (UI → Worker, crossing #6).
///
/// Latest-wins: a queued job superseded by a newer profile edit is worthless,
/// so a single slot suffices — the UI re-requests from its `curve_dirty` flag
/// if a send finds the slot full, and the worker coalesces to the newest job
/// on receipt. One in-flight compute + this one pending slot is the whole
/// pipeline depth a curve recompute ever needs.
pub const WORKER_JOB_QUEUE_CAPACITY: usize = 1;

/// Buffers the [`AudioPool`] holds, sized so a `pop` can never fail.
///
/// Starvation is silent — `pop` returning `None` means the capture simply never
/// starts, with nothing on screen to say so — so the pool is sized to the worst
/// case rather than the expected one. A capture borrows **two** buffers (the
/// stable record and the full-event diagnostic), and three can be outstanding
/// at once: one filling in the pipeline, [`CAPTURE_QUEUE_CAPACITY`] queued for
/// the worker, and one the worker is processing.
pub const AUDIO_POOL_CAPACITY: usize = 2 * (1 + CAPTURE_QUEUE_CAPACITY + 1);

/// A single key's recompiled discovery template, in transit UI → DSP (crossing #4).
///
/// Heap-free (`KeyProfile` is `{f32, f32, [f32; MAX_PARTIALS], usize}`), so it is
/// legal across the real-time boundary.
pub struct KeyProfileUpdate {
    /// 0–87 piano key index whose template this replaces.
    pub key_index: u8,
    /// The recompiled template (ET-centered, measured-`B`).
    pub profile: KeyProfile,
}

/// Frontend-side producer for the profile-update channel (crossing #4). Lives in
/// [`HostHandle`](crate::audio::HostHandle) and hides the `ringbuf` producer.
pub struct ProfileSender {
    tx: HeapProd<KeyProfileUpdate>,
}

impl ProfileSender {
    /// Pushes the authoritative template for one key to the DSP thread.
    ///
    /// Pass the key's current [`KeyMeasurement`] (from the UI's `InharmonicityProfile`)
    /// or `None`. A valid measured `B` yields a measured template; otherwise the key
    /// is reset to its Rigaud-prior template — so undoing a capture cleanly reverts
    /// the live engine. A full ring buffer drops the update (the next capture
    /// re-sends, and startup-load reconciles), so this never blocks the UI.
    pub fn update_key_profile(&mut self, key_index: u8, measurement: Option<&KeyMeasurement>) {
        // Gated: measured B regresses discovery on the one validated instrument.
        // See `APPLY_MEASURED_B_TO_DISCOVERY`. No-op keeps the engine on the prior.
        if !APPLY_MEASURED_B_TO_DISCOVERY {
            return;
        }
        let profile = measurement
            .and_then(KeyProfile::from_measurement)
            .unwrap_or_else(|| KeyProfile::prior(key_index));
        let _ = self.tx.try_push(KeyProfileUpdate { key_index, profile });
    }

    /// Pushes the whole instrument's templates (e.g. a mid-session profile load):
    /// every key is set from its measurement when present, else its prior.
    pub fn update_all(&mut self, profile: &InharmonicityProfile) {
        // See `APPLY_MEASURED_B_TO_DISCOVERY` — gated pending a second instrument.
        if !APPLY_MEASURED_B_TO_DISCOVERY {
            return;
        }
        for key in 0..88u8 {
            self.update_key_profile(key, profile.active(key));
        }
    }
}

/// Frontend-side producer for the strobe-reference channel (the second
/// crossing-#4 instance: grouped UI → DSP parameters applied on a frame
/// boundary). Lives in [`HostHandle`](crate::audio::HostHandle) and hides
/// the `ringbuf` producer.
pub struct StrobeSender {
    tx: HeapProd<StrobeRefUpdate>,
}

impl StrobeSender {
    /// Pushes a new reference set for the strobe (key change /
    /// re-lock; `count: 0` clears the strobe). Returns `false` when the ring
    /// is full — the caller re-sends on its next tick, the same retry
    /// pattern as the curve-job dirty flag.
    pub fn set_refs(&mut self, update: StrobeRefUpdate) -> bool {
        self.tx.try_push(update).is_ok()
    }
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

/// What a recording in progress is: how long it fills for, and what it is *of*.
///
/// Sampled together at `Armed → Recording` rather than read at dispatch,
/// because a record can run for seconds and these describe the audio: by the
/// end the operator may have selected the next key or changed the declaration,
/// and reading them then would file the record under a later moment.
struct CaptureLatch {
    /// Samples to fill to, clamped into `HOP_SIZE..=CAPTURE_MAX_SAMPLES`.
    target_samples: usize,
    /// The key the UI had selected; 255 = Auto.
    target_note: u8,
    /// The string declaration, packed by [`SoundingStrings::to_bits`]. Carried,
    /// not consumed — the pipeline is simply the only place that knows when the
    /// audio began.
    declared_strings: u8,
}

impl Default for CaptureLatch {
    fn default() -> Self {
        Self {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            target_note: 255,
            declared_strings: SoundingStrings::UNDECLARED.to_bits(),
        }
    }
}

/// A lock-free Object Pool for audio captures.
///
/// Thread 2 (The Brains) borrows a pre-allocated buffer from this pool when the
/// Gatekeeper triggers. Once filled to the current fill target, the buffer is
/// dispatched to Thread 3 (Background Worker) for heavy DSP (like inharmonicity
/// mapping). Thread 3 returns it to the pool when finished.
///
/// Every buffer is [`CAPTURE_MAX_SAMPLES`] long regardless of the target, so a
/// changed target never allocates on the audio thread.
pub type AudioPool = ArrayQueue<Box<[f32]>>;

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

impl Default for ProcessingFrame {
    fn default() -> Self {
        Self::new()
    }
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
/// risk of priority inversion or lock contention.
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
    /// UI → DSP: samples a capture fills to, clamped into
    /// `HOP_SIZE..=CAPTURE_MAX_SAMPLES` and latched at `Armed → Recording`.
    /// Above [`CAPTURE_DEFAULT_SAMPLES`] it also suppresses the decay stop.
    pub capture_samples: AtomicU32,
    /// UI → DSP: the operator's per-capture string declaration, packed by
    /// [`SoundingStrings::to_bits`]. `0` = undeclared, the ordinary state; the
    /// pipeline decodes it onto the [`CapturePayload`] it dispatches.
    pub capture_strings: AtomicU8,
    /// UI → DSP: drop the recording in progress. A *request*, not a transition
    /// — the pipeline consumes it and makes the `Recording → Idle` move itself,
    /// so the baton keeps one writer per transition. Cleared when a recording
    /// starts, so a stale request cannot kill the next take.
    pub capture_abort: AtomicBool,
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
            capture_samples: AtomicU32::new(CAPTURE_DEFAULT_SAMPLES as u32),
            capture_strings: AtomicU8::new(0), // Default to undeclared
            capture_abort: AtomicBool::new(false),
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
/// Created via [`AudioPipeline::new()`].
pub struct AudioPipeline {
    /// The Gatekeeper — pure DSP, evaluates signal stability.
    pub gatekeeper: Gatekeeper,
    /// The Engine — F0 detection chain
    pub engine: Engine,

    /// Live per-key discovery templates, lent to `engine.process` each frame.
    /// Allocated once; updated in place by draining `profile_rx`.
    live_profiles: Box<[KeyProfile; 88]>,
    /// Crossing #4 consumer for template updates; drained into `live_profiles`.
    profile_rx: HeapCons<KeyProfileUpdate>,
    /// The strobe phase-comparator (Path A) — runs every hop while
    /// references are set, independent of the engine's note lock.
    strobe: Strobe,
    /// Crossing-#4-instance consumer for strobe reference updates; drained
    /// (newest wins) into the strobe each hop.
    strobe_rx: HeapCons<StrobeRefUpdate>,

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
    capture_buffer: Option<Box<[f32]>>,
    capture_count: usize,
    /// What the recording in progress is, sampled at `Armed → Recording`.
    latch: CaptureLatch,
    /// Parallel accumulator for the diagnostic `full_event_buffer`.
    full_event_buffer: Option<Box<[f32]>>,
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

/// Frontend-side handle to the pipeline's shareable atomic state (crossing #3):
/// wait-free config writes and runtime reads. Cloneable.
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

/// The frontend's side of the **Split / Handle** pattern: everything
/// [`AudioPipeline::new()`] hands back when the pipeline itself is moved to the
/// audio thread. Transient — `spawn_analysis_thread` immediately distributes these
/// into a [`HostHandle`](crate::audio::HostHandle).
pub struct PipelinePorts {
    /// Shareable atomic config/runtime view (crossing #3).
    pub handle: PipelineHandle,
    /// Worker → UI receiver for `WorkerOutput` results — `KeyMeasurement`
    /// measurements and `CurveBundle` curve recomputes (crossing #5).
    pub worker_rx: Receiver<WorkerOutput>,
    /// UI → Worker sender for background jobs (crossing #6). Latest-wins;
    /// see [`WORKER_JOB_QUEUE_CAPACITY`].
    pub worker_job_tx: Sender<WorkerJob>,
    /// UI → DSP producer for template updates (crossing #4).
    pub profiles: ProfileSender,
    /// UI → DSP producer for strobe reference updates (crossing #4's second
    /// instance).
    pub strobe_refs: StrobeSender,
}

impl AudioPipeline {
    /// Creates a new AudioPipeline and the frontend's [`PipelinePorts`].
    ///
    /// This follows the **Split / Handle pattern**: the `AudioPipeline` is moved to
    /// the audio thread; the `PipelinePorts` (atomics handle + worker receiver +
    /// profile producer) are kept by the frontend. `spawn_analysis_thread`
    /// distributes the ports into a [`HostHandle`](crate::audio::HostHandle).
    ///
    /// Startup also seeds the live templates from a persisted
    /// [`InharmonicityProfile`] at [`PROFILE_PATH`] — but only when
    /// [`APPLY_MEASURED_B_TO_DISCOVERY`] is enabled. It is `false` by default
    /// (see that flag), so the engine runs on the Rigaud prior.
    ///
    /// # Returns
    /// `(AudioPipeline, PipelinePorts)`.
    pub fn new(dump_dir: Option<PathBuf>) -> (Self, PipelinePorts) {
        let audio_pool = Arc::new(ArrayQueue::new(AUDIO_POOL_CAPACITY));
        // Every buffer takes the ceiling, not the current fill target: the
        // target is a runtime knob and the audio thread allocates nothing.
        for _ in 0..AUDIO_POOL_CAPACITY {
            let _ = audio_pool.push(vec![0.0; CAPTURE_MAX_SAMPLES].into_boxed_slice());
        }

        let atomics = Arc::new(PipelineAtomics::default());

        let gatekeeper = Gatekeeper::new(Arc::clone(&audio_pool));
        let engine = Engine::new(44100);

        // Rigaud prior by default. When the measured-B path is enabled (see
        // `APPLY_MEASURED_B_TO_DISCOVERY`), a persisted profile seeds measured keys
        // here so a calibrated instrument is live on frame one.
        let mut live_profiles = build_default_profiles();
        if APPLY_MEASURED_B_TO_DISCOVERY
            && let Ok(profile) = InharmonicityProfile::from_file(PROFILE_PATH)
        {
            let mut applied = 0usize;
            for (key, measurement) in profile.active_entries() {
                if let Some(kp) = KeyProfile::from_measurement(measurement)
                    && let Some(slot) = live_profiles.get_mut(key as usize)
                {
                    *slot = kp;
                    applied += 1;
                }
            }
            eprintln!(
                "[PIPELINE] Loaded inharmonicity profile from {PROFILE_PATH}: {applied} measured key(s) applied."
            );
        }

        let mut planner = realfft::RealFftPlanner::<f32>::new();
        let fft_instance = planner.plan_fft_forward(WINDOW_SIZE);
        let fft_bass_instance = planner.plan_fft_forward(BASS_WINDOW_SIZE);

        // Crossing #5, DSP → Worker: capture dispatch (`CapturePayload`).
        let (capture_tx, capture_rx) = bounded(CAPTURE_QUEUE_CAPACITY);
        // Crossing #5, Worker → UI: results (`WorkerOutput` — measurements + curves).
        let (result_tx, worker_rx) = bounded(WORKER_RESULT_QUEUE_CAPACITY);
        // Crossing #6, UI → Worker: background jobs (`WorkerJob` — curve recomputes).
        let (worker_job_tx, worker_job_rx) = bounded(WORKER_JOB_QUEUE_CAPACITY);

        // Crossing #4: SPSC profile-update channel (UI producer → DSP consumer).
        let (profile_tx, profile_rx) =
            HeapRb::<KeyProfileUpdate>::new(PROFILE_QUEUE_CAPACITY).split();

        // Crossing #4, second instance: strobe reference updates (UI → DSP).
        let (strobe_tx, strobe_rx) =
            HeapRb::<StrobeRefUpdate>::new(STROBE_REF_QUEUE_CAPACITY).split();

        WorkerManager::new(
            Arc::clone(&audio_pool),
            Arc::clone(&atomics),
            capture_rx,
            worker_job_rx,
            result_tx,
            dump_dir,
        )
        .start_workers();

        let pipeline = Self {
            gatekeeper,
            engine,
            live_profiles,
            profile_rx,
            strobe: Strobe::new(SAMPLE_RATE),
            strobe_rx,
            atomics: Arc::clone(&atomics),
            audio_pool,
            cola: CircularFifo::new(BASS_WINDOW_SIZE),
            fft_instance,
            fft_bass_instance,
            processing_frame: ProcessingFrame::new(),
            capture_tx,
            capture_buffer: None,
            capture_count: 0,
            latch: CaptureLatch::default(),
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

        let ports = PipelinePorts {
            handle: PipelineHandle { atomics },
            worker_rx,
            worker_job_tx,
            profiles: ProfileSender { tx: profile_tx },
            strobe_refs: StrobeSender { tx: strobe_tx },
        };

        (pipeline, ports)
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
        // ─── Step 0: Drain Profile Updates (Crossing #4) ───
        // Apply any measured-B templates the UI pushed since the last hop, before
        // discovery runs this frame. `try_pop` is wait-free and the swap is a plain
        // move into the pre-allocated array — no allocation on the audio thread.
        while let Some(update) = self.profile_rx.try_pop() {
            if let Some(slot) = self.live_profiles.get_mut(update.key_index as usize) {
                *slot = update.profile;
            }
        }

        // Strobe references: drain to the newest update — a superseded
        // reference set is worthless (the key changed again mid-hop).
        let mut strobe_update = None;
        while let Some(update) = self.strobe_rx.try_pop() {
            strobe_update = Some(update);
        }
        if let Some(update) = strobe_update {
            self.strobe.retarget(update);
        }

        // ─── Step 1: COLA & Windowing ───

        // Read the FULL history of audio out of the sliding queue
        self.cola.read_window(
            BASS_WINDOW_SIZE,
            &mut self.processing_frame.audio_buffer[..BASS_WINDOW_SIZE],
        );

        // Populate the frame's generic frequency buffer in place
        // The newest WINDOW_SIZE samples are at the END of the buffer
        let newest_start = BASS_WINDOW_SIZE - WINDOW_SIZE;
        spectral::fft(
            &self.processing_frame.audio_buffer[newest_start..BASS_WINDOW_SIZE],
            &mut self.processing_frame.time_buffer[..WINDOW_SIZE],
            &mut self.processing_frame.frequency_buffer[..],
            &self.fft_instance,
            WINDOW_SIZE,
        );

        spectral::fft(
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
        let is_silence = gate_result.state == SignalState::Silence;
        let is_stable = gate_result.state == SignalState::Stable;

        // Sync runtime observations to shared atomics for framework consumers
        store_f32(&self.atomics.runtime.current_rms_ema, gate_result.rms_ema);
        store_f32(&self.atomics.runtime.current_nhwrsf, gate_result.nhwrsf);

        // ─── Step 4: Magnitude Extraction ───

        let mag_count = WINDOW_SIZE / 2;
        spectral::magnitude_spectrum(
            &self.processing_frame.frequency_buffer[..],
            WINDOW_SIZE,
            &mut self.processing_frame.treble_magnitude_buffer[..mag_count],
        );

        let mag_count_bass = BASS_WINDOW_SIZE / 2;
        spectral::magnitude_spectrum(
            &self.processing_frame.bass_frequency_buffer[..],
            BASS_WINDOW_SIZE,
            &mut self.processing_frame.bass_magnitude_buffer[..mag_count_bass],
        );

        // ─── Step 5: Pitch Detection (Engine) ───

        if gate_result.is_new_onset {
            self.last_measured_f0 = None;
        }

        let pitch_result = self.engine.process(
            &self.processing_frame,
            &self.live_profiles,
            is_silence,
            is_stable,
            gate_result.is_new_onset,
            gate_result.is_transient_bypass,
            target_note,
        );

        // ─── Step 5b: Strobe & Coarse Readout ───

        let strobe_result = self.strobe.process(
            &self.processing_frame,
            self.gatekeeper.config.silence_threshold,
            is_silence,
        );

        // ─── Step 6: Capture Accumulation & Worker Dispatch ───

        let current_capture_state = self.atomics.capture_state.load(Ordering::Relaxed);

        if gate_result.state == SignalState::Silence {
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
                && gate_result.state == SignalState::Stable
                && let Some(buf) = self.audio_pool.pop()
            {
                self.capture_onset_pending = false;
                self.capture_buffer = Some(buf);
                self.capture_count = 0;
                self.latch = CaptureLatch {
                    // Clamped, not trusted: another thread writes this.
                    target_samples: (self.atomics.capture_samples.load(Ordering::Relaxed) as usize)
                        .clamp(HOP_SIZE, CAPTURE_MAX_SAMPLES),
                    target_note: self.atomics.config.target_note.load(Ordering::Relaxed),
                    declared_strings: self.atomics.capture_strings.load(Ordering::Relaxed),
                };
                // A request that arrived before this take is not about it.
                self.atomics.capture_abort.store(false, Ordering::Relaxed);
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
            let remaining = FULL_EVENT_SAMPLES - self.full_event_count;
            let to_copy = src_slice.len().min(remaining);
            buf[self.full_event_count..self.full_event_count + to_copy]
                .copy_from_slice(&src_slice[..to_copy]);
            self.full_event_count += to_copy;
            self.full_event_buffer = Some(buf);
        }

        if current_capture_state == CaptureState::Recording as u8
            && self.atomics.capture_abort.swap(false, Ordering::Relaxed)
        {
            // Nothing is dispatched, so the take reaches neither the profile
            // nor the disk.
            if let Some(buf) = self.capture_buffer.take() {
                let _ = self.audio_pool.push(buf);
            }
            if let Some(dbuf) = self.full_event_buffer.take() {
                let _ = self.audio_pool.push(dbuf);
            }
            self.capture_count = 0;
            self.full_event_count = 0;
            self.latched_auto_key = None;
            self.atomics
                .capture_state
                .store(CaptureState::Idle as u8, Ordering::Relaxed);
        } else if current_capture_state == CaptureState::Recording as u8 {
            // ── Latch ──
            if let Some(ref result) = pitch_result {
                self.latched_auto_key = Some(result.key_index);
                self.last_measured_f0 = result.measured_f0;
            }

            if let Some(mut buf) = self.capture_buffer.take() {
                let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
                let src_slice = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];

                let remaining = self.latch.target_samples - self.capture_count;
                let to_copy = src_slice.len().min(remaining);

                buf[self.capture_count..self.capture_count + to_copy]
                    .copy_from_slice(&src_slice[..to_copy]);

                self.capture_count += to_copy;

                let done = self.capture_count == self.latch.target_samples;
                // An extended record is a request for the audio past the decay,
                // so only the shipped length keeps the short-dispatch valve.
                let decayed = gate_result.state == SignalState::Silence
                    && self.latch.target_samples <= CAPTURE_DEFAULT_SAMPLES;

                if done || decayed {
                    let target_note = self.latch.target_note;

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
                            captured_in_auto: target_note == 255,
                            sounding_strings: SoundingStrings::from_bits(
                                self.latch.declared_strings,
                            ),
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

        // Strobe telemetry — unconditional: the band must render (frozen or
        // spinning) whether or not the engine holds a note lock.
        frame_output.strobe_angle = strobe_result.angle;
        frame_output.strobe_gated = strobe_result.gated;
        frame_output.strobe_beat_hz = strobe_result.beat_hz;
        frame_output.strobe_count = strobe_result.count;
        frame_output.strobe_amplitude = strobe_result.amplitude;
        frame_output.unison_lines = strobe_result.lines;
        frame_output.unison_line_count = strobe_result.line_count;
        frame_output.unison_resolution_hz = strobe_result.line_resolution_hz;
        frame_output.unison_verdict = strobe_result.verdict;
        frame_output.coarse_hz = strobe_result.coarse_hz;
        // The buffer is held only while a record is in progress, so its
        // presence is exactly the condition a progress figure is meaningful in.
        frame_output.capture_progress_samples = if self.capture_buffer.is_some() {
            self.capture_count
        } else {
            0
        };

        if let Some(result) = pitch_result {
            frame_output.detected_frequency = result.measured_f0;
            frame_output.confidence = None;
            frame_output.note_index = Some(result.key_index);

            // Populate live tracked partials (cap at buffer capacity for rendering)
            let n = result.partial_count.min(frame_output.tracked_freqs.len());
            frame_output.tracked_freqs[..n].copy_from_slice(&result.partial_freqs[..n]);
            frame_output.tracked_ns[..n].copy_from_slice(&result.partial_ns[..n]);
            frame_output.tracked_count = n;
        }

        Some(frame_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{KeyMeasurement, NOTES, get_expected_beta};
    use crate::strobe::MAX_STROBE_REFS;

    fn measurement(key_index: u8, calculated_b: Option<f32>) -> KeyMeasurement {
        KeyMeasurement {
            key_index,
            // Deliberately implausible measured_f0 — it must NOT leak into the
            // template (ET-centered, β-only is the contract).
            measured_f0: 9999.0,
            partials: Vec::new(),
            calculated_b,
            last_captured: String::new(),
            captured_in_auto: true,
            sounding_strings: None,
        }
    }

    /// The capture lengths are *durations* — 1.5 s is the figure every capture
    /// set was recorded at, and the one ADR 0009's σ model was measured on.
    #[test]
    fn capture_lengths_are_the_durations_they_claim() {
        let secs = |n: usize| n as f32 / SAMPLE_RATE as f32;
        assert!((secs(CAPTURE_DEFAULT_SAMPLES) - 1.5).abs() < 1e-6);
        assert!((secs(CAPTURE_MAX_SAMPLES) - 5.0).abs() < 1e-6);
        assert_eq!(CAPTURE_ANALYSIS_SAMPLES, CAPTURE_DEFAULT_SAMPLES);
        assert_eq!(FULL_EVENT_SAMPLES, CAPTURE_DEFAULT_SAMPLES);
        // The pool allocates the ceiling, so nothing the knob can ask for
        // reaches past a buffer.
        const { assert!(CAPTURE_DEFAULT_SAMPLES <= CAPTURE_MAX_SAMPLES) };
    }

    #[test]
    fn conversion_uses_measured_b_at_et_center() {
        let key = 5u8;
        let kp = KeyProfile::from_measurement(&measurement(key, Some(0.002)))
            .expect("valid B should convert");
        // Measured B adopted…
        assert_eq!(kp.beta, 0.002);
        // …at the equal-temperament center, NOT the (stale) measured_f0.
        assert_eq!(kp.f0_et, NOTES[key as usize].frequency);
        assert_ne!(kp.f0_et, 9999.0);
    }

    #[test]
    fn conversion_rejects_unmeasured_or_invalid_b() {
        let key = 10u8;
        assert!(KeyProfile::from_measurement(&measurement(key, None)).is_none());
        assert!(KeyProfile::from_measurement(&measurement(key, Some(0.0))).is_none());
        assert!(KeyProfile::from_measurement(&measurement(key, Some(-0.001))).is_none());
        assert!(KeyProfile::from_measurement(&measurement(key, Some(f32::NAN))).is_none());
        assert!(KeyProfile::from_measurement(&measurement(key, Some(f32::INFINITY))).is_none());
    }

    #[test]
    fn profile_sender_round_trip_or_gated() {
        let (tx, mut rx) = HeapRb::<KeyProfileUpdate>::new(PROFILE_QUEUE_CAPACITY).split();
        let mut sender = ProfileSender { tx };

        // A measured key would carry its measured B; an unmeasured key (e.g.
        // undo-to-nothing) would reset to the Rigaud prior.
        sender.update_key_profile(5, Some(&measurement(5, Some(0.0031))));
        sender.update_key_profile(7, None);

        if APPLY_MEASURED_B_TO_DISCOVERY {
            let first = rx.try_pop().expect("first update queued");
            assert_eq!(first.key_index, 5);
            assert_eq!(first.profile.beta, 0.0031);
            assert_eq!(first.profile.f0_et, NOTES[5].frequency);

            let second = rx.try_pop().expect("second update queued");
            assert_eq!(second.key_index, 7);
            assert_eq!(second.profile.beta, get_expected_beta(7));
            assert_eq!(second.profile.f0_et, NOTES[7].frequency);

            assert!(rx.try_pop().is_none(), "exactly two updates were pushed");
        } else {
            // Gated off (default): measured B must not reach discovery at all.
            assert!(rx.try_pop().is_none(), "gated ProfileSender must not push");
        }
    }

    #[test]
    fn update_all_pushes_every_key_or_gated() {
        let (tx, mut rx) = HeapRb::<KeyProfileUpdate>::new(PROFILE_QUEUE_CAPACITY).split();
        let mut sender = ProfileSender { tx };

        let mut profile = InharmonicityProfile::default();
        profile.record(measurement(3, Some(0.0017)));
        sender.update_all(&profile);

        let mut seen = 0usize;
        let mut key3_beta = None;
        while let Some(u) = rx.try_pop() {
            if u.key_index == 3 {
                key3_beta = Some(u.profile.beta);
            }
            seen += 1;
        }

        if APPLY_MEASURED_B_TO_DISCOVERY {
            // All 88 keys refreshed (measured where present, prior elsewhere),
            // none dropped at capacity == 88.
            assert_eq!(seen, 88);
            assert_eq!(key3_beta, Some(0.0017));
        } else {
            assert_eq!(seen, 0, "gated update_all must not push");
        }
    }

    /// Step 5b end to end: push a reference set over crossing #4, stream a
    /// detuned sine, and read `coarse_hz` back out of `FrameOutput`. Covers the
    /// whole wiring — drain, retain, coarse-target resolution, dual-window
    /// selection, and the crossing-#2 widen — which the unit tests on
    /// `peaks::coarse_read` cannot reach.
    #[test]
    fn coarse_readout_reaches_frame_output() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);

        // A4's fundamental as the reference; the string is 3 Hz sharp (≈ +12 ¢),
        // inside the ±100 ¢ search band and outside the strobe band's ±18 Hz.
        let f_ref = 440.0f32;
        let f_live = 443.0f32;
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        refs[0] = f_ref;
        assert!(ports.strobe_refs.set_refs(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: f_ref,
        }));

        // Two seconds of audio, one hop at a time — enough for the gatekeeper to
        // leave Silence and for the COLA buffer to fill.
        let mut readings = Vec::new();
        let mut n = 0u64;
        // f64 phase from an absolute sample index: an f32 accumulator loses
        // enough precision over two seconds to detune the *test signal*.
        let step = 2.0 * std::f64::consts::PI * f_live as f64 / SAMPLE_RATE as f64;
        let mut hop = [0.0f32; HOP_SIZE];
        for _ in 0..(2 * SAMPLE_RATE as usize / HOP_SIZE) {
            for s in hop.iter_mut() {
                *s = 0.2 * (step * n as f64).sin() as f32;
                n += 1;
            }
            if let Some(frame) = pipeline.push_audio(&hop)
                && let Some(hz) = frame.coarse_hz
            {
                readings.push(hz);
            }
        }

        assert!(
            readings.len() > 20,
            "a sustained tone at the reference must read on most hops, got {}",
            readings.len()
        );
        // Skip the COLA fill-in: until 8192 samples have arrived the long window
        // analyses a zero-padded fragment, whose leakage biases the refiner. Real
        // captures pay the same cost for the same 8 hops.
        let settled = BASS_WINDOW_SIZE / HOP_SIZE;
        let worst = readings[settled..]
            .iter()
            .map(|hz| (hz - f_live).abs())
            .fold(0.0f32, f32::max);
        assert!(
            worst < 0.05,
            "coarse read must land on the live partial, worst error {worst:.3} Hz"
        );
    }

    /// Silence withholds the coarse read: the CFAR null is broadband noise, so a
    /// number taken from room rumble would be colored noise dressed as a partial.
    #[test]
    fn coarse_readout_withheld_in_silence() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let mut refs = [0.0f32; MAX_STROBE_REFS];
        refs[0] = 440.0;
        assert!(ports.strobe_refs.set_refs(StrobeRefUpdate {
            count: 1,
            refs,
            coarse_index: 1,
            spacing_hz: 440.0,
        }));

        let hop = [0.0f32; HOP_SIZE];
        for _ in 0..(SAMPLE_RATE as usize / HOP_SIZE) {
            if let Some(frame) = pipeline.push_audio(&hop) {
                assert!(
                    frame.coarse_hz.is_none(),
                    "silence must publish no coarse reading"
                );
            }
        }
    }

    #[test]
    fn default_profiles_match_rigaud_prior() {
        let profiles = build_default_profiles();
        assert_eq!(profiles[0].beta, get_expected_beta(0));
        assert_eq!(profiles[87].beta, get_expected_beta(87));
        assert_eq!(profiles[40].f0_et, NOTES[40].frequency);
    }
}
