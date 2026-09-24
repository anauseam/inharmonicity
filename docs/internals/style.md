# Style & Idioms

The project follows standard Rust conventions. This file collects the few
style choices that have come up often enough to be worth writing down. Memory
and allocation discipline is covered in [`hot-path.md`](hot-path.md) and
[`thread-crossings.md`](thread-crossings.md); this file is purely about code
shape.

Every rule here binds every crate unless it names its own scope. The ones that
do exist because `tuner-core` is a library: every public item documented,
public exports kept to what a frontend needs, and nothing in it describing a
frontend. `tuner-lab` cites reports freely, since reproducing them is its job.

## Naming and layout

- `snake_case` for functions and modules, `CamelCase` for types,
  `SCREAMING_SNAKE_CASE` for constants. `cargo fmt` is the source of truth for
  whitespace and import ordering.
- One concept per file. A file that hosts a struct is usually named after it
  (`gatekeeper.rs` → `Gatekeeper`).
- Name a module for the specific thing it owns, by the same rule as functions
  below: a module implementing one cited method takes that method's acronym
  (`twm`, `mat`) or, when it has none, its eponym (`jacobsen`), with the
  doc-comment carrying the citation — never a broad domain word, which
  overclaims and reads ambiguously at the import site.
- `pub(crate)` is the default for shared internals. In `tuner-core`, public
  exports belong only on items that frontends (the GUI or a future external
  consumer) actually need.
- Name a computed quantity for what it is, and what is built on it for the role
  it plays. The gate's metric is `inverse_participation_ratio` — long, but it
  has two call sites — while its threshold, telemetry and panel are
  `sustain_*`, so swapping the measure would not rename the surface around it.
- Renaming a persisted field keeps a `#[serde(alias = "…")]` for every earlier
  spelling, asserted by a test. Without one, `serde(default)` silently replaces
  the saved value.

### Function names

Following the Rust API guidelines (no `get_`/`calculate_`/`compute_` stutter —
the noun already says what it is), `algorithms/` functions are named for the
thing itself:

- **A named algorithm or precise standard quantity → the bare name.** `fft`,
  `cspe`, `goertzel`, `jacobsen`, `rms`, `ema`, `nhwrsf`: the doc-comment
  carries the citation and the units, the name carries the identity, and no
  verb prefixes it. This is for *uniquely-identified* methods (an acronym like
  `twm`/`mat`, an eponym like `jacobsen`) or *precise* DSP quantities — not
  broad domain words that are ambiguous at the call site. A routine that
  minimizes sensory dissonance is not `dissonance`; name it for the specific
  method.
- **A transform that fills a buffer → a descriptive output noun.**
  `magnitude_spectrum`: the complex modulus is a quantity, not a named
  algorithm, so it gets the standard DSP noun rather than a bare acronym.
- **An action on data → a verb-object.** `extract_peaks`, `mask_peaks`,
  `score_candidate`, `discover`, `refine_scale`.

When in doubt, prefer the shortest name that is unambiguous at the call site,
and let the doc-comment do the explaining. Beware the stutter the module name
creates: inside `whittaker`, the smoother is `smooth`, not `whittaker` —
`whittaker::smooth(…)` reads, and `whittaker::whittaker(…)` does not.

### Constants ported from a paper keep the paper's symbols

A constant that comes straight out of a cited equation keeps that equation's
symbol (`B1`, `B2`, `X_STAR`, `S1`, `S2` — Giordano Eqs. 4–6), cryptic as the
names look in isolation. Ported maths must be **auditable against its
source**: a reviewer puts the code next to the paper and checks it
symbol-for-symbol, and renaming `B1` to `ROUGHNESS_DECAY_LOW` makes that
harder. The doc-comment names the equation; the symbol names the term. This
applies **only** to symbols lifted from a source: a constant that is ours gets
a descriptive name and states its provenance (`SCAN_MARGIN_CENTS`,
`NEGATIVE_STRETCH_TOL_CENTS`).

### Cross-module references — `use` at the top

Bring another module's item into scope with a `use` at the top of the file,
not a fully-qualified `crate::…::item` path inline in a function body: the
import block is the module's dependency manifest, where a reviewer auditing the
module graph sees every cross-module dependency in one place. What to import
follows the Rust Book's idiom ("Creating Idiomatic `use` Paths"):

- **Functions → import the parent module** and keep the qualifier at the call
  site: `use crate::algorithms::spectral;` … `spectral::jacobsen(…)`. The
  qualifier is the point — it shows the call is not local; importing the bare
  function hides its origin and is no improvement on the inline path.
- **Types, traits, and constants → import the item itself**:
  `use crate::models::KeyProfile;`.
- Both at once: `use crate::algorithms::twm::{self, TwmConfig};`, the pattern
  `discovery.rs` and `curves.rs` use.

Const-context references and one-off `#[cfg(test)]` helpers are the pragmatic
exceptions. This is a review convention, not a lint: `clippy::absolute_paths`
is restriction-group and too noisy to enable.

## Doc comments

Every public item in `tuner-core` carries a `///` doc comment. For algorithm
functions it is the source of truth for the algorithm: any non-obvious math,
the source paper or DAFx citation if applicable, and the units of the inputs
and outputs. Parameters and the return value are described in prose — the
units, contracts and meanings the signature cannot carry — not in
`# Arguments` or `# Returns` lists, which restate the signature and drift
because nothing checks them (five of the project's thirty had). The headings
that belong are the ones RFC 1574 names and std uses — `# Panics`, `# Errors`,
`# Safety`, `# Examples` — plus this project's one addition, `# References`,
for the works an algorithm ports. Anything else is prose: a formula, or the
statement of which part is ours, sits in the paragraphs. Inline `//` comments
are for code-level explanation (why this branch, why this constant) and are
not a substitute for doc comments.

### Comments explain the code, not the conventions and not the history

Do **not** restate project rules in code — module-naming choices, layering
rules ("`models` must not depend on `algorithms`"), or any other style or
architecture decision. Those live in `docs/internals/` and `ARCHITECTURE.md`;
a second copy adds noise, goes stale the moment the rule moves, and lectures
the reader instead of informing them. Nor does code narrate what it replaced
("the former `FOO = 8` hard switch parked a trust boundary that moved the
treble ±5 ¢, so we replaced it with…"): a contributor needs the present rigor,
the history belongs in the report that argued the change, and superseded
exposition accretes until the comment is longer than the maths. The commit
message, the design note and the report are where a decision is *argued*; the
code is where it is *applied*.

A comment earns its place only by stating something the code cannot show:

- a constraint or invariant the caller must uphold;
- why a particular branch or constant exists;
- a citation (the paper and equation a formula comes from);
- a non-obvious unit, convention, or numerical caveat;
- a **live compatibility contract** — profile entries written before a field
  existed still deserialize, and how;
- a **guard against a known-bad change** — a terse "do not lower this below X;
  it re-admits sub-harmonic locks (report 0006)": a constraint the code cannot
  express, which stops a contributor repeating a measured failure. Phrase it as
  a rule, never as a story, and only on code that is still there: something
  removed has no line to guard, and its record is the report.

A comment that repeats the next line has not earned it: `// Create sidebar`
above `let sidebar = create_sidebar(…)` says nothing, while `// Skip DC bin`
above `.skip(1)` says why the index is 1. Pointing at a spec that *governs
runtime behaviour* is fine (naming which sanctioned crossing a channel
implements); pointing at a spec to justify where a file lives or what it is
called is not.

### Provenance: rendered docs cite external authority, `//` carries the guard

`///` and `//!` are **rendered API documentation**: when `tuner-core` ships as
a library, docs.rs shows them to every user, and a pointer into this
repository's records is noise to a reader who cannot open it. So the two kinds
of provenance are split:

- **Rendered docs (`///`, `//!`) state meaning, units, invariants and
  preconditions, and cite *external* authority** — the paper an algorithm
  ports, with its equation. They may state physical or mathematical behaviour
  ("a decaying sinusoid's frequency resolution saturates near its own decay
  constant"), which any reader can check, and keep a measured *figure* where it
  is an API property the caller can observe (a resolution, a bound, a rate).
  Never a path into `docs/`, `reports/` or `ARCHITECTURE.md`, and never a
  section title of one.
- **The guard comment states the fact, in a non-rendered `//`**: the rule, the
  number, and what would reopen it — "do not raise α: availability falls
  93.3 → 87.4 %; the lever is the order". A constant's definition is its
  record. **Where a report exists, a `// report NNNN` pointer follows** —
  number only, never a section; a decision that shapes the system points at
  `// ARCHITECTURE.md` instead. One pointer per design choice, at its
  definition. A guard is for provenance, not narrative — where the reader
  could not otherwise tell where a value or mechanism came from: an
  empirically calibrated constant (`SIGMA_LNB_COEFF` → the σ law and its
  floor, then `// report 0009`), or a technique that is ours or departs from
  its source paper (`mask_peaks` → why −30 dB and not Cano's 40, then
  `// report 0002`).

Say precisely which part is ours. An established measure applied in a new way
is cited to its source, and only the application is ours:
`inverse_participation_ratio` is Bell & Dean's participation ratio inverted,
and using it as the gate is the project's own choice. Where `tuner-core/tests/`
asserts the figure, **the test is the provenance** — cite it
(`// asserted: tests/unison_resolution.rs`) rather than the report, because a
reader can run it. `tuner-lab` reproduces evidence, so it cites
`report NNNN §X` freely.

### No comment asserts what a reader cannot open

Capture sets, instruments and measurement sessions exist only on a developer's
disk: a contributor cannot verify the claim, reproduce it, or tell when it has
gone stale, but the comment reads like evidence. An empirically derived
constant says *that* it is ours-and-measured and points at the repo document
recording the derivation (`capture-sets.md`, a report), whose job is to
describe the data. A conversation is outside the repository too: "Prompt N",
"the path review" and "the handoff" are session labels no reader can open, so
cite the report the work became, or state the fact. The rule also holds across
module boundaries: a `tuner-core` item does not document what the frontend does
with it — `tuner-core` is headless and cannot keep such a claim true — nor does
any doc say who calls an item, since callers move and the claim goes false
unnoticed. The text a frontend shows is the frontend's too: a button's label or
a panel title lives in `tuner-gui`, never on a `tuner-core` type.

### Name the thing, not its address

In every crate, test and bench, a pointer names the document by type and
number (`report 0006`, `audit 01`), or by name where it has none
(`capture-sets.md`, the Duan likelihood design note) — never by path or line
number, which break the moment the tree moves. A harness is named by its
command (`cargo lab strobe replay`), never by the example it replaced
(`strobe_replay`); code is named by its item, not its file
(`TunerApp::update_strobe`, not `app.rs`). A design-note label (`§2`, `R3`,
`D7`) is not a name either: two notes number their decisions independently,
and a note is deleted once its work ships. Name the thing — the
negative-stretch detector, the long-window rule, the amplitude gate. A cited
paper's own sections (MAT's §2.4) are fine.

### Keep it compact

- **No asides.** "(these bite)", "crucially", "load-bearing", "Honest note:" add
  emphasis and no information; the sentence carries the weight.
- **No emphasis markup.** Bold is only a label — the lead of a list item, or a
  short paragraph label such as `**Index convention:**` — and italics only a
  title or a variable. Capitals are not emphasis either. The standard library
  bolds about 3 doc lines in 1,000; `**` in a `//` comment renders nowhere.
- **A module doc says what the module is.** No feature list: a list of claims
  goes stale with nothing to notice it.
- **Cutting a pointer means re-reading the sentence.** A parenthetical that
  carried a path or a label often carried the grammar too.
- **Commented-out code is deleted.** Git history holds it, and a disabled block
  drifts out of step with the types around it.
- **Section banners stay** (`// ── Stage B: refinement ──`) where a long body needs
  them to be navigable. They name the step that follows, which an editor's
  outline cannot show inside a function.

### Rustdoc mechanics

- A module's documentation is its own `//!`. A `///` on the `pub mod` line is
  merged into it and resolves intra-doc links from the crate root, so ordinary
  links in it fail.
- Escape a numbered citation marker, `\[1\]`: bare `[1]` is parsed as an
  intra-doc link. Author–year citations need no escape.
- A public item's docs never link to a private one. Unlink the name rather than
  widening its visibility.

## `assert!` vs `Option` / `Result`

`assert!` / `debug_assert!` are for programming errors — invariants the caller
is responsible for upholding, whose violation is a bug in the calling code (a
slice of the wrong length passed to a function with a documented size
contract). `Option` / `Result` are for runtime conditions the caller is
expected to handle (file not found, ringbuf full, no pitch detected this hop).
In hot-path code prefer `debug_assert!` so the check is compiled out of
release builds.

## Code shape

- **No placeholder items.** A variant, field or method reserved for a deferred
  feature is dead code, and the plan belongs in `TODO.md`. `tuner-gui` is a
  binary crate, so the compiler reports an unused item there; giving it a
  library target would silence that. A disabled control on screen is a product choice, not
  a placeholder item, but the feature behind it still has its `TODO.md` entry.
- **One method per transition.** A state change that must keep several things
  coherent lives in one method every path calls — entering Auto clears the DSP
  target, the meter's history and the string declaration together. Hand-written
  copies drift apart.
- **`#[allow]` is the last resort.** Restructure first: `if cfg!(test) { return; }`
  needs no `unreachable_code` allowance, and a closure that ignores its argument
  (`.map(|_| …)`) needs no `dead_code` one.
- **User-facing text states what the code does.** A threshold a message quotes
  comes from its constant (`format!("±{BAND_READABLE_HZ:.0} Hz")`), and a behaviour
  it describes is the one the handler implements.

## Where tests live

Three locations are in use, and the choice is decided by **what the test needs
to see**, not by file length:

- **Inline `#[cfg(test)] mod tests` with `use super::*`** — the default, and
  the only option for a test that touches module-private items. Most of the
  algorithm suites are here because they pin internal constants and helpers.
- **`src/tests/<subject>_tests.rs`** — for tests written against the
  crate-visible API (`pub(crate)` / `pub`), registered in `lib.rs`'s
  `#[cfg(test)] mod tests`. **Do not relocate tests to shorten a file**: a
  file here cannot reach module-private items, so moving a suite forces
  `pub(crate)` on everything it touches — widening the API surface to satisfy
  a layout preference, against the visibility default above. Test volume is
  not a sizing-rule concern.
- **Crate-root `tests/`** — compiles as a separate crate and reaches only
  `pub` items, so it is the home for a synthetic check that drives the public
  API and asserts a number a report states (`mat_b_recovery`,
  `unison_resolution`). Size it to the claim: a characterisation sweep belongs
  in `tuner-lab`, and an assertion that takes minutes stops being run.

**Hot-path cost belongs in `benches/`**, not in a harness that prints
microseconds: `hot-path.md` makes latency a hard rule, and a criterion bench is
what notices the number moving.

## Feature Flags vs Debug Assertions

When instrumenting the code for diagnostic logging:

- Use `#[cfg(debug_assertions)]` for simple, lightweight textual traces (e.g., `eprintln!("[ENGINE] Lock Acquired")`) that you want visible during day-to-day development but automatically stripped from `--release` builds to prevent console I/O blocking.
- Use `#[cfg(feature = "telemetry")]` for heavy structural data gathering (e.g., adding `[f32; 128]` arrays to data structures) required for offline mathematical analysis and Python plotting. Because DSP must be tested in `--release` mode to prevent audio dropouts, tying structural data to debug builds is physically unusable for acoustic analysis; the feature also keeps the arrays out of the pipeline's per-frame structures in production, and `cargo lab-telemetry` builds the optimized binary with them in.

The gate binds the audio and analysis threads, where console I/O can stall a hop.
The GUI thread is not real-time, and the app runs in release to keep its audio,
so its traces stay ungated; an error report is never a trace, on any thread.

## `#[inline]` discipline

`#[inline]` is reserved for small functions called from hot-path code where the
call overhead would be measurable (a few instructions of work, called per
sample or per bin); on everything else it defeats the purpose, so let the
compiler decide. `#[inline(always)]` is rarer still — only after a benchmark
has shown the compiler declining to inline a function that must be inlined for
the surrounding optimisation to hold (typically tight numerical loops in
`algorithms/`).
