# TODO

The project backlog. Each entry says **what is not done**, what it is blocked
on, and **where the argument lives** — it does not restate the argument. A
decision is argued in its ADR or design note; this file only tracks that it is
open.

**Status vocabulary:** `Planned` · `Deferred` (wanted, not scheduled) ·
`Gated on X` (blocked on a specific input) · `Investigating` (outcome unknown) ·
`Built, gated off` (shipped behind a flag).

---

## User-facing

- **Persist calibration; load a profile at startup** — `Planned`. The
  noise-floor, NHWRSF onset and NINOS² thresholds live only in the runtime
  atomics and are lost on restart, and a saved profile is only loaded by button.
  Splitting rig state (noise floor, remeasured each session) from instrument
  state (the two dimensionless thresholds, which belong to the profile) is the
  design question.
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
- **Flagged-key styling** — `Deferred`. `CurveKeyFlags` is computed per key
  (negative stretch, excluded, Giordano-excluded, B fallback) but not surfaced.
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
- **Diagnostics viewer / importer** — `Deferred`. A settings panel to read and
  graph `analysis.json`, and to import one into the profile. Developer tooling.

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

- **MAT serial-vs-simultaneous on a second instrument** — `Gated on` a second
  instrument. Confirming the serial order generalizes retires the simultaneous
  fallback and re-enables the paper's tighter §2.4 peak-detection band.
- **MAT $f_0$ vs the tracked $f_0$** — `Deferred` (by design, recorded).
  The Worker reports the Goertzel-tracked $f_0$ as `measured_f0`; MAT's jointly
  refined $f_0$ is diagnostic only. Final $B$ accuracy awaits a second
  instrument.

## Pipeline and architecture

- **Dynamic sample rate** — `Planned`. The rate is threaded through the
  `Engine`, but the capture path still requests 44.1 kHz and the buffer sizes,
  COLA window and Gatekeeper timings are all dimensioned for it. New code must
  read the rate from the single source of truth so this stays a one-point change.
  → [`docs/internals/03-dsp-pipeline.md`](docs/internals/03-dsp-pipeline.md)
- **`CaptureState` `compare_exchange`** — `Planned`. The three-thread baton-pass
  is convention-only today; `compare_exchange` would enforce it at the atomic
  level. → [`docs/internals/02-cross-thread-communication.md`](docs/internals/02-cross-thread-communication.md)
- **Band-slope estimator belongs in `tuner-core`** — `Planned`. The strobe's
  primary readout is least-squares-fitted in `app.rs`, which holds cross-hop
  state and estimates a signal — both of which the architecture rules put in
  `tuner-core`. Moving it to `strobe.rs` also removes a dropped-frame guard that
  only exists because the GUI reads a lossy buffer.
  → [`docs/internals/01-architecture.md`](docs/internals/01-architecture.md)
- **`audio.rs` three-way split** — `Deferred`. Three concerns in one file: the
  CPAL stream, the cross-cutting DSP constants, and the thread host. Splitting
  them retires the accepted `models ↔ audio` cycle. A deliberate whole-codebase
  pass, not a re-export shim.
- **`models/` growth pattern** — `Deferred`. `models.rs` becomes a directory
  when the single file stops feeling right; no threshold beyond that.
  → [`docs/internals/04-algorithms-and-models.md`](docs/internals/04-algorithms-and-models.md)
- **`ninos2` → `spectral_sparsity` rename** — `Deferred`. The gate is ours, not
  Mounir's NINOS²; the historical name persists across ~11 files plus CSV
  headers. Deferred until the Gatekeeper is reworked anyway.
  → [audit 05](docs/audits/faithfulness-audit-05-metrics.md)
- **"Engine" is overloaded** — `Investigating`. `engine.rs` is the F0 detector;
  the tuning curve also has "engines (a)–(d)". Two unrelated meanings in one
  codebase and in the docs. Needs one of them renamed.
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
