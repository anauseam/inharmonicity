# Contributing

Thanks for your interest in `inharmonicity`. This file covers the mechanics of
contributing (building, linting, reporting issues), the three rules a change
resting on a measurement must meet, and where the rest of the documentation
lives.

## Docs hierarchy

| Doc | Audience | Purpose |
| --- | --- | --- |
| [`README.md`](README.md) | Users | What the project is, how to run it, current status. |
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Contributors | How the program is built: the threads, the codemap, the invariants, and the decisions that shape it. |
| [`docs/internals/`](docs/internals/) | Contributors | The binding contracts: thread crossings, hot path, layering, style, capture sets. |
| [`docs/design/`](docs/design/) | Contributors | Proposals still being argued, and the template for a new one. |
| [`reports/`](reports/) | Reviewers | The evidence behind the decisions, and the methods standard. |
| [`tuner-lab/README.md`](tuner-lab/README.md) | Reviewers | The measurement harnesses, and what each one reproduces. |
| [`TODO.md`](TODO.md) | Everyone | The backlog, with what each item is blocked on. |

If you're touching DSP code, [`docs/internals/hot-path.md`](docs/internals/hot-path.md)
and [`docs/internals/layering.md`](docs/internals/layering.md)
are the relevant guidelines. If you're touching cross-thread state,
[`docs/internals/thread-crossings.md`](docs/internals/thread-crossings.md)
is the contract.

## Evidence, when you are changing a number

Three rules bind any change that rests on a measurement. The full methods
standard — pre-registration, nulls, the confirmatory/exploratory split, and what
the record is for — is [`reports/README.md`](reports/README.md).

### Anchor a threshold to a measured quantity

A pre-registered bar is only honest if the number in it came from somewhere. In
order of preference:

1. **A prior measurement on this project's own data** — "≤ 5 %, because report 0012
   §5 measured 4 % on this instrument".
2. **A derivation from a published criterion** — `UNISON_MIN_BINS = 25` is
   Rohling §V solved for record length, not a hand-picked floor.
3. **The estimator's own scatter** — "within σ and the known bias", where both
   were measured.

An invented percentage is none of these. If no anchor exists, the bar is a
**product judgment** and must be labelled as one, with a human making the call
rather than an analysis appearing to derive it. That is not a defect; pretending
otherwise is.

This is the inference-side twin of [`layering.md`](docs/internals/layering.md)'s
Topological Scrutiny Test, which bans fragile magic numbers in the *code*. The
same reasoning applies to the numbers in an *argument*.

### Two instruments cannot select a configuration

The full rule, with the capture sets it applies to, is in
[`capture-sets.md`](docs/internals/capture-sets.md) — "Validation only". The short form: with n = 1 or 2
instruments, a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence. Report per-register counts and which keys moved. Never tune on the
validation instruments, and never recalibrate the synthetic generator to match
them.

Lock-accuracy scores are **relative**: a higher score beats a lower one *on that
instrument*, and neither is an accuracy claim about pianos.

### A threshold-dependent question has no threshold-free answer

If the quantity you are measuring is defined by a threshold, the answer inherits
the threshold, and quoting a single number hides that.

The live example: "how long does the note stay above the noise floor?" has no
answer, because `noise_floor` is not measured. It is the silence threshold, 1.5 ×
the loudest room level seen at calibration or whatever the operator sets, which
three detection gates then scale by a Neyman–Pearson factor. The decay stop that
ends a capture is the same kind of human-calibrated threshold.

The way out is one of:

- report the **trajectory** rather than a crossing time — no threshold needed;
- report crossings against **several named references** as a family, so the
  reader sees the spread the choice produces;
- use a **dimensionless** reference (−20/−40/−60 dB below a signal's own peak),
  which survives a gain change.

Naming the reference is not a caveat. It is the result.

## Building and running

```bash
# Build the product (the lab is a workspace member, not a default one)
cargo build

# Run the GUI, in release: a debug build is too slow for the microphone and drops audio
cargo run --release -p tuner-gui

# Run tests
cargo test --workspace

# Time the hot path against the audio callback's budget
cargo bench -p tuner-core

# Run a measurement harness (see tuner-lab/README.md; `cargo lab` builds in release)
cargo lab engine lock
```

## Code style

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --workspace --all-targets -- -D warnings

# Docs: a broken doc link fails the build
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --document-private-items
```

`cargo fmt` is the source of truth for whitespace and import ordering.
Clippy and rustdoc warnings are treated as errors. The narrative style guidelines
(naming, doc comments, `#[inline]` discipline) are in
[`docs/internals/style.md`](docs/internals/style.md).

## Reporting issues

Open an issue on the [GitHub repository](https://github.com/anauseam/inharmonicity).
For a bug in a capture, please attach that capture's folder if you can. The
diagnostics directory's path is printed at startup
(`~/.local/share/inharmonicity/diagnostics/` on Linux); it holds one folder per
instrument and one `key_…` folder per capture, with the capture's `audio.raw`,
`audio_full_event.raw` and `analysis.json`. Those files make the difference
between a half-day repro and a five-minute fix.
