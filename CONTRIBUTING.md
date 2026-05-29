# Contributing

Thanks for your interest in `inharmonicity`. This file is a pointer
sheet — it explains where the documentation lives and how to do the
mechanical parts of contributing (build, lint). Substantive
context lives in the docs it points at.

## Docs hierarchy

| Doc                                                                        | Audience    | Purpose                                                                |
| -------------------------------------------------------------------------- | ----------- | ---------------------------------------------------------------------- |
| [`README.md`](README.md)                                                   | Users       | What the project is, how to run it, current status.                    |
| [`ARCHITECTURE.md`](ARCHITECTURE.md)                                       | Maintainers | Why the project is shaped the way it is (design rationale, tradeoffs). |
| [`docs/internals/`](docs/internals/)                                       | Maintainers | Internal architecture, constraints, and hardware contracts.            |
| [`docs/internals/suspected-issues.md`](docs/internals/suspected-issues.md) | Maintainers | Descriptive notes on unreproduced defensive code.                      |

If you're touching DSP code, [`docs/internals/03-dsp-pipeline.md`](docs/internals/03-dsp-pipeline.md)
and [`docs/internals/04-algorithms-and-models.md`](docs/internals/04-algorithms-and-models.md)
are the relevant guidelines. If you're touching cross-thread state,
[`docs/internals/02-cross-thread-communication.md`](docs/internals/02-cross-thread-communication.md)
is the contract.

## Building and running

```bash
# Build everything
cargo build

# Run the GUI
cargo run -p tuner-gui

# Run tests
cargo test

# Run a single example (see tuner-core/examples and tuner-gui/examples)
cargo run -p tuner-gui --example dashboard_test
```

## Code style

```bash
# Format
cargo fmt --all

# Lint
cargo clippy --workspace --all-targets -- -D warnings
```

`cargo fmt` is the source of truth for whitespace and import ordering.
Clippy warnings are treated as errors in CI. The narrative style guidelines
(naming, doc comments, `#[inline]` discipline) are in
[`docs/internals/05-style.md`](docs/internals/05-style.md).

## Reporting issues

Open an issue on the [GitHub repository](https://github.com/anauseam/inharmonicity).
For diagnostic-capture-related bugs, please include the
`diagnostics/audio.raw` and `diagnostics/analysis.json` files
produced for the offending note if you can — they make the difference
between a half-day repro and a five-minute fix.

For other inquiries, see the contact note at the bottom of the
[README](README.md).
