//! The iced application: the [`Message`] set, the state [`TunerApp`] holds, and
//! the [`AppDisplayData`] mirror every view renders from.

pub(crate) mod strobe;

use crate::app::strobe::{
    LockedTargets, StrobeLock, StrobeLockView, StrobeState, TrustedSetEdit, UnisonState,
};
use crate::calibration::{self, SettingsDisplayData};
use crate::library::{self, AppSettings, ProfileSort};
use crate::session::{ProfileSession, UndoneCapture};
use crate::views::{
    inspector_view::InspectorRow, library_view::IdentityField, main_view, settings_view,
};
use crate::widgets::envelope::ENVELOPE_HISTORY_LENGTH;
use crate::widgets::guitar_strings::GUITAR_STRING_KEYS;
use iced::{self, Element, Subscription, Theme};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use tuner_core::{
    FrameOutput,
    audio::{self, AudioSource, HOP_RATE_HZ, HostHandle, SAMPLE_RATE},
    models::{self, CurveInput, EngineChoice, InharmonicityProfile, ReferenceMode},
    pipeline::{
        self, ArmRequest, CAPTURE_DEFAULT_SAMPLES, CAPTURE_MAX_SAMPLES, CaptureCommand,
        CaptureState, PipelineHandle,
    },
    worker::{self, CurveBundle, CurveJob, WorkerOutput},
};

/// The span the cent meter's displayed value averages its per-hop readings over.
// Ours, set by eye: one hop's reading is too erratic to read. The mean costs
// ≈ 58 ms of lag for ≈ 2.2× less white noise.
const SMOOTHING_SECS: f32 = 0.116;

/// [`SMOOTHING_SECS`] in hops.
// Rounded, not truncated: a bare cast gives 4.
const SMOOTHING_HOPS: usize = (SMOOTHING_SECS * HOP_RATE_HZ + 0.5) as usize;

/// Runs the application: builds the iced program over [`TunerApp`] and blocks
/// until the window closes.
pub fn run() -> iced::Result {
    eprintln!("[MAIN] Starting Inharmonicity application...");
    eprintln!("[MAIN] Initializing GUI framework...");
    let result = iced::application(TunerApp::new, TunerApp::update, TunerApp::view)
        .title("Inharmonicity")
        .subscription(TunerApp::subscription)
        .theme(TunerApp::theme)
        // A close request becomes `Message::Exit`, which stops the audio host
        // before exiting: exiting with the stream live segfaulted on Linux/ALSA.
        .window(iced::window::Settings {
            exit_on_close_request: false,
            ..Default::default()
        })
        .run();
    eprintln!("[MAIN] Application finished with result: {:?}", result);
    result
}

/// An operator action, or the tick that refreshes the display.
#[derive(Debug, Clone)]
pub enum Message {
    // Tuning and capture
    KeySelected(u8),
    ToggleMeasurementMode,
    CaptureButtonClicked,
    UndoLastCapture,
    /// Writes the profile now; it also autosaves.
    SaveProfile,

    // Profile library
    /// Opens a profile and adopts its settings.
    OpenProfile(PathBuf),
    NewProfile,
    /// Deletes a profile document; never the open one.
    DeleteProfile(PathBuf),
    DuplicateProfile(PathBuf),
    LibrarySortChanged(ProfileSort),
    LibrarySearchChanged(String),
    ToggleLibrary,
    IdentityFieldChanged(IdentityField, String),
    InstrumentKindChanged(models::InstrumentKind),

    // Measurement inspector
    ToggleInspector,
    /// Reviews a key in the open inspector.
    InspectKey(u8),
    /// Opens the inspector on a key.
    ReviewKey(u8),
    ToggleInspectorHistory,
    ToggleInspectorUnused,
    /// Discards one retained measurement of a key, keeping its audio.
    DropMeasurement(u8, usize),
    /// Selects a key and arms a capture of it.
    RemeasureKey(u8),

    // String isolation
    ToggleStringIsolationPanel,
    SetStringIsolation(bool),
    SetSoundingStrings(models::SoundingStrings),

    // Capture duration
    ToggleExtendedCapturePanel,
    SetExtendedCapture(bool),
    SetExtendedCaptureSecs(f32),
    AbortCapture,

    // Main-view panels
    ToggleSpectrum,
    ToggleCentMeter,
    ToggleKeySelect,
    ToggleCurvePlot,
    ToggleStrobe,
    ToggleUnisonDisplayed,
    ToggleUnisonAll,
    ToggleUnisonAssistPanel,
    SetUnisonAssist(bool),
    SetReferenceMode(ReferenceMode),
    RequestRelock,
    ConfirmRelock,
    CancelRelock,

    // Curve gallery
    ToggleCurveSelect,
    CurveDetailOpened(EngineChoice),
    CurveDetailClosed,
    EngineSelected(EngineChoice),

    // Settings view and the silence threshold
    ToggleSettingsView,
    ToggleNoiseFloorAdjustment,
    SilenceThresholdChanged(f32),
    RecalibrateNoiseFloor,

    // Onset threshold
    ToggleTransientCalibration,
    ResetTransientScope,
    NhwrsfThresholdChanged(f32),

    // Sustain threshold
    ToggleSustainCalibration,
    ResetSustainScope,
    SustainThresholdChanged(f32),

    // Note picker
    ToggleInstrumentSelect,
    SetInstrument(Instrument),

    // Lifecycle
    Exit,
    Tick,
}

/// Whether discovery names the key being tuned, or the operator has picked it.
#[derive(Debug, Clone, PartialEq)]
pub enum TuningMode {
    Auto,
    Manual { key_index: u8, note_name: String },
}

/// Which note picker the main view shows: the 88-key keyboard, or six
/// standard-tuning guitar strings for exercising the strobe on a non-piano
/// source. Only the picker changes; a key is still its 0–87 piano index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Instrument {
    #[default]
    Piano,
    Guitar,
}

/// The state every view renders from.
#[derive(Debug, Clone)]
pub struct AppDisplayData {
    // Audio and the live pitch reading
    pub audio_worker_active: bool,
    pub last_frame: Option<FrameOutput>,
    /// The last locked note: held while [`Self::is_stale`], cleared on silence.
    pub last_note_index: Option<u8>,
    pub last_frequency: Option<f32>,
    /// Cents from the note's tuning target, the curve's or ET's.
    pub last_cents: Option<f32>,
    pub smoothing_buffer: Vec<f32>,
    /// There is audio but no locked note.
    pub is_stale: bool,

    // Main-view panels
    pub spectrum_visible: bool,
    pub cent_meter_visible: bool,
    pub key_select_visible: bool,
    pub curve_plot_visible: bool,
    pub strobe_visible: bool,
    /// Unison assist is on; the two unison panels exist only while it is.
    pub unison_assist: bool,
    /// The Unison Assist settings panel is open.
    pub unison_assist_visible: bool,
    /// The magnified one-partial unison panel is on screen.
    pub unison_displayed_visible: bool,
    /// The stacked all-partials unison panel is on screen.
    pub unison_all_visible: bool,

    pub settings_view_visible: bool,

    // Profile library
    pub library_visible: bool,
    pub library_entries: Vec<library::ProfileEntry>,
    pub library_sort: ProfileSort,
    pub library_search: String,
    pub open_identity: models::InstrumentIdentity,
    pub open_profile_path: Option<PathBuf>,

    // Measurement inspector
    pub inspector_visible: bool,
    /// The key under review.
    pub inspector_key: Option<u8>,
    /// Every retained measurement of [`Self::inspector_key`], oldest first.
    pub inspector_rows: Vec<InspectorRow>,
    /// Show the reviewed key's earlier measurements, not only the one in use.
    pub inspector_expanded: bool,
    /// Show the reviewed key's captures that no consumer reads.
    pub inspector_unused_expanded: bool,

    // Curves and the strobe
    pub curve_select_visible: bool,
    /// The engine whose detail view is open in the gallery.
    pub curve_detail: Option<EngineChoice>,
    /// The engine whose curve the plot and the strobe read.
    pub selected_engine: EngineChoice,
    pub strobe: StrobeState,
    pub unison: UnisonState,
    /// The target every readout measures against: the strobe band, its cents
    /// readout and the cent meter.
    pub reference_mode: ReferenceMode,
    /// `None` when there is no lock to show: disengaged, or ET mode.
    pub strobe_lock_view: Option<StrobeLockView>,
    pub relock_confirm_open: bool,
    pub instrument_select_visible: bool,

    pub settings_data: SettingsDisplayData,
    pub tuning_mode: TuningMode,
    pub instrument: Instrument,

    // Capture
    pub measurement_mode_active: bool,
    pub capture_state: CaptureState,
    pub undo_target_note: Option<String>,
    /// The strings the next capture records as sounding. Sticky across captures:
    /// a mute pattern is set at the instrument and several notes are taken
    /// through it.
    pub sounding_strings: models::SoundingStrings,
    pub string_isolation: bool,
    pub string_isolation_visible: bool,
    /// The operator has set a count or a sounding string since the control was
    /// switched on: a chosen 3 and the default 3 both encode as nothing declared.
    pub strings_touched: bool,
    pub extended_capture: bool,
    pub extended_capture_secs: f32,
    pub extended_capture_visible: bool,
    /// A curve job is with the Worker. It cannot preempt a job in progress, so
    /// a capture taken now waits for the recompute.
    pub curve_recomputing: bool,
    /// Seconds recorded so far by a capture in progress; `0.0` otherwise.
    pub capture_progress_secs: f32,
}

/// The application: the open session, the curve and strobe state, and the
/// handles to the pipeline and the Worker.
pub struct TunerApp {
    host_handle: Option<HostHandle>,
    frame_rx: Option<triple_buffer::Output<FrameOutput>>,
    session: ProfileSession,
    app_settings: AppSettings,
    /// The latest bundle from the Worker; derived, never persisted.
    curve_bundle: Option<CurveBundle>,
    /// A trusted-set edit awaits a recompute the job channel has not accepted.
    curve_dirty: bool,
    /// Held until the Worker accepts it.
    pending_dump_dir: Option<PathBuf>,
    /// Held until the DSP ring accepts it. A newer command replaces it, as the
    /// pipeline keeps only the newest of a hop.
    pending_capture_command: Option<CaptureCommand>,
    /// Incremented on every trusted-set edit and echoed on the bundle, so a
    /// superseded bundle is dropped.
    curve_generation: u64,
    /// The frozen curve the strobe reads, distinct from the live `curve_bundle`.
    strobe_lock: StrobeLock,
    /// The `(key, et_mode, generation, engine)` identity of the reference set
    /// the bank last accepted, `Some(None)` for none; `None` forces a push. A
    /// failed push leaves it stale, so the next tick retries.
    strobe_pushed: Option<Option<(u8, bool, u64, EngineChoice)>>,
    pipeline_handle: PipelineHandle,
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
    /// The state before audio starts, with the silence calibration pending.
    fn default() -> Self {
        eprintln!("[MAIN] Creating TunerApp...");

        Self {
            host_handle: None,
            frame_rx: None,
            session: ProfileSession::default(),
            app_settings: AppSettings::default(),
            curve_bundle: None,
            curve_dirty: false,
            pending_dump_dir: None,
            pending_capture_command: None,
            curve_generation: 0,
            strobe_lock: StrobeLock::Disengaged,
            strobe_pushed: None,
            pipeline_handle: PipelineHandle::default(),
            display_data: AppDisplayData {
                audio_worker_active: false,
                last_frame: None,
                last_note_index: None,
                last_frequency: None,
                last_cents: None,
                smoothing_buffer: Vec::new(),
                is_stale: false,
                spectrum_visible: true,
                cent_meter_visible: true,
                key_select_visible: true,
                curve_plot_visible: true,
                strobe_visible: true,
                unison_assist: false,
                unison_assist_visible: false,
                unison_displayed_visible: false,
                unison_all_visible: false,
                settings_view_visible: false,
                library_visible: false,
                library_entries: Vec::new(),
                library_sort: ProfileSort::default(),
                library_search: String::new(),
                open_identity: models::InstrumentIdentity::default(),
                open_profile_path: None,
                inspector_visible: false,
                inspector_key: None,
                inspector_rows: Vec::new(),
                inspector_expanded: false,
                inspector_unused_expanded: false,
                curve_select_visible: false,
                curve_detail: None,
                selected_engine: EngineChoice::MultiBalanced,
                strobe: StrobeState::default(),
                unison: UnisonState::default(),
                reference_mode: ReferenceMode::default(),
                strobe_lock_view: None,
                relock_confirm_open: false,
                settings_data: SettingsDisplayData::default(),
                instrument_select_visible: false,
                tuning_mode: TuningMode::Auto,
                instrument: Instrument::Piano,
                measurement_mode_active: false,
                capture_state: CaptureState::Idle,
                undo_target_note: None,
                sounding_strings: models::SoundingStrings::UNDECLARED,
                string_isolation: false,
                string_isolation_visible: false,
                strings_touched: false,
                curve_recomputing: false,
                extended_capture: false,
                extended_capture_secs: 5.0,
                extended_capture_visible: false,
                capture_progress_secs: 0.0,
            },
        }
    }
}

impl TunerApp {
    /// Refreshes the Undo label from the head of the undo history. It names the
    /// capture undo would discard by its timestamp, since repeat captures share
    /// a key.
    fn refresh_undo_label(&mut self) {
        self.display_data.undo_target_note = self.session.undo_target().map(|(key, epoch)| {
            let note = models::find_nearest_note_by_index(key).0;
            match epoch {
                Some(e) => format!("{note} · {e}"),
                None => note,
            }
        });
    }

    /// Makes `profile` the open instrument.
    fn adopt_profile(&mut self, profile: InharmonicityProfile, path: PathBuf) {
        self.session.adopt(profile, path, &mut self.app_settings);
        self.sync_open_profile();
    }

    /// Brings everything that follows the open instrument into line with it:
    /// its settings, the live engine, the dump directory, the curve, the strobe
    /// lock and the inspector.
    fn sync_open_profile(&mut self) {
        self.save_app_settings();
        let settings = self.session.profile().settings.clone();
        self.display_data.selected_engine = settings.engine;
        self.display_data.reference_mode = settings.reference_mode;
        self.apply_profile_settings(&settings);
        self.sync_identity_mirror();
        self.sync_dump_dir();
        if let Some(host) = self.host_handle.as_mut() {
            host.profiles.update_all(self.session.profile());
        }
        self.mark_curve_dirty();
        self.strobe_lock.on_trusted_set_edit(TrustedSetEdit::Loaded);
        self.refresh_undo_label();
        // The reviewed key indexed the previous instrument's measurements.
        self.display_data.inspector_key = None;
        self.refresh_inspector();
    }

    /// Hides every settings panel; the settings view shows one at a time.
    fn close_settings_panels(&mut self) {
        let d = &mut self.display_data;
        d.library_visible = false;
        d.inspector_visible = false;
        d.curve_select_visible = false;
        d.instrument_select_visible = false;
        d.string_isolation_visible = false;
        d.unison_assist_visible = false;
        d.extended_capture_visible = false;
        d.settings_data.rms.visible = false;
        d.settings_data.transient.visible = false;
        d.settings_data.sustain.visible = false;
    }

    /// Refreshes the inspector's rows from the open profile.
    ///
    /// A reviewed key left with no measurements stays under review: dropping its
    /// last entry removes the key from the profile, and moving the panel then
    /// would hide the outcome, and the key's Re-measure button with it.
    fn refresh_inspector(&mut self) {
        let profile = self.session.profile();
        let selected = match &self.display_data.tuning_mode {
            TuningMode::Manual { key_index, .. } => Some(*key_index),
            TuningMode::Auto => None,
        };
        let key = self
            .display_data
            .inspector_key
            .or(selected)
            .or_else(|| profile.measurements.keys().next().copied());
        self.display_data.inspector_key = key;

        let profile = self.session.profile();
        let rows: Vec<InspectorRow> = key
            .and_then(|k| {
                let entries = profile.measurements.get(&k)?;
                // Matched by address: repeats of a key can be equal field for
                // field. `None` when nothing is trusted, so no row is in use.
                let active = profile.active(k);
                Some(
                    entries
                        .iter()
                        .enumerate()
                        .map(|(index, m)| InspectorRow {
                            index,
                            epoch: m.last_captured.clone(),
                            manual: !m.captured_in_auto,
                            partials: m.partials.len(),
                            b: m.calculated_b,
                            sounding_strings: m.sounding_strings,
                            in_use: active.is_some_and(|a| std::ptr::eq(m, a)),
                            trusted: m.is_trusted(),
                        })
                        .collect(),
                )
            })
            .unwrap_or_default();
        self.display_data.inspector_rows = rows;
    }

    /// What the next record should be: the fill target and the operator's
    /// standing string declaration, which ride every [`CaptureCommand::Arm`].
    fn arm_request(&self) -> ArmRequest {
        let target_samples = if self.display_data.extended_capture {
            let requested =
                (self.display_data.extended_capture_secs * SAMPLE_RATE as f32).round() as usize;
            requested.clamp(CAPTURE_DEFAULT_SAMPLES, CAPTURE_MAX_SAMPLES)
        } else {
            CAPTURE_DEFAULT_SAMPLES
        };
        ArmRequest {
            target_samples,
            declared_strings: self.display_data.sounding_strings.declared(),
        }
    }

    /// Sends a capture-lifecycle command to the DSP thread (crossing #4),
    /// holding it for the next tick if the ring is full.
    fn send_capture_command(&mut self, command: CaptureCommand) {
        self.pending_capture_command = Some(command);
        self.pump_capture_command();
    }

    /// Retries the held command, if any.
    fn pump_capture_command(&mut self) {
        let Some(command) = self.pending_capture_command else {
            return;
        };
        if let Some(host) = self.host_handle.as_mut()
            && host.capture_commands.send(command)
        {
            self.pending_capture_command = None;
        }
    }

    /// Re-sends the `Arm` when its parameters change while armed. Otherwise a
    /// change made after arming, which auto mode's re-arm makes routine, would
    /// reach the capture after next.
    fn republish_arm(&mut self) {
        if self.display_data.capture_state == CaptureState::Armed {
            self.send_capture_command(CaptureCommand::Arm(self.arm_request()));
        }
    }

    /// Records the string declaration the next capture will carry.
    fn set_capture_strings(&mut self, strings: models::SoundingStrings) {
        self.display_data.sounding_strings = strings;
        self.republish_arm();
    }

    /// Deletes an undone capture's diagnostics dump.
    fn remove_dump(root: &Path, undone: &UndoneCapture) {
        let dir = root.join(worker::dump_dir_name(undone.key, &undone.epoch));
        if dir.is_dir() {
            match std::fs::remove_dir_all(&dir) {
                Ok(()) => eprintln!(
                    "[MAIN] Removed diagnostics of undone capture: {}",
                    dir.display()
                ),
                Err(e) => eprintln!("[MAIN] Could not remove {}: {e}", dir.display()),
            }
        }
    }

    /// Enters manual mode on `key_index`.
    fn enter_manual_mode(&mut self, key_index: u8) {
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
        self.reset_strobe();
    }

    /// Returns to Auto. The string declaration is retracted, not hidden: it names
    /// strings of the key the operator picked, and a capture in Auto would
    /// inherit it.
    fn enter_auto_mode(&mut self) {
        self.display_data.tuning_mode = TuningMode::Auto;
        self.pipeline_handle
            .atomics
            .config
            .target_note
            .store(255, Ordering::Relaxed);
        self.display_data.smoothing_buffer.clear();
        self.display_data.strings_touched = false;
        self.set_capture_strings(models::SoundingStrings::UNDECLARED);
    }

    /// Saves the app-level settings, logging a failure rather than propagating
    /// it.
    fn save_app_settings(&self) {
        if let Err(e) = self.app_settings.save() {
            eprintln!("[MAIN] Could not save app settings: {e}");
        }
    }

    /// Points the Worker's dump root at the open instrument's directory
    /// (crossing #6). A rename does not move it: the directory is named for the
    /// instrument's id.
    fn sync_dump_dir(&mut self) {
        let identity = &self.session.profile().identity;
        let dir = library::diagnostics_dir_for(&identity.id);
        library::write_manifest(&dir, identity);
        eprintln!(
            "[MAIN] Capture diagnostics → {} ('{}')",
            dir.display(),
            identity.name
        );
        self.pending_dump_dir = Some(dir);
        self.pump_dump_dir();
    }

    /// Hands the Worker the pending dump directory (crossing #6) until it is
    /// accepted. It must not be dropped: a recompute holding the job slot would
    /// file every capture until the next instrument change under the previous
    /// instrument's directory.
    fn pump_dump_dir(&mut self) {
        let Some(dir) = self.pending_dump_dir.clone() else {
            return;
        };
        if let Some(host) = self.host_handle.as_ref()
            && host.send_dump_dir(Some(dir))
        {
            self.pending_dump_dir = None;
        }
    }

    /// Copies the open instrument's identity and path into the display data;
    /// every identity edit needs it.
    fn sync_identity_mirror(&mut self) {
        self.display_data.open_identity = self.session.profile().identity.clone();
        self.display_data.open_profile_path = self.session.path().map(|p| p.to_path_buf());
    }

    /// Pushes the profile's onset and sustain thresholds into the live atomics.
    /// Both are level-independent, so they describe the instrument and travel
    /// with it. The silence threshold is not a profile setting: it is an absolute
    /// RMS that moves with mic, gain and room, so it is calibrated at launch.
    fn apply_profile_settings(&mut self, settings: &models::ProfileSettings) {
        let config = &self.pipeline_handle.atomics.config;
        pipeline::store_f32(&config.nhwrsf_threshold, settings.nhwrsf_threshold);
        pipeline::store_f32(
            &config.sustain_stability_threshold,
            settings.sustain_stability_threshold,
        );
        self.display_data.settings_data.transient.current_threshold = settings.nhwrsf_threshold;
        self.display_data.settings_data.sustain.current_threshold =
            settings.sustain_stability_threshold;
    }

    /// Marks the tuning curve stale. The input is snapshotted when the job is
    /// sent, not here, so edits made before then collapse into one recompute.
    fn mark_curve_dirty(&mut self) {
        self.curve_generation = self.curve_generation.wrapping_add(1);
        self.curve_dirty = true;
    }

    /// Sends a pending recompute to the Worker (crossing #6), snapshotting the
    /// trusted [`CurveInput`] now; a full job slot leaves it pending.
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
                self.display_data.curve_recomputing = true;
            }
        }
    }

    /// Turns unison assist on or off, and both panels with it: a panel left on
    /// screen with the mode off would have no toggle to dismiss it.
    fn set_unison_assist(&mut self, on: bool) {
        self.display_data.unison_assist = on;
        self.display_data.unison_displayed_visible = on;
        self.display_data.unison_all_visible = on;
    }

    /// Swaps the note picker, which must have changed. The profile is left
    /// alone: the picker is an operator preference, while the reference mode
    /// belongs to the instrument.
    ///
    /// Entering Guitar with a key that is not an open string drops to Auto, since
    /// the guitar picker could neither show nor clear that selection.
    fn set_instrument(&mut self, inst: Instrument) {
        self.display_data.instrument = inst;
        self.reset_strobe();

        let drop_target = inst == Instrument::Guitar
            && matches!(
                &self.display_data.tuning_mode,
                TuningMode::Manual { key_index, .. } if !GUITAR_STRING_KEYS.contains(key_index)
            );
        if drop_target {
            self.enter_auto_mode();
        }
    }

    /// Creates the app, starts the audio and resumes the last instrument.
    pub fn new() -> (Self, iced::Task<Message>) {
        let mut app = Self::default();
        app.start_audio_processing();
        app.app_settings = AppSettings::load();
        app.display_data.string_isolation = app.app_settings.string_isolation;
        app.set_unison_assist(app.app_settings.unison_assist);
        app.display_data.extended_capture = app.app_settings.extended_capture;
        app.display_data.extended_capture_secs = app.app_settings.extended_capture_secs;
        app.display_data.instrument = app.app_settings.instrument;
        app.session.open_at_startup(&mut app.app_settings);
        app.sync_open_profile();
        (app, iced::Task::none())
    }

    /// Starts the analysis thread and keeps its handles.
    fn start_audio_processing(&mut self) {
        // Headless tests would hang trying to open physical audio hardware.
        if cfg!(test) {
            eprintln!("[AUDIO-THREAD] Disabled for unit testing.");
            return;
        }

        // The root only: the open instrument's subdirectory follows once a
        // profile is adopted.
        let dump_dir = library::diagnostics_dir();

        match audio::spawn_analysis_thread(AudioSource::Default, Some(dump_dir)) {
            Ok(mut handle) => {
                eprintln!("[AUDIO] Hardware stream active.");

                // The new pipeline starts from its own default threshold.
                pipeline::store_f32(
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
                // Clicking the selected key again returns to Auto.
                if let TuningMode::Manual {
                    key_index: current_key,
                    ..
                } = &self.display_data.tuning_mode
                    && *current_key == key_index
                {
                    self.enter_auto_mode();
                    return iced::Task::none();
                }

                self.enter_manual_mode(key_index);
                // The inspector follows the key being tuned.
                self.display_data.inspector_key = Some(key_index);
                self.refresh_inspector();
            }
            Message::ToggleMeasurementMode => {
                self.display_data.measurement_mode_active =
                    !self.display_data.measurement_mode_active;

                if self.display_data.measurement_mode_active {
                    eprintln!("[MAIN] Measurement mode ON - arming");
                    let request = self.arm_request();
                    self.send_capture_command(CaptureCommand::Arm(request));
                } else {
                    eprintln!("[MAIN] Measurement mode OFF");
                    self.send_capture_command(CaptureCommand::Cancel);
                }
            }
            Message::CaptureButtonClicked => {
                // The DSP thread owns the lifecycle, so this toggles from the
                // state it last reported.
                if self.display_data.measurement_mode_active {
                    match self.display_data.capture_state {
                        CaptureState::Idle => {
                            let request = self.arm_request();
                            self.send_capture_command(CaptureCommand::Arm(request));
                        }
                        CaptureState::Armed => {
                            self.send_capture_command(CaptureCommand::Cancel);
                        }
                        CaptureState::Recording | CaptureState::Processing => {}
                    }
                }
            }
            Message::UndoLastCapture => {
                if let Some(undone) = self.session.undo() {
                    let idx = undone.key;
                    // An undo declares the capture bad, so its audio goes too.
                    let root = library::diagnostics_dir_for(&self.session.profile().identity.id);
                    Self::remove_dump(&root, &undone);
                    if let Some(host) = self.host_handle.as_mut() {
                        host.profiles
                            .update_key_profile(idx, self.session.profile().active(idx));
                    }
                    eprintln!("[MAIN] Undoing profile change at index {}", idx);
                    // Undoing an untrusted entry recomputes an identical curve:
                    // harmless, and undo is rare.
                    self.mark_curve_dirty();
                    self.strobe_lock.on_trusted_set_edit(TrustedSetEdit::Undone);
                }
                self.refresh_undo_label();
                self.refresh_inspector();
            }
            Message::ToggleInspector => {
                let visible = !self.display_data.inspector_visible;
                self.close_settings_panels();
                self.display_data.inspector_visible = visible;
                if visible {
                    self.display_data.settings_view_visible = true;
                    self.refresh_inspector();
                }
            }
            Message::InspectKey(key) => {
                // Another key's history is another question.
                self.display_data.inspector_expanded = false;
                self.display_data.inspector_unused_expanded = false;
                self.display_data.inspector_key = Some(key);
                self.refresh_inspector();
            }
            Message::ToggleInspectorHistory => {
                self.display_data.inspector_expanded = !self.display_data.inspector_expanded;
            }
            Message::ToggleInspectorUnused => {
                self.display_data.inspector_unused_expanded =
                    !self.display_data.inspector_unused_expanded;
            }
            Message::ReviewKey(key) => {
                self.close_settings_panels();
                self.display_data.inspector_visible = true;
                self.display_data.settings_view_visible = true;
                self.display_data.inspector_expanded = false;
                self.display_data.inspector_unused_expanded = false;
                self.display_data.inspector_key = Some(key);
                self.refresh_inspector();
            }
            Message::DropMeasurement(key, index) => {
                if let Some(dropped) = self.session.remove(key, index) {
                    // The audio stays: a drop distrusts the measurement, and the
                    // recording may still yield a good one.
                    eprintln!(
                        "[MAIN] Capture audio kept at {}",
                        library::diagnostics_dir_for(&self.session.profile().identity.id)
                            .join(worker::dump_dir_name(key, &dropped.last_captured))
                            .display()
                    );
                    // The key may now resolve to a different entry, or to none.
                    if let Some(host) = self.host_handle.as_mut() {
                        host.profiles
                            .update_key_profile(key, self.session.profile().active(key));
                    }
                    eprintln!("[MAIN] Dropped measurement {index} of key {key}");
                    self.mark_curve_dirty();
                    // The lock takes a drop as it takes an undo.
                    self.strobe_lock.on_trusted_set_edit(TrustedSetEdit::Undone);
                }
                self.refresh_undo_label();
                self.refresh_inspector();
            }
            Message::RemeasureKey(key) => {
                self.enter_manual_mode(key);
                self.display_data.inspector_key = Some(key);
                self.display_data.measurement_mode_active = true;
                let request = self.arm_request();
                self.send_capture_command(CaptureCommand::Arm(request));
                // Back to the main view, where the note is played.
                self.display_data.settings_view_visible = false;
                self.refresh_inspector();
            }
            Message::ToggleStringIsolationPanel => {
                let visible = !self.display_data.string_isolation_visible;
                self.close_settings_panels();
                self.display_data.string_isolation_visible = visible;
                if visible {
                    self.display_data.settings_view_visible = true;
                }
            }
            Message::ToggleUnisonAssistPanel => {
                let visible = !self.display_data.unison_assist_visible;
                self.close_settings_panels();
                self.display_data.unison_assist_visible = visible;
                if visible {
                    self.display_data.settings_view_visible = true;
                }
            }
            Message::SetUnisonAssist(on) => {
                self.set_unison_assist(on);
                self.app_settings.unison_assist = on;
                self.save_app_settings();
            }
            Message::SetStringIsolation(on) => {
                self.display_data.string_isolation = on;
                self.display_data.strings_touched = false;
                self.app_settings.string_isolation = on;
                self.save_app_settings();
                // Off retracts the declaration rather than hiding it: an unseen
                // one would keep landing on ordinary captures.
                if !on {
                    self.set_capture_strings(models::SoundingStrings::UNDECLARED);
                }
            }
            Message::ToggleExtendedCapturePanel => {
                let visible = !self.display_data.extended_capture_visible;
                self.close_settings_panels();
                self.display_data.extended_capture_visible = visible;
                if visible {
                    self.display_data.settings_view_visible = true;
                }
            }
            Message::SetExtendedCapture(on) => {
                self.display_data.extended_capture = on;
                self.app_settings.extended_capture = on;
                self.save_app_settings();
                self.republish_arm();
            }
            Message::SetExtendedCaptureSecs(secs) => {
                self.display_data.extended_capture_secs = secs;
                self.app_settings.extended_capture_secs = secs;
                self.save_app_settings();
                self.republish_arm();
            }
            Message::AbortCapture => {
                // Cancel also drops a take in progress.
                self.send_capture_command(CaptureCommand::Cancel);
            }
            Message::SetSoundingStrings(strings) => {
                self.display_data.strings_touched = true;
                self.set_capture_strings(strings);
            }
            Message::SaveProfile => {
                self.session.persist();
            }
            Message::ToggleLibrary => {
                let visible = !self.display_data.library_visible;
                self.close_settings_panels();
                self.display_data.library_visible = visible;
                if visible {
                    self.display_data.settings_view_visible = true;
                    self.display_data.library_entries =
                        library::list_profiles(self.display_data.library_sort);
                } else if self.session.is_dirty() {
                    // Leaving the form: a pending rename should not wait out the
                    // quiet delay.
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
                if self.session.delete(&path) {
                    self.app_settings.note_removed(&path);
                    self.save_app_settings();
                }
                self.display_data.library_entries =
                    library::list_profiles(self.display_data.library_sort);
            }
            Message::DuplicateProfile(path) => {
                match InharmonicityProfile::from_file(&path) {
                    Ok(mut profile) => {
                        profile.identity.name = format!("{} (copy)", profile.identity.name);
                        // A copy is another instrument. It mints its own id when
                        // first opened, so its dumps stay apart, and the serial
                        // number names the original.
                        profile.identity.id = String::new();
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
            Message::ToggleSpectrum => {
                eprintln!(
                    "[MAIN] Toggling spectrum visibility: {} -> {}",
                    self.display_data.spectrum_visible, !self.display_data.spectrum_visible
                );
                self.display_data.spectrum_visible = !self.display_data.spectrum_visible;
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
            Message::ToggleCurvePlot => {
                self.display_data.curve_plot_visible = !self.display_data.curve_plot_visible;
            }
            Message::ToggleStrobe => {
                self.display_data.strobe_visible = !self.display_data.strobe_visible;
            }
            Message::ToggleUnisonDisplayed => {
                self.display_data.unison_displayed_visible =
                    !self.display_data.unison_displayed_visible;
            }
            Message::ToggleUnisonAll => {
                self.display_data.unison_all_visible = !self.display_data.unison_all_visible;
            }
            Message::SetReferenceMode(mode) => {
                self.display_data.reference_mode = mode;
                self.reset_strobe();
                // Both tuning selections persist with the instrument, so
                // reopening it reproduces the targets it was tuned to.
                self.session.profile_mut().settings.reference_mode = mode;
                self.session.persist();
            }
            Message::RequestRelock => {
                if self.display_data.strobe_lock_view.is_some_and(|v| v.newer) {
                    self.display_data.relock_confirm_open = true;
                }
            }
            Message::ConfirmRelock => {
                if let Some(live) = &self.curve_bundle {
                    self.strobe_lock =
                        StrobeLock::Engaged(LockedTargets::freeze(live, self.session.profile()));
                    self.reset_strobe();
                }
                self.display_data.relock_confirm_open = false;
            }
            Message::CancelRelock => {
                self.display_data.relock_confirm_open = false;
            }
            Message::ToggleCurveSelect => {
                let vis = !self.display_data.curve_select_visible;
                self.close_settings_panels();
                self.display_data.curve_select_visible = vis;
                if !vis {
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
                // Every engine is already in the bundle, so nothing recomputes.
                self.display_data.selected_engine = choice;
                self.session.profile_mut().settings.engine = choice;
                self.session.persist();
            }

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
                self.close_settings_panels();
                self.display_data.settings_data.rms.visible = vis;
            }
            Message::SilenceThresholdChanged(value) => {
                pipeline::store_f32(
                    &self.pipeline_handle.atomics.config.silence_threshold,
                    value,
                );
                self.display_data.settings_data.rms.current_threshold = value;
            }
            Message::RecalibrateNoiseFloor => {
                self.display_data.settings_data.rms.recalibrate();
            }
            Message::ToggleTransientCalibration => {
                let vis = !self.display_data.settings_data.transient.visible;
                self.close_settings_panels();
                self.display_data.settings_data.transient.visible = vis;
                if vis {
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
                pipeline::store_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold, val);
                self.display_data.settings_data.transient.current_threshold = val;
                // Slider-rate: coalesced.
                self.session.profile_mut().settings.nhwrsf_threshold = val;
                self.session.touch();
            }
            Message::ToggleSustainCalibration => {
                let vis = !self.display_data.settings_data.sustain.visible;
                self.close_settings_panels();
                self.display_data.settings_data.sustain.visible = vis;
                if vis {
                    self.display_data.settings_data.sustain.history.clear();
                }
            }
            Message::ResetSustainScope => {
                self.display_data.settings_data.sustain.history.clear();
            }
            Message::SustainThresholdChanged(val) => {
                pipeline::store_f32(
                    &self
                        .pipeline_handle
                        .atomics
                        .config
                        .sustain_stability_threshold,
                    val,
                );
                self.display_data.settings_data.sustain.current_threshold = val;
                self.session
                    .profile_mut()
                    .settings
                    .sustain_stability_threshold = val;
                self.session.touch();
            }
            Message::ToggleInstrumentSelect => {
                let vis = !self.display_data.instrument_select_visible;
                self.close_settings_panels();
                self.display_data.instrument_select_visible = vis;
            }
            Message::SetInstrument(inst) => {
                if self.display_data.instrument != inst {
                    self.set_instrument(inst);
                    self.app_settings.instrument = inst;
                    self.save_app_settings();
                }
            }
            Message::Tick => self.tick(),
        }
        iced::Task::none()
    }

    /// One display tick: mirrors the freshest frame, applies the Worker's
    /// results, retries the held crossings, and advances the strobe and the
    /// calibration panels.
    fn tick(&mut self) {
        let mut frame_pushed = false;

        // ── Read freshest FrameOutput from triple buffer ──
        if let Some(ref mut frame_rx) = self.frame_rx
            && frame_rx.update()
        {
            frame_pushed = true;
            let frame = frame_rx.read().clone();
            self.display_data.capture_progress_secs =
                frame.capture_progress_samples as f32 / SAMPLE_RATE as f32;
            self.display_data.last_frame = Some(frame.clone());

            // A non-finite scalar becomes no reading: lyon asserts on
            // non-finite path coordinates, so one bad frame would crash
            // the session. Logged, since a producer emitting one is a bug.
            let finite = |v: Option<f32>| v.filter(|x| x.is_finite());
            if frame.detected_frequency.is_some_and(|v| !v.is_finite()) {
                eprintln!(
                    "[MAIN] Dropped non-finite frame scalar: f={:?}",
                    frame.detected_frequency
                );
            }
            self.display_data.last_frequency = finite(frame.detected_frequency);

            if let Some(idx) = frame.note_index {
                self.display_data.last_note_index = Some(idx);
            }

            // Against the note's tuning target, not the engine's
            // `cents_deviation`: that references the discovery template's
            // f_ET·√(1+B) and reads the treble flat.
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
                if self.display_data.smoothing_buffer.len() > SMOOTHING_HOPS {
                    self.display_data.smoothing_buffer.remove(0);
                }
            } else {
                self.display_data.smoothing_buffer.clear();
            }

            if frame.is_silence {
                self.display_data.last_frequency = None;
                self.display_data.last_note_index = None;
                self.display_data.last_cents = None;
                self.display_data.smoothing_buffer.clear();
                self.display_data.is_stale = false;
            } else if frame.note_index.is_none() {
                self.display_data.smoothing_buffer.clear();
                self.display_data.is_stale = true;
            } else {
                self.display_data.is_stale = false;
            }

            self.display_data.capture_state = frame.capture_state;
        }

        // ── Worker results ──
        // Collected first, so handling an output can borrow `self` mutably.
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

                    // Persisted at once: a full-compass pass is hours of
                    // work, and nothing else guarantees a write.
                    self.session.record(measurement);

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

                    // Bumped now, so a bundle queued behind this output
                    // in the same drain is dropped as stale.
                    self.mark_curve_dirty();
                    self.strobe_lock
                        .on_trusted_set_edit(TrustedSetEdit::Captured);

                    self.refresh_inspector();

                    if let TuningMode::Auto = self.display_data.tuning_mode {
                        eprintln!("[MAIN] Auto-mode rearming...");
                        let request = self.arm_request();
                        self.send_capture_command(CaptureCommand::Arm(request));
                    }
                }
                WorkerOutput::Curve(bundle) => {
                    self.display_data.curve_recomputing = false;
                    if bundle.generation == self.curve_generation {
                        self.curve_bundle = Some(*bundle);
                    }
                }
            }
        }

        self.pump_dump_dir();
        self.pump_curve_job();
        self.pump_capture_command();

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
                    pipeline::load_f32(&self.pipeline_handle.atomics.config.silence_threshold);
            } else if self.display_data.settings_data.transient.visible {
                let flux = self
                    .display_data
                    .last_frame
                    .as_ref()
                    .map(|f| f.nhwrsf)
                    .unwrap_or(0.0);
                let current_threshold =
                    pipeline::load_f32(&self.pipeline_handle.atomics.config.nhwrsf_threshold);

                calibration::process_transient_tick(
                    &mut self.display_data.settings_data.transient,
                    flux,
                    current_threshold,
                );

                self.display_data.settings_data.transient.current_threshold = current_threshold;
            } else if self.display_data.settings_data.sustain.visible {
                let sustain_stability = self
                    .display_data
                    .last_frame
                    .as_ref()
                    .map(|f| f.sustain_stability)
                    .unwrap_or(0.0);

                let current_threshold = pipeline::load_f32(
                    &self
                        .pipeline_handle
                        .atomics
                        .config
                        .sustain_stability_threshold,
                );

                calibration::process_sustain_tick(
                    &mut self.display_data.settings_data.sustain,
                    sustain_stability,
                );

                self.display_data.settings_data.sustain.current_threshold = current_threshold;
            }
        }

        // ── Silence calibration ──
        if self.display_data.settings_data.rms.is_calibrating() {
            let current_rms = self
                .display_data
                .last_frame
                .as_ref()
                .map(|f| f.rms_ema)
                .unwrap_or(0.0);
            if let Some(silence_val) = calibration::process_silence_tick(
                &mut self.display_data.settings_data.rms,
                current_rms,
                frame_pushed,
            ) {
                pipeline::store_f32(
                    &self.pipeline_handle.atomics.config.silence_threshold,
                    silence_val,
                );
                self.display_data.settings_data.rms.current_threshold = silence_val;

                eprintln!(
                    "[MAIN] Lock-Free Calibration complete. Threshold set to: {:.6}",
                    silence_val
                );
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        if self.display_data.settings_view_visible {
            settings_view::view(&self.display_data, self.curve_bundle.as_ref())
        } else {
            main_view::view(
                &self.display_data,
                Message::CaptureButtonClicked,
                self.curve_bundle.as_ref(),
            )
        }
    }

    /// Ticks every 16 ms (≈ 60 Hz) to refresh the live readouts, and turns a
    /// window close request into [`Message::Exit`] so shutdown joins the audio
    /// thread first.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            iced::time::every(std::time::Duration::from_millis(16)).map(|_| Message::Tick),
            iced::event::listen_with(|event, _status, _window_id| match event {
                iced::Event::Window(iced::window::Event::CloseRequested) => Some(Message::Exit),
                _ => None,
            }),
        ])
    }

    fn theme(&self) -> Theme {
        Theme::Dark
    }
}
