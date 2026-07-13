# Style & Idioms

The project follows standard Rust conventions. This file collects the
few style choices that have come up often enough to be worth writing
down. Memory and allocation discipline is covered in
[`03-dsp-pipeline.md`](03-dsp-pipeline.md) and
[`02-cross-thread-communication.md`](02-cross-thread-communication.md);
this file is purely about code shape.

## Naming and layout

- `snake_case` for functions and modules, `CamelCase` for types,
  `SCREAMING_SNAKE_CASE` for constants. `cargo fmt` is the source of
  truth for whitespace and import ordering.
- One concept per file. A file that hosts a struct should usually be
  named after that struct (`gatekeeper.rs` → `Gatekeeper`).
- Name a module for the specific thing it owns, by the same rule as
  functions below: a module implementing one cited method takes that
  method's acronym (`twm`, `mat`) or, when it has none, its method/author
  eponym (`jacobsen`), with the doc-comment carrying the citation. Avoid
  a broad domain word that names a whole field rather than the one
  algorithm in the file — it overclaims (implying the module owns *all*
  of that domain) and reads ambiguously at the import site.
- `pub(crate)` is the default for shared internals. Public exports
  belong only on items that frontends (the GUI or a future external
  consumer) actually need.

### Function names

Following the Rust API guidelines (no `get_`/`calculate_`/`compute_`
stutter — the noun already says what it is), `algorithms/` functions
are named for the thing itself:

- **A named algorithm or precise standard quantity → the bare name.**
  `fft`, `cspe`, `goertzel`, `jacobsen`, `rms`, `ema`, `ninos2`,
  `nhwrsf`. The doc-comment carries the citation and the units; the
  name carries the identity. Do **not** prefix these with a verb.
  This is for *uniquely-identified* methods (an acronym like `twm`/`mat`,
  or a method eponym like `jacobsen`) or *precise* DSP quantities — **not**
  broad domain words that are ambiguous at the call site. A routine that
  minimizes sensory dissonance is not `dissonance`; name it for the
  specific method.
- **A transform that fills a buffer → a descriptive output noun.**
  `magnitude_spectrum` (the complex modulus is not a named algorithm,
  just a quantity, so it gets the standard DSP noun rather than a bare
  acronym).
- **An action on data → a verb-object.** `extract_peaks`,
  `mask_peaks`, `score_candidate`, `discover`, `refine_scale`.

When in doubt, prefer the shortest name that is unambiguous at the call
site, and let the doc-comment do the explaining.

Beware the stutter the module name creates: inside `whittaker`, the smoother
is `smooth`, not `whittaker` — `whittaker::smooth(…)` reads, and
`whittaker::whittaker(…)` does not. The module already carries the identity.

### Constants ported from a paper keep the paper's symbols

A constant that comes straight out of a cited equation keeps that equation's
symbol (`B1`, `B2`, `X_STAR`, `S1`, `S2` — Giordano Eqs. 4–6), even though the
names look cryptic in isolation. Ported maths must be **auditable against its
source**: a reviewer puts the code next to the paper and checks it
symbol-for-symbol, and renaming `B1` to `ROUGHNESS_DECAY_LOW` makes that
harder, not easier. The doc-comment names the equation; the symbol names the
term.

This applies **only** to symbols lifted from a source. A constant that is ours
gets a descriptive name and states its provenance (`SCAN_MARGIN_CENTS`,
`NEGATIVE_STRETCH_TOL_CENTS`).

### Cross-module references — `use` at the top

When a module uses an item from another module, bring it into scope with
a `use` at the top of the file, not a fully-qualified `crate::…::item`
path inline in a function body. The import block is the module's
dependency manifest: a reader — or a reviewer auditing the module graph —
should see every cross-module dependency in one place, not have to grep
function bodies for `crate::`.

Follow the standard idiom for *what* to import (Rust Book, "Creating
Idiomatic `use` Paths"):

- **Functions → import the parent module** and keep the qualifier at the
  call site: `use crate::algorithms::spectral;` … `spectral::jacobsen(…)`.
  The qualifier is the point — it shows the call is not local. Importing
  the bare function (`use …::jacobsen;` … `jacobsen(…)`) hides its origin
  and is *not* an improvement on the inline path.
- **Types, traits, and constants → import the item itself**:
  `use crate::models::KeyProfile;`.
- Both at once: `use crate::algorithms::twm::{self, TwmConfig};` — the
  pattern `discovery.rs` and `curves.rs` already use.

Const-context references and one-off `#[cfg(test)]` helpers are the
pragmatic exceptions. Note this is a review convention, not a lint:
`clippy::absolute_paths` is restriction-group and too noisy to enable.

## Doc comments

Every public item in `tuner-core` carries a `///` doc comment. For
algorithm functions, the doc comment is treated as the source of truth
for the algorithm — including any non-obvious math, the source paper
or DAFx citation if applicable, and the units of the inputs and
outputs.

Inline `//` comments are reserved for code-level explanation (why this
branch, why this constant). They are not a substitute for doc
comments.

### Comments explain the code, not the conventions

Do **not** restate project rules in code — module-naming choices, layering
rules ("`models` must not depend on `algorithms`"), or any other style or
architecture decision. Those live in `docs/internals/` and the ADRs; a second
copy in a comment adds noise, goes stale the moment the rule moves, and
lectures the reader instead of informing them. The commit message and the ADR
are where a decision is *argued*; the code is where it is *applied*.

A comment earns its place only by stating something the code cannot show:

- a constraint or invariant the caller must uphold,
- why a particular branch or constant exists,
- a citation (the paper and equation a formula comes from),
- a non-obvious unit, convention, or numerical caveat.

Pointing at a spec that *governs runtime behaviour* is fine (e.g. naming which
sanctioned cross-thread crossing a channel implements). Pointing at a spec to
justify where a file lives or what it is called is not.

### Document what the code *is*, not what it replaced

Do not narrate superseded implementations ("the former `FOO = 8` hard switch
parked a trust boundary that moved the treble ±5 ¢, so we replaced it with…").
A contributor reading an algorithm needs to see its present rigor, not its
history. The history belongs in the ADR that argued the change; the code shows
the result. Superseded exposition also rots — it accretes with every revision
until the comment is longer than the maths.

Cite an ADR from the code for **provenance**, not for narrative — that is, when
the reader cannot otherwise tell where a value or a mechanism came from:

- an empirically calibrated constant (`SIGMA_LNB_COEFF` → "ours, measured;
  data in ADR 0009"),
- a technique that is ours, or that adds to or departs from its source paper
  (`mask_peaks` → ADR 0002).

Two things are **not** history and must stay:

- a **live compatibility contract** — e.g. profile entries written before a
  field existed still deserialize, and how,
- a **guard against a known-bad change** — a terse "do not lower this below X;
  it re-admits sub-harmonic locks (ADR 0006)". This is a constraint the code
  cannot express, and it stops a contributor repeating a measured failure.
  Phrase it as a rule, never as a story.

## `assert!` vs `Option` / `Result`

- `assert!` / `debug_assert!` are for programming errors —
  invariants that the caller is responsible for upholding, and whose
  violation indicates a bug in the calling code (for example, a
  caller passing a slice of the wrong length to a function with a
  documented size contract).
- `Option` / `Result` are for runtime conditions — situations the
  caller is expected to handle (file not found, ringbuf full, no
  pitch detected this hop).

In hot-path code prefer `debug_assert!` so the check is compiled out
of release builds.

## Feature Flags vs Debug Assertions

When instrumenting the code for diagnostic logging:

- Use `#[cfg(debug_assertions)]` for simple, lightweight textual traces (e.g., `eprintln!("[ENGINE] Lock Acquired")`) that you want visible during day-to-day development but automatically stripped from `--release` builds to prevent console I/O blocking.
- Use `#[cfg(feature = "telemetry")]` for heavy structural data gathering (e.g., adding `[f32; 128]` arrays to data structures) required for offline mathematical analysis and Python plotting. Because DSP must be tested in `--release` mode to prevent audio dropouts, tying structural data to debug builds is physically unusable for acoustic analysis.

## `#[inline]` discipline

`#[inline]` is reserved for small functions called from hot-path code
where the call overhead would be measurable (a few instructions of
work, called per sample or per bin). Slapping `#[inline]` on
everything defeats the purpose; let the compiler decide for ordinary
functions.

`#[inline(always)]` is rarer still — use it only after a benchmark
has shown that the compiler is declining to inline a function that
must be inlined for correctness of the surrounding optimisation
(typically tight numerical loops in `algorithms/`).
