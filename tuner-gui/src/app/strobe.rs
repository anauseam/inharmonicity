//! # Strobe lock and display state
//!
//! [`TunerApp`]'s strobe logic: the lock that freezes which tuning curve the
//! strobe and the cent meter read, the reference set pushed to the DSP bank, and
//! the bank's telemetry mirrored into [`StrobeState`] and [`UnisonState`].

use tuner_core::{
    algorithms::curves,
    algorithms::peaks::MAX_UNISON_LINES,
    audio::HOP_RATE_HZ,
    models::{self, EngineChoice, InharmonicityProfile, NOTES, ReferenceMode},
    strobe::StrobeRefUpdate,
    strobe::unison::UnisonVerdict,
    worker::CurveBundle,
};

use crate::app::{TunerApp, TuningMode};
use crate::widgets::unison_display::UnisonRow;

/// Display state of the manual-mode strobe, mirrored from the DSP bank.
#[derive(Debug, Clone)]
pub struct StrobeState {
    /// The displayed partial's accumulated beat phase, in cycles in [0, 1);
    /// held while gated.
    pub beat_phase: f32,
    /// The displayed partial, n*.
    pub n_star: u8,
    /// The displayed partial's target, in Hz.
    pub ref_hz: Option<f32>,
    /// The bank's amplitude gate is shut, so the band is frozen.
    pub gated: bool,
    /// The fine readout: cents from the target, from the bank's beat rate.
    /// Aliases past `BAND_READABLE_HZ`; `None` while the bank's fit fills.
    pub band_cents: Option<f32>,
    /// The partial the coarse read is centred on; not necessarily
    /// [`Self::n_star`].
    pub coarse_n: u8,
    /// The band's offset is past `BAND_READABLE_HZ`, debounced over
    /// `READOUT_SWITCH_HOPS`.
    pub out_of_range: bool,
    /// Consecutive hops whose range verdict opposes [`Self::out_of_range`].
    range_run: u8,
    /// The wide-range readout: cents from the coarse partial's target, valid at
    /// any detuning; `None` on a hop that yields no read.
    ///
    /// A partial's cents offset from its target is the string's: for
    /// `fₙ = n·f₀·√(1+Bn²)` the offset is linear in f₀.
    pub coarse_cents: Option<f32>,
}

impl Default for StrobeState {
    fn default() -> Self {
        Self {
            beat_phase: 0.0,
            n_star: 1,
            ref_hz: None,
            gated: true,
            band_cents: None,
            coarse_n: 1,
            coarse_cents: None,
            out_of_range: false,
            range_run: 0,
        }
    }
}

/// Display state of the unison panel: the note's resolved strings, in cents
/// against each partial's own target.
#[derive(Debug, Clone, Default)]
pub struct UnisonState {
    /// One row per partial the bank targets, ascending.
    pub rows: Vec<UnisonRow>,
    /// Index into [`Self::rows`] of the displayed partial, if the bank targets it.
    pub displayed: Option<usize>,
    /// Every pair beat of the displayed partial's lines, widest first, in Hz:
    /// the rate a tuner counts by ear.
    pub beats_hz: Vec<f32>,
    pub verdict: UnisonVerdict,
    /// Half-width of the cents axis, a `UNISON_SPAN_LADDER` step; `0.0` before
    /// the first frame.
    pub span_cents: f32,
    /// Consecutive hops whose content would fit a narrower step.
    shrink_run: u8,
}

/// Cents half-widths the unison axis may take.
// Fixed steps, not a fit to the content: a marker that moved because the axis
// rescaled reads as a string that moved. ±3 ¢ is a unison being finished,
// ±100 ¢ one not yet started.
pub(crate) const UNISON_SPAN_LADDER: [f32; 4] = [3.0, 10.0, 30.0, 100.0];

/// Fraction of the axis the widest marker may reach before the axis steps up;
/// below 1 so no marker sits on the frame edge.
const UNISON_SPAN_HEADROOM: f32 = 0.8;

/// How far (Hz) the band's beat can be from its reference before the per-hop
/// unwrap folds, less a noise margin. A hop limit, not a Goertzel one.
pub(crate) const BAND_READABLE_HZ: f32 = BAND_ALIAS_HZ - BAND_UNWRAP_MARGIN_HZ;

/// Half a cycle of beat phase per hop, past which the per-hop unwrap folds.
const BAND_ALIAS_HZ: f32 = 0.5 * HOP_RATE_HZ;

/// Noise margin below [`BAND_ALIAS_HZ`]: one noisy hop folds the branch.
// The pooled p99.9 per-hop phase noise, f_hop·z·σ_d = 3.33 Hz. Pooled over a 16×
// register spread, so generous in the bass and thin in the treble, where the
// band folds on its own noise and the coarse read takes over.
// report 0011
const BAND_UNWRAP_MARGIN_HZ: f32 = 3.5;

/// Hops of unbroken opposing evidence before the readout changes source.
// A bare threshold flips source while a string sits at the boundary, wherever
// the boundary is put. 8 is the window-overlap length (8192/1024), the first
// independent verdict; the band's fill time (≈ 11 hops) is the ceiling. Flips fall
// 5.6 % → 0.4 % of hops, for 163 ms. Symmetric: asymmetric variants flip 2–3.5×
// as often at the boundary and need a second constant.
// report 0011
const READOUT_SWITCH_HOPS: u8 = 8;

/// Advances a debounced boolean by one hop: `state` adopts `verdict` after
/// [`READOUT_SWITCH_HOPS`] consecutive opposing hops, and an agreeing hop resets
/// the run.
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

/// Which curve the strobe and the cent meter read.
///
/// Whether a newer live curve exists is not stored: it is the live generation
/// compared with the locked one.
pub(super) enum StrobeLock {
    /// The strobe engages on the next curve-mode tick with a live bundle. Also
    /// the resting state in ET mode, which reads no curve.
    Disengaged,
    /// The frozen targets the strobe reads, until a re-lock. Boxed so
    /// `Disengaged` stays small.
    Engaged(Box<LockedTargets>),
}

/// Everything a strobe reference set is built from, frozen together.
pub(super) struct LockedTargets {
    bundle: CurveBundle,
    /// Per-key measured B at lock time, as [`InharmonicityProfile::active`]
    /// presents it.
    b_raw: [Option<f32>; 88],
}

impl LockedTargets {
    /// Freezes the live bundle together with the profile's current B per key.
    pub(super) fn freeze(bundle: &CurveBundle, profile: &InharmonicityProfile) -> Box<Self> {
        Box::new(Self {
            bundle: bundle.clone(),
            b_raw: snapshot_b(profile),
        })
    }
}

/// The B every key currently presents — [`InharmonicityProfile::active`], the
/// same entry the curve input, the inspector and the keyboard resolve to.
fn snapshot_b(profile: &InharmonicityProfile) -> [Option<f32>; 88] {
    std::array::from_fn(|k| profile.active(k as u8).and_then(|m| m.calculated_b))
}

impl StrobeLock {
    /// The frozen bundle, if engaged.
    fn engaged(&self) -> Option<&CurveBundle> {
        match self {
            StrobeLock::Engaged(l) => Some(&l.bundle),
            StrobeLock::Disengaged => None,
        }
    }

    /// The frozen measured B for `key`, if engaged.
    fn locked_b(&self, key: u8) -> Option<f32> {
        match self {
            StrobeLock::Engaged(l) => l.b_raw[key as usize],
            StrobeLock::Disengaged => None,
        }
    }

    /// The lock's response to a change in the trusted measurement set.
    pub(super) fn on_trusted_set_edit(&mut self, edit: TrustedSetEdit) {
        match edit {
            // The pass in progress is never moved under the tuner: the live
            // curve advances, and only a re-lock moves the targets.
            TrustedSetEdit::Captured | TrustedSetEdit::Undone => {}
            // The frozen curve may be another instrument's.
            TrustedSetEdit::Loaded => *self = StrobeLock::Disengaged,
        }
    }
}

/// A change to the trusted measurement set, which moves the live tuning curve.
pub(super) enum TrustedSetEdit {
    Captured,
    Undone,
    Loaded,
}

/// The curve lock as the strobe panel shows it, projected from the lock each tick.
#[derive(Debug, Clone, Copy)]
pub struct StrobeLockView {
    /// Generation of the locked bundle.
    pub generation: u64,
    /// The live curve has advanced past the lock.
    pub newer: bool,
}

impl TunerApp {
    /// Forces a strobe push on the next tick and drops the readouts taken
    /// against the old references.
    pub(super) fn reset_strobe(&mut self) {
        self.strobe_pushed = None;
        self.display_data.strobe = StrobeState::default();
        self.display_data.unison = UnisonState::default();
    }

    /// The target (Hz) the cent meter reads `key` against: the selected curve's
    /// `target_f1`, from the locked bundle as the strobe reads it, or the ET
    /// pitch in ET mode and before any curve exists.
    pub(super) fn meter_target_hz(&self, key: u8) -> f32 {
        let et = models::NOTES[key as usize].frequency;
        if self.display_data.reference_mode == ReferenceMode::Et {
            return et;
        }
        match self.strobe_lock.engaged().or(self.curve_bundle.as_ref()) {
            Some(bundle) => bundle
                .curve(self.display_data.selected_engine)
                .target_f1(key),
            None => et,
        }
    }

    /// Keeps the DSP strobe bank targeted and mirrors its telemetry into the
    /// display state. A reference set is pushed (crossing #4) when the key, the
    /// locked bundle or the engine changes; a full ring retries next tick.
    pub(super) fn update_strobe(&mut self, frame_pushed: bool) {
        let et_mode = self.display_data.reference_mode == ReferenceMode::Et;
        let manual_key = match &self.display_data.tuning_mode {
            TuningMode::Manual { key_index, .. } => Some(*key_index),
            _ => None,
        };

        // The lock engages the first time the strobe would target a curve.
        if !et_mode
            && manual_key.is_some()
            && matches!(self.strobe_lock, StrobeLock::Disengaged)
            && let Some(live) = &self.curve_bundle
        {
            self.strobe_lock =
                StrobeLock::Engaged(LockedTargets::freeze(live, self.session.profile()));
        }

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

        // The identity of what the bank should target; the bank is re-pushed only
        // when it changes. In ET mode the generation and engine are placeholders.
        // Every input to the reference set below must be reachable from it: one
        // read from live state instead of the lock would move `refs` without
        // re-pushing the bank.
        let desired: Option<(u8, bool, u64, EngineChoice)> = manual_key.and_then(|key| {
            if et_mode {
                Some((key, true, 0, EngineChoice::MultiBalanced))
            } else {
                self.strobe_lock
                    .engaged()
                    .map(|b| (key, false, b.generation, self.display_data.selected_engine))
            }
        });

        let mut refs = [0.0f32; 12];
        let mut count = 0usize;
        let mut spacing = 0.0f32;
        let (n_star, ref_hz) = match desired {
            // ET: the fundamental only, whose target holds for any B.
            Some((key, true, _, _)) => {
                let f_et = NOTES[key as usize].frequency;
                refs[0] = f_et;
                count = 1;
                spacing = f_et;
                (1u8, Some(f_et))
            }
            // Curve: an unmeasured key falls back to the Rigaud prior, not B = 0.
            // A harmonic reference at the coarse read's higher partial would
            // report the string's whole stretch as mistuning (≈ 24 ¢ at A0).
            // report 0011
            Some((key, false, _, engine)) => {
                let bundle = self
                    .strobe_lock
                    .engaged()
                    .expect("curve desired implies an engaged lock");
                let b_raw = self.strobe_lock.locked_b(key);
                // Without a measured B only the fundamental's target is exact.
                let n_star = match b_raw {
                    Some(_) => bundle.display_partials[key as usize],
                    None => 1,
                };
                let curve = bundle.curve(engine);
                let b = b_raw.unwrap_or_else(|| models::get_expected_beta(key));
                count = curve.strobe_partials(key, b, &mut refs);
                // The f₀ `strobe_partials` built the series from.
                spacing = curve.target_f1(key) / (1.0 + b).sqrt();
                let ref_hz = (n_star as usize <= count).then(|| refs[n_star as usize - 1]);
                (n_star, ref_hz)
            }
            None => (1, None),
        };

        let coarse_n = desired.map_or(1, |(key, _, _, _)| curves::coarse_read_partial(key));
        let coarse_ref_hz = ((coarse_n as usize) <= count).then(|| refs[coarse_n as usize - 1]);

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

        let bank = self
            .display_data
            .last_frame
            .as_ref()
            .filter(|f| desired.is_some() && (n_star as usize) <= f.strobe_count)
            .map(|f| {
                let i = n_star as usize - 1;
                (f.strobe_angle[i], f.strobe_gated[i], f.strobe_beat_hz[i])
            });

        let strobe = &mut self.display_data.strobe;
        // Frames computed before the bank took new references carry the old
        // key's numbers.
        if strobe.ref_hz != ref_hz || strobe.n_star != n_star {
            strobe.out_of_range = false;
            strobe.range_run = 0;
            strobe.band_cents = None;
        }
        strobe.n_star = n_star;
        strobe.ref_hz = ref_hz;

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

        // Debounced over new frames only: the triple buffer redelivers the last
        // one. The coarse cents hold at any partial, so the band's offset at r is
        // r·(2^(¢/1200) − 1). With no coarse read, the band stands.
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
            Some((angle, gated, beat_hz)) => {
                strobe.beat_phase = angle;
                strobe.gated = gated;
                if self.strobe_pushed == Some(desired) {
                    strobe.band_cents = beat_hz
                        .zip(ref_hz.filter(|r| *r > 0.0))
                        .map(|(hz, r)| 1200.0 * ((r + hz) / r).log2());
                }
            }
            // The bank is not targeting this key yet: hold the last angle, gated.
            None => strobe.gated = true,
        }

        self.update_unison(&refs, count, n_star, desired, frame_pushed);
    }

    /// Mirrors the bank's resolved lines into the unison panel, in cents against
    /// each row's own reference. Every row is then on the string's scale, so a
    /// unison's markers line up across rows and a false beat's do not.
    fn update_unison(
        &mut self,
        refs: &[f32; 12],
        count: usize,
        n_star: u8,
        desired: Option<(u8, bool, u64, EngineChoice)>,
        frame_pushed: bool,
    ) {
        let stale = desired.is_none() || self.strobe_pushed != Some(desired);
        let Some(frame) = self.display_data.last_frame.as_ref().filter(|_| !stale) else {
            self.display_data.unison = UnisonState::default();
            return;
        };

        let held = &self.display_data.unison;
        let mut state = UnisonState {
            verdict: frame.unison_verdict,
            span_cents: held.span_cents,
            shrink_run: held.shrink_run,
            ..UnisonState::default()
        };
        // A row per targeted reference, resolved or not, so nothing reflows
        // while tuning.
        let live = count.min(frame.strobe_count).min(refs.len());
        // Levels are relative to the loudest row: the bank's amplitudes have no
        // absolute meaning.
        let loudest = frame.strobe_amplitude[..live]
            .iter()
            .copied()
            .fold(0.0f32, f32::max);
        let mut widest = 0.0f32;
        for (i, &f_ref) in refs.iter().enumerate().take(live) {
            if f_ref <= 0.0 {
                continue;
            }
            let to_cents = |hz: f32| 1200.0 * (1.0 + hz / f_ref).log2();
            let lines = (frame.unison_line_count[i] as usize).min(MAX_UNISON_LINES);
            let mut row = UnisonRow {
                partial: i as u8 + 1,
                count: lines as u8,
                resolution_cents: to_cents(frame.unison_resolution_hz[i]),
                resolution_hz: frame.unison_resolution_hz[i],
                ref_hz: f_ref,
                level: if loudest > 0.0 {
                    frame.strobe_amplitude[i] / loudest
                } else {
                    0.0
                },
                gated: frame.strobe_gated[i],
                ..UnisonRow::default()
            };
            for (line, slot) in frame.unison_lines[i][..lines].iter().enumerate() {
                row.cents[line] = to_cents(slot.offset_hz);
                row.amplitude[line] = slot.relative_amplitude;
                widest = widest.max(row.cents[line].abs());
            }
            widest = widest.max(row.resolution_cents / 2.0);
            if i + 1 == n_star as usize {
                state.displayed = Some(state.rows.len());
                let offsets = &frame.unison_lines[i][..lines];
                for (a, first) in offsets.iter().enumerate() {
                    for second in &offsets[a + 1..] {
                        state
                            .beats_hz
                            .push((second.offset_hz - first.offset_hz).abs());
                    }
                }
                state.beats_hz.sort_by(|a, b| b.total_cmp(a));
            }
            state.rows.push(row);
        }
        state.span_cents = Self::unison_span(
            state.span_cents,
            &mut state.shrink_run,
            widest,
            frame_pushed,
        );
        self.display_data.unison = state;
    }

    /// Picks this hop's axis step from [`UNISON_SPAN_LADDER`]. It grows at once,
    /// so no marker is hidden, and shrinks only after [`READOUT_SWITCH_HOPS`]
    /// hops that fit, so the axis does not flicker while a marker sits at a step.
    fn unison_span(held: f32, shrink_run: &mut u8, widest: f32, frame_pushed: bool) -> f32 {
        let needed = *UNISON_SPAN_LADDER
            .iter()
            .find(|step| widest <= **step * UNISON_SPAN_HEADROOM)
            .unwrap_or(UNISON_SPAN_LADDER.last().expect("ladder is not empty"));
        if held <= 0.0 || needed > held {
            *shrink_run = 0;
            return needed;
        }
        if needed == held || !frame_pushed {
            *shrink_run = 0;
            return held;
        }
        *shrink_run += 1;
        if *shrink_run >= READOUT_SWITCH_HOPS {
            *shrink_run = 0;
            needed
        } else {
            held
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The frozen B must be the entry the key presents — the newest trusted
    /// one — so locking cannot retarget a key onto a capture the curve excluded.
    /// A key with nothing trusted carries no B and falls back to the prior.
    #[test]
    fn locked_b_snapshots_the_active_entry() {
        let mut profile = InharmonicityProfile::new("test");
        let entry = |key, b, auto| models::KeyMeasurement {
            key_index: key,
            measured_f0: 100.0,
            partials: Vec::new(),
            calculated_b: Some(b),
            last_captured: String::new(),
            captured_in_auto: auto,
            sounding_strings: None,
        };
        let solo = |key, b| models::KeyMeasurement {
            sounding_strings: Some(models::SoundingStrings::UNDECLARED.toggled(1)),
            ..entry(key, b, false)
        };
        profile.record(entry(3, 1e-4, false));
        profile.record(entry(3, 9e-4, true)); // newer, but unattended
        profile.record(entry(4, 2e-4, true)); // auto only
        profile.record(solo(6, 3e-4)); // isolation only

        let b = snapshot_b(&profile);
        assert_eq!(b[3], Some(1e-4), "a newer auto entry must not displace it");
        assert_eq!(b[4], None, "an auto-only key falls back to the prior");
        assert_eq!(b[5], None, "unmeasured keys carry no B");
        assert_eq!(b[6], None, "a solo measured one string, not the note");
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

    /// The axis must never hide a marker, and must never flicker between two
    /// steps while one sits at a boundary: grow on the spot, shrink only after a
    /// full run of hops that would fit.
    #[test]
    fn unison_axis_grows_at_once_and_shrinks_slowly() {
        let mut run = 0u8;

        // Cold start takes the smallest step that fits.
        let span = TunerApp::unison_span(0.0, &mut run, 1.0, true);
        assert_eq!(span, 3.0);

        // A marker past the headroom of the current step grows it immediately —
        // one hop, no run, because content off the frame is not readable.
        let span = TunerApp::unison_span(span, &mut run, 2.9, true);
        assert_eq!(span, 10.0, "2.9 ¢ is past 80 % of the ±3 ¢ step");
        let span = TunerApp::unison_span(span, &mut run, 40.0, true);
        assert_eq!(span, 100.0, "growth skips straight to a step that fits");

        // Shrinking waits out the full run, and any hop that does not fit
        // resets it.
        for _ in 0..READOUT_SWITCH_HOPS - 1 {
            assert_eq!(TunerApp::unison_span(100.0, &mut run, 1.0, true), 100.0);
        }
        assert_eq!(TunerApp::unison_span(100.0, &mut run, 40.0, true), 100.0);
        assert_eq!(run, 0, "a hop needing the wide step resets the run");
        for _ in 0..READOUT_SWITCH_HOPS - 1 {
            assert_eq!(TunerApp::unison_span(100.0, &mut run, 1.0, true), 100.0);
        }
        assert_eq!(
            TunerApp::unison_span(100.0, &mut run, 1.0, true),
            3.0,
            "the full run shrinks"
        );

        // Idle ticks carry no evidence: the triple buffer redelivers the last
        // frame, and counting it would let the run advance without new data.
        let mut idle = 0u8;
        for _ in 0..READOUT_SWITCH_HOPS * 2 {
            assert_eq!(TunerApp::unison_span(100.0, &mut idle, 1.0, false), 100.0);
        }
    }
}
