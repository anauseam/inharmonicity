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
- `pub(crate)` is the default for shared internals. Public exports
  belong only on items that frontends (the GUI or a future external
  consumer) actually need.

### Function names

Following the Rust API guidelines (no `get_`/`calculate_`/`compute_`
stutter — the noun already says what it is), `algorithms/` functions
are named for the thing itself:

- **A named algorithm or standard quantity → the bare name.**
  `fft`, `cspe`, `goertzel`, `jacobsen`, `rms`, `ema`, `ninos2`,
  `nhwrsf`. The doc-comment carries the citation and the units; the
  name carries the identity. Do **not** prefix these with a verb.
- **A transform that fills a buffer → a descriptive output noun.**
  `magnitude_spectrum` (the complex modulus is not a named algorithm,
  just a quantity, so it gets the standard DSP noun rather than a bare
  acronym).
- **An action on data → a verb-object.** `extract_peaks`,
  `mask_peaks`, `score_candidate`, `discover`, `refine_scale`.

When in doubt, prefer the shortest name that is unambiguous at the call
site, and let the doc-comment do the explaining.

## Doc comments

Every public item in `tuner-core` carries a `///` doc comment. For
algorithm functions, the doc comment is treated as the source of truth
for the algorithm — including any non-obvious math, the source paper
or DAFx citation if applicable, and the units of the inputs and
outputs.

Inline `//` comments are reserved for code-level explanation (why this
branch, why this constant). They are not a substitute for doc
comments.

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
