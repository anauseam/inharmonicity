# Layering

`tuner-core` separates _stateless DSP math_ from _domain data types_: code that
does math on buffers lives in `algorithms/`, code that describes the
piano-tuning domain — notes, partials, measurements — in `models.rs`.

## `algorithms/` — stateless DSP

Every function in `algorithms/` is stateless: it takes input buffers, returns
computed values, and has no side effects — no lock-free channels, atomics, or
global mutable state — which makes each one trivially unit-testable and freely
composable inside the pipeline. Algorithms accept `&[T]` / `&mut [T]` slices
and never allocate; the caller (typically the `Engine` or `Gatekeeper`) owns
the scratch buffers, which is what lets them run on the hot path. The current
set of files is listed in the module root (`algorithms.rs`), and each file's
doc-comment is the source of truth for what it does; these files churn the most
as approaches are refined, so this rule set does not duplicate them.

**Sizing rule.** An algorithm that exceeds roughly 200 lines or introduces its
own internal types (`BiquadCoeffs`, `PitchCandidate`) gets its own file;
otherwise it belongs in the file for its group.

**Shared primitives.** Functions used across several algorithm files (the FFT,
`magnitude_spectrum`, `cspe` and `jacobsen` in `spectral.rs`, shared by the
Worker, MAT and peak extraction) live in their group file. Internal-only
helpers are `pub(crate)`; a primitive a harness or frontend needs is `pub`.

### Analytical vs Ad-Hoc Solutions

When fixing edge cases (false locks, wild outliers), prefer peer-reviewed
mathematical solutions over ad-hoc heuristics: if an algorithm is failing, find
the analytical fix (upgrading to CSPE, trusting MAT's median rejection) rather
than clamping the output with `if x > max`.

**The Topological Scrutiny Test for Heuristics.** A new heuristic or empirical
constant must pass strict scrutiny:

- **Fragile thresholds (banned):** magic numbers that depend on absolute
  amplitude, microphone gain, or a specific room (`if magnitude < 50.0`). These
  break when hardware changes.
- **Topological constraints (allowed):** heuristics that define or alter the
  geometric shape of the information or search space, and are scale-invariant
  (dimensionless amplitude ratios $a/A_{max}$, percentage frequency limits like
  $0.029 \times f_n$). A valid heuristic changes the _shape_ of the
  mathematical topology: an unbounded search becomes a bounded resonance zone,
  an exponent curves a linear error.

When adding an algorithm or heuristic, cite the source paper (Maher &
Beauchamp 1994, Hodgkinson DAFx-09, Candan 2015, Short & Garcia 2006) in the
doc-comment, and verify the citation against the actual source — the
faithfulness-audit series caught one fabricated section reference
(`reports/audits/faithfulness-audit-04-peaks.md`). If a constant is
empirically calibrated, document _how_ it alters the topology. If the mechanism
is ours rather than a port, say so and cite the validating report instead
(`mask_peaks` → report 0002).

## `models.rs` — domain data types

`models.rs` holds non-DSP types: domain knowledge, lookup tables, and
serializable structures consumed by the GUI or by external tooling — `Note`,
the 88-key lookup tables, `Partial`, `KeyMeasurement`, `InharmonicityProfile`.

## Where new code goes

- **Math on buffers** → a function in the appropriate `algorithms/*.rs`, or a
  new file if the sizing rule applies.
- **A new domain type** (something serialized, displayed, or stored in a
  profile) → `models.rs`.
- **State that lives across hops** (filter memory, running counters,
  state-machine variables) → a field on an existing component (`Engine`,
  `Gatekeeper`, `Strobe`) or the pipeline itself, _not_ in `algorithms/`. A
  concern that fits no existing component gets its own component file — an
  architecture-level change whose bar is *The processing chain is sacred* in
  [`hot-path.md`](hot-path.md); the stateless math it calls still belongs in
  `algorithms/` (the `Strobe` / `spectral::goertzel_windowed` split is the
  pattern).

## A profile holds repeats; the product reads one and a statistic reads all

`InharmonicityProfile::active()` returns the **newest** trusted capture of a
key, and that is what the curve and the strobe consume — deliberately: the
piano is a changing object, and a key that was re-tuned or re-strung should be
represented by its current state, not by a median that spans weeks. Whether to
pool *within a session* is an open design question; on the current evidence it
would not move the curve (audit 06).

A **validation statistic** is a different consumer and must not inherit that
choice: which capture happened to be newest in four keys moved a
treble-asymptote fit on instrument 2 by 1.9 SE between two days on which the
curve did not change (audit 06). Two rules follow:

- **Pool repeats per key, by median, before fitting or comparing.** The
  estimator's failure is one-sided — a starved capture collapses `B` toward
  zero, never toward infinity — so a mean inherits the collapse and a median
  discards it; the pooled fit is stable across days (audit 06, report 0009).
- **Treat a nominal SE as optimistic when the input is one capture per key.**
  It counts scatter about the fitted line and nothing else — not which capture
  was active, not the collapse bias — so a z-score computed from it overstates
  significance in both directions.

Anyone taking several captures of a key — every validation session — should
know that the profile keeps them (bounded per class,
[`capture-sets.md`](capture-sets.md)) and the app reads one. The repeats exist
for the statistics, not for the curve.

## `tuner-gui` layering

- **Widgets are stateless renderers.** They receive data and return
  `Element`s; they do not own application state.
- **Views compose widgets.** The files in `views/` arrange widgets into screens
  and panels; they do not implement DSP.
- **`app` is the state hub.** All application state, message handling, and
  thread management live in `app.rs` and its child modules, and it holds **no
  DSP**: signal processing lives entirely in `tuner-core`, and `app` only
  reads per-hop telemetry (`FrameOutput`) and worker results and writes back
  over the crossings.
- **The GUI owns the profile, so the GUI owns where it goes.** The *schema* is
  a domain type (`models::InharmonicityProfile`, shared with the Worker and
  the offline harnesses), but every file-location policy — the per-user
  directories, the app-settings document, the listing the browser renders, the
  one-time import of a pre-move profile — lives in `library.rs`; `tuner-core`
  stays headless and knows nothing about `directories` or XDG. Persistence
  *timing* (autosave on capture and undo, the session `.bak`) is likewise
  `app.rs` policy, not core's.
- **File locations are injected, never assumed.** The frontend hands the
  Worker its dump root (`Option<PathBuf>`; `None` writes none, so an embedded
  host can opt out) rather than `tuner-core` resolving one. The directory
  *name* for a capture is `worker::dump_dir_name`, next to the code that
  writes it and public because the GUI deletes the dump of an undone capture —
  when both sides hardcoded the path independently, a change on one silently
  turned the other into a no-op.
