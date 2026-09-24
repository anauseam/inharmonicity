//! # Audio pipeline
//!
//! [`AudioPipeline::new`] returns two halves. The [`AudioPipeline`] moves to the
//! audio thread and runs the [`Gatekeeper`], the [`Engine`] and the [`Strobe`] on
//! every hop pushed into it; the [`PipelinePorts`] stay with the frontend: the
//! shared [`PipelineHandle`] and the frontend's end of each channel.

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

/// Raw-stream history kept for a capture's pre-roll: ≈ 743 ms, over the 15-hop
/// pre-roll.
const ONSET_HISTORY_SAMPLES: usize = 32768;

/// Shipped fill target for a capture: 1.5 s. A session wanting longer records
/// raises [`ArmRequest::target_samples`] instead.
// Do not move it: every capture set was recorded at this length.
// capture-sets.md
pub const CAPTURE_DEFAULT_SAMPLES: usize = 3 * SAMPLE_RATE as usize / 2;

/// How much of a record the Worker analyses, however long the record is.
// Equal to `CAPTURE_DEFAULT_SAMPLES` but not derived from it: if the default
// fill moves, the analysed span must not, or new measurements stop being
// comparable with the capture sets.
pub const CAPTURE_ANALYSIS_SAMPLES: usize = 3 * SAMPLE_RATE as usize / 2;

/// Ceiling the pool allocates every buffer to, so raising the fill target never
/// allocates on the audio thread. The memory is resident whatever the target:
/// 8 buffers × 5 s × 4 B ≈ 7 MB.
// 5 s: a struck string's record ends when it decays into the recording's noise,
// and noise-only samples sharpen no line (a decaying sinusoid's frequency
// resolution saturates near its own decay constant).
// capture-sets.md
pub const CAPTURE_MAX_SAMPLES: usize = 5 * SAMPLE_RATE as usize;

/// Length of the diagnostic full-event record: pre-roll, attack and decay. Fixed;
/// a longer take grows the stable record instead.
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
    /// The full event (pre-roll, strike and decay), written to the dump for
    /// offline analysis and never measured.
    pub full_event_buffer: Option<Box<[f32]>>,
    /// Number of valid samples written to the full event buffer.
    pub full_event_sample_count: usize,
    /// The key the capture is of: the named target, or in Auto the key
    /// discovery latched.
    pub target_note: u8,
    /// Fixed sampling rate of the pipeline.
    pub sample_rate: u32,
    /// The silence threshold when the capture was dispatched.
    pub noise_floor: f32,
    /// The gate's NHWRSF onset threshold when the capture was dispatched.
    pub nhwrsf_threshold: f32,
    /// The gate's sustain-stability threshold when the capture was dispatched.
    pub sustain_stability_threshold: f32,
    /// The engine's partial-1 frequency, MAT's seed; `None` if tracking failed.
    pub measured_f0: Option<f32>,
    /// The key came from discovery's latch rather than a named target.
    pub captured_in_auto: bool,
    /// The operator's string declaration, latched from the standing
    /// [`ArmRequest`] when the audio began; `None` when nothing was declared,
    /// which is the ordinary case.
    pub sounding_strings: Option<SoundingStrings>,
}

// ─── Profile Updates (Crossing #4: UI → DSP) ─────────────────────────────────

/// Whether measured per-key B seeds the discovery templates. While `false`,
/// discovery runs on the Rigaud prior; the Worker still measures B.
// Off: measured B also sharpens the sub-harmonic impostors' templates, and no
// regulariser separates the true bass key from them; at settings that reach the
// real bass B it netted −2 keys. Re-open with an in-tune instrument.
// report 0006
pub const APPLY_MEASURED_B_TO_DISCOVERY: bool = false;

/// Ring-buffer capacity for the profile-update channel: one slot per key, so a
/// whole-profile refresh arrives between two hops without dropping one.
pub const PROFILE_QUEUE_CAPACITY: usize = 88;

/// Ring-buffer capacity for the strobe-reference channel (UI → DSP, crossing #4).
/// The pipeline keeps only each hop's newest update; two slots absorb a
/// same-tick pair.
pub const STROBE_REF_QUEUE_CAPACITY: usize = 2;

/// Ring-buffer capacity for the capture-command channel (UI → DSP, crossing #4).
/// The pipeline keeps only each hop's newest command; two slots absorb a
/// same-tick pair.
pub const CAPTURE_COMMAND_QUEUE_CAPACITY: usize = 2;

/// Capacity of the capture-dispatch channel (DSP → Worker, crossing #5).
/// Captures are serialised at the source, so at most one is in flight; the
/// second slot holds a just-completed one before `try_send` backpressure trips.
/// The pool, not this channel, is the real ceiling.
pub const CAPTURE_QUEUE_CAPACITY: usize = 2;

/// Capacity of the worker-result channel (Worker → UI, crossing #5), shared by
/// measurements and curve bundles. The Worker drops a result on a full channel,
/// which costs a re-capture; four is ample for a consumer that drains every tick.
pub const WORKER_RESULT_QUEUE_CAPACITY: usize = 4;

/// Capacity of the worker-job channel (UI → Worker, crossing #6). One slot: a
/// superseded job is worthless, and the Worker coalesces to the newest.
pub const WORKER_JOB_QUEUE_CAPACITY: usize = 1;

/// Buffers the [`AudioPool`] holds, sized for the worst case because starvation
/// is silent: the capture just never starts. A capture borrows two, and one can
/// be filling, [`CAPTURE_QUEUE_CAPACITY`] queued and one with the Worker.
pub const AUDIO_POOL_CAPACITY: usize = 2 * (1 + CAPTURE_QUEUE_CAPACITY + 1);

/// One key's recompiled discovery template, in transit UI → DSP (crossing #4).
/// Heap-free, so it may cross to the audio thread.
pub struct KeyProfileUpdate {
    /// 0–87 piano key index whose template this replaces.
    pub key_index: u8,
    /// The recompiled template (ET-centered, measured-`B`).
    pub profile: KeyProfile,
}

/// The frontend's producer for the profile-update channel (crossing #4).
pub struct ProfileSender {
    tx: HeapProd<KeyProfileUpdate>,
}

impl ProfileSender {
    /// Pushes one key's template to the DSP thread: from `measurement`'s B when it
    /// is valid, else the Rigaud prior. Never blocks; a full ring drops the update.
    pub fn update_key_profile(&mut self, key_index: u8, measurement: Option<&KeyMeasurement>) {
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
        if !APPLY_MEASURED_B_TO_DISCOVERY {
            return;
        }
        for key in 0..88u8 {
            self.update_key_profile(key, profile.active(key));
        }
    }
}

/// The frontend's producer for the strobe-reference channel (crossing #4).
pub struct StrobeSender {
    tx: HeapProd<StrobeRefUpdate>,
}

impl StrobeSender {
    /// Pushes a new reference set for the strobe; `count: 0` clears it. Returns
    /// `false` when the ring is full.
    pub fn set_refs(&mut self, update: StrobeRefUpdate) -> bool {
        self.tx.try_push(update).is_ok()
    }
}

/// The frontend's producer for the capture-command channel (crossing #4).
pub struct CaptureSender {
    tx: HeapProd<CaptureCommand>,
}

impl CaptureSender {
    /// Pushes one capture-lifecycle command to the DSP thread. Returns `false`
    /// when the ring is full.
    pub fn send(&mut self, command: CaptureCommand) -> bool {
        self.tx.try_push(command).is_ok()
    }
}

/// Where the capture lifecycle stands: Idle → Armed → Recording → Processing
/// → Idle. The pipeline makes every transition; the Worker ends `Processing` by
/// clearing `capture_in_flight`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptureState {
    /// Nothing pending.
    #[default]
    Idle,
    /// Waiting for a strike.
    Armed,
    /// Filling the record.
    Recording,
    /// Dispatched; the Worker still holds it.
    Processing,
}

/// What a [`CaptureCommand::Arm`] asks the next record to be: how long it
/// fills for, and what it is of. Heap-free and `Copy`.
#[derive(Debug, Clone, Copy)]
pub struct ArmRequest {
    /// Samples to fill to. Clamped into `HOP_SIZE..=CAPTURE_MAX_SAMPLES` on
    /// receipt rather than trusted — another thread writes it.
    pub target_samples: usize,
    /// The operator's string declaration, or `None` when nothing was declared
    /// (the ordinary case). Carried, not consumed.
    pub declared_strings: Option<SoundingStrings>,
}

impl Default for ArmRequest {
    fn default() -> Self {
        Self {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            declared_strings: None,
        }
    }
}

/// A capture-lifecycle command, UI → DSP (crossing #4's third instance).
/// Heap-free and `Copy`.
#[derive(Debug, Clone, Copy)]
pub enum CaptureCommand {
    /// Arm for one record, and state what that record will be. While `Armed` it
    /// only updates the request, so the last `Arm` before the strike is the one
    /// the record carries.
    Arm(ArmRequest),
    /// Disarms from `Armed`, and drops the take in progress from `Recording`;
    /// nothing is dispatched.
    Cancel,
}

/// What the recording in progress is: the request that armed it, and the key
/// selected when the string was struck. Sampled at `Armed → Recording`, since
/// by dispatch, seconds later, the operator may have selected the next key.
struct CaptureLatch {
    /// The request in force when the audio began, already clamped.
    request: ArmRequest,
    /// The key the UI had selected; 255 = Auto.
    target_note: u8,
}

impl Default for CaptureLatch {
    fn default() -> Self {
        Self {
            request: ArmRequest::default(),
            target_note: 255,
        }
    }
}

/// A lock-free pool of capture buffers: the pipeline borrows one for a record
/// and the Worker returns it. Every buffer is [`CAPTURE_MAX_SAMPLES`] long, so a
/// changed fill target never allocates on the audio thread.
pub type AudioPool = ArrayQueue<Box<[f32]>>;

/// The analysis thread's per-hop buffers, allocated once and reused, so a hop
/// never allocates.
pub struct ProcessingFrame {
    /// The hop's window of raw samples, [`BASS_WINDOW_SIZE`] long, newest last.
    pub audio_buffer: Box<[f32]>,
    /// Time-domain scratch for windowing before the FFT.
    pub time_buffer: Box<[f32]>,
    /// The [`WINDOW_SIZE`] FFT of the newest samples.
    pub frequency_buffer: Box<[Complex<f32>]>,
    /// The [`BASS_WINDOW_SIZE`] FFT of the whole window.
    pub bass_frequency_buffer: Box<[Complex<f32>]>,
    /// Magnitudes of `frequency_buffer`.
    pub treble_magnitude_buffer: Box<[f32]>,
    /// Magnitudes of `bass_frequency_buffer`.
    pub bass_magnitude_buffer: Box<[f32]>,
}

impl Default for ProcessingFrame {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessingFrame {
    /// Zeroed buffers, allocated once at startup.
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

/// Parameters a frontend writes and the audio thread only reads, each its own
/// atomic.
pub struct ConfigAtomics {
    /// Minimum RMS amplitude required to leave `Silence`.
    pub silence_threshold: AtomicU32,
    /// NHWRSF above which a frame is an onset.
    pub nhwrsf_threshold: AtomicU32,
    /// Sustain stability above which the signal counts as stable.
    pub sustain_stability_threshold: AtomicU32,
    /// The nominated key (0–87), or 255 for none.
    pub target_note: AtomicU8,
}

/// Observations the audio thread writes each hop, readable from any number of
/// threads (the `FrameOutput` buffer has a single reader).
pub struct RuntimeAtomics {
    /// Smoothed RMS amplitude.
    pub current_rms_ema: AtomicU32,
    /// Spectral flux.
    pub current_nhwrsf: AtomicU32,
}

/// The wait-free state the DSP thread and a frontend share. `Ordering::Relaxed`
/// throughout: the scalars are independent, with no happens-before chain.
pub struct PipelineAtomics {
    /// UI → DSP parameters.
    pub config: ConfigAtomics,
    /// DSP → UI observations.
    pub runtime: RuntimeAtomics,
    /// UI → DSP: shutdown, checked every loop iteration.
    pub shutdown: AtomicBool,
    /// Worker → DSP: a dispatched capture is still with the Worker. Set by the
    /// pipeline as it dispatches, and cleared by the Worker once the buffers are
    /// back, which returns the lifecycle to Idle.
    // No `compare_exchange` needed: the pipeline holds `Processing` until it
    // reads the flag clear, so the two writers strictly alternate.
    pub(crate) capture_in_flight: AtomicBool,
}

impl Default for PipelineAtomics {
    fn default() -> Self {
        Self {
            config: ConfigAtomics {
                silence_threshold: AtomicU32::new(0.005_f32.to_bits()),
                nhwrsf_threshold: AtomicU32::new(0.9_f32.to_bits()),
                sustain_stability_threshold: AtomicU32::new(10.0_f32.to_bits()),
                target_note: AtomicU8::new(255), // No key nominated
            },
            runtime: RuntimeAtomics {
                current_rms_ema: AtomicU32::new(0.0_f32.to_bits()),
                current_nhwrsf: AtomicU32::new(0.0_f32.to_bits()),
            },
            shutdown: AtomicBool::new(false),
            capture_in_flight: AtomicBool::new(false),
        }
    }
}

// ─── AudioPipeline (Mediator) ────────────────────────────────────────────────

/// The audio thread's half of the pipeline: it owns the DSP components and
/// exchanges parameters and observations with a frontend through
/// [`PipelineAtomics`].
pub struct AudioPipeline {
    /// The signal validator.
    pub gatekeeper: Gatekeeper,
    /// The fundamental-frequency engine.
    pub engine: Engine,
    /// Live per-key discovery templates, updated in place from `profile_rx`.
    live_profiles: Box<[KeyProfile; 88]>,
    /// Crossing #4 consumer for template updates.
    profile_rx: HeapCons<KeyProfileUpdate>,
    /// The strobe bank, which runs every hop while references are set, whatever
    /// the engine's lock.
    strobe: Strobe,
    /// Crossing #4 consumer for strobe reference updates.
    strobe_rx: HeapCons<StrobeRefUpdate>,
    /// Crossing #4 consumer for capture-lifecycle commands.
    command_rx: HeapCons<CaptureCommand>,
    atomics: Arc<PipelineAtomics>,
    audio_pool: Arc<AudioPool>,
    cola: CircularFifo,
    fft_instance: Arc<dyn RealToComplex<f32>>,
    fft_bass_instance: Arc<dyn RealToComplex<f32>>,
    processing_frame: ProcessingFrame,
    pub capture_tx: Sender<CapturePayload>,
    /// Where the lifecycle stands. Every transition is made here, so an
    /// out-of-sequence one has no code path to come from.
    capture_state: CaptureState,
    capture_buffer: Option<Box<[f32]>>,
    capture_count: usize,
    /// This hop's capture command, drained at step 0 and consumed at step 6 —
    /// so an arm can start a record on the hop it arrives.
    pending_command: Option<CaptureCommand>,
    /// What the next record will be: the standing [`CaptureCommand::Arm`],
    /// already clamped. Latched at `Armed → Recording`.
    arm: ArmRequest,
    /// What the recording in progress is, sampled at `Armed → Recording`.
    latch: CaptureLatch,
    full_event_buffer: Option<Box<[f32]>>,
    full_event_count: usize,
    /// Circular history of the raw stream, for a diagnostic capture's pre-roll.
    history_buffer: Box<[f32; ONSET_HISTORY_SAMPLES]>,
    history_idx: usize,
    /// An onset arrived while Armed; cleared when a record starts or on silence.
    capture_onset_pending: bool,
    /// The key discovery latched during the record: what an Auto capture is
    /// filed under.
    latched_auto_key: Option<u8>,
    /// The engine's last partial-1 frequency.
    pub last_measured_f0: Option<f32>,
}

/// Frontend-side handle to the pipeline's shareable atomic state (crossing #3):
/// wait-free config writes and runtime reads. Cloneable.
#[derive(Clone)]
pub struct PipelineHandle {
    /// The shared atomic state.
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

/// The frontend's end of every channel, returned by [`AudioPipeline::new`] beside
/// the pipeline.
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
    /// UI → DSP producer for capture-lifecycle commands (crossing #4's third
    /// instance).
    pub capture_commands: CaptureSender,
}

impl AudioPipeline {
    /// Creates the pipeline, to move to the audio thread, and the frontend's
    /// [`PipelinePorts`], and starts the Worker. `dump_dir` is where the Worker
    /// writes capture dumps; `None` writes none. With
    /// [`APPLY_MEASURED_B_TO_DISCOVERY`] on, it also seeds the templates from the
    /// profile at [`PROFILE_PATH`].
    pub fn new(dump_dir: Option<PathBuf>) -> (Self, PipelinePorts) {
        let audio_pool = Arc::new(ArrayQueue::new(AUDIO_POOL_CAPACITY));
        for _ in 0..AUDIO_POOL_CAPACITY {
            let _ = audio_pool.push(vec![0.0; CAPTURE_MAX_SAMPLES].into_boxed_slice());
        }

        let atomics = Arc::new(PipelineAtomics::default());

        let gatekeeper = Gatekeeper::new();
        let engine = Engine::new(SAMPLE_RATE);

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

        // Crossing #4, third instance: capture-lifecycle commands (UI → DSP).
        let (command_tx, command_rx) =
            HeapRb::<CaptureCommand>::new(CAPTURE_COMMAND_QUEUE_CAPACITY).split();

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
            command_rx,
            atomics: Arc::clone(&atomics),
            audio_pool,
            cola: CircularFifo::new(BASS_WINDOW_SIZE),
            fft_instance,
            fft_bass_instance,
            processing_frame: ProcessingFrame::new(),
            capture_tx,
            capture_state: CaptureState::Idle,
            capture_buffer: None,
            capture_count: 0,
            pending_command: None,
            arm: ArmRequest::default(),
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
            capture_commands: CaptureSender { tx: command_tx },
        };

        (pipeline, ports)
    }

    /// Pushes raw samples into the pipeline, returning the hop's [`FrameOutput`]
    /// when a hop completes.
    pub fn push_audio(&mut self, samples: &[f32]) -> Option<FrameOutput> {
        self.cola.push_samples(samples);

        if self.cola.is_hop_ready(HOP_SIZE) {
            self.process_cola_hop()
        } else {
            None
        }
    }

    /// Returns every buffer the pending take borrowed and clears what it
    /// accumulated. Nothing is dispatched, so the take reaches neither the
    /// profile nor the disk.
    fn abandon_take(&mut self) {
        if let Some(buf) = self.capture_buffer.take() {
            let _ = self.audio_pool.push(buf);
        }
        if let Some(dbuf) = self.full_event_buffer.take() {
            let _ = self.audio_pool.push(dbuf);
        }
        self.capture_count = 0;
        self.full_event_count = 0;
        self.capture_onset_pending = false;
        self.latched_auto_key = None;
    }

    /// Processes one hop of the COLA buffer.
    fn process_cola_hop(&mut self) -> Option<FrameOutput> {
        // ─── Step 0: Drain Profile Updates (Crossing #4) ───
        // A plain move into the pre-allocated table: no allocation.
        while let Some(update) = self.profile_rx.try_pop() {
            if let Some(slot) = self.live_profiles.get_mut(update.key_index as usize) {
                *slot = update.profile;
            }
        }

        // Strobe references: only the newest update matters.
        let mut strobe_update = None;
        while let Some(update) = self.strobe_rx.try_pop() {
            strobe_update = Some(update);
        }
        if let Some(update) = strobe_update {
            self.strobe.retarget(update);
        }

        // Capture commands: the newest supersedes the rest. Applied at step 6.
        while let Some(command) = self.command_rx.try_pop() {
            self.pending_command = Some(command);
        }

        // ─── Step 1: COLA & Windowing ───

        self.cola.read_window(
            BASS_WINDOW_SIZE,
            &mut self.processing_frame.audio_buffer[..BASS_WINDOW_SIZE],
        );

        // The newest WINDOW_SIZE samples are at the end of the buffer.
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

        // The pre-roll history takes the hop's newest samples, so it follows the
        // DSP clock rather than the driver's chunks.
        let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
        let new_samples = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];
        for &s in new_samples {
            self.history_buffer[self.history_idx] = s;
            self.history_idx = (self.history_idx + 1) % self.history_buffer.len();
        }

        // ─── Step 2: Read Shared Atomics ───

        self.gatekeeper.config.silence_threshold = load_f32(&self.atomics.config.silence_threshold);
        self.gatekeeper.config.nhwrsf_threshold = load_f32(&self.atomics.config.nhwrsf_threshold);
        self.gatekeeper.config.sustain_stability_threshold =
            load_f32(&self.atomics.config.sustain_stability_threshold);
        self.engine.noise_floor = load_f32(&self.atomics.config.silence_threshold);

        let target_note = match self.atomics.config.target_note.load(Ordering::Relaxed) {
            255 => None,
            val if val < 88 => Some(val),
            _ => None,
        };

        // ─── Step 3: Signal Gating (Gatekeeper) ───

        let gate_result = self.gatekeeper.process_frame(&self.processing_frame);
        let is_silence = gate_result.state == SignalState::Silence;
        let is_stable = gate_result.state == SignalState::Stable;

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

        // The Worker owns the end of the lifecycle, so read that before
        // anything else: a command arriving this hop can then act on a
        // capture that has just finished.
        if self.capture_state == CaptureState::Processing
            && !self.atomics.capture_in_flight.load(Ordering::Relaxed)
        {
            self.capture_state = CaptureState::Idle;
        }

        // Then the hop's command, so an arm can start a record on the hop it
        // arrives rather than the one after.
        if let Some(command) = self.pending_command.take() {
            match command {
                CaptureCommand::Arm(request) => {
                    self.arm = ArmRequest {
                        // Clamped, not trusted: another thread writes this.
                        target_samples: request.target_samples.clamp(HOP_SIZE, CAPTURE_MAX_SAMPLES),
                        ..request
                    };
                    // A re-arm while armed updates the request above and
                    // nothing else; from `Recording` or `Processing` it does
                    // not apply.
                    if self.capture_state == CaptureState::Idle {
                        self.capture_state = CaptureState::Armed;
                    }
                }
                CaptureCommand::Cancel => {
                    if matches!(
                        self.capture_state,
                        CaptureState::Armed | CaptureState::Recording
                    ) {
                        self.capture_state = CaptureState::Idle;
                        self.abandon_take();
                    }
                }
            }
        }

        if gate_result.state == SignalState::Silence {
            self.capture_onset_pending = false;
            // A false transient's diagnostic buffer goes back to the pool.
            if self.capture_state == CaptureState::Armed {
                if let Some(dbuf) = self.full_event_buffer.take() {
                    let _ = self.audio_pool.push(dbuf);
                }
                self.full_event_count = 0;
            }
        }

        // Two `if`s, not `else if`: a capture that starts recording here must
        // record this same hop in the `Recording` block below.

        if self.capture_state == CaptureState::Armed {
            if gate_result.is_new_onset {
                self.capture_onset_pending = true;
                // An abandoned diagnostic buffer goes back to the pool first.
                if let Some(old_buf) = self.full_event_buffer.take() {
                    let _ = self.audio_pool.push(old_buf);
                }
                self.full_event_count = 0;

                // Pre-roll from the history.
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
                self.capture_state = CaptureState::Recording;
                self.capture_onset_pending = false;
                self.capture_buffer = Some(buf);
                self.capture_count = 0;
                self.latch = CaptureLatch {
                    request: self.arm,
                    target_note: self.atomics.config.target_note.load(Ordering::Relaxed),
                };
            }
        }

        // The diagnostic record, after the block above so it includes the
        // onset's first hop.
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

        if self.capture_state == CaptureState::Recording {
            // ── Latch ──
            if let Some(ref result) = pitch_result {
                self.latched_auto_key = Some(result.key_index);
                self.last_measured_f0 = result.measured_f0;
            }

            if let Some(mut buf) = self.capture_buffer.take() {
                let start_idx = BASS_WINDOW_SIZE - HOP_SIZE;
                let src_slice = &self.processing_frame.audio_buffer[start_idx..BASS_WINDOW_SIZE];

                let target_samples = self.latch.request.target_samples;
                let remaining = target_samples - self.capture_count;
                let to_copy = src_slice.len().min(remaining);

                buf[self.capture_count..self.capture_count + to_copy]
                    .copy_from_slice(&src_slice[..to_copy]);

                self.capture_count += to_copy;

                let done = self.capture_count == target_samples;
                // An extended record is a request for the audio past the decay,
                // so only the shipped length keeps the short-dispatch valve.
                let decayed = gate_result.state == SignalState::Silence
                    && target_samples <= CAPTURE_DEFAULT_SAMPLES;

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
                            sample_rate: SAMPLE_RATE,
                            noise_floor: load_f32(&self.atomics.config.silence_threshold),
                            nhwrsf_threshold: load_f32(&self.atomics.config.nhwrsf_threshold),
                            sustain_stability_threshold: load_f32(
                                &self.atomics.config.sustain_stability_threshold,
                            ),
                            measured_f0: self.last_measured_f0,
                            captured_in_auto: target_note == 255,
                            sounding_strings: self.latch.request.declared_strings,
                        };
                        self.full_event_count = 0;

                        // Raised before the payload goes over, so the Worker
                        // cannot clear it before this thread has set it.
                        self.atomics
                            .capture_in_flight
                            .store(true, Ordering::Relaxed);
                        self.capture_state = CaptureState::Processing;

                        // Recover the buffers if the worker is backed up —
                        // nothing was handed over, so nothing is in flight.
                        if let Err(e) = self.capture_tx.try_send(payload) {
                            let dropped = e.into_inner();
                            let _ = self.audio_pool.push(dropped.stable_buffer);
                            if let Some(dbuf) = dropped.full_event_buffer {
                                let _ = self.audio_pool.push(dbuf);
                            }
                            self.atomics
                                .capture_in_flight
                                .store(false, Ordering::Relaxed);
                            self.capture_state = CaptureState::Armed;
                        }
                    } else {
                        // No key to file it under: recycle and re-arm.
                        let _ = self.audio_pool.push(buf);
                        if let Some(dbuf) = self.full_event_buffer.take() {
                            let _ = self.audio_pool.push(dbuf);
                        }
                        self.full_event_count = 0;
                        self.capture_state = CaptureState::Armed;
                    }
                    self.latched_auto_key = None;
                } else {
                    self.capture_buffer = Some(buf);
                }
            }
        }

        // ─── Step 7: Triple Buffer Telemetry Assembly ───

        let mut frame_output = FrameOutput::default();
        frame_output.magnitudes[..mag_count]
            .copy_from_slice(&self.processing_frame.treble_magnitude_buffer[..mag_count]);
        frame_output.magnitude_len = mag_count;

        // The onset flag stays internal.
        frame_output.rms_ema = gate_result.rms_ema;
        frame_output.nhwrsf = gate_result.nhwrsf;
        frame_output.sustain_stability = gate_result.sustain_stability_ema;
        frame_output.is_silence = is_silence;

        // Strobe telemetry, whether or not the engine holds a lock.
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
        frame_output.capture_state = self.capture_state;
        // The buffer is held exactly while a record is in progress.
        frame_output.capture_progress_samples = if self.capture_buffer.is_some() {
            self.capture_count
        } else {
            0
        };

        if let Some(result) = pitch_result {
            frame_output.detected_frequency = result.measured_f0;
            frame_output.note_index = Some(result.key_index);

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
            // Deliberately implausible measured_f0 — it must not leak into the
            // template (ET-centered, β-only is the contract).
            measured_f0: 9999.0,
            partials: Vec::new(),
            calculated_b,
            last_captured: String::new(),
            captured_in_auto: true,
            sounding_strings: None,
        }
    }

    // The capture lengths are durations — 1.5 s is the figure every capture
    // set was recorded at, and the one report 0009's σ model was measured on.
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
        // …at the equal-temperament center, not the (stale) measured_f0.
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
        // enough precision over two seconds to detune the test signal.
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

    /// A command asks; the pipeline is what moves the lifecycle, on the hop the
    /// command arrives — and it publishes where it stands on every frame.
    #[test]
    fn arm_and_cancel_commands_move_the_lifecycle() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let hop = [0.0f32; HOP_SIZE];

        assert_eq!(pipeline.capture_state, CaptureState::Idle);

        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            declared_strings: None,
        })));
        let frame = pipeline.push_audio(&hop).expect("a hop's worth of audio");
        assert_eq!(
            pipeline.capture_state,
            CaptureState::Armed,
            "the pipeline makes Idle → Armed itself"
        );
        assert_eq!(
            frame.capture_state,
            CaptureState::Armed,
            "and publishes it on the same frame"
        );

        assert!(ports.capture_commands.send(CaptureCommand::Cancel));
        pipeline.push_audio(&hop);
        assert_eq!(pipeline.capture_state, CaptureState::Idle, "Cancel disarms");
    }

    /// A command that does not apply in the current state is an ordinary
    /// outcome: it leaves the lifecycle alone rather than forcing it.
    #[test]
    fn a_command_that_does_not_apply_leaves_the_lifecycle_alone() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let hop = [0.0f32; HOP_SIZE];

        // Cancel from Idle: nothing pending, nothing to drop.
        assert!(ports.capture_commands.send(CaptureCommand::Cancel));
        pipeline.push_audio(&hop);
        assert_eq!(pipeline.capture_state, CaptureState::Idle);

        // An Arm cannot jump the queue while the Worker still holds a capture.
        pipeline.capture_state = CaptureState::Processing;
        pipeline
            .atomics
            .capture_in_flight
            .store(true, Ordering::Relaxed);
        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            declared_strings: None,
        })));
        pipeline.push_audio(&hop);
        assert_eq!(pipeline.capture_state, CaptureState::Processing);

        // Clearing the flag is what ends it — the Worker's only say.
        pipeline
            .atomics
            .capture_in_flight
            .store(false, Ordering::Relaxed);
        pipeline.push_audio(&hop);
        assert_eq!(pipeline.capture_state, CaptureState::Idle);
    }

    /// A request changed after arming must still reach the capture, so a
    /// second `Arm` restates it without disturbing the lifecycle — which is
    /// what lets a sender that arms on its own schedule stay correct.
    #[test]
    fn a_second_arm_restates_the_request_and_holds_the_lifecycle() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let hop = [0.0f32; HOP_SIZE];

        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            declared_strings: None,
        })));
        pipeline.push_audio(&hop);

        let solo = SoundingStrings::UNDECLARED.toggled(0);
        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            target_samples: 3 * SAMPLE_RATE as usize,
            declared_strings: solo.declared(),
        })));
        pipeline.push_audio(&hop);

        assert_eq!(
            pipeline.capture_state,
            CaptureState::Armed,
            "a re-arm while armed moves nothing"
        );
        assert_eq!(pipeline.arm.target_samples, 3 * SAMPLE_RATE as usize);
        assert_eq!(pipeline.arm.declared_strings, Some(solo));
    }

    /// The fill target crosses from another thread, so the pipeline clamps it
    /// rather than trusting it — a buffer is [`CAPTURE_MAX_SAMPLES`] long and
    /// a hop is the shortest record that can complete.
    #[test]
    fn an_arms_fill_target_is_clamped_on_receipt() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let hop = [0.0f32; HOP_SIZE];

        for (asked, expected) in [(1usize, HOP_SIZE), (usize::MAX, CAPTURE_MAX_SAMPLES)] {
            assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
                target_samples: asked,
                declared_strings: None,
            })));
            pipeline.push_audio(&hop);
            assert_eq!(pipeline.arm.target_samples, expected);
        }
    }

    /// Feeds `secs` of a pure tone one hop at a time, recording every capture
    /// state the run passes through. Stops early once `stop` says so.
    fn drive(
        pipeline: &mut AudioPipeline,
        hz: f32,
        secs: f32,
        n: &mut u64,
        seen: &mut Vec<CaptureState>,
        mut stop: impl FnMut(&[CaptureState]) -> bool,
    ) {
        let step = 2.0 * std::f64::consts::PI * hz as f64 / SAMPLE_RATE as f64;
        let mut hop = [0.0f32; HOP_SIZE];
        for _ in 0..((secs * SAMPLE_RATE as f32) as usize / HOP_SIZE) {
            for s in hop.iter_mut() {
                *s = 0.2 * (step * *n as f64).sin() as f32;
                *n += 1;
            }
            pipeline.push_audio(&hop);
            if seen.last() != Some(&pipeline.capture_state) {
                seen.push(pipeline.capture_state);
            }
            if stop(seen) {
                return;
            }
        }
    }

    /// The whole lifecycle, end to end: an `Arm` command, a struck note, the
    /// record filling, and a `KeyMeasurement` back from the Worker.
    #[test]
    fn a_capture_runs_from_arm_command_to_measurement() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let atomics = ports.handle.atomics.clone();
        // Manual mode on A4, so the dispatch gate does not depend on discovery
        // locking a pure sine.
        atomics.config.target_note.store(48, Ordering::Relaxed);

        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            target_samples: CAPTURE_DEFAULT_SAMPLES,
            declared_strings: None,
        })));

        let mut n = 0u64;
        let mut seen = Vec::new();
        drive(&mut pipeline, 440.0, 6.0, &mut n, &mut seen, |seen| {
            seen.contains(&CaptureState::Processing)
        });

        assert!(seen.contains(&CaptureState::Armed), "lifecycle {seen:?}");
        assert!(
            seen.contains(&CaptureState::Recording),
            "the strike must start a record; lifecycle {seen:?}"
        );
        assert!(
            seen.contains(&CaptureState::Processing),
            "the filled record must dispatch; lifecycle {seen:?}"
        );

        match ports
            .worker_rx
            .recv_timeout(std::time::Duration::from_secs(30))
        {
            Ok(WorkerOutput::Measurement(m)) => assert_eq!(m.key_index, 48),
            other => panic!("expected a measurement, got {:?}", other.is_ok()),
        }

        // The Worker's completion is a flag, not a state write: the pipeline
        // reads it on its next hop and ends the lifecycle itself.
        drive(&mut pipeline, 440.0, 0.1, &mut n, &mut seen, |_| false);
        assert_eq!(pipeline.capture_state, CaptureState::Idle);
        assert_eq!(
            pipeline.audio_pool.len(),
            AUDIO_POOL_CAPACITY,
            "and the Worker's buffers are home before it clears"
        );
    }

    /// A `Cancel` mid-record: the take is dropped, its buffers go back to the
    /// pool, and nothing reaches the Worker.
    #[test]
    fn cancel_drops_the_record_in_progress() {
        let (mut pipeline, mut ports) = AudioPipeline::new(None);
        let atomics = ports.handle.atomics.clone();
        atomics.config.target_note.store(48, Ordering::Relaxed);

        assert!(ports.capture_commands.send(CaptureCommand::Arm(ArmRequest {
            // Long enough that the record cannot finish before the Stop.
            target_samples: CAPTURE_MAX_SAMPLES,
            declared_strings: None,
        })));

        let mut n = 0u64;
        let mut seen = Vec::new();
        drive(&mut pipeline, 440.0, 4.0, &mut n, &mut seen, |seen| {
            seen.contains(&CaptureState::Recording)
        });
        assert!(
            seen.contains(&CaptureState::Recording),
            "lifecycle {seen:?}"
        );

        assert!(ports.capture_commands.send(CaptureCommand::Cancel));
        drive(&mut pipeline, 440.0, 0.1, &mut n, &mut seen, |_| false);

        assert_eq!(
            pipeline.capture_state,
            CaptureState::Idle,
            "Cancel drops the take; lifecycle {seen:?}"
        );
        assert!(
            ports.worker_rx.try_recv().is_err(),
            "a dropped take reaches neither the profile nor the disk"
        );
        assert_eq!(
            pipeline.audio_pool.len(),
            AUDIO_POOL_CAPACITY,
            "its buffers go back to the pool"
        );
    }

    #[test]
    fn default_profiles_match_rigaud_prior() {
        let profiles = build_default_profiles();
        assert_eq!(profiles[0].beta, get_expected_beta(0));
        assert_eq!(profiles[87].beta, get_expected_beta(87));
        assert_eq!(profiles[40].f0_et, NOTES[40].frequency);
    }
}
