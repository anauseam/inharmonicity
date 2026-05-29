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
