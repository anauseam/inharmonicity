# Algorithms and Models

`tuner-core` separates _stateless DSP math_ from _domain data types_.
Code that does math on buffers lives in `algorithms/`. Code that
describes the piano-tuning domain — notes, partials, measurements —
lives in `models.rs` (or eventually a `models/` directory).

## `algorithms/` — stateless DSP

Every function in `algorithms/` is stateless: it takes input buffers,
returns computed values, and has no side effects. No lock-free
channels, atomics, or global mutable state. This makes each function
trivially unit-testable and freely composable inside the pipeline.

Algorithms accept `&[T]` / `&mut [T]` slices and never allocate; the
caller (typically the `Engine` or `Gatekeeper`) owns the scratch
buffers. This is what allows them to run inside the hot path.

The current set of algorithm files is listed in the `algorithms` module
root (`algorithms.rs`). Each file's doc-comment is the source of truth
for what that algorithm does; these files churn the most as approaches
are refined, so the rule set deliberately does not duplicate their
contents.

### Sizing rule

If an algorithm exceeds roughly 200 lines or introduces its own internal
types (for example `BiquadCoeffs`, `PitchCandidate`), it gets its own
file. Otherwise it belongs in the file for its group.

### Shared primitives

Functions used across several algorithm files (for example the FFT,
`magnitude_spectrum`, `cspe`, and `jacobsen` transforms in
`spectral.rs`, shared by the Worker, MAT, and peak extraction) live in
their group file. Internal-only shared helpers use `pub(crate)`
visibility; a primitive a diagnostic example or frontend needs is
`pub`.

### Analytical vs Ad-Hoc Solutions

When fixing edge cases (like false locks or wild outliers), prefer peer-reviewed mathematical solutions over ad-hoc heuristics.

If an algorithm is failing, the goal is to find the proper analytical fix (e.g., upgrading to CSPE or trusting MAT's median-rejection) rather than clamping the output with `if x > max`.

**The Topological Scrutiny Test for Heuristics**
If a new heuristic or empirical constant must be introduced, it must pass strict scrutiny:

- **Fragile Thresholds (Banned):** Simple magic numbers that depend on absolute amplitude, microphone gain, or specific room environments (e.g., `if magnitude < 50.0`). These break when hardware changes.
- **Topological Constraints (Allowed):** Heuristics that fundamentally define or alter the geometric shape of the information or search space. These must be scale-invariant (e.g., dimensionless amplitude ratios $a/A_{max}$, or percentage-based frequency limits like $0.029 \times f_n$). A valid heuristic changes the _shape_ of the mathematical topology (e.g., changing an unbounded search to a bounded resonance zone, or adding an exponent to curve a linear error).

When adding a new algorithm or heuristic, cite the source paper (e.g., Maher & Beauchamp 1994, Hodgkinson DAFx-09, Candan 2015, Short & Garcia 2006) in the doc-comment so the next contributor knows the theoretical basis — and verify the citation against the actual source (the faithfulness-audit series caught one fabricated section reference; see `docs/audits/faithfulness-audit-04-peaks.md`). If a constant is empirically calibrated, document _how_ it alters the mathematical topology. If the mechanism is ours rather than a port, say so explicitly and cite the validating ADR instead (e.g., `mask_peaks` → ADR 0002).

## `models.rs` — domain data types

`models.rs` holds non-DSP types: domain knowledge, lookup tables, and
serializable structures consumed by the GUI or by external tooling.
This includes things like `Note`, the 88-key lookup tables, `Partial`,
`KeyMeasurement`, and `InharmonicityProfile`.

`models.rs` may grow into a `models/` directory with submodules
(`models/note.rs`, `models/partial.rs`, …) once it exceeds a comfortable
single-file size.

## Where new code goes

- **Math on buffers** → a new function in the appropriate
  `algorithms/*.rs`, or a new file if the sizing rule above applies.
- **A new domain type** (something that needs to be serialized, displayed,
  or stored in a profile) → `models.rs`.
- **State that lives across hops** (filter memory, running counters,
  state-machine variables) → a field on an existing component
  (`Engine`, `Gatekeeper`, `Strobe`) or the pipeline itself, _not_
  in `algorithms/`. A concern that fits no existing component gets its
  own component file — an architecture-level change; see the review
  requirements in [`01-architecture.md`](01-architecture.md). The
  stateless math it calls still belongs in `algorithms/` (the
  `Strobe` / `spectral::goertzel_windowed` split is the pattern).
