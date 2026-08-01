//! # Inharmonicity - Professional Piano Tuning GUI
//!
//! This module contains the main GUI application for the Inharmonicity piano tuning software.
//! It provides a real-time interface for audio analysis, spectrogram visualization, and
//! interactive piano keyboard controls.
//!
//! ## Architecture
//! - **Main Thread**: Iced GUI application with dark theme
//! - **Audio Thread**: Dedicated thread for real-time audio processing
//! - **Communication**: Wait-free SPSC primitives (rtrb + triple_buffer + atomics)
//! - **Updates**: 60 FPS continuous updates via subscription system

use crate::library::{self, AppSettings, ProfileSort};
use crate::session::ProfileSession;
use crate::views::{main_view::create_main_view, settings_view::create_settings_view};
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use iced::{self, Element, Subscription, Theme};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::Instant;
use tuner_core::{
    FrameOutput,
    algorithms::curves,
    audio::{self, AudioSource, HOP_RATE_HZ, HostHandle},
    models::{self, CurveInput, InharmonicityProfile, NOTES},
    pipeline::{CaptureState, PipelineHandle, load_f32, store_f32},
    strobe::StrobeRefUpdate,
    worker::{self, CurveBundle, CurveJob, WorkerOutput},
};

/// Boxcar length for the cent meter's displayed value, as a duration: the mean
/// of the last `SMOOTHING_SECS` of per-hop cents readings.
///
/// Ours, by feel — it exists because the raw per-frame estimate was erratic to
/// read. Its cost is the usual one for a mean: half the window in lag (≈ 58 ms)
/// for a √n ≈ 2.2× reduction in white noise. Nothing has superseded it *for this
/// readout*; the strobe panel's band-slope read is the better instrument (it fits
/// a line to accumulated phase rather than averaging per-frame estimates, ≈ 0.05 ¢
/// on a clean signal) but it is per-partial and manual-mode only, so the cent
/// meter cannot borrow it without being restructured around phase.
const SMOOTHING_SECS: f32 = 0.116;

/// The above at the hop cadence — a count, but derived from the duration that is
/// actually meant, so it survives a change to the hop size or sample rate.
/// Rounded, not truncated: a bare cast would silently drop this to 4 frames.
const SMOOTHING_FACTOR: usize = (SMOOTHING_SECS * HOP_RATE_HZ + 0.5) as usize;

/// Main entry point for the Inharmonicity application.
///
/// Initializes the Iced GUI application with dark theme, real-time audio processing,
/// and continuous updates for smooth visualization.
pub fn main() -> iced::Result {
    eprintln!("[MAIN] Starting Inharmonicity application...");
    eprintln!("[MAIN] Initializing GUI framework...");
    let result = iced::application(TunerApp::new, TunerApp::update, TunerApp::view)
        .title("Inharmonicity")
        .subscription(TunerApp::subscription)
        .theme(TunerApp::theme)
        // Disable instant exit on close to allow the audio thread
        // to cleanly join/drop without causing a CPAL/ALSA segfault.
        .window(iced::window::Settings {
            exit_on_close_request: false,
            ..Default::default()
        })
        .run();
    eprintln!("[MAIN] Application finished with result: {:?}", result);
    result
}

/// Application message types for the Iced GUI framework.
///
/// These messages are sent between the GUI and the application logic to handle
/// user interactions, audio processing updates, and tool visibility toggles.
#[derive(Debug, Clone)]
pub enum Message {
    // Piano keyboard interactions
    KeySelected(u8),  // User selected a piano key (0-87)
    SwitchToAutoMode, // Switch from manual to automatic pitch detection

    // --- Messages for Inharmonicity Measurement & Profile ---
    ToggleMeasurementMode, // Toggle the partial measurement mode
    CaptureButtonClicked,  // Capture button was clicked (behavior depends on current state)
    UndoLastCapture,       // Reverts the last capture of a key (Manual or Auto)
    SaveProfile,           // Explicit flush; the profile also auto-saves
    // ----------------------------------------------

    // --- Profile library (the browser over the profiles directory) ---
    /// Open this instrument's profile, adopting its settings.
    OpenProfile(PathBuf),
    /// Start a fresh, empty instrument.
    NewProfile,
    /// Delete a profile document (never the open one).
    DeleteProfile(PathBuf),
    /// Copy a profile as a new instrument record.
    DuplicateProfile(PathBuf),
    /// Reorder the browser.
    LibrarySortChanged(ProfileSort),
    /// Filter the browser across every identifying field.
    LibrarySearchChanged(String),
    /// Show or hide the instrument-library settings panel.
    ToggleLibrary,
    /// Edit one text field of the open instrument's identity.
    IdentityFieldChanged(IdentityField, String),
    /// Change the open instrument's family.
    InstrumentKindChanged(models::InstrumentKind),
    // ----------------------------------------------

    // Settings menu items (placeholder for future implementation)
    Temperament,     // Temperament selection
    TuningStandard,  // Tuning standard (A440, etc.)
    InharmonicCurve, // Inharmonicity curve adjustment
    SampleBuffer,    // Sample buffer size adjustment
    TuningProfile,   // Tuning profile management

    // Application control
    Exit, // Application exit request

    // Working tool visibility toggles
    ToggleSpectrogram,               // Show/hide spectrogram panel
    ToggleCentMeter,                 // Show/hide cent meter panel
    ToggleKeySelect,                 // Show/hide piano keyboard
    TogglePartials,                  // Show/hide partials panel
    ToggleCurvePlot,                 // Show/hide the live tuning-curve plot (design §10)
    ToggleStrobe,                    // Show/hide the strobe panel (design §5)
    SetReferenceMode(ReferenceMode), // Which target function all readouts use (design §5)
    RequestRelock,                   // Open the re-lock confirm modal (design §8)
    ConfirmRelock,                   // Copy the live bundle into the strobe lock (design §8)
    CancelRelock,                    // Dismiss the re-lock modal (design §8)
    // ToggleInharmonicityGraph, // Show/hide inharmonicity graph

    // --- Curve gallery messages (design §9; display-only selection, D7) ---
    ToggleCurveSelect,               // Open/close the curve gallery in settings
    CurveDetailOpened(EngineChoice), // Thumbnail clicked → detail view
    CurveDetailClosed,               // Back from detail to the gallery
    EngineSelected(EngineChoice),    // Sets which curve the plot/strobe show

    // Settings view toggles
    ToggleSettingsView,           // Toggle main view versus settings view
    ToggleNoiseFloorAdjustment,   // Show/hide noise floor envelope viewer
    SilenceThresholdChanged(f32), // User dragged the silence threshold slider
    RecalibrateNoiseFloor,        // User clicked the recalibrate button

    // --- Transient Calibration Messages ---
    ToggleTransientCalibration,
    ResetTransientScope,
    NhwrsfThresholdChanged(f32),

    // --- NINOS2 Calibration Messages ---
    ToggleNinosCalibration,
    ResetNinosScope,
    NinosThresholdChanged(f32),

    // --- Instrument select (debug) ---
    ToggleInstrumentSelect, // Open/close the instrument-select settings panel
    SetInstrument(Instrument), // Swap the main-view note-select surface

    // Continuous update message
    Tick, // Timer tick for real-time updates
}

/// Tuning mode for the piano tuner application.
///
/// Determines whether the application is in automatic pitch detection mode
/// or manual key selection mode.
#[derive(Debug, Clone, PartialEq)]
pub enum TuningMode {
    /// Automatic pitch detection mode - detects any note being played
    Auto,
    /// Manual mode - user has selected a specific piano key to tune
    Manual {
        key_index: u8,     // Piano key index (0-87)
        note_name: String, // Note name (e.g., "A4", "C#3")
    },
}

/// Which note-select surface the main view shows (debug convenience). The
/// manual-mode and DSP contract are instrument-agnostic — everything downstream
/// keys off the 0–87 `key_index` — so this only swaps the on-screen picker:
/// `Piano` renders the 88-key keyboard, `Guitar` six standard-tuning string
/// buttons (EADGBE). Guitar exists to exercise the strobe/manual path against a
/// non-piano source; it measures no new inharmonicity. Switching is handled by
/// `App::set_instrument`, which keeps the strobe reference and manual selection
/// coherent across the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Instrument {
    Piano,
    Guitar,
}

/// Which text field of [`models::InstrumentIdentity`] an edit targets. One
/// message with a field tag rather than seven near-identical messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityField {
    /// Display name — the only field that is not optional.
    Name,
    /// Manufacturer.
    Make,
    /// Model designation.
    Model,
    /// Serial number.
    Serial,
    /// Body form within the family.
    Form,
    /// Owner.
    Owner,
    /// Free text.
    Notes,
}

/// The engine and reference-mode selections both persist with the profile
/// (reopening an instrument reproduces the targets it was tuned to), so they
/// are defined in `tuner-core::models` alongside the profile that carries them.
/// Re-exported here because the whole frontend refers to them unqualified.
pub use tuner_core::models::{EngineChoice, ReferenceMode};

/// Display state of the manual-mode strobe (design §5, §13). Path A: the
/// beat phase is **read** from `FrameOutput.strobe_angle` — the DSP-side
/// `Strobe` accumulates it against the pushed curve references (R1/R2)
/// — so this is pure display data, no GUI-side integration.
#[derive(Debug, Clone)]
pub struct StrobeState {
    /// The displayed partial's accumulated beat phase (cycles, [0, 1)),
    /// from the freshest frame; held at its last value while gated.
    pub beat_phase: f32,
    /// Displayed partial n* — `CurveBundle::display_partials` for measured
    /// keys; forced to 1 when the key has no measured B, because the n = 1
    /// target is exact for any B (R4) while higher partials need the raw
    /// measured B to avoid a false beat.
    pub n_star: u8,
    /// The displayed partial's curve target (Hz), if resolvable.
    pub ref_hz: Option<f32>,
    /// D3 amplitude gate, as reported by the strobe bank — band frozen.
    pub gated: bool,
    /// Unwrapped accumulated beat phase (cycles) for the band-slope readout,
    /// reconstructed on the GUI side from the wrapped [`Self::beat_phase`].
    pub band_unwrapped: f32,
    /// Previous frame's wrapped angle, for the unwrap delta. `None` = no
    /// baseline yet (first ungated frame after a (re)target or re-strike).
    pub band_prev_raw: Option<f32>,
    /// Time-stamped `(t, band_unwrapped)` history over the last
    /// [`BAND_SLOPE_WINDOW_SECS`] — least-squares-fit to the beat rate.
    pub band_history: VecDeque<(Instant, f32)>,
    /// Cents-vs-target from the band's rotation *rate* (slope of the history).
    /// The low-jitter readout: the band integrates phase, so its slope has
    /// variance ~1/T³ vs the instantaneous estimate's per-hop noise. `None`
    /// until the window spans [`BAND_SLOPE_MIN_SPAN_SECS`] with enough of its
    /// frames ([`BAND_SLOPE_MIN_FRAME_FRACTION`]); held while gated.
    pub band_cents: Option<f32>,
    /// Partial the coarse read is centred on (`curves::coarse_read_partial`).
    /// Independent of [`Self::n_star`]: the band and the coarse number answer
    /// different questions, so the same key can use different partials.
    pub coarse_n: u8,
    /// Debounced verdict: is the band's offset past [`BAND_READABLE_HZ`], where
    /// its unwrap aliases? Decided here rather than in the view because it needs
    /// state — a bare threshold on a noisy estimate chatters at the threshold
    /// (see [`READOUT_SWITCH_HOPS`]). The *hard* conditions (gated band, unfilled
    /// window) are facts and act immediately; only this one is debounced.
    pub out_of_range: bool,
    /// Consecutive hops whose range verdict opposes [`Self::out_of_range`].
    pub range_run: u8,
    /// Cents-vs-target from `FrameOutput.coarse_hz` — the wide-range readout,
    /// valid at any detuning. Jitterier than [`Self::band_cents`] and dropped
    /// the moment a hop yields nothing, so it never shows a stale number.
    ///
    /// Read against the *coarse* partial's own reference, which is exact: for
    /// `fₙ = n·f₀·√(1+Bn²)` the cents offset is linear in f₀, so a partial's
    /// deviation from its target equals the string's deviation from its target.
    pub coarse_cents: Option<f32>,
}

impl Default for StrobeState {
    fn default() -> Self {
        Self {
            beat_phase: 0.0,
            n_star: 1,
            ref_hz: None,
            gated: true,
            band_unwrapped: 0.0,
            band_prev_raw: None,
            band_history: VecDeque::new(),
            band_cents: None,
            coarse_n: 1,
            coarse_cents: None,
            out_of_range: false,
            range_run: 0,
        }
    }
}

/// Sliding window the band-slope readout fits over (§5.5 / R12).
///
/// The trade is **linear cost against an inelastic benefit** (ADR 0011 §11): the
/// fit lags a turning peg by exactly `T/2` — an OLS slope estimates the rate at
/// the window's *midpoint* — while the jitter it buys falls far slower than the
/// `T⁻³` law, because the bank's analysis windows overlap 75–87 % and extra hops
/// are mostly the same audio. Measured, tripling `T` buys 1.8× on piano and
/// nothing measurable on guitar.
///
/// 0.6 s is the responsive-enough end of that trade in use, and 1 ¢ of steadiness
/// is inside the range a professional tuner works in. To trade the 300 ms of lag
/// back, shorten this; two lengths are derived rather than arbitrary, and each
/// *removes* a constant by absorbing [`BAND_SLOPE_MIN_SPAN_SECS`] (which can never
/// exceed it):
/// `BASS_WINDOW_SIZE / SAMPLE_RATE` ≈ 0.186 s matches the coarse read's own group
/// delay, so the two readouts lag equally and the source switch stops stepping
/// the number; 0.25 s simply collapses the two constants. Escaping the trade
/// rather than moving along it needs a two-state (g–h / Kalman) estimator on
/// (phase, rate), which weights old data instead of discarding it.
const BAND_SLOPE_WINDOW_SECS: f32 = 0.6;

/// Minimum span before a rate is shown; the coarse read covers the fill-in.
///
/// Floored by the analysis window itself: the bank integrates over
/// `BASS_WINDOW_SIZE` samples ≈ 0.186 s, so a fit spanning less than that is
/// drawing a line through repeats of *one* window's audio and has no independent
/// information about the rate. This value clears that floor by 1.34×
/// (`band_slope_span_clears_the_analysis_window` pins it).
const BAND_SLOPE_MIN_SPAN_SECS: f32 = 0.25;

/// Fraction of a span's expected frames the history must still hold for its fit
/// to be published — the **frame-loss guard**, not a precision floor (two points
/// already determine a slope).
///
/// Derived from what frame loss costs: for points spread over a fixed span,
/// `Var(slope) ∝ 1/n`, so retaining a quarter of them doubles the slope's
/// standard error. That is the point at which the fit stops being worth
/// publishing, and it is scale-free — it follows [`BAND_SLOPE_WINDOW_SECS`]
/// instead of silently becoming lax if the window shortens. At the shipped window
/// that is ≈ 6 of 26 expected frames, and it trips when the UI falls below
/// ≈ 11 fps against a 43 Hz hop.
const BAND_SLOPE_MIN_FRAME_FRACTION: f32 = 0.25;

/// Readable range (Hz) of the fixed-reference strobe band before its per-hop
/// phase drift aliases. The boundary is exact — half a cycle per hop is
/// `fs/(2·HOP)` = 21.53 Hz — and this is that boundary less the measured
/// **per-hop delta noise**: a single noisy hop folds the unwrap branch, so the
/// margin is `f_hop · z · σ_d`. Pooled p99.9 of σ_d over piano #1 is 0.0772
/// cycles = 3.33 Hz (`strobe_replay` E3), which this 3.5 Hz margin covers.
///
/// It is a *pooled* figure over a 16× register spread (σ_d 0.0046 cycles in the
/// bass, 0.0733 in the treble), so it is conservative low and thin high; the
/// treble band is limited by its own noise well before this boundary, which is
/// what the coarse read is for. A hop/unwrap limit, not a Goertzel one.
const BAND_READABLE_HZ: f32 = BAND_ALIAS_HZ - BAND_UNWRAP_MARGIN_HZ;

/// Half a cycle of beat phase per hop — the exact point past which the GUI's
/// per-frame unwrap folds. Computed, not written down: a hard-coded 21.5 would
/// silently become wrong if the hop or sample rate moved.
const BAND_ALIAS_HZ: f32 = 0.5 * HOP_RATE_HZ;

/// Noise budget below [`BAND_ALIAS_HZ`], measured: a single noisy hop folds the
/// branch, so the margin is `f_hop·z·σ_d`, and the pooled p99.9 of the per-hop
/// delta noise over piano #1 is 3.33 Hz (ADR 0011 §10).
const BAND_UNWRAP_MARGIN_HZ: f32 = 3.5;

/// Hops of unbroken opposing evidence before the readout changes source.
///
/// The range test compares one noisy estimate against a fixed threshold, so
/// while a string sits *at* the boundary the verdict flips: measured on piano #1,
/// 5.6 % of consecutive hops on average and 31 % on the worst key, at 33 of 87
/// keys. Resizing the margin cannot fix that — the flip rate tracks the boundary
/// wherever it is put — so the switch is debounced instead.
///
/// Both bounds are already fixed by the system, and this sits at the lower one:
/// **8 hops is the 8192/1024 window-overlap correlation length**, so consecutive
/// verdicts share 7/8 of their data and 8 is the first independent sample (a
/// shorter run re-counts the same evidence); and
/// `BAND_SLOPE_MIN_SPAN_SECS · f_hop` ≈ 11 hops is the band's own fill time,
/// above which a band-ward switch would start costing latency. Measured effect:
/// mean flip rate 5.58 % → 0.38 %, worst key 31 % → 2.4 %, for 163 ms.
///
/// Applied symmetrically. Asymmetric variants were measured (ADR 0011 §10): they
/// avoid holding a folded band reading past the alias limit, but cost 2–3× the
/// flip rate at the boundary — where both reads are still correct — and need a
/// second, underived constant. The out-ward exposure is bounded at this many hops
/// per crossing by construction.
const READOUT_SWITCH_HOPS: u8 = 8;

/// Advances a debounced boolean by one hop: `state` adopts `verdict` only after
/// [`READOUT_SWITCH_HOPS`] consecutive opposing hops, and any agreeing hop resets
/// the run. Used for the readout's range verdict, where the alternative — a bare
/// threshold on a noisy estimate — flips source while a string sits at the edge.
fn debounce_verdict(state: &mut bool, run: &mut u8, verdict: bool) {
    if verdict == *state {
        *run = 0;
    } else {
        *run += 1;
        if *run >= READOUT_SWITCH_HOPS {
            *state = verdict;
            *run = 0;
        }
    }
}

/// Least-squares slope of `(t, unwrapped-cycles)` → beat rate (Hz) → cents vs
/// `ref_hz`. `None` until the history spans [`BAND_SLOPE_MIN_SPAN_SECS`] and has
/// kept [`BAND_SLOPE_MIN_FRAME_FRACTION`] of its frames. The rate is the strobe's
/// true observable
/// (design §3): rotation = f_live − f_ref, so its slope is the detuning.
fn band_slope_cents(history: &VecDeque<(Instant, f32)>, ref_hz: Option<f32>) -> Option<f32> {
    let ref_hz = ref_hz?;
    if ref_hz <= 0.0 {
        return None;
    }
    let t0 = history.front()?.0;
    let span = history.back()?.0.duration_since(t0).as_secs_f32();
    if span < BAND_SLOPE_MIN_SPAN_SECS {
        return None;
    }
    // Frame-loss guard, against the frames this span *should* have delivered.
    if (history.len() as f32) < BAND_SLOPE_MIN_FRAME_FRACTION * span * HOP_RATE_HZ {
        return None;
    }
    let (mut sx, mut sy, mut sxx, mut sxy, mut n) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
    for &(t, y) in history {
        let x = t.duration_since(t0).as_secs_f64();
        let y = y as f64;
        sx += x;
        sy += y;
        sxx += x * x;
        sxy += x * y;
        n += 1.0;
    }
    let denom = n * sxx - sx * sx;
    if denom.abs() < 1e-9 {
        return None;
    }
    let beat_hz = ((n * sxy - sx * sy) / denom) as f32; // cycles/s
    Some(1200.0 * ((ref_hz + beat_hz) / ref_hz).log2())
}

#[derive(Debug, Clone)]
pub struct RmsCalibrationState {
    pub warmup_hops: Option<u32>,
    pub countdown: Option<u32>,
    pub max_seen_rms: f32,
}

#[derive(Debug, Clone)]
pub struct NoiseFloorSettings {
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
    pub calibration_complete: bool,
    pub visible: bool,
    pub active_calibration: Option<RmsCalibrationState>,
}

#[derive(Debug, Clone)]
pub struct TransientSettings {
    pub noise_floor_baseline: f32,
    pub visible: bool,
    pub is_frozen: bool,
    pub freeze_countdown: Option<u32>,
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
}

#[derive(Debug, Clone)]
pub struct NinosSettings {
    pub visible: bool,
    pub history: VecDeque<f32>,
    pub current_threshold: f32,
}

/// Settings-view-specific display data.
#[derive(Debug, Clone)]
pub struct SettingsDisplayData {
    pub rms: NoiseFloorSettings,
    pub transient: TransientSettings,
    pub ninos: NinosSettings,
}

/// UI-specific data needed for rendering the interface.
///
/// This struct contains only the data that the UI components need
#[derive(Debug, Clone)]
pub struct AppDisplayData {
    // Audio state
    pub audio_worker_active: bool,
    /// Most recent visualization frame from the triple buffer.
    pub last_frame: Option<FrameOutput>,
    /// Most recent note index from NoteEvent (0–87), or None if no note locked.
    pub last_note_index: Option<u8>,
    /// Most recent detected frequency in Hz.
    pub last_frequency: Option<f32>,
    /// Most recent detection confidence (0.0–1.0).
    pub last_confidence: Option<f32>,
    /// Most recent cents deviation from nearest ET note.
    pub last_cents: Option<f32>,
    pub smoothing_buffer: Vec<f32>,
    /// Whether the last confident pitch metric is currently stale
    pub is_stale: bool,

    // Calibration state
    pub is_calibrating: bool,
    pub calibration_progress: usize,
    pub calibration_total: usize,

    // UI visibility states
    pub spectrogram_visible: bool,
    pub cent_meter_visible: bool,
    pub key_select_visible: bool,
    pub partials_visible: bool,
    pub curve_plot_visible: bool,
    pub strobe_visible: bool,
    // pub inharmonicity_graph_visible: bool,

    // View state
    pub settings_view_visible: bool,

    // --- Profile library state ---
    /// The instrument-library settings panel is the active one.
    pub library_visible: bool,
    /// Rows of the profiles directory, refreshed whenever the library changes.
    pub library_entries: Vec<library::ProfileEntry>,
    /// Current browser ordering.
    pub library_sort: ProfileSort,
    /// Current browser search term, matched across every identifying field.
    pub library_search: String,
    /// Identity of the open instrument, mirrored here so the header and the
    /// library form can render it without borrowing the profile.
    pub open_identity: models::InstrumentIdentity,
    /// Path of the open profile, so the browser can mark and protect its row.
    pub open_profile_path: Option<PathBuf>,

    // --- Curve display state (design §9/§13) ---
    /// The curve gallery is open in the settings main panel.
    pub curve_select_visible: bool,
    /// An engine's detail view is open within the gallery.
    pub curve_detail: Option<EngineChoice>,
    /// Which engine the live plot (and later the strobe) displays.
    /// Default (d) BALANCED per D7.
    pub selected_engine: EngineChoice,
    /// Manual-mode strobe display state (design §5, Path A).
    pub strobe: StrobeState,
    /// Which target function every readout is measured against — the strobe
    /// band, its cents readout, and the cent meter alike.
    pub reference_mode: ReferenceMode,

    /// Curve-lock indicator (design §8), projected from [`StrobeLock`] each
    /// tick; `None` when there is nothing to show (disengaged, or ET mode).
    pub strobe_lock_view: Option<StrobeLockView>,
    /// The re-lock confirmation modal is open (design §8; `stack!` overlay).
    pub relock_confirm_open: bool,

    /// The instrument-select debug panel is open in the settings main panel.
    pub instrument_select_visible: bool,

    // Settings view data
    pub settings_data: SettingsDisplayData,

    // Tuning mode
    pub tuning_mode: TuningMode,

    /// Which note-select surface the main view shows (debug convenience).
    pub instrument: Instrument,

    // Capture state
    pub measurement_mode_active: bool,
    pub capture_state: CaptureState,
    pub undo_target_note: Option<String>,
}

/// Curve-lock state (design §8, D6) — the single source of truth for what
/// curve the strobe (and its cent-meter needle) read.
///
/// Modelling it as a sum type keeps the illegal states unrepresentable: there
/// is no separate "is it locked" flag to fall out of sync with the frozen
/// bundle, and every consumer must `match` both arms. The "a newer curve is
/// available" condition is **not** stored — it is computed on demand from the
/// two generations (see [`TunerApp::update_strobe`]), so it cannot drift.
enum StrobeLock {
    /// Not engaged: the strobe auto-engages on the next curve-mode tick that
    /// has a live bundle. Also the resting state under ET mode, which bypasses
    /// the curve entirely.
    Disengaged,
    /// The frozen snapshot the strobe reads. Advances **only** via re-lock; a
    /// capture or undo landing mid-pass moves the *live* `curve_bundle` but
    /// never this (R6). Boxed so the common `Disengaged` state stays small —
    /// the same reason [`crate::worker::CurveBundle`] is boxed on its channel.
    Engaged(Box<CurveBundle>),
}

impl StrobeLock {
    /// The frozen bundle, if engaged.
    fn engaged(&self) -> Option<&CurveBundle> {
        match self {
            StrobeLock::Engaged(c) => Some(c),
            StrobeLock::Disengaged => None,
        }
    }

    /// The lock's response to a change in the trusted measurement set (design
    /// §8 transition table). This `match` is the whole policy in one place;
    /// adding a [`TrustedSetEdit`] variant fails to compile until its effect on
    /// the lock is decided here — the type system enumerating the states for us.
    fn on_trusted_set_edit(&mut self, edit: TrustedSetEdit) {
        match edit {
            // Refinements of the current tuning session: the freeze holds. The
            // live curve advances (higher generation), lighting the "newer
            // curve available" affordance, and only an explicit re-lock shifts
            // the targets — the tuner's in-progress pass is never moved under
            // them (R6).
            TrustedSetEdit::Captured | TrustedSetEdit::Undone => {}
            // A new baseline — often a different instrument. The frozen curve
            // no longer describes what is being tuned, so disengage; the strobe
            // re-locks onto the loaded curve on the next curve-mode tick.
            TrustedSetEdit::Loaded => *self = StrobeLock::Disengaged,
        }
    }
}

/// A change to the trusted measurement set — the events that move the live
/// tuning curve. Each routes through [`StrobeLock::on_trusted_set_edit`] so the
/// lock's response to every one is decided in a single exhaustive `match`.
enum TrustedSetEdit {
    /// A new measurement merged from the Worker.
    Captured,
    /// A capture reverted via the undo history.
    Undone,
    /// A whole profile loaded from disk.
    Loaded,
}

/// View-model of the curve lock for the strobe panel (design §8). `None` when
/// there is no indicator to show — the lock is disengaged, or ET mode is on
/// (it bypasses the curve). Projected each tick from [`StrobeLock`]; never a
/// stored source of truth.
#[derive(Debug, Clone, Copy)]
pub struct StrobeLockView {
    /// Generation of the frozen (locked) bundle.
    pub generation: u64,
    /// The live curve has advanced past the lock — a re-lock would shift every
    /// target (R6). Drives the "newer curve available" affordance.
    pub newer: bool,
}

/// Main application state for the Inharmonicity piano tuner.
///
/// Contains all the state necessary for the GUI application including
/// audio processing, analysis results, and UI visibility controls.
pub struct TunerApp {
    // Audio processing — managed by tuner_core::audio::HostHandle
    host_handle: Option<HostHandle>,

    /// Triple buffer output for continuous visualization frames (lossy, freshest only).
    frame_rx: Option<triple_buffer::Output<FrameOutput>>,

    // --- Inharmonicity State ---
    /// The open instrument, its file, and the write policy around it.
    session: ProfileSession,
    /// App-level state — the resume pointer and recents. Not a property of any
    /// instrument, so it is owned here rather than by the session.
    app_settings: AppSettings,
    // --- Tuning-curve State (cold path, recomputed off-thread by the Worker) ---
    /// Latest curve bundle returned by the Worker (all engines). `None` until
    /// the first recompute lands. Derived data — never persisted (design §9).
    curve_bundle: Option<CurveBundle>,
    /// A trusted-set edit has occurred and a fresh recompute has not yet been
    /// accepted by the job queue; the tick loop retries the send while true.
    curve_dirty: bool,
    /// Monotonic job generation. Incremented on every trusted-set edit; the
    /// Worker echoes it on the returned bundle so a superseded result is
    /// dropped (latest-wins).
    curve_generation: u64,
    /// The curve lock (design §8, D6): the frozen curve the strobe reads,
    /// distinct from the live `curve_bundle` the gallery/plot preview. See
    /// [`StrobeLock`].
    strobe_lock: StrobeLock,
    /// The last strobe reference set successfully pushed to the DSP bank,
    /// as the `(key, et_mode, bundle generation, engine)` identity, or
    /// `None` before any send / when nothing should be targeted. A failed
    /// `set_refs` leaves this stale so the tick loop retries — the same
    /// pattern as `curve_dirty`.
    strobe_pushed: Option<Option<(u8, bool, u64, EngineChoice)>>,

    // Frontend handle to the AudioPipeline's shared atomic state
    pipeline_handle: PipelineHandle,

    // Single source of truth for all display data
    pub display_data: AppDisplayData,
}

impl std::fmt::Debug for TunerApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TunerApp")
            .field("host_handle", &self.host_handle)
            .field("has_frame_rx", &self.frame_rx.is_some())
            .field("pipeline_handle", &self.pipeline_handle)
            .field("display_data", &self.display_data)
            .finish()
    }
}

impl Default for TunerApp {
    /// Creates a new TunerApp instance with default settings.
    ///
    /// Initializes the application state but does NOT start audio processing.
    /// Audio processing starts after calibration completes (see `CalibrationComplete`).
    fn default() -> Self {
        eprintln!("[MAIN] Creating TunerApp...");

        Self {
            host_handle: None,
            frame_rx: None,
            session: ProfileSession::default(),
            app_settings: AppSettings::default(),
            curve_bundle: None,
            curve_dirty: false,
            curve_generation: 0,
            strobe_lock: StrobeLock::Disengaged,
            strobe_pushed: None,
            pipeline_handle: PipelineHandle::default(),
            display_data: AppDisplayData {
                audio_worker_active: false,
                last_frame: None,
                last_note_index: None,
                last_frequency: None,
                last_confidence: None,
                last_cents: None,
                smoothing_buffer: Vec::new(),
                is_stale: false,
                is_calibrating: true,
                calibration_progress: 0,
                calibration_total: crate::calibration::CALIBRATION_FRAMES as usize,
                spectrogram_visible: true,
                cent_meter_visible: true,
                key_select_visible: true,
                partials_visible: true,
                curve_plot_visible: true,
                strobe_visible: true,
                // inharmonicity_graph_visible: true,
                settings_view_visible: false,
                library_visible: false,
                library_entries: Vec::new(),
                library_sort: ProfileSort::default(),
                library_search: String::new(),
                open_identity: models::InstrumentIdentity::default(),
                open_profile_path: None,
                curve_select_visible: false,
                curve_detail: None,
                selected_engine: EngineChoice::MultiBalanced,
                strobe: StrobeState::default(),
                reference_mode: ReferenceMode::default(),
                strobe_lock_view: None,
                relock_confirm_open: false,
                settings_data: SettingsDisplayData {
                    rms: NoiseFloorSettings {
                        history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                        current_threshold: 0.005,
                        calibration_complete: false,
                        visible: false,
                        active_calibration: Some(RmsCalibrationState {
                            warmup_hops: Some(crate::calibration::WARMUP_FRAMES),
                            countdown: Some(crate::calibration::CALIBRATION_FRAMES),
                            max_seen_rms: 0.0,
                        }),
                    },
                    transient: TransientSettings {
                        noise_floor_baseline: 0.0,
                        visible: false,
                        is_frozen: false,
                        freeze_countdown: None,
                        history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                        current_threshold: 0.5,
                    },
                    ninos: NinosSettings {
                        visible: false,
                        history: VecDeque::with_capacity(ENVELOPE_HISTORY_LENGTH),
                        current_threshold: 10.0,
                    },
                },
                instrument_select_visible: false,
                tuning_mode: TuningMode::Auto,
                instrument: Instrument::Piano,
                measurement_mode_active: false,
                capture_state: CaptureState::Idle,
                undo_target_note: None,
            },
        }
    }
}

impl TunerApp {
    /// Refreshes the Undo button label from the head of the undo history.
    /// The label carries the timestamp of the capture that would be undone
    /// (the profile's *current* entry at that key), not the entry it restores —
    /// repeat captures share a key, so the epoch is the only thing
    /// distinguishing which one is about to be discarded. The epoch matches the
    /// on-disk `key_<idx>_<note>_<epoch>` diagnostics dir.
    fn refresh_undo_label(&mut self) {
        self.display_data.undo_target_note = self.session.undo_target().map(|(key, epoch)| {
            let note = tuner_core::models::find_nearest_note_by_index(key).0;
            match epoch {
                Some(e) => format!("{note} · {e}"),
                None => note,
            }
        });
    }

    /// Adopts `profile` as the open instrument: syncs the live engine, applies
    /// its persisted settings, recomputes the curve, and records it as the
    /// profile to resume next launch.
    ///
    /// Undo history is dropped, because it indexes measurements of the
    /// instrument being closed — replaying it against a different one would
    /// write a stranger's measurement into this profile.
    fn adopt_profile(&mut self, profile: InharmonicityProfile, path: PathBuf) {
        self.display_data.selected_engine = profile.settings.engine;
        self.display_data.reference_mode = profile.settings.reference_mode;
        self.apply_profile_settings(&profile.settings.clone());
        self.session.adopt(profile, path, &mut self.app_settings);
        if let Err(e) = self.app_settings.save() {
            eprintln!("[MAIN] Could not save app settings: {e}");
        }
        self.sync_identity_mirror();

        if let Some(host) = self.host_handle.as_mut() {
            host.profiles.update_all(self.session.profile());
        }
        // Recompute for the newly-opened instrument (the curve is never
        // persisted; recompute-on-load).
        self.mark_curve_dirty();
        // A load is a new instrument — drop the lock so the strobe re-engages
        // on the loaded curve (design §8).
        self.strobe_lock.on_trusted_set_edit(TrustedSetEdit::Loaded);
        self.refresh_undo_label();
    }

    /// Refreshes the display mirror of the open instrument's identity and
    /// path. The views render from `display_data` only, so any edit to the
    /// profile's identity has to land here too or the header goes stale.
    fn sync_identity_mirror(&mut self) {
        self.display_data.open_identity = self.session.profile().identity.clone();
        self.display_data.open_profile_path = self.session.path().map(|p| p.to_path_buf());
    }

    /// Pushes the profile's instrument-character thresholds into the live
    /// atomics. The noise floor is deliberately absent: it is an absolute RMS
    /// in the room's own units and moves with mic, gain, room and HVAC, so it
    /// is rig state and stays measured-at-launch. NHWRSF (normalized by Σ|X|)
    /// and NINOS² (a dimensionless sparsity ratio) are level-independent and
    /// describe the instrument, so they travel with it.
    fn apply_profile_settings(&mut self, settings: &models::ProfileSettings) {
        let config = &self.pipeline_handle.atomics.config;
        store_f32(&config.nhwrsf_threshold, settings.nhwrsf_threshold);
        store_f32(
            &config.ninos2_stability_threshold,
            settings.ninos2_stability_threshold,
        );
        self.display_data.settings_data.transient.current_threshold = settings.nhwrsf_threshold;
        self.display_data.settings_data.ninos.current_threshold =
            settings.ninos2_stability_threshold;
    }

    /// Marks the tuning curve stale after a trusted-set edit (capture merge,
    /// undo of a trusted entry, profile load, or a curve-parameter change).
    /// Bumps the generation so any in-flight bundle for an older edit is
    /// dropped on arrival, and flags the tick loop to (re)send a job. The
    /// snapshot is taken at *send* time, not here, so coalesced edits collapse
    /// to one recompute of the final state.
    fn mark_curve_dirty(&mut self) {
        self.curve_generation = self.curve_generation.wrapping_add(1);
        self.curve_dirty = true;
    }

    /// If a recompute is pending, snapshots the trust-filtered [`CurveInput`]
    /// and hands the Worker a [`CurveJob`] (crossing #6). Clears the dirty flag
    /// on acceptance; a full job slot (worker still busy) leaves it set to
    /// retry next tick. Called once per UI tick.
    fn pump_curve_job(&mut self) {
        if !self.curve_dirty {
            return;
        }
        if let Some(host) = self.host_handle.as_ref() {
            let job = CurveJob {
                generation: self.curve_generation,
                input: CurveInput::from_profile(self.session.profile()),
            };
            if host.send_curve_job(job) {
                self.curve_dirty = false;
            }
        }
    }

    /// Swaps the main-view note-select surface (debug convenience) and keeps the
    /// couplings coherent. The manual-mode/DSP contract is instrument-agnostic —
    /// it keys off the 0–87 index — so this touches only presentation-adjacent
    /// state, but two things must stay consistent:
    ///
    /// * **Strobe reference.** Guitar defaults to ET (fundamental-only, curve-free,
    ///   the sensible reference for a non-piano string); Piano reverts to the
    ///   tuning curve. Either way the pending phase and push cache are cleared so
    ///   [`Self::update_strobe`] re-targets the bank next tick — the same reset
    ///   the manual `SetReferenceMode` performs. The user can still override
    ///   the reference afterward from the strobe panel.
    /// * **Manual selection.** Every key is a valid piano key, so Guitar→Piano
    ///   always preserves the target. Entering Guitar with a target that is *not*
    ///   one of the six open strings would leave a selection the guitar surface
    ///   cannot show and cannot clear; rather than keep that hidden state, drop to
    ///   Auto (mirroring [`Message::SwitchToAutoMode`]).
    ///
    /// Precondition: the instrument actually changed (guarded by the caller).
    fn set_instrument(&mut self, inst: Instrument) {
        self.display_data.instrument = inst;

        // Instrument-appropriate strobe reference default. The reference set
        // moves under the bank regardless (ET mode and/or a dropped target),
        // so force a re-push and clear the accumulated phase either way.
        self.display_data.reference_mode = match inst {
            Instrument::Guitar => ReferenceMode::Et,
            Instrument::Piano => ReferenceMode::Curve,
        };
        self.session.profile_mut().settings.reference_mode = self.display_data.reference_mode;
        self.strobe_pushed = None;
        self.display_data.strobe = StrobeState::default();

        // Drop a manual target the guitar surface can't represent.
        let drop_target = inst == Instrument::Guitar
            && matches!(
                &self.display_data.tuning_mode,
                TuningMode::Manual { key_index, .. }
                    if !crate::widgets::guitar_strings::GUITAR_STRING_KEYS.contains(key_index)
            );
        if drop_target {
            self.display_data.tuning_mode = TuningMode::Auto;
            self.pipeline_handle
                .atomics
                .config
                .target_note
                .store(255, Ordering::Relaxed);
            self.display_data.smoothing_buffer.clear();
        }
    }

    /// The tuning target (Hz) the cent meter reads against for `key`: the
    /// selected curve's stretched `target_f1` in curve mode, or the raw ET
    /// pitch in ET/guitar mode (and before any curve exists). Mirrors the
    /// strobe's reference choice so the needle and the band agree — and folds
    /// in the stretch the discovery template's f_ET·√(1+B) never carried.
    fn meter_target_hz(&self, key: u8) -> f32 {
        let et = models::NOTES[key as usize].frequency;
        if self.display_data.reference_mode == ReferenceMode::Et {
            return et;
        }
        // The **locked** bundle (design §8), falling back to the live one
        // before the lock has engaged — so the needle reads the same frozen
        // target the strobe band does.
        match self.strobe_lock.engaged().or(self.curve_bundle.as_ref()) {
            Some(bundle) => bundle
                .curve(self.display_data.selected_engine)
                .target_f1(key),
            None => et,
        }
    }

    /// Keeps the DSP strobe bank targeted and mirrors its telemetry into the
    /// display state (Path A, design §5.2). Reference sets are pushed over
    /// the crossing-#4 strobe channel on any change of (key, bundle, engine)
    /// — infrequent, user-rate events; a full ring retries next tick. The
    /// beat phase itself is read back from `FrameOutput.strobe_angle`, where
    /// the bank accumulated it (R2).
    fn update_strobe(&mut self, frame_pushed: bool) {
        let et_mode = self.display_data.reference_mode == ReferenceMode::Et;
        let manual_key = match &self.display_data.tuning_mode {
            TuningMode::Manual { key_index, .. } => Some(*key_index),
            _ => None,
        };

        // Curve lock (design §8): auto-engage the first time the strobe would
        // target a curve — freeze the live bundle. From then the strobe reads
        // the frozen copy; a recapture updates `curve_bundle` (and the
        // gallery/plot previews) but never the strobe targets until re-lock.
        if !et_mode
            && manual_key.is_some()
            && matches!(self.strobe_lock, StrobeLock::Disengaged)
            && let Some(live) = &self.curve_bundle
        {
            self.strobe_lock = StrobeLock::Engaged(Box::new(live.clone()));
        }

        // Project the lock into the panel's view-model: the frozen generation,
        // and whether the live curve has advanced past it (computed on demand
        // from the two generations — never a stored flag). Hidden in ET mode.
        self.display_data.strobe_lock_view = if et_mode {
            None
        } else {
            self.strobe_lock.engaged().map(|locked| StrobeLockView {
                generation: locked.generation,
                newer: self
                    .curve_bundle
                    .as_ref()
                    .is_some_and(|live| live.generation > locked.generation),
            })
        };

        // What the bank should be targeting — the identity used to dedup
        // retargets. `(key, et_mode, locked_gen, engine)`; in ET mode the
        // gen/engine components are inert (0 / default) so a mode flip still
        // changes the identity and forces a re-push. Curve mode keys off the
        // **locked** generation, so a re-lock (and only a re-lock) re-pushes.
        let desired: Option<(u8, bool, u64, EngineChoice)> = manual_key.and_then(|key| {
            if et_mode {
                Some((key, true, 0, EngineChoice::MultiBalanced))
            } else {
                self.strobe_lock
                    .engaged()
                    .map(|b| (key, false, b.generation, self.display_data.selected_engine))
            }
        });

        // The displayed partial + full reference set. `spacing` is the key's
        // partial spacing f₀* — the coarse read's neighbour cap and CFAR
        // reference width both scale with it.
        let mut refs = [0.0f32; 12];
        let mut count = 0usize;
        let mut spacing = 0.0f32;
        let (n_star, ref_hz) = match desired {
            // ET reference (instrument-agnostic, e.g. a guitar): target the
            // key's pure equal-temperament pitch, fundamental only. The n = 1
            // target is B-immune (R4), so no per-string inharmonicity is
            // needed and no false beat appears on a correctly-pitched string
            // — the reason ET mode shows only the fundamental.
            Some((key, true, _, _)) => {
                let f_et = NOTES[key as usize].frequency;
                refs[0] = f_et;
                count = 1;
                spacing = f_et;
                (1u8, Some(f_et))
            }
            // Curve reference (piano). The targets use the key's raw measured
            // B (the `strobe_partials` contract — they must match the physical
            // string or a correctly tuned partial shows a false beat).
            //
            // An unmeasured key falls back to the Rigaud prior, *not* to B = 0.
            // The band is indifferent — its n = 1 target is f₁ for any B — but
            // the coarse read is centred on a higher partial, and a harmonic
            // reference there would report the string's whole stretch as
            // mistuning (≈ 24 ¢ at A0). The prior is also the reference the
            // cold-start honesty bound was measured against (ADR 0011).
            Some((key, false, _, engine)) => {
                let bundle = self
                    .strobe_lock
                    .engaged()
                    .expect("curve desired implies an engaged lock");
                let b_raw = self
                    .session
                    .profile()
                    .active(key)
                    .and_then(|m| m.calculated_b);
                let n_star = match b_raw {
                    Some(_) => bundle.display_partials[key as usize],
                    None => 1,
                };
                let curve = bundle.curve(engine);
                let b = b_raw.unwrap_or_else(|| models::get_expected_beta(key));
                count = curve.strobe_partials(key, b, &mut refs);
                // The f₀ the reference series was built from — `strobe_partials`
                // divides the target f₁ by √(1+B) for exactly this quantity.
                spacing = curve.target_f1(key) / (1.0 + b).sqrt();
                let ref_hz = (n_star as usize <= count).then(|| refs[n_star as usize - 1]);
                (n_star, ref_hz)
            }
            None => (1, None),
        };

        // Coarse-read partial: the derived ADR-0011 rule, not the display
        // table. Resolved here because the reference set is the frontend's
        // policy — the DSP only searches where it is told to.
        let coarse_n = desired.map_or(1, |(key, _, _, _)| curves::coarse_read_partial(key));
        let coarse_ref_hz = ((coarse_n as usize) <= count).then(|| refs[coarse_n as usize - 1]);

        // Retarget the bank when the desired set changed.
        if self.strobe_pushed != Some(desired)
            && let Some(host) = self.host_handle.as_mut()
            && host.strobe_refs.set_refs(StrobeRefUpdate {
                count,
                refs,
                coarse_index: coarse_n,
                spacing_hz: spacing,
            })
        {
            self.strobe_pushed = Some(desired);
        }

        // Mirror the bank's telemetry for the displayed partial.
        let bank = self
            .display_data
            .last_frame
            .as_ref()
            .filter(|f| desired.is_some() && (n_star as usize) <= f.strobe_count)
            .map(|f| {
                (
                    f.strobe_angle[n_star as usize - 1],
                    f.strobe_gated[n_star as usize - 1],
                )
            });

        let strobe = &mut self.display_data.strobe;
        // A retarget (key / partial / ref change) invalidates the accumulated
        // beat phase — reset the band-slope history so the rate is only ever
        // fit over a contiguous run against one fixed reference.
        if strobe.ref_hz != ref_hz || strobe.n_star != n_star {
            strobe.band_unwrapped = 0.0;
            strobe.band_prev_raw = None;
            strobe.band_history.clear();
            strobe.out_of_range = false;
            strobe.range_run = 0;
            strobe.band_cents = None;
        }
        strobe.n_star = n_star;
        strobe.ref_hz = ref_hz;

        // Coarse readout. Held only while the DSP is searching the reference set
        // we asked for: a frame computed against the previous key would be a
        // wrong number rather than a late one. Dropped whenever a hop yields
        // nothing, so the panel falls back rather than freezing a stale value.
        strobe.coarse_n = coarse_n;
        strobe.coarse_cents = match (coarse_ref_hz, self.strobe_pushed == Some(desired)) {
            (Some(r), true) if r > 0.0 => self
                .display_data
                .last_frame
                .as_ref()
                .and_then(|f| f.coarse_hz)
                .map(|hz| 1200.0 * (hz / r).log2()),
            _ => None,
        };

        // Readable-range verdict, debounced over hops (never over idle ticks —
        // the triple buffer redelivers the last frame, and re-counting it would
        // let the run advance without new evidence). The coarse read supplies
        // the offset in cents, exact at any partial by the equal-cents identity,
        // so the offset in Hz at the displayed reference r is r·(2^(¢/1200) − 1).
        // With no coarse read to contradict it the band stands: it is then the
        // only evidence there is.
        if frame_pushed {
            let verdict = match (ref_hz, strobe.coarse_cents) {
                (Some(r), Some(off)) if r > 0.0 => {
                    (r * ((off / 1200.0).exp2() - 1.0)).abs() >= BAND_READABLE_HZ
                }
                _ => false,
            };
            debounce_verdict(&mut strobe.out_of_range, &mut strobe.range_run, verdict);
        }

        match bank {
            Some((angle, gated)) => {
                strobe.beat_phase = angle;
                strobe.gated = gated;
                // Band-slope readout: accumulate only on a *new* frame (the
                // triple buffer redelivers the last frame on idle ticks, and
                // re-counting it would flatten the slope) and only while
                // ungated (a frozen band holds its angle — including gated
                // hops would read the detuning toward zero). A gate freezes the
                // last rate; a re-strike resumes from a fresh baseline.
                if frame_pushed && !gated {
                    match strobe.band_prev_raw {
                        Some(prev) => {
                            let mut d = angle - prev;
                            if d > 0.5 {
                                d -= 1.0;
                            } else if d < -0.5 {
                                d += 1.0;
                            }
                            strobe.band_unwrapped += d;
                        }
                        // First ungated frame (fresh target or re-strike):
                        // seed the baseline, publish no drift, and drop the old
                        // rate so a re-strike shows "listening…" until the
                        // window refills — never a stale number.
                        None => {
                            strobe.band_history.clear();
                            strobe.band_cents = None;
                        }
                    }
                    strobe.band_prev_raw = Some(angle);

                    let now = Instant::now();
                    strobe.band_history.push_back((now, strobe.band_unwrapped));
                    while strobe.band_history.front().is_some_and(|&(t, _)| {
                        now.duration_since(t).as_secs_f32() > BAND_SLOPE_WINDOW_SECS
                    }) {
                        strobe.band_history.pop_front();
                    }
                    if let Some(c) = band_slope_cents(&strobe.band_history, ref_hz) {
                        strobe.band_cents = Some(c);
                    }
                } else if gated {
                    // Freeze: hold the last rate, drop the baseline so the
                    // next strike starts clean (no phantom jump across the gap).
                    strobe.band_prev_raw = None;
                }
            }
            // Bank not (yet) targeting this key: hold the last angle, gated.
            None => strobe.gated = true,
        }
    }

    /// Creates the app and initiates audio processing immediately.
    /// Noise floor calibration runs wait-free from the UI's Tick loop.
    pub fn new() -> (Self, iced::Task<Message>) {
        let mut app = Self::default();
        app.start_audio_processing();
        // Resolve which instrument the session starts on, then mirror its
        // settings into the display state the same way an explicit open does.
        app.app_settings = AppSettings::load();
        app.session.open_at_startup(&mut app.app_settings);
        if let Err(e) = app.app_settings.save() {
            eprintln!("[MAIN] Could not save app settings: {e}");
        }
        let profile = app.session.profile();
        app.display_data.selected_engine = profile.settings.engine;
        app.display_data.reference_mode = profile.settings.reference_mode;
        let settings = profile.settings.clone();
        app.apply_profile_settings(&settings);
        app.sync_identity_mirror();
        // Prior-only curve at launch (design §10): with zero trusted
        // measurements the bundle is the B_ξ-default curve, so the live plot
        // renders from the first frame and morphs as captures land.
        app.mark_curve_dirty();
        (app, iced::Task::none())
    }
    /// Starts the dedicated audio processing thread via [`audio::spawn_analysis_thread()`].
    ///
    /// All audio thread boilerplate (CPAL setup, ring buffer polling, analysis loop)
    /// is handled by the `tuner-core` host extension. This method simply calls it
    /// and stores the returned [`HostHandle`] channels.
    #[allow(unreachable_code)]
    fn start_audio_processing(&mut self) {
        // Prevent headless tests from hanging indefinitely while trying to initialize physical audio hardware
        #[cfg(test)]
        {
            eprintln!("[AUDIO-THREAD] Disabled for unit testing.");
            return;
        }

        let dump_dir = library::diagnostics_dir();
        // The dev workflow feeds these to the offline harnesses, which take the
        // directory as an argument — so it has to be discoverable.
        eprintln!("[MAIN] Capture diagnostics → {}", dump_dir.display());

        match audio::spawn_analysis_thread(AudioSource::Default, Some(dump_dir)) {
            Ok(mut handle) => {
                eprintln!("[AUDIO] Hardware stream active.");

                // Write the current threshold to the new pipeline's config
                // (since AudioPipeline::new() creates fresh defaults)
                store_f32(
                    &handle.pipeline_handle.atomics.config.silence_threshold,
                    self.display_data.settings_data.rms.current_threshold,
                );

                self.pipeline_handle = handle.pipeline_handle.clone();
                self.frame_rx = handle.frame_rx.take();
                self.host_handle = Some(handle);
                self.display_data.audio_worker_active = true;
            }
            Err(e) => {
                eprintln!("[AUDIO ERROR] Could not start hardware: {}", e);
            }
        }
    }

    /// Handles application state updates based on incoming messages.
    pub fn update(&mut self, message: Message) -> iced::Task<Message> {
        match message {
            Message::Exit => {
                eprintln!("[MAIN] Window close requested - starting cleanup...");
                if self.session.is_dirty() {
                    self.session.persist();
                }
                if let Some(mut handle) = self.host_handle.take() {
                    eprintln!("[MAIN] Shutting down audio host...");
                    handle.stop();
                    eprintln!("[MAIN] Audio host stopped.");
                }
                eprintln!("[MAIN] Clearing channels...");
                self.frame_rx = None;
                eprintln!("[MAIN] Cleanup completed - forcing clean exit");
                std::process::exit(0);
            }
            Message::KeySelected(key_index) => {
                // Check if the same key is already selected - if so, switch to auto mode
                if let TuningMode::Manual {
                    key_index: current_key,
                    ..
                } = &self.display_data.tuning_mode
                    && *current_key == key_index
                {
                    // Same key clicked again - switch to auto mode
                    self.display_data.tuning_mode = TuningMode::Auto;
                    self.pipeline_handle
                        .atomics
                        .config
                        .target_note
                        .store(255, Ordering::Relaxed);
                    self.display_data.smoothing_buffer.clear();
                    return iced::Task::none();
                }

                // Different key or not in manual mode - switch to manual mode with new key
                let (note_name, _et_hz) = models::find_nearest_note_by_index(key_index);
                self.display_data.tuning_mode = TuningMode::Manual {
                    key_index,
                    note_name,
                };
                self.pipeline_handle
                    .atomics
                    .config
                    .target_note
                    .store(key_index, Ordering::Relaxed);
                self.display_data.smoothing_buffer.clear();
                // New key ⇒ new strobe references; the accumulated phase is
                // meaningless against them.
                self.display_data.strobe = StrobeState::default();
            }
            Message::SwitchToAutoMode => {
                self.display_data.tuning_mode = TuningMode::Auto;
                self.pipeline_handle
                    .atomics
                    .config
                    .target_note
                    .store(255, Ordering::Relaxed);
                self.display_data.smoothing_buffer.clear();
            }
            Message::ToggleMeasurementMode => {
                // This toggles the measurement mode on/off
                self.display_data.measurement_mode_active =
                    !self.display_data.measurement_mode_active;
                let mut new_state = CaptureState::Idle;

                if self.display_data.measurement_mode_active {
                    eprintln!("[MAIN] Measurement mode ON - starting in Armed state");
                    new_state = CaptureState::Armed;
                } else {
                    eprintln!("[MAIN] Measurement mode OFF");
                }

                self.display_data.capture_state = new_state.clone();
                self.pipeline_handle
                    .atomics
                    .capture_state
                    .store(new_state as u8, Ordering::Relaxed);
            }
            Message::CaptureButtonClicked => {
                // Clicked the active button
                // Wait-free: if we are in measurement mode, and state is Idle, we Arm it.
                // If it is Armed, we can optionally go to Idle (to cancel), but measurement mode remains active.
                if self.display_data.measurement_mode_active {
                    let mut new_state = self.display_data.capture_state.clone();
                    if new_state == CaptureState::Idle {
                        new_state = CaptureState::Armed;
                    } else if new_state == CaptureState::Armed {
                        new_state = CaptureState::Idle;
                    }
                    self.display_data.capture_state = new_state.clone();
                    self.pipeline_handle
                        .atomics
                        .capture_state
                        .store(new_state as u8, Ordering::Relaxed);
                }
            }
            Message::UndoLastCapture => {
                if let Some((idx, bad)) = self.session.undo() {
                    {
                        // The dump is per-capture (timestamped dir, so repeat
                        // captures are all retained) — an undone capture is the
                        // user declaring it bad, and offline tools glob every
                        // `key_*` dir, so the dump must go with it.
                        let dir = library::diagnostics_dir().join(worker::dump_dir_name(&bad));
                        if dir.is_dir() {
                            match std::fs::remove_dir_all(&dir) {
                                Ok(()) => eprintln!(
                                    "[MAIN] Removed diagnostics of undone capture: {}",
                                    dir.display()
                                ),
                                Err(e) => {
                                    eprintln!("[MAIN] Could not remove {}: {e}", dir.display())
                                }
                            }
                        }
                    }
                    // Revert the live engine template too (measured B if a prior
                    // measurement remains, else back to the Rigaud prior).
                    if let Some(host) = self.host_handle.as_mut() {
                        host.profiles
                            .update_key_profile(idx, self.session.profile().active(idx));
                    }
                    eprintln!("[MAIN] Undoing profile change at index {}", idx);
                    // The trusted set may have changed; recompute the curve.
                    // (A no-op edit on an untrusted entry just recomputes an
                    // identical bundle off-thread — harmless, and undo is rare.)
                    self.mark_curve_dirty();
                    // The strobe lock holds through an undo (R6); the live curve
                    // advances and the "newer curve available" affordance lights.
                    self.strobe_lock.on_trusted_set_edit(TrustedSetEdit::Undone);
                }
                // History shrank; repoint (or clear) the Undo label.
                self.refresh_undo_label();
            }
            Message::SaveProfile => {
                // The profile auto-saves; this stays as an explicit flush.
                self.session.persist();
            }
            Message::ToggleLibrary => {
                let visible = !self.display_data.library_visible;
                self.display_data.library_visible = visible;
                if visible {
                    // One settings panel at a time, as the calibration
                    // panels already do.
                    self.display_data.settings_view_visible = true;
                    self.display_data.curve_select_visible = false;
                    self.display_data.instrument_select_visible = false;
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.transient.visible = false;
                    self.display_data.settings_data.ninos.visible = false;
                    self.display_data.library_entries =
                        library::list_profiles(self.display_data.library_sort);
                } else if self.session.is_dirty() {
                    // Leaving the form: don't make a pending rename wait out
                    // the quiet delay.
                    self.session.persist();
                }
            }
            Message::OpenProfile(path) => {
                match InharmonicityProfile::from_file(&path) {
                    Ok(profile) => self.adopt_profile(profile, path),
                    Err(e) => eprintln!("[MAIN] Error loading profile {}: {e}", path.display()),
                }
                self.display_data.library_entries =
                    library::list_profiles(self.display_data.library_sort);
            }
            Message::NewProfile => {
                let name = library::default_profile_name();
                let path = library::unique_path_for(&name);
                self.adopt_profile(InharmonicityProfile::new(name), path);
                self.display_data.library_entries =
                    library::list_profiles(self.display_data.library_sort);
            }
            Message::DeleteProfile(path) => {
                // Never delete the open instrument out from under the session.
                if self.session.delete(&path) {
                    self.app_settings.note_removed(&path);
                    let _ = self.app_settings.save();
                }
                self.display_data.library_entries =
                    library::list_profiles(self.display_data.library_sort);
            }
            Message::DuplicateProfile(path) => {
                match InharmonicityProfile::from_file(&path) {
                    Ok(mut profile) => {
                        profile.identity.name = format!("{} (copy)", profile.identity.name);
                        // A copy is a *different* instrument record: it must not
                        // inherit the original's serial number, which is the one
                        // field that claims identity.
                        profile.identity.serial = None;
                        let target = library::unique_path_for(&profile.identity.name);
                        if let Err(e) = profile.to_file(&target) {
                            eprintln!("[MAIN] Could not duplicate profile: {e}");
                        }
                    }
                    Err(e) => eprintln!("[MAIN] Could not read {}: {e}", path.display()),
                }
                self.display_data.library_entries =
                    library::list_profiles(self.display_data.library_sort);
            }
            Message::LibrarySortChanged(sort) => {
                self.display_data.library_sort = sort;
                self.display_data.library_entries = library::list_profiles(sort);
            }
            Message::LibrarySearchChanged(needle) => {
                self.display_data.library_search = needle;
            }
            Message::IdentityFieldChanged(field, value) => {
                let identity = &mut self.session.profile_mut().identity;
                let value = if value.is_empty() { None } else { Some(value) };
                match field {
                    IdentityField::Name => {
                        identity.name = value.unwrap_or_default();
                    }
                    IdentityField::Make => identity.make = value,
                    IdentityField::Model => identity.model = value,
                    IdentityField::Serial => identity.serial = value,
                    IdentityField::Form => identity.form = value,
                    IdentityField::Owner => identity.owner = value,
                    IdentityField::Notes => identity.notes = value,
                }
                self.sync_identity_mirror();
                // Keystroke-rate: coalesced, not one write per character.
                self.session.touch();
            }
            Message::InstrumentKindChanged(kind) => {
                self.session.profile_mut().identity.kind = kind;
                self.sync_identity_mirror();
                self.session.persist();
            }
            // ------------------------------------------
            Message::Temperament => {
                // Placeholder for temperament settings
            }
            Message::TuningStandard => {
                // Placeholder for tuning standard settings
            }
            Message::InharmonicCurve => {
                // Placeholder for inharmonic curve adjustment
            }
            Message::SampleBuffer => {
                // Placeholder for sample buffer adjustment
            }
            Message::TuningProfile => {
                // Placeholder for tuning profile settings
            }
            Message::ToggleSpectrogram => {
                eprintln!(
                    "[MAIN] Toggling spectrogram visibility: {} -> {}",
                    self.display_data.spectrogram_visible, !self.display_data.spectrogram_visible
                );
                self.display_data.spectrogram_visible = !self.display_data.spectrogram_visible;
            }
            Message::ToggleCentMeter => {
                eprintln!(
                    "[MAIN] Toggling cent meter visibility: {} -> {}",
                    self.display_data.cent_meter_visible, !self.display_data.cent_meter_visible
                );
                self.display_data.cent_meter_visible = !self.display_data.cent_meter_visible;
            }
            Message::ToggleKeySelect => {
                eprintln!(
                    "[MAIN] Toggling key select visibility: {} -> {}",
                    self.display_data.key_select_visible, !self.display_data.key_select_visible
                );
                self.display_data.key_select_visible = !self.display_data.key_select_visible;
            }
            Message::TogglePartials => {
                eprintln!(
                    "[MAIN] Toggling partials visibility: {} -> {}",
                    self.display_data.partials_visible, !self.display_data.partials_visible
                );
                self.display_data.partials_visible = !self.display_data.partials_visible;
            }
            Message::ToggleCurvePlot => {
                self.display_data.curve_plot_visible = !self.display_data.curve_plot_visible;
            }
            Message::ToggleStrobe => {
                self.display_data.strobe_visible = !self.display_data.strobe_visible;
            }
            Message::SetReferenceMode(mode) => {
                self.display_data.reference_mode = mode;
                // Force a re-push next tick and reset the shown phase — the
                // reference set is about to change under the bank.
                self.strobe_pushed = None;
                self.display_data.strobe = StrobeState::default();
                // Both tuning selections persist with the instrument, so
                // reopening it reproduces the targets it was tuned to.
                self.session.profile_mut().settings.reference_mode = mode;
                self.session.persist();
            }
            Message::RequestRelock => {
                // Only meaningful when a newer curve exists; the button is
                // hidden otherwise, but guard anyway.
                if self.display_data.strobe_lock_view.is_some_and(|v| v.newer) {
                    self.display_data.relock_confirm_open = true;
                }
            }
            Message::ConfirmRelock => {
                // Advance the lock to the live bundle — shifts every strobe
                // target (design §8). Force a re-push and reset the shown
                // phase, exactly like a key change.
                if let Some(live) = &self.curve_bundle {
                    self.strobe_lock = StrobeLock::Engaged(Box::new(live.clone()));
                    self.strobe_pushed = None;
                    self.display_data.strobe = StrobeState::default();
                }
                self.display_data.relock_confirm_open = false;
            }
            Message::CancelRelock => {
                self.display_data.relock_confirm_open = false;
            }
            Message::ToggleCurveSelect => {
                let vis = !self.display_data.curve_select_visible;
                self.display_data.curve_select_visible = vis;
                if vis {
                    // Same mutual exclusion as the calibration sub-views:
                    // one settings main panel at a time.
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.transient.visible = false;
                    self.display_data.settings_data.ninos.visible = false;
                    self.display_data.instrument_select_visible = false;
                } else {
                    self.display_data.curve_detail = None;
                }
            }
            Message::CurveDetailOpened(choice) => {
                self.display_data.curve_detail = Some(choice);
            }
            Message::CurveDetailClosed => {
                self.display_data.curve_detail = None;
            }
            Message::EngineSelected(choice) => {
                // Display-only (D7): all engines are in the bundle already,
                // so no recompute — just repoint the plot (and later strobe).
                self.display_data.selected_engine = choice;
                // Persisted with the instrument: which curve it was tuned with
                // is a fact about the tuning, not an app-wide preference.
                self.session.profile_mut().settings.engine = choice;
                self.session.persist();
            }

            // Message::ToggleInharmonicityGraph => {
            //     eprintln!(
            //         "[MAIN] Toggling inharmonicity graph visibility: {} -> {}",
            //         self.display_data.inharmonicity_graph_visible,
            //         !self.display_data.inharmonicity_graph_visible
            //     );
            //     self.display_data.inharmonicity_graph_visible =
            //         !self.display_data.inharmonicity_graph_visible;
            // }
            Message::ToggleSettingsView => {
                eprintln!(
                    "[MAIN] Toggling settings view visibility: {} -> {}",
                    self.display_data.settings_view_visible,
                    !self.display_data.settings_view_visible
                );
                self.display_data.settings_view_visible = !self.display_data.settings_view_visible;
            }
            Message::ToggleNoiseFloorAdjustment => {
                let vis = !self.display_data.settings_data.rms.visible;
                self.display_data.settings_data.rms.visible = vis;
                if vis {
                    self.display_data.settings_data.transient.visible = false;
                    self.display_data.settings_data.ninos.visible = false;
                    self.display_data.curve_select_visible = false;
                    self.display_data.instrument_select_visible = false;
                }
            }
            Message::SilenceThresholdChanged(value) => {
                // Write to shared atomics so the audio thread picks it up immediately
                store_f32(
                    &self.pipeline_handle.atomics.config.silence_threshold,
                    value,
                );
                // Update local display data for immediate UI feedback
                self.display_data.settings_data.rms.current_threshold = value;
            }
            Message::RecalibrateNoiseFloor => {
                self.display_data.is_calibrating = true;
                self.display_data.settings_data.rms.calibration_complete = false;
                self.display_data.settings_data.rms.active_calibration =
                    Some(crate::app::RmsCalibrationState {
                        warmup_hops: Some(crate::calibration::WARMUP_FRAMES),
                        countdown: Some(crate::calibration::CALIBRATION_FRAMES),
                        max_seen_rms: 0.0,
                    });
            }
            Message::ToggleTransientCalibration => {
                let vis = !self.display_data.settings_data.transient.visible;
                self.display_data.settings_data.transient.visible = vis;

                if self.display_data.settings_data.transient.visible {
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.ninos.visible = false;
                    self.display_data.curve_select_visible = false;
                    self.display_data.instrument_select_visible = false;
                    self.display_data.settings_data.transient.is_frozen = false;
                    self.display_data.settings_data.transient.freeze_countdown = None;
                    self.display_data.settings_data.transient.history.clear();
                }
            }
            Message::ResetTransientScope => {
                self.display_data.settings_data.transient.is_frozen = false;
                self.display_data.settings_data.transient.freeze_countdown = None;
                self.display_data.settings_data.transient.history.clear();
            }
            Message::NhwrsfThresholdChanged(val) => {
                store_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold, val);
                self.display_data.settings_data.transient.current_threshold = val;
                // Level-independent (normalized by Σ|X|) ⇒ instrument
                // character, so it persists and travels with the profile.
                // Recalibrating stays available regardless: level-independent
                // is not noise-independent.
                // Slider-rate, like the identity fields: coalesced.
                self.session.profile_mut().settings.nhwrsf_threshold = val;
                self.session.touch();
            }
            Message::ToggleNinosCalibration => {
                let vis = !self.display_data.settings_data.ninos.visible;
                self.display_data.settings_data.ninos.visible = vis;

                if self.display_data.settings_data.ninos.visible {
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.transient.visible = false;
                    self.display_data.curve_select_visible = false;
                    self.display_data.instrument_select_visible = false;
                    self.display_data.settings_data.ninos.history.clear();
                }
            }
            Message::ResetNinosScope => {
                self.display_data.settings_data.ninos.history.clear();
            }
            Message::NinosThresholdChanged(val) => {
                store_f32(
                    &self
                        .pipeline_handle
                        .atomics
                        .config
                        .ninos2_stability_threshold,
                    val,
                );
                self.display_data.settings_data.ninos.current_threshold = val;
                // Dimensionless sparsity ratio ⇒ instrument character, same
                // argument as the NHWRSF threshold above.
                self.session
                    .profile_mut()
                    .settings
                    .ninos2_stability_threshold = val;
                self.session.touch();
            }
            Message::ToggleInstrumentSelect => {
                let vis = !self.display_data.instrument_select_visible;
                self.display_data.instrument_select_visible = vis;
                if vis {
                    self.display_data.settings_data.rms.visible = false;
                    self.display_data.settings_data.transient.visible = false;
                    self.display_data.settings_data.ninos.visible = false;
                    self.display_data.curve_select_visible = false;
                }
            }
            Message::SetInstrument(inst) => {
                if self.display_data.instrument != inst {
                    self.set_instrument(inst);
                }
            }
            Message::Tick => {
                let mut frame_pushed = false;

                // ── Read freshest FrameOutput from triple buffer ──
                if let Some(ref mut frame_rx) = self.frame_rx
                    && frame_rx.update()
                {
                    frame_pushed = true;
                    let frame = frame_rx.read().clone();
                    self.display_data.last_frame = Some(frame.clone());

                    // 1. Independent Decoupling of Scalar Data
                    // Assign values individually. If one drops out (e.g. confidence),
                    // we still display the others.
                    //
                    // Non-finite scalars degrade to "no reading" here — the
                    // canvas layer tessellates path coordinates and lyon
                    // asserts on non-finite values, so one bad DSP frame must
                    // never reach a widget. Log it (a producer emitting
                    // NaN/∞ is a bug worth chasing), don't crash a tuning
                    // session over it.
                    let finite = |v: Option<f32>| v.filter(|x| x.is_finite());
                    if frame.detected_frequency.is_some_and(|v| !v.is_finite())
                        || frame.confidence.is_some_and(|v| !v.is_finite())
                    {
                        eprintln!(
                            "[MAIN] Dropped non-finite frame scalar(s): f={:?} conf={:?}",
                            frame.detected_frequency, frame.confidence
                        );
                    }
                    self.display_data.last_frequency = finite(frame.detected_frequency);
                    self.display_data.last_confidence = finite(frame.confidence);

                    if let Some(idx) = frame.note_index {
                        self.display_data.last_note_index = Some(idx);
                    }

                    // The displayed deviation is computed here, GUI-side, against
                    // the note's tuning target — the curve's stretched `target_f1`
                    // in curve mode, ET in ET/guitar mode — not the engine's
                    // `cents_deviation`, which references the discovery template's
                    // f_ET·√(1+B) and reads the treble flat. The core measures the
                    // frequency; the display owns the reference.
                    let target_key = match &self.display_data.tuning_mode {
                        TuningMode::Manual { key_index, .. } => Some(*key_index),
                        TuningMode::Auto => frame.note_index,
                    };
                    let cents = target_key
                        .zip(finite(frame.detected_frequency))
                        .map(|(k, f)| models::calculate_cents_deviation(f, self.meter_target_hz(k)))
                        .filter(|c| c.is_finite());
                    self.display_data.last_cents = cents;

                    if let Some(c) = cents {
                        self.display_data.smoothing_buffer.push(c);
                        if self.display_data.smoothing_buffer.len() > SMOOTHING_FACTOR {
                            self.display_data.smoothing_buffer.remove(0);
                        }
                    } else {
                        self.display_data.smoothing_buffer.clear();
                    }

                    // Render State Logic: Silence vs Stale vs Valid
                    if frame.is_silence {
                        // Valid Silence: Drop all old measurements.
                        self.display_data.last_frequency = None;
                        self.display_data.last_note_index = None;
                        self.display_data.last_confidence = None;
                        self.display_data.last_cents = None;
                        self.display_data.smoothing_buffer.clear();
                        self.display_data.is_stale = false;
                    } else if frame.note_index.is_none() {
                        // Valid Audio but No Pitch Lock: Freeze scalars but flag as stale to mute visual output
                        self.display_data.smoothing_buffer.clear();
                        self.display_data.is_stale = true;
                    } else {
                        // Valid Lock: Ensure we are not stale.
                        self.display_data.is_stale = false;
                    }

                    // Sync capture state from atomics for UI rendering
                    let state_val = self
                        .pipeline_handle
                        .atomics
                        .capture_state
                        .load(Ordering::Relaxed);
                    self.display_data.capture_state = match state_val {
                        1 => CaptureState::Armed,
                        2 => CaptureState::Recording,
                        3 => CaptureState::Processing,
                        _ => CaptureState::Idle,
                    };
                }

                // ── Drain Result Channel from Worker ──
                // `worker_rx` and the `profiles` producer both live in the single-owner
                // `HostHandle` (crossing #5 receiver / crossing #4 producer). Drain the
                // WorkerOutput queue into a local vec first so per-item handling can
                // re-borrow the host and call `&mut self` helpers freely.
                let mut worker_outputs = Vec::new();
                if let Some(host) = self.host_handle.as_mut() {
                    while let Ok(output) = host.worker_rx.try_recv() {
                        worker_outputs.push(output);
                    }
                }
                for output in worker_outputs {
                    match output {
                        WorkerOutput::Measurement(measurement) => {
                            let target_idx = measurement.key_index;

                            // Appends and autosaves — a full-compass pass is
                            // hours of work and the app has no other
                            // guaranteed write point.
                            self.session.record(measurement);

                            // …then push the recompiled (measured-B) template to the live
                            // engine via crossing #4.
                            if let Some(host) = self.host_handle.as_mut() {
                                host.profiles.update_key_profile(
                                    target_idx,
                                    self.session.profile().active(target_idx),
                                );
                            }

                            eprintln!(
                                "[MAIN] Successfully slotted new capture data into Inharmonicity Profile at index {}",
                                target_idx
                            );

                            // A new trusted measurement changes the curve; bump the
                            // generation now so any bundle still queued behind this
                            // output is recognised as stale and dropped below.
                            self.mark_curve_dirty();
                            // The strobe lock holds through a capture (R6); the
                            // live curve advances and "newer curve available" lights.
                            self.strobe_lock
                                .on_trusted_set_edit(TrustedSetEdit::Captured);

                            // Re-arm automatically if in Auto mode
                            if let TuningMode::Auto = self.display_data.tuning_mode {
                                eprintln!("[MAIN] Auto-mode rearming...");
                                self.pipeline_handle
                                    .atomics
                                    .capture_state
                                    .store(CaptureState::Armed as u8, Ordering::Relaxed);
                                self.display_data.capture_state = CaptureState::Armed;
                            }
                        }
                        WorkerOutput::Curve(bundle) => {
                            // Latest-wins: accept only the bundle matching the most
                            // recent requested generation; an older one is superseded.
                            if bundle.generation == self.curve_generation {
                                self.curve_bundle = Some(*bundle);
                            }
                        }
                    }
                }

                // Send a (re)compute job if the trusted set changed this tick.
                self.pump_curve_job();

                self.update_strobe(frame_pushed);

                self.refresh_undo_label();
                self.session.flush_if_quiet();

                if self.display_data.settings_view_visible {
                    if self.display_data.settings_data.rms.visible {
                        let rms = self
                            .display_data
                            .last_frame
                            .as_ref()
                            .map(|f| f.rms_ema)
                            .unwrap_or(0.0);
                        let history = &mut self.display_data.settings_data.rms.history;
                        history.push_back(rms);
                        if history.len() > ENVELOPE_HISTORY_LENGTH {
                            history.pop_front();
                        }
                        self.display_data.settings_data.rms.current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.silence_threshold);
                    } else if self.display_data.settings_data.transient.visible {
                        let flux = self
                            .display_data
                            .last_frame
                            .as_ref()
                            .map(|f| f.nhwrsf)
                            .unwrap_or(0.0);
                        let current_threshold =
                            load_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold);

                        crate::views::transient_calibration::process_telemetry_tick(
                            &mut self.display_data.settings_data.transient,
                            flux,
                            current_threshold,
                        );

                        self.display_data.settings_data.transient.current_threshold =
                            current_threshold;
                    } else if self.display_data.settings_data.ninos.visible {
                        let ninos2 = self
                            .display_data
                            .last_frame
                            .as_ref()
                            .map(|f| f.ninos2)
                            .unwrap_or(0.0);

                        let current_threshold = load_f32(
                            &self
                                .pipeline_handle
                                .atomics
                                .config
                                .ninos2_stability_threshold,
                        );

                        crate::views::ninos2_calibration::process_telemetry_tick(
                            &mut self.display_data.settings_data.ninos,
                            ninos2,
                        );

                        self.display_data.settings_data.ninos.current_threshold = current_threshold;
                    }
                }

                // ── Calibration Hook ──
                if self.display_data.is_calibrating {
                    let current_rms = self
                        .display_data
                        .last_frame
                        .as_ref()
                        .map(|f| f.rms_ema)
                        .unwrap_or(0.0);
                    if let Some(silence_val) = crate::calibration::process_calibration_tick(
                        &mut self.display_data.settings_data.rms,
                        current_rms,
                        frame_pushed,
                    ) {
                        // Finished
                        self.display_data.is_calibrating = false;
                        self.display_data.settings_data.rms.calibration_complete = true;

                        store_f32(
                            &self.pipeline_handle.atomics.config.silence_threshold,
                            silence_val,
                        );
                        self.display_data.settings_data.rms.current_threshold = silence_val;

                        // Seed the transient wizard's baseline directly from this calculation point:
                        if let Some(active) =
                            &self.display_data.settings_data.rms.active_calibration
                        {
                            self.display_data
                                .settings_data
                                .transient
                                .noise_floor_baseline = active.max_seen_rms;
                        }

                        eprintln!(
                            "[MAIN] Lock-Free Calibration complete. Threshold set to: {:.6}",
                            silence_val
                        );
                    } else if let Some(active) =
                        &self.display_data.settings_data.rms.active_calibration
                        && let Some(countdown) = active.countdown
                    {
                        self.display_data.calibration_progress =
                            (crate::calibration::CALIBRATION_FRAMES.saturating_sub(countdown))
                                as usize;
                    }
                }
            }
        }
        iced::Task::none()
    }

    /// Renders the main application interface.
    pub fn view(&self) -> Element<'_, Message> {
        if self.display_data.settings_view_visible {
            create_settings_view(&self.display_data, self.curve_bundle.as_ref())
        } else {
            create_main_view(
                &self.display_data,
                self.session.profile(),
                Message::CaptureButtonClicked,
                self.curve_bundle.as_ref(),
            )
        }
    }

    /// Creates a subscription for continuous application updates.
    ///
    /// Returns a timer subscription that fires every 16ms (60 FPS) to ensure
    /// smooth real-time audio visualization and responsive UI updates.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _status, _window_id| match event {
                iced::Event::Window(iced::window::Event::CloseRequested) => Some(Message::Exit),
                _ => None,
            }),
        ])
    }

    /// Returns the application theme.
    fn theme(&self) -> Theme {
        Theme::Dark
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Builds a `(t, unwrapped-cycles)` history for a steady `beat_hz` at the
    /// DSP hop cadence and asserts the fit recovers the corresponding cents.
    fn fit(beat_hz: f32, ref_hz: f32, secs: f32) -> Option<f32> {
        let base = Instant::now();
        let dt = 1024.0 / 44_100.0; // one hop
        let mut hist = VecDeque::new();
        let mut t = 0.0f32;
        while t <= secs {
            hist.push_back((base + Duration::from_secs_f32(t), beat_hz * t));
            t += dt;
        }
        band_slope_cents(&hist, Some(ref_hz))
    }

    /// A +0.5 Hz beat at A2 must read the matching sharp cents; −0.5 Hz the
    /// mirror flat; and an in-tune (0 Hz) band must read ≈ 0.
    #[test]
    fn band_slope_recovers_beat_rate() {
        let expect = |hz: f32| 1200.0 * ((110.0 + hz) / 110.0f32).log2();
        let sharp = fit(0.5, 110.0, 0.6).expect("enough span");
        assert!(
            (sharp - expect(0.5)).abs() < 0.05,
            "sharp {sharp} vs {}",
            expect(0.5)
        );
        let flat = fit(-0.5, 110.0, 0.6).expect("enough span");
        assert!(
            (flat - expect(-0.5)).abs() < 0.05,
            "flat {flat} vs {}",
            expect(-0.5)
        );
        let in_tune = fit(0.0, 110.0, 0.6).expect("enough span");
        assert!(in_tune.abs() < 0.02, "in-tune must read ~0, got {in_tune}");
    }

    /// Below the minimum span the fit withholds a number (the instantaneous
    /// readout covers the fill-in), and a missing reference yields `None`.
    /// The minimum span must clear one analysis window, or the fit is drawing a
    /// line through repeats of the same audio (ADR 0011 §11).
    #[test]
    fn band_slope_span_clears_the_analysis_window() {
        let analysis_secs = audio::BASS_WINDOW_SIZE as f32 / audio::SAMPLE_RATE as f32;
        assert!(
            BAND_SLOPE_MIN_SPAN_SECS >= analysis_secs,
            "min span {BAND_SLOPE_MIN_SPAN_SECS} s is under one analysis window ({analysis_secs} s)"
        );
    }

    /// The frame-loss guard must reproduce the 6-of-26 floor it replaced at the
    /// shipped window, and must scale if that window moves.
    #[test]
    fn band_slope_frame_guard_scales_with_the_window() {
        let needed = |span: f32| (BAND_SLOPE_MIN_FRAME_FRACTION * span * HOP_RATE_HZ).ceil() as u32;
        assert_eq!(needed(BAND_SLOPE_WINDOW_SECS), 7, "≈6–7 of 26 at 0.6 s");
        assert_eq!(
            needed(BAND_SLOPE_MIN_SPAN_SECS),
            3,
            "≈3 of 11 at the min span"
        );
        assert!(
            needed(BAND_SLOPE_WINDOW_SECS / 2.0) < needed(BAND_SLOPE_WINDOW_SECS),
            "the guard must follow the window, not sit at an absolute count"
        );
    }

    /// A verdict that alternates — the measured behaviour at the boundary — must
    /// never move the displayed source, however long it flaps.
    #[test]
    fn readout_switch_ignores_a_flapping_verdict() {
        let (mut state, mut run) = (false, 0u8);
        for i in 0..200 {
            debounce_verdict(&mut state, &mut run, i % 2 == 0);
        }
        assert!(!state, "an alternating verdict must not switch the source");
    }

    /// A sustained verdict switches, and only on the full run: one agreeing hop
    /// resets the evidence.
    #[test]
    fn readout_switch_needs_an_unbroken_run() {
        let (mut state, mut run) = (false, 0u8);
        for _ in 0..READOUT_SWITCH_HOPS - 1 {
            debounce_verdict(&mut state, &mut run, true);
        }
        assert!(!state, "must not switch one hop early");
        debounce_verdict(&mut state, &mut run, false); // evidence broken
        assert_eq!(run, 0, "an agreeing hop resets the run");
        for _ in 0..READOUT_SWITCH_HOPS - 1 {
            debounce_verdict(&mut state, &mut run, true);
        }
        assert!(!state, "the run restarted, so still no switch");
        debounce_verdict(&mut state, &mut run, true);
        assert!(state, "the full run switches the source");
    }

    #[test]
    fn band_slope_withholds_until_ready() {
        assert!(
            fit(0.5, 110.0, 0.1).is_none(),
            "too short a baseline must be None"
        );
        assert!(
            fit(0.5, 0.0, 0.6).is_none(),
            "non-positive ref must be None"
        );
    }
}
