# TODO

The project backlog. Each entry says **what is not done**, what it is blocked
on, and **where the argument lives** — it does not restate the argument. A
decision is argued in its ADR or design note; this file only tracks that it is
open.

**Status vocabulary:** `Planned` · `Deferred` (wanted, not scheduled) ·
`Gated on X` (blocked on a specific input) · `Investigating` (outcome unknown) ·
`Built, gated off` (shipped behind a flag).

## Sequencing

Most of this file has no ordering. These four steps do, agreed 2026-08-01:

1. **Finish the in-flight user-facing work** — session durability, the per-key
   inspector + flagged-key styling, and the band-slope move have landed. Then
   unison assist and the Neyman–Pearson gate measurement.
2. **Upgrade to CPAL 0.18.1** (below). Before the refactors, so the module
   boundaries are drawn around the API we are keeping rather than redrawn after.
3. **The structural work revised this session** — the `worker.rs` / `audio.rs` /
   `models.rs` boundary pass, the `app.rs` split, and the two open arguments
   recorded with them.
4. Everything else, unordered.

---

## User-facing

- **Profile export / import from an arbitrary path** — `Deferred`. The library
  browser covers new / open / resume / duplicate / delete over the profiles
  directory with no new dependency; sending a profile to a colleague, or opening
  one they sent, needs a native file dialog (`rfd`), which is a real system
  dependency on Linux. The field's answer is cloud sync or share-sheet export
  (§5.1 of the note below) — decide when the need is real.
  → [`docs/design/session-persistence-and-profile-library.md`](docs/design/session-persistence-and-profile-library.md)
- **Capture-dump retention, and a user-settable dump location** — `Planned`.
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
  → [`docs/design/session-persistence-and-profile-library.md`](docs/design/session-persistence-and-profile-library.md)
- **Pre-built binaries** — `Planned`. The tuner should be usable without a Rust
  toolchain now that the measure → curve → strobe path is complete.
- **Reference pitch other than A440** — `Deferred`. `TuningCurve.d_g` already
  carries the offset; only the UI is missing.
- **Temperament selection** — `Deferred`. Equal temperament only today.
- **Pitch raise (over-pull)** — `Deferred`. Detection and display already work
  through a raise (the coarse readout is lock-independent and has no ±21.5 Hz
  limit); what is missing is over-pull *targets* — a deficit-measurement pass and
  a model of how a raise redistributes tension.
- **Unison assist** — `Deferred`. A multi-string note needs its strings
  zero-beat against each other. The beat is already present as the amplitude
  envelope of a single strobe reference; it is not yet estimated or displayed.
  See `docs/design/strobe-and-manual-tuning-ui-design.md` §7.4.
- **Show-all partials strobe mode** — `Deferred`. The v1 strobe shows one
  Smart-Partials-selected band per key; a band-per-partial toggle is planned but
  unbuilt (`strobe_partials` already emits every partial). Two costs kept it out
  of v1: high-treble bands would carry raw-B target uncertainty, and deep-bass
  low-partial bands would sit frozen behind the amplitude gate.
  See `docs/design/strobe-and-manual-tuning-ui-design.md` §6.4 (R5).
- **Per-note partial override** — `Deferred`. TuneLab's escape hatch for a
  dead auto-chosen partial: an on-the-fly, non-persisted per-note override of
  the displayed partial. Pairs with show-all. Same source, §6.4.
- **Cents-normalized strobe rotation** — `Deferred`, and conditional. The band
  rotates at the physical beat rate (D2). If a real tuning session shows Hz
  rotation is unreadable across registers *despite* Smart-Partials selection and
  the coarse readout, add an optional per-band angle rescale — cheap, no new
  crossing. The trigger is explicitly a use-testing outcome, not a preference.
  Same source, §5.5 (R9).
- **Engine (c) ρ Low/High presets** — `Deferred`. Computing three (c) presets
  naively re-runs the ~1.3 s Giordano scan three times; the calibration has to be
  factored out of the per-preset path first. Rendered as greyed placeholders.
- **"Advanced" mode** — `Deferred`. Curve selection, comparison metrics, and the
  offline diagnostics are all currently visible to every user with no separation
  between the ordinary tuning path and the research surface. See
  *Curve comparison metrics* below.
- **Curve auralization playback** — `Deferred`. `tuner_core::synth` renders a
  curve to audio offline today (the `auralize` example writes WAVs). A live
  "hear the curve" control needs an audio **output** stream, which is a
  sanctioned seventh crossing living in `tuner_core::audio`, never in the GUI.
  → [`docs/internals/01-architecture.md`](docs/internals/01-architecture.md)
- **Capture importer** — `Deferred`, and an **example executable, not a GUI
  panel** (developer tooling; decided 2026-08-02). Reads capture dumps and
  merges measurements back into a profile — the recovery path for an inspector
  drop, which deliberately keeps the audio. It must **re-run the analysis on
  `audio.raw`**, never import the dump's cached `analysis.json`: that cache is
  exactly what was wrong about piano #2's deep bass while its audio was fine
  (`06-capture-sets.md`). `examples/regenerate_partials` already does the
  re-analysis half.

## Engine and discovery

- **TWM parameter validation on a second instrument** — `Gated on` a second
  instrument. The shipped `TwmConfig::default()` was MOBO-tuned against a
  synthetic dataset and validated on one piano (n = 1).
  → [ADR 0001](docs/adr/0001-mobo-tuning.md),
  [ADR 0006](docs/adr/0006-discovery-refinement-validation.md)
- **TWM bass-lock bias (deep-bass stable-wrong core)** — `Gated on` a second
  instrument. The decision-level half is solved (M-of-N acquisition lock,
  validated on two instruments); the residual is a wrong-$B$-template problem in
  the bass that scoring constants cannot fix.
  → [ADR 0006](docs/adr/0006-discovery-refinement-validation.md),
  [ADR 0010](docs/adr/0010-m-of-n-lock-rule-replay.md)
- **Measured-$B$ discovery seeding** — `Built, gated off`
  (`pipeline::APPLY_MEASURED_B_TO_DISCOVERY`). Flip the flag once a second,
  in-tune instrument validates the measured values.
  → [ADR 0006](docs/adr/0006-discovery-refinement-validation.md)
- **Lock-release / re-lock hysteresis** — `Deferred`. The M-of-N rule covers
  acquisition only. → [ADR 0010](docs/adr/0010-m-of-n-lock-rule-replay.md)
- **Per-bin noise floor** — `Planned`. The Neyman–Pearson gate uses a single
  broadband floor measured during silence; real room noise is coloured, so the
  scalar is too permissive in the bass and too strict in the treble.
- **Neyman–Pearson σ misspecification** — `Investigating`. The engine tracker's
  and strobe bank's amplitude gates threshold against ambient-silence σ while
  running during sustain. Mechanism confirmed; the two gates are unmeasured and
  the CFAR fix is deliberately not ported yet.
  → [`docs/internals/suspected-issues.md`](docs/internals/suspected-issues.md)

## Worker and measurement

- **`key_index` is not a sufficient measurement identity on every instrument** —
  `Deferred`, and only matters once a non-piano workflow exists. A fretted note
  is producible on several strings of different gauge and speaking length, hence
  different $B$; a piano note is 1–3 strings. A per-string/course discriminator
  on `KeyMeasurement` is the obvious shape, and costs nothing to add later — an
  additive `#[serde(default)]` field needs no migration, which is why one was
  *not* reserved speculatively.
  → [`docs/design/session-persistence-and-profile-library.md`](docs/design/session-persistence-and-profile-library.md) §1.1

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
  → [ADR 0009](docs/adr/0009-repeat-capture-noise-decomposition.md),
  [`docs/design/session-persistence-and-profile-library.md`](docs/design/session-persistence-and-profile-library.md) §5.3
- **MAT serial-vs-simultaneous on a second instrument** — `Gated on` a second
  instrument. Confirming the serial order generalizes retires the simultaneous
  fallback and re-enables the paper's tighter §2.4 peak-detection band.
- **MAT $f_0$ vs the tracked $f_0$** — `Deferred` (by design, recorded).
  The Worker reports the Goertzel-tracked $f_0$ as `measured_f0`; MAT's jointly
  refined $f_0$ is diagnostic only. Final $B$ accuracy awaits a second
  instrument.

## Pipeline and architecture

- **Upgrade to CPAL 0.18.1** — `Planned`, and **before** the module-boundary
  work (see Sequencing). The workspace pins `cpal = "0.17.3"`. The upgrade
  touches `open_input_stream`, `find_supported_config` and the device/config
  negotiation types, which is precisely the code the `audio.rs` split will move
  — doing it after would mean drawing the new boundaries around an API we are
  about to change. Verify the negotiated-rate path and the mono `f32` filter
  still hold, and that a device offering no 44.1 kHz mono config still fails
  with the clear error rather than panicking.
- **Dynamic sample rate** — `Planned`. The rate is threaded through the
  `Engine`, but the capture path still requires 44.1 kHz and the buffer sizes,
  COLA window and Gatekeeper timings are all dimensioned for it. A device that
  cannot offer it now fails with a clear error instead of panicking, but it
  still cannot run. New code must read the rate from the single source of truth
  so this stays a one-point change.
  → [`docs/internals/03-dsp-pipeline.md`](docs/internals/03-dsp-pipeline.md)
- **`CaptureState` `compare_exchange`** — `Planned`. The three-thread baton-pass
  is convention-only today; `compare_exchange` would enforce it at the atomic
  level. → [`docs/internals/02-cross-thread-communication.md`](docs/internals/02-cross-thread-communication.md)
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

  **Two arguments are open and should be settled by this pass, not before:**
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

  A deliberate whole-codebase pass, not a re-export shim; zero behaviour
  change; its own commit.
  → [`docs/internals/04-algorithms-and-models.md`](docs/internals/04-algorithms-and-models.md),
  [`docs/internals/05-style.md`](docs/internals/05-style.md)
- **`ninos2` → `spectral_sparsity` rename** — `Deferred`. The gate is ours, not
  Mounir's NINOS²; the historical name persists across ~11 files plus CSV
  headers. Deferred until the Gatekeeper is reworked anyway.
  → [audit 05](docs/audits/faithfulness-audit-05-metrics.md)
- **"Engine" is overloaded** — `Investigating`. `engine.rs` is the F0 detector;
  the tuning curve also has "engines (a)–(d)". Two unrelated meanings in one
  codebase and in the docs. Needs one of them renamed.
- **Reconcile `peaks`' split test suite** — `Deferred`. The placement rule is
  now written down ([05-style](docs/internals/05-style.md), "Where tests live"),
  and `peaks` is the one module with tests in both locations. Low stakes: both
  halves pass and each sits on the correct side of the visibility line, so this
  is tidying, not a defect.
- **Review the `unsafe` byte-slice transmute** in `worker.rs::write_diagnostics`
  — `Deferred`. Functionally correct; worth checking whether `bytemuck` replaces
  it without a performance cost.

## Known limitations

These are measured and understood, not defects awaiting a fix.

- **Noisy environments.** Sympathetic noise is filtered by a −30 dB relative
  amplitude mask; below ~30 dB SNR, noise bleeds past it and can destabilize
  detection.
- **DC blocker corner sits at 35 Hz, above A0.** Measured and deliberately kept:
  raising it is actively harmful, and a steeper filter changes MAT's $B$ by a
  median of 0.00 %. Reopens only if a consumer starts using the bottom octave's
  fundamental.
  → [ARCHITECTURE.md](ARCHITECTURE.md#the-dc-blockers-corner-sits-at-35-hz-above-a0)
- **Stereo DC blocker (latent).** Unreachable through `open_input_stream`, which
  accepts mono `f32` configs only; live only for `AudioSource::External`.
- **No automatic bad-capture detector.** Two candidates have been measured and
  rejected: MAT's `b_confidence` (self-consistency, not accuracy — ADR 0006
  item 4) and a σ_lnB repeat-disagreement threshold (fires on 29 of 88
  well-behaved keys, and is weakly correlated with how much the disagreement
  moves the target). The human is the gate, through the measurement inspector.
  → [`docs/design/session-persistence-and-profile-library.md`](docs/design/session-persistence-and-profile-library.md) §5.3
- **Coarse readout caveats.** Bounded and measured — the search loss correction,
  the register where the gate degenerates to a ratio test, and the motion tail.
  → [`docs/internals/suspected-issues.md`](docs/internals/suspected-issues.md),
  [ADR 0011](docs/adr/0011-coarse-spectral-readout.md)

## No ETA

- **Curve comparison metrics.** The offline `curve_compare` harness already
  computes beat-rate smoothness, leave-keys-out prediction error, Giordano
  cross-scoring and curvature. Surfacing them in-GUI is held because the
  comparison semantics are themselves a research question — there is no
  ground-truth-free "best" curve.
- **Register-aware gatekeeper gate.** The A/B in audit 05 found the paper's
  sparsity core wins bass/mid while ours wins treble decisively. A register-aware
  gate is a candidate upgrade, gated on a second instrument.
  → [audit 05](docs/audits/faithfulness-audit-05-metrics.md)
- **Interval-beat strobe.** Build-if-requested, not planned: intervals are
  correct by construction once every note sits on the curve.
  → `docs/design/strobe-and-manual-tuning-ui-design.md` §7
