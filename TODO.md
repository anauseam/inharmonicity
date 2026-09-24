# TODO

The project backlog. Each entry says **what is not done**, what it is blocked
on, and **where the argument lives** — it does not restate the argument. A
decision is recorded in `ARCHITECTURE.md`'s Decisions or its report; this file only tracks that it is
open.

**Status vocabulary:** `Planned` · `Deferred` (wanted, not scheduled) ·
`Gated on X` (blocked on a specific input) · `Investigating` (outcome unknown) ·
`Built, gated off` (shipped behind a flag).

## Sequencing

Most of this file has no ordering. These steps do, agreed 2026-08-01 and
extended 2026-08-07 and 2026-09-15:

0. **Capture the as-found unisons before the piano is tuned** — the only
   irreversible item in this file. Tuning destroys that state; the set state can
   be recreated by tuning again. It doubles as the bass-attribution mute test.
   The capture-metadata mechanism it was waiting on is **built** (which strings
   sounded, declared per capture and changeable while armed —
   `docs/internals/capture-sets.md`), so
   nothing blocks the session: isolation replaces resolution, and the longer
   records the protocol sketched are wanted only for per-string decay τ.
   → report 0012 Limitations, [report 0013](reports/0013-bass-extra-lines-attribution.md) D3

   The session protocol, moved here from report 0014 §9 so the plan lives with
   the backlog rather than inside the evidence:

   - **An as-found isolation pass on piano #1**, no detuning required. C1 needs
     unisons wider than the panel's floor and piano #2's were already well set;
     piano #1 has never been isolation-recorded and sits further from equal
     temperament (median +4.4 ¢ against −0.8 ¢). Solos then open, ~4 strikes
     each, on eight or so keys across the compass. If its unisons are also
     tight, that is itself the answer and the feature's case is weaker.
   - **Long open captures above C6** — the cheapest item. Six notes, open only,
     5 s each: E6, A6, C#7, F7, A7, C8. Every capture above C6 today is
     0.37–0.56 s and the panel needs 0.58 s; whether that is the note or our
     decay stop is untestable without these six strikes.
   - **Repeat C6 and add a neighbour.** C6's strings scatter 16.5 / 6.3 / 4.2 ¢
     across repeats against 0.11–0.45 ¢ at C5 — nearly as wide as the unison
     being measured. Take ~8 strikes per string, and record A5 or C#6 as an
     isolation set so the register has one reliable key.
   - **If a bass unison is ever found genuinely out, record it.** Not a request
     to create one: every bass key sampled was within 4 ¢, so nothing is known
     about the panel's bass behaviour on a genuinely spread unison.

   Piano #2 is not to be deliberately detuned (decided 2026-08-20): it is an
   instrument in use whose only goal is to be properly tuned.
1. **The noise floor** — the next major development once the 2026-09 refactor
   lands (*No noise floor*, under Engine and discovery).
2. **The structural work** — *Structural work*, below: the module-boundary
   pass, the naming fixes and the item-order pass.
3. Everything else, unordered.

## When an in-tune instrument is captured

None exists yet. Every capture set is of a detuned piano — the regime the app
targets, and the reason the items below have no reference to test against.
They open together, so plan one session for them rather than one each.

- **Which tuning curve to offer.** No computation on detuned data can choose
  between the curve families, which is why engines (b) and (c) are withheld;
  an **aurally** tuned instrument is the missing ground truth. Record how it was tuned — a tuning set with an
  electronic tuner reproduces that tuner's curve, and scoring against it is
  circular. → [`reports/tuning-curve-grounding.md`](reports/tuning-curve-grounding.md)
- **MAT's serial order**, and with it the `Simultaneous` fallback and the
  paper's tighter §2.4 peak band (`mat.rs`). The deletion rides with the
  `(f₀, B)` seam work, which injects `MatOrder` as a parameter.
- **Measured-`B` discovery seeding** (`pipeline::APPLY_MEASURED_B_TO_DISCOVERY`),
  and behind it the TWM bass-lock bias, which is a wrong-template problem.
- **Report 0014 §7's prediction**, which needs solo captures on the tuned
  instrument: in the plain-wire treble a unison's per-string `B` spread is
  mostly tension and should largely vanish once the unison is tuned; in the
  wound bass it belongs to the wire and should not.

Session shape: full compass at ≥ 5 repeats per key as on piano #2, analysed
through the shipped 1.5 s window so every number stays comparable, plus
isolation sets on a few bass and treble unisons. If the instrument is piano #1
or #2 after tuning, its as-found state is captured first (step 0) — piano #1
has no isolation pass yet. `capture-sets.md` gains a set row either way; the
instrument count in `CONTRIBUTING.md` rises only if it is a third piano.

## Structural work

Sequencing step 2, after the 2026-09 refactor's commit. Each entry moves code
between files or renames a module, and none adds behaviour, so each is checked
as a pure move: the build and tests, plus the sorted non-blank lines of the
files it touches, taken together, staying identical. That check needs a
committed baseline to diff against, which is why the step waits for the commit.
The boundary pass goes first, since it carries the open arguments and reorders
its files anyway; the item-order pass goes last. The
`unreachable_pub` / `missing_docs` sweep (*Pipeline and architecture*) is not a
move, so it runs after the step as its own change.

- **Module-boundary pass across `worker.rs`, `audio.rs` and `models.rs`** —
  `Deferred`, and one item rather than three because they would otherwise
  reshuffle each other. Each file now mixes categories that want separating, and
  the ordering *within* each file has drifted too — DSP, message types and file
  I/O interleaved rather than grouped:
  - **`worker.rs`** holds four concerns: the message types
    (`CurveJob`/`CurveBundle`/`WorkerOutput`), the threading
    (`WorkerManager`), the heavy DSP (`process_payload`), and disk I/O
    (`write_diagnostics`, `dump_dir_name`). The likeliest split is
    message-types out and I/O out; whether the I/O leaf is a shared
    `diagnostics` module used across `tuner-core`, or stays worker-local, is
    the open question — a first attempt at a standalone module was rejected as
    a broad domain name owning four lines of logic.
  - **`audio.rs`** holds three: the CPAL stream, the cross-cutting DSP
    constants, and the thread host. Splitting retires the accepted
    `models ↔ audio` cycle.
  - **`models.rs`** becomes a directory when the single file stops feeling
    right; it now carries the persisted profile schema as well as the note
    tables and the discovery templates.

  **Three arguments are open and should be settled by this pass, not before:**
  - *Should worker construction leave `AudioPipeline::new`?* It spawns the
    Worker thread as a side effect, which is why the dump root traverses a
    constructor that never reads it. **Against separating it:** the whole point
    of `spawn_analysis_thread` is that a frontend calls one turnkey function
    and gets a running system — pushing worker lifecycle onto every consumer
    costs exactly the ergonomics the host extension exists to provide.
    **For:** host policy would then be supplied where the worker is built,
    instead of threaded through two layers that ignore it.
  - *Should the host-assembly entry take a `HostConfig { source, dump_dir }`?*
    Today it is two positional arguments and `spawn_analysis_thread(src, None)`
    does not say what `None` means. Deferred so the public shape changes once,
    with the split, rather than twice.
  - *Does `RuntimeAtomics` stay?* The pipeline writes both its values every hop,
    and only `PipelineHandle`'s `Debug` reads either: the GUI takes the same RMS
    and NHWRSF from `FrameOutput`. `thread-crossings.md` keeps the struct for
    observations several consumers poll, and none polls it. Removing it changes
    the public handle, so it waits for the same pass.

  A deliberate whole-codebase pass, not a re-export shim; zero behaviour
  change; its own commit.
  → [`docs/internals/layering.md`](docs/internals/layering.md),
  [`docs/internals/style.md`](docs/internals/style.md)
- **Three names break `style.md`'s naming rule** — `Investigating`. "Engine" is
  overloaded: `engine.rs` is the F0 detector, and the tuning curve also has
  "engines (a)–(d)"; one of them needs renaming. `algorithms::spectral` is a
  broad domain word over five methods (the windowed FFT and its magnitudes,
  CSPE, Jacobsen–Candan, Goertzel, the Neyman–Pearson factor), so naming it for
  what it owns means splitting it, which moves paths the lab, tests and benches
  import. And `cola.rs` is named for a property the code does not use: COLA,
  constant overlap-add, is the condition for overlap-*add* resynthesis, and the
  pipeline only ever reads two overlapping analysis windows from a FIFO — its
  module doc calls them "(constant overlap-add) analysis windows", which is the
  misuse in one phrase. The file holds one struct with one caller,
  `AudioPipeline`, so the choice is between naming it for what it is (a sample
  FIFO the windows are read from) and folding it into `pipeline.rs`; either way
  `process_cola_hop` loses the word, and that name is cited from
  `ARCHITECTURE.md`, `hot-path.md`, `thread-crossings.md`, three lab files and
  `CLAUDE.md`, so the rename is mechanical but wide. All three wait for the item-order pass.
- **One file per main-screen panel** — `Planned`, before the owned-parts split.
  `main_view.rs` (1,250 lines) builds every panel on the main screen itself, so
  its imports are the sum of theirs: the unison panels (≈ 280 lines), the sidebar
  with its capture controls (≈ 250), the strobe panel (≈ 180), and the small
  curve-plot, cent-meter, keyboard and spectrum panels. `settings_view` already
  gives each of its panels a file; moving these the same way, as pure moves,
  leaves `main_view` the screen's layout. A trait over the panels was considered
  and rejected: they are not interchangeable (each has one fixed place and its
  own inputs), and iced deprecated its own `Component` trait in 0.13 for the
  Elm-architecture composition the owned-parts split uses.
- **Item order within files** — `Planned`, last in this step.
  Constants, types and functions sit where they were added, so files do not read
  top-down, and a type's methods are not grouped by concern. Write the order into
  `style.md` first. rust-analyzer's style guide ("Order of Items") is a model:
  public items first, then types in top-down order, then functions and impls.
  Two things it leaves open must be decided in the same rule: whether an `impl`
  follows its own type (as `calibration.rs` does) or every `impl` follows every
  type (as rust-analyzer's guide reads), and whether tests, already last by
  clippy's `items_after_test_module`, are the only exception. No official Rust
  document rules on item order: the Style Guide covers formatting, which
  rustfmt enforces, and the API Guidelines cover naming, traits and docs, which
  `style.md` already cites. clippy's `arbitrary_source_item_ordering` can
  enforce the order of item kinds, since its alphabetical sorting within a kind
  can be switched off, but not the order within a kind, which stays a review
  rule. Then reorder file by file as pure moves, checked by each file's sorted
  non-blank lines staying identical, plus the build and tests. `worker.rs`,
  `audio.rs` and `models.rs` wait for their split, which reorders them anyway.
- **Reconcile `peaks`' split test suite** — `Deferred`. The placement rule is
  now written down ([`style.md`](docs/internals/style.md), "Where tests live"),
  and `peaks` is the one module with tests in both locations. Low stakes: both
  halves pass and each sits on the correct side of the visibility line, so this
  is tidying, not a defect.
- **The crate split** — `Deferred`, unscheduled, and after the step. The
  algorithms become a library of their own (the operator's working name is
  `inharmonicity`) and the GUI separates formally, along the line the
  `tuner-gui/src/` map in `ARCHITECTURE.md` already draws; `tuner-lab` stays a
  workspace member. Until then a crate name stays provisional and cheap to
  rename: reproduction lines cite the `cargo lab` alias, not the package.

  **Where the boundary falls is open, and it is not `algorithms/` as it
  stands.** Four files there import `crate::models` — `KeyProfile` and
  `SpectralPeak` (`twm`, `discovery`), `SpectralPeak` and `UnisonLine`
  (`peaks`), the note tables and curve types (`curves`). So either those domain
  types travel with the algorithms, or the seam carries them as parameters.
  Decide that before anything moves.

  **The leaf modules' nesting waits for the same decision.** Whether the cited
  leaves sit under their composer (`curves/{rigaud,giordano,whittaker}.rs`) or
  stay flat beside it is a question about the library's public shape, not about
  tidiness: `whittaker` is used only by `curves` today, but its smoother and
  its banded solver are general by construction and a library user can want
  them alone. The flat layout states one cited method per file, with the
  composer named beside them. `rigaud` holds two things, and the plan is to
  split it along them: the model (the `B_ξ` and `ρ_φ` curves, their evaluation
  and inversion, which the curve engines read) moves into `models`, which
  already evaluates `B_ξ` in `f32` for the discovery prior; the fits (Eqs. 29
  and 31, whose L1 objectives are the paper's but whose grid-search optimizers
  are ours and replaceable by any other technique) stay with the curve engines.
  → [`ARCHITECTURE.md`](ARCHITECTURE.md)

---

## User-facing

- **Unison assist: the discriminator is near-silent, and the bass second lines
  are unattributed** — `Gated on longer captures` / `Gated on a mute-isolation
  capture set`. The test that separates a real unison from one string beating
  with itself returns `Undetermined` on 70–87 % of tenor captures, because a fit
  over three neighbouring partials has almost no lever arm in frequency; the fix
  is more resolved partials, i.e. longer recordings, not a looser test.
  Separately, keys 0–27 produce a second line on essentially every capture of
  both instruments — including single-strung keys. Those lines are real and are
  measurably not a second string; every remaining candidate needs a capture with
  one string muted, which no set has. ≈10 % of *third* lines remain
  unattributed.
  *(Partly discharged 2026-08-20 by [report 0014](reports/0014-unison-panel-against-isolation-truth.md):
  the mute-isolation set exists and supplied the positive control — a single
  string resolves two lines on 65 % of captures — and the near-silence now has a
  second measured cause, the real 1.7–8.5 % per-string B spread. What remains is
  the accuracy criterion, which that set cannot score: the piano's as-found
  unisons are mostly tighter than the panel's own floor, so it is gated on the
  post-tuning detuning ladder.)*
  → [report 0012](reports/0012-unison-line-estimator.md) §§5–6, §8,
  [report 0013](reports/0013-bass-extra-lines-attribution.md),
  [report 0014](reports/0014-unison-panel-against-isolation-truth.md)
- **The README screenshot is stale** — `Planned`, before the first release. It
  predates the instrument library, Curve Select and the measurement inspector:
  its sidebar still shows *Spectrogram* and *Load Profile*, both of which are
  gone. Retake it once the window layout settles.
- **Profile export / import from an arbitrary path** — `Deferred`. The library
  browser covers new / open / resume / duplicate / delete over the profiles
  directory with no new dependency; sending a profile to a colleague, or opening
  one they sent, needs a native file dialog (`rfd`), which is a real system
  dependency on Linux. The field's answer is cloud sync or share-sheet export
  — decide when the need is real.
- **Capture-dump retention, and a user-settable dump location** — `Planned`.
  *(Half done 2026-08-15: dumps now follow the open instrument —
  `diagnostics/<identity.id>/` via `WorkerJob::SetDumpDir`, with an
  `instrument.json` manifest beside them. What remains is the retention policy
  and letting the operator choose the root.)*
  Every capture writes raw audio to `data_local_dir()/diagnostics/` and nothing
  ever prunes it, so a working tuner's disk use grows without bound. Needs a
  policy (age or total size) and somewhere to show it, and it is now the *only*
  thing bounding disk: undo deletes the dump of the capture it reverts, but the
  inspector's drop deliberately keeps it, since a distrusted measurement may
  still have good audio behind it. Making the *location* settable needs no new
  crossing: the dump root is Worker state, and `WorkerJob` (crossing #6) was
  built so a new kind of request is a new variant — `WorkerJob::SetDumpDir`
  carries a `PathBuf` fine, since that channel is crossbeam rather than a
  wait-free ring. Left unbuilt only because no UI changes it yet.
- **Pre-built binaries** — `Planned`. The tuner should be usable without a Rust
  toolchain now that the measure → curve → strobe path is complete.
- **Reference pitch other than A440** — `Deferred`. `TuningCurve.d_g` already
  carries the offset; only the UI is missing. Settings shows its entry,
  *Tuning Standard*, disabled.
- **Temperament selection** — `Deferred`. Equal temperament only today. Settings
  shows its entry, *Temperament*, disabled.
- **User adjustment of the curve** — `Deferred`, unscoped. Settings shows an
  *Inharmonic curve adjustment* entry, disabled; what it should change has never
  been decided. Choosing among the engines is built (*Curve Select*), and the (c)
  ρ presets are their own entry below.
- **Sample buffer size** — `Deferred`, unscoped. Settings shows a *Sample Buffer
  Adjustment* entry, disabled; which buffer it would size has never been decided.
  The capture length is already adjustable (*Capture Duration*), and the analysis
  windows are fixed by the 44.1 kHz design.
- **Pitch raise (over-pull)** — `Deferred`. Detection and display already work
  through a raise (the coarse readout is lock-independent and has no ±21.5 Hz
  limit); what is missing is over-pull *targets* — a deficit-measurement pass and
  a model of how a raise redistributes tension.
- **Show-all partials strobe mode** — `Deferred`. The v1 strobe shows one
  Smart-Partials-selected band per key; a band-per-partial toggle is planned but
  unbuilt (`strobe_partials` already emits every partial). Two costs kept it out
  of v1: high-treble bands would carry raw-B target uncertainty, and deep-bass
  low-partial bands would sit frozen behind the amplitude gate.
- **Per-note partial override** — `Deferred`. TuneLab's escape hatch for a
  dead auto-chosen partial: an on-the-fly, non-persisted per-note override of
  the displayed partial. Pairs with show-all.
- **Cents-normalized strobe rotation** — `Deferred`, and conditional. The band
  rotates at the physical beat rate. If a real tuning session shows Hz
  rotation is unreadable across registers *despite* Smart-Partials selection and
  the coarse readout, add an optional per-band angle rescale — cheap, no new
  crossing. The trigger is explicitly a use-testing outcome, not a preference.
- **The cent meter's smoothing is set by eye, and its stale needle never draws**
  — `Deferred`. The meter averages its per-hop readings over `SMOOTHING_SECS`
  (116 ms: ≈ 58 ms of lag for ≈ 2.2× less white noise), a span chosen by eye.
  The strobe's band-slope read is the sharper instrument, a line fitted to
  accumulated phase that reads ≈ 0.05 ¢ on a clean signal, but it is per-partial
  and manual-only, so the meter cannot use it without being rebuilt around
  phase. The meter is kept because it is Auto mode's only deviation readout; if
  Auto mode ever strobes, it goes instead.
  → [report 0011](reports/0011-coarse-spectral-readout.md)

  The grey needle `cent_meter` documents for a stale reading never draws: a stale
  frame has no note, and a frame with no note carries no frequency, so there are
  no cents to grey and the needle vanishes instead. Holding the last reading grey
  would be new behaviour; otherwise the branch goes.
- **Strobe gate flicker** — `Deferred`, an observation to make at the instrument.
  A band freezes and dims when its partial drops below the amplitude gate;
  whether it flickers at the threshold, and so needs hysteresis, was left to be
  judged by eye once the strobe was in use.
- **Engine (c) ρ Low/High presets** — `Deferred`. Computing three (c) presets
  naively re-runs the ~1.3 s Giordano scan three times; the calibration has to be
  factored out of the per-preset path first. Rendered as greyed placeholders.
- **"Advanced" mode** — `Deferred`. Curve selection, comparison metrics, and the
  offline diagnostics are all currently visible to every user with no separation
  between the ordinary tuning path and the research surface. See
  *Curve comparison metrics* below.
- **Curve auralization playback** — `Deferred`. `tuner_core::synth` renders a
  curve to audio offline today (`cargo lab curve auralize` writes WAVs). A live
  "hear the curve" control needs an audio **output** stream, which is a
  sanctioned seventh crossing living in `tuner_core::audio`, never in the GUI.
  → [`ARCHITECTURE.md`](ARCHITECTURE.md)
- **Capture importer** — `Deferred`, and a **`tuner-lab` mode, not a GUI
  panel** (developer tooling; decided 2026-08-02). Reads capture dumps and
  merges measurements back into a profile — the recovery path for an inspector
  drop, which deliberately keeps the audio. It must **re-run the analysis on
  `audio.raw`**, never import the dump's cached `analysis.json`: that cache is
  exactly what was wrong about piano #2's deep bass while its audio was fine
  (`capture-sets.md`). `cargo lab mat regen` already does the
  re-analysis half.

## Engine and discovery

- **TWM bass-lock bias (deep-bass stable-wrong core)** — `Gated on` an
  in-tune instrument, behind measured-$B$ seeding (see *When an in-tune
  instrument is captured*). The decision-level half is solved (M-of-N acquisition lock,
  validated on two instruments); the residual is a wrong-$B$-template problem in
  the bass that scoring constants cannot fix.
  → [report 0006](reports/0006-discovery-refinement-validation.md),
  [report 0010](reports/0010-m-of-n-lock-rule-replay.md)
- **Measured-$B$ discovery seeding** — `Built, gated off`
  (`pipeline::APPLY_MEASURED_B_TO_DISCOVERY`). Flip the flag once an in-tune
  instrument validates the measured values. Before flipping it, delete
  `AudioPipeline::new`'s read of `models::PROFILE_PATH`: it loads the pre-library
  `tuning_profile.json` from the working directory, a file location `layering.md`
  keeps out of `tuner-core`, and startup's `ProfileSender::update_all` already
  seeds the open instrument. The constant then belongs in `library.rs`, whose
  legacy import is its one other user.
  → [report 0006](reports/0006-discovery-refinement-validation.md)
- **Lock-release / re-lock hysteresis** — `Deferred`. The M-of-N rule covers
  acquisition only. → [report 0010](reports/0010-m-of-n-lock-rule-replay.md)
- **Silence closes the gate's own senses** — `Investigating`. Under
  `silence_threshold` the gate returns early: it zeroes the sustain and NHWRSF
  metrics, clears `transient_active` and the stable counter, and never reaches
  the spectral flux, so `prev_spectrum` keeps whatever the last audible hop left
  in it. Three consequences, none yet measured. The first audible hop after any
  quiet stretch is compared against a stale spectrum, so its flux is meaningless
  — it usually clears the threshold anyway, since a strike is loud. The metrics
  read zero in silence rather than reading the room, so nothing observes the
  ambient level the gates are supposed to be set against, which is the same gap
  the noise-floor entry above describes from the other side. And the sustain EMA
  restarts from zero at every note rather than from where the room sits. Keep
  the silence test ahead of the onset test whatever changes: a room louder than
  the threshold must not be able to report a strike. Moving any of this moves
  every lock baseline, so it needs a `gatekeeper dump` replay over all three
  capture sets first.
- **The transient is not the wait** — `Investigating`, unreported. In a
  `gatekeeper dump` over all three capture sets, stability lands exactly four
  hops after the last onset hop in 1,055 of 1,067 captures — that is the
  `stable_counter` requirement and nothing else. Exploratory work separately
  found the onset line-dominated from its first frame, the broadband component
  40–55 dB under the tone. If both hold, the gate's transient handling is not
  what delays a measurement; the analysis window filling is. Whether the
  four-hop wait buys anything is then a replay question, and both numbers need a
  report before anything moves on them.
- **The gate's onset and bypass flags are one signal** — `Investigating`.
  `Gatekeeper::process_transient_detection` raises `is_new_onset` on exactly the
  frames it returns as the bypass, so `GateResult`'s two flags are always equal,
  and `Engine::process`'s `is_new_onset` branch never runs, because the bypass
  returns first. Either they were meant to differ or one goes; removing one
  changes `Engine::process`'s signature and the lab harnesses that pass both.
- **No noise floor; the silence threshold stands in for one** — `Planned`.
  Nothing measures the room's noise. RMS calibration sets the
  silence threshold to the loudest ambient RMS it reads × 1.5, and three
  detectors use that one broadband number as their noise σ: discovery's peak
  floor, which every lock baseline rests on; the tracker's per-partial gate,
  which seeds MAT; and the strobe bank's per-reference gate. As a σ it is right
  nowhere, about 4× too low in the bass and 16–20× too high in the top octave. A
  per-bin floor recorded at startup is ruled out (it fails its false-alarm
  guard), and a local-reference (CFAR) gate, already shipping in the coarse read,
  has no reference cells in the bass, so a replacement must answer the bass
  separately. Replacing the gates moves every lock baseline. The same work
  retires the `noise_floor` name (the capture key, `engine.noise_floor`, the
  strobe's argument, and in the GUI `NoiseFloorSettings`,
  `DEFAULT_NOISE_MULTIPLIER`, `ToggleNoiseFloorAdjustment` and
  `RecalibrateNoiseFloor`) and the lab's fixed values in place of
  the silence threshold: 0.001 in `strobe truth`, `strobe readout` and the
  `gates ab` ambient row, 0.005 in `gates pfa`, 0.003 in `strobe isolation`.
  → [report 0015](reports/retiring/0015-ambient-sigma-gates-measured.md)

## Worker and measurement

- **`key_index` is not a sufficient measurement identity on every instrument** —
  `Deferred`, and only matters once a non-piano workflow exists. A fretted note
  is producible on several strings of different gauge and speaking length, hence
  different $B$; a piano note is 1–3 strings. A per-string/course discriminator
  on `KeyMeasurement` is the obvious shape, and costs nothing to add later — an
  additive `#[serde(default)]` field needs no migration, which is why one was
  *not* reserved speculatively.

- **Per-key measured σ_m instead of the σ_lnB(n) model** — `Investigating`, and
  the most promising use of repeat captures. The curve's shrinkage weight
  w = σ_p²/(σ_p² + σ_m²) takes σ_m from a *model* of partial count,
  σ_lnB(n) = max(19.3·n⁻³, 0.0035). Measured against the repeats themselves, an
  individual key's true σ departs from that model by up to 3.5× (bass), 7.9×
  (mid) and 23× (treble), and every one of those errors goes straight into how
  much the curve trusts that key. With k repeats σ_m is directly measurable.
  **This, not averaging, is where more captures pay:** averaging k B-values
  gains ~0.05 ¢ of target movement in the bass and cannot beat shrinkage in the
  treble, whereas a mis-weighted key is a systematic error. Needs a shrinkage
  estimator for small k (k = 2–3 gives a very noisy sample SD — pooled or
  hierarchical, not the raw SD) and validation on both instruments.
  → [report 0009](reports/0009-repeat-capture-noise-decomposition.md)
- **What replaces an excluded measurement, and the exclusion on partial
  coverage** — `Investigating`. When the curve's negative-stretch check excludes
  a key's measured B, three things follow that nobody has examined:
  - **The curve falls back to the model's `B_ξ`, not to the key's earlier
    trusted capture.** An earlier capture is better evidence: B moves little with
    tuning (B ∝ 1/tension). But `CurveInput` carries only the newest trusted
    capture per key, so the curve cannot see older ones.
  - **The strobe still builds that key's partial targets from the excluded B.**
    `strobe_partials` reads the active entry, so an excluded key's displayed
    partial follows the measurement the curve rejected. The key carries the
    red ✗, but its strobe is off by the bad B.
  - **The check was validated on full-compass sets only.** An octave's stretch
    turns negative only when the upper note's B exceeds about 4–6.5× the lower
    note's (for octave types ρ ≥ 1.5; never at ρ = 1). An unmeasured partner
    carries the prior, so a session started mid-compass can put one correct key
    well above a default prior as the upper note of such a pair, and the check
    excludes it. Recapturing reproduces the same B, and the same exclusion.
    Replaying the capture sets in session order (middle outward) would count
    the false exclusions.

  Candidates, none measured: judge only pairs whose two keys are both measured;
  fall back to the key's previous trusted capture before the prior; give the
  strobe the same fallback.
- **Remove MAT's confidence score?** — `Investigating`. `MatEstimate::confidence`
  is ours, not the paper's: pairwise self-consistency, scaled by how many pairs
  backed the median. It is not accuracy (a coherent wrong series, such as an
  octave mis-seed giving 4×B, scores high), nothing in the app reads it, and as a
  bad-capture detector it was measured and rejected (*No automatic bad-capture
  detector*, under Known limitations). What still uses it: the Worker writes it
  into every capture's `analysis.json` as `b_confidence`, and `cargo lab mat
  validate`, `mat regen` and `mat recovery` print it. Removing it changes the
  dump format and those modes' output; keeping it costs a field whose docs must
  keep saying what it is not.
  → [report 0006](reports/0006-discovery-refinement-validation.md) (item 4)
- **MAT serial-vs-simultaneous on an in-tune instrument** — `Gated on` an
  in-tune instrument (see *When an in-tune instrument is captured*). Confirming the serial order
  generalizes retires the simultaneous fallback and re-enables the paper's
  tighter §2.4 peak-detection band. The deletion itself waits for the `(f₀, B)`
  seam, which injects `MatOrder` as a parameter.
- **MAT $f_0$ vs the tracked $f_0$** — `Deferred` (by design, recorded).
  The Worker reports the Goertzel-tracked $f_0$ as `measured_f0`; MAT's jointly
  refined $f_0$ is diagnostic only. Final $B$ accuracy awaits an in-tune
  instrument.
- **Soundboard fingerprint** — `Deferred`, not yet investigated. A soundboard,
  room or undamped-string resonance sits at the same frequency whatever key is
  struck, so a per-instrument spectral fingerprint could mask it during MAT's
  partial association. Report 0013 §5 leaves two candidates: 170.7 Hz on piano
  #1 and 145.6 Hz on piano #2, each seen through several partials of many keys
  and above its null.
  → [report 0013](reports/0013-bass-extra-lines-attribution.md)

## Pipeline and architecture

- **Dynamic sample rate** — `Planned`. `Engine::new` takes a rate, but the
  pipeline hands it the `SAMPLE_RATE` constant rather than the stream's rate
  (`CapturePayload` carries the same), the
  capture path requires 44.1 kHz, and the buffer sizes,
  COLA window and Gatekeeper timings are all dimensioned for it. A device that
  cannot offer it fails with a clear error instead of panicking (not re-checked
  since the CPAL 0.18 upgrade), but it still cannot run. New code must read the rate from the single source of truth
  so this stays a one-point change. One thing a higher rate would **not** buy
  is a measured top-octave B for the curve: the partials it would admit are
  weaker than the ones already failing (report 0009, analysis 7). It is still
  worth having for analysis — more treble partials on record, even ones too
  quiet to move the curve — which is a reason to build it, just not that one.
  → [`docs/internals/hot-path.md`](docs/internals/hot-path.md)
- **Review the `unsafe` byte-slice transmute** in `worker.rs::write_diagnostics`
  — `Deferred`. Functionally correct; worth checking whether `bytemuck` replaces
  it without a performance cost.
- **Two `style.md` rules have no lint behind them** — `Planned`. Every public
  `tuner-core` item carries a `///` doc except 18: `GateResult`'s fields (two of
  them wait on the onset-flag entry above), `CurveJob`'s, `WorkerOutput`'s
  variants, `WorkerManager`'s two methods, `Engine`'s two public fields, and one
  field each on `Gatekeeper`, `AudioPipeline` and `CurveBundle`.
  `#![warn(missing_docs)]` would hold the rule under clippy's `-D warnings`. And
  `pub(crate)` is the default for shared internals, but `tuner-gui`, a binary
  crate, spells 134 items `pub`; `#![warn(unreachable_pub)]` finds them, with
  machine-applicable fixes. Keep that sweep out of the item-order pass, whose
  check needs pure moves.
- **Four algorithm seams become traits, and audits become differential tests** —
  `Planned`, after the structural step. The signal gate (`Gatekeeper`), discovery,
  the `(f₀, B)` estimator (MAT) and the curve engines each get a trait, matching
  four of the lab's six subcommands (`gatekeeper`, `engine`, `mat`, `curve`), so a
  researcher's implementation runs in the app and the lab alike. Registration is
  static and in-tree; no dynamic plugins. Tracking is deliberately not a seam: no
  second implementation is in sight, and its failure costs display, not a
  measurement. Each trait gets a conformance test, and each faithfulness audit an
  executable reference implementation with a differential test; an audit in
  `reports/retiring/` is deleted only once its test exists. Verification means
  those tests plus metamorphic properties and panic-freedom checks, never
  formal proofs about floating point. Riding with it: `TwmConfig` and `MatOrder`
  become injected parameters (and `MatOrder::Simultaneous` goes once an in-tune
  instrument clears it), and the hot-path benches. Until then, leave each of the
  four with one entry point, keep `TwmConfig::default()` in the Engine and
  `MatOrder::Serial` in the Worker at their call sites, and keep `algorithms/` and
  `models.rs` free of anything device-shaped. The argument is the 2026-09
  modularity-and-verification investigation, still outside the tree; it becomes
  a `docs/design/` note when this work starts.
- **Split `app.rs` into owned parts: `Message`, `update` and `AppDisplayData`**
  — `Planned`, after the structural step; not a pure move, so its own change.
  `Message` is one flat enum of 59 variants, `update` matches all of them, and
  `AppDisplayData` is one flat mirror of 52 fields. iced's crate docs
  ("Scaling Applications") split an application into parts that each own their
  state, `Message` enum, `update` and `view`, with the parent routing messages
  (`Message::Library(library::Message)`). Candidates, each a group of variants,
  handlers and display fields: the strobe, the profile library, the measurement
  inspector, the threshold calibration panels, and the capture controls (arm,
  string isolation, capture duration). The strobe comes first and has the most
  to gain: a type owning its private fields, with its inputs passed in, makes
  the lock's invariant (every input to the reference set is reachable from the
  identity it pushes) a signature instead of a comment. That step alone leaves
  `Message` alone; nesting messages is the second step. Decide the pattern once
  rather than per part: it changes every view that builds one of those
  messages, and it moves message handling out of `app.rs`, so `layering.md`'s
  hub rule changes with it. `TunerApp` stays the root that holds the parts, and
  each part's view starts from its per-panel file (*Structural work*). The
  split also removes a module cycle: every view and `library.rs` import
  `app`'s types (`AppDisplayData`, `Message`, `Instrument`, `TuningMode`) while
  `app` imports the views' types and functions. Once a part owns its state and
  messages, its view reads the part and nothing reads `app`.
  → [`docs/internals/layering.md`](docs/internals/layering.md)
- **The strobe's readout rules live in the GUI, and the lab replays copies** —
  `Deferred`, best done with the owned-parts split. `app/strobe.rs` builds the
  reference set the bank is asked for (ET or curve, the prior-B fallback for an
  unmeasured key, the displayed and coarse partials, the spacing) and decides
  which readout is shown (`BAND_READABLE_HZ` and the `READOUT_SWITCH_HOPS`
  debounce). Both are measured decisions, and the lab replicates them by hand:
  `strobe chatter` keeps its own `BAND_READABLE_HZ = 18.0` against the GUI's
  18.03 Hz, and the `gates` harness rebuilds "the shipping reference set, exactly
  as the GUI builds it". `ARCHITECTURE.md` puts a decision worth replaying
  offline in `tuner-core`, where the app and the lab read one implementation, so
  both belong beside the strobe bank; the GUI keeps the curve lock and the
  display. Moving them changes the lab's output where its copies differ, so
  re-run the report 0011 modes when it lands.
  → [report 0011](reports/0011-coarse-spectral-readout.md)
- **Bench coverage of the hot-path chain** — `Planned`, as one deliberate pass.
  `hot-path.md` makes per-hop cost a hard rule, and `benches/` measures two
  things: the strobe bank and `resolve_lines` (report 0012 E9), and split
  discovery per frame. The strobe is a *tap* — removing it leaves gating,
  detection and measurement bit-identical — so of the chain stages the rule
  actually binds, only discovery is measured. The FFT front end (step 1), the
  Gatekeeper (3), the bass magnitude spectrum (4) and Goertzel tracking (5)
  have no bench. Do them together, after the algorithm seams become traits,
  since each bench drives one seam's entry point and would otherwise be
  written twice.
- **Every gate replay reads the logged thresholds** — `Planned`. Captures log the
  NHWRSF and sustain thresholds beside the silence threshold (the `noise_floor`
  key), and `gatekeeper dump` and `gatekeeper plot` replay at them. `engine lock`,
  `engine from-onset` and `engine dump` read only the silence threshold and build
  the gate at its defaults. Nothing moves on the existing sets, which logged
  neither, so this is due before the first replay of a set recorded with them.
  `strobe truth` and `gates pfa` fix the silence threshold instead of reading it,
  so switching them would move the numbers they reproduce; they are listed under
  *No noise floor*. The gate's defaults are also written out in several places,
  and for the NHWRSF threshold they disagree: `GatekeeperConfig::default()`,
  which these replays build, and the GUI's `SettingsDisplayData::default` hold 0.5, while
  `PipelineAtomics` and a new profile (`models::DEFAULT_NHWRSF_THRESHOLD`) start
  the app at 0.9. One should derive from the other.

## Known limitations

These are measured and understood, not defects awaiting a fix.

- **Noisy environments.** Sympathetic noise is filtered by a −30 dB relative
  amplitude mask; below ~30 dB SNR, noise bleeds past it and can destabilize
  detection.
- **DC blocker corner sits at 35 Hz, above A0.** Measured and deliberately kept:
  raising it is actively harmful, and a steeper filter changes MAT's $B$ by a
  median of 0.00 %. Reopens only if a consumer starts using the bottom octave's
  fundamental.
  → [report 0019](reports/retiring/0019-dc-blocker-corner.md)
- **Stereo DC blocker (latent).** Unreachable through `open_input_stream`, which
  accepts mono `f32` configs only; live only for `AudioSource::External`.
- **No automatic bad-capture detector.** Two candidates have been measured and
  rejected: MAT's `b_confidence` (self-consistency, not accuracy — report 0006
  item 4) and a σ_lnB repeat-disagreement threshold (fires on 29 of 88
  well-behaved keys, and is weakly correlated with how much the disagreement
  moves the target). The human is the gate, through the measurement inspector.
- **Coarse readout caveats.** Bounded and measured — the search loss correction,
  the register where the gate degenerates to a ratio test, and the motion tail.
  → [report 0011](reports/0011-coarse-spectral-readout.md)
- **Unison assist is resolution-bound, and says so.** A split resolves only once
  it clears `2/T`; at or below the limit the *reported* separation collapses onto
  the limit rather than the truth, and it is unreliable either way out to ≈1.6 ×.
  The panel therefore always states what the current record is worth. 1.5 s of
  observation cannot reach the endgame (a set unison is well under 1 ¢), which
  is a property of observation time and not of the estimator.
  → [report 0012](reports/0012-unison-line-estimator.md) §4
- **The stated resolution is geometric, and over-promises for a quiet second
  string.** Measured: two strings within 6 dB resolve at the limit itself, but
  one 12 dB or more down needs ≈1.6 × it. The panel does not qualify the number,
  because the condition is unobservable precisely when it bites — the corner is
  the one where the second line is *not* found, so there is no second amplitude
  to condition on. Closing it is an estimator change (single-kernel fit,
  asymmetric-shoulder residual), not a display one.
  → [report 0012](reports/0012-unison-line-estimator.md) §4

## No ETA

- **CPAL/ALSA shutdown workaround, unverified against current CPAL.** The GUI
  sets `exit_on_close_request: false` and `HostHandle::stop()` joins the analysis
  thread before the stream drops. Both were the fix for a shutdown segfault on
  Linux/ALSA, and there has been none since. Whether current CPAL still needs
  them is untested, and testing it means deliberately reintroducing a crash on
  exit — so it stays as is until something gives a reason to look.
  → `tuner-gui/src/app.rs`, `tuner-core/src/audio.rs` (`HostHandle::stop`)
- **Analysis-thread start-up delay, untested.** `audio::spawn_analysis_thread`
  sleeps 100 ms before its first read, commented as letting the GUI initialize,
  and has since the first commit. `tuner-core` cannot see a GUI, so either
  something at start-up still depends on the delay or it does nothing. Remove it
  and watch the first frames after launch and a capture armed straight away.
- **Treble capture-window placement.** Treble upper partials run 20–45 dB
  stronger before the Golden Window opens than inside it, so up there the
  gatekeeper may be selecting the least informative segment of the note. Offline
  test against the full-event dumps; a negative result closes it.
  → [report 0009](reports/0009-repeat-capture-noise-decomposition.md) analysis 7
- **Curve comparison metrics.** The offline `cargo lab curve compare` harness already
  computes beat-rate smoothness, leave-keys-out prediction error, Giordano
  cross-scoring and curvature. Surfacing them in-GUI is held because the
  comparison semantics are themselves a research question — there is no
  ground-truth-free "best" curve.
- **Register-aware gatekeeper gate.** The A/B in audit 05 found the paper's
  sparsity core wins bass/mid while ours wins treble decisively. A register-aware
  gate is a candidate upgrade. The A/B has run on one instrument; running it on
  piano #2 (`cargo lab gatekeeper sparsity diagnostics_piano2`) is what the gate
  waits on. Tonality discrimination does not depend on tuning, so this needs no
  in-tune instrument.
  → [audit 05](reports/retiring/faithfulness-audit-05-metrics.md)
- **Interval-beat strobe.** Build-if-requested, not planned: intervals are
  correct by construction once every note sits on the curve.
