# ADR 0016 — The measurement harnesses become a crate

## Status

**Accepted, 2026-09-03.** Structural; no shipped behaviour changes and no
measured figure moves. Supersedes nothing.

## Context

`tuner-core/examples/` held 17 harnesses, **14 900 lines**. Cargo defines
`examples/` as *example uses of the library's functionality* — what someone opens
to learn the API. None of these are that: they are validation instruments that
produced the evidence in ADRs 0001–0015 and the audit series, and they read
capture sets that are not in the repository.

The cost was not only naming. `cargo build --examples` — which `--all-targets`
implies, so every lint and test run — compiled all of it. And because `examples/`
cannot share a module except by each example compiling its own copy,
`examples/common/` needed a blanket `#![allow(dead_code)]`, switching off the
compiler's only check on plumbing nobody calls. Meanwhile Cargo's three homes for
this work sat unused: `tuner-core/tests/` existed and was **empty**, neither crate
had a `benches/`, and all 152 `#[test]`s were unit tests inside `src/`.

## Decision

**Three homes, sorted by what a mode does rather than what it is about.**

- **`tuner-core/tests/`** — a mode that **asserts** a number an ADR states. These
  run under `cargo test`, and when the algorithms split into their own crate they
  travel with the code rather than the tooling.
- **`tuner-core/benches/`** — a mode that **times** the hot path. `03` makes
  latency a hard rule and nothing enforced it: the ADRs' cost figures came from
  ad-hoc `Instant` timing, which reports once and cannot notice a number moving.
- **`tuner-lab/`** — a mode that **reports**: measurement against captured data,
  which has no pass/fail and no home in Cargo's test model. A `publish = false`
  workspace member — the mechanism `cargo xtask` formalised, and what tokio, bevy,
  wgpu and symphonia (`symphonia-check`) use for the same reason.

One binary, six subcommands — `engine`, `gatekeeper`, `mat`, `curve`, `strobe`,
`gates` — each a module directory whose files are the old harnesses, `clap`
derive for the CLI, `--help` at each level as the index. Five name the subsystem
they measure; **`gates` names none**, because the one calibrated ambient scalar
is thresholded at three hot-path detectors in *two* modules
([ADR 0015](0015-ambient-sigma-gates-measured.md) §Context), so filing that study
under either would misstate what it covers.

The lab depends on **`tuner-core` only, never `tuner-gui`**, and drives the public
API: a harness needing something private has found an API question, and a decision
that needs replaying belongs in `tuner-core` rather than in a second copy inside a
harness. Shared plumbing (`capture`, `raw`, `regen`, `truth`) is compiled once, so
dead code is a warning again. `cargo lab` bakes in `--release`, retiring the
"debug builds drop audio" footgun from every reproduce line; docs cite the alias,
never `-p tuner-lab`, so the planned rename of `tuner-core` moves the lab by
editing one line. `default-members` keeps `cargo build` product-only while
`--workspace` still covers the lab.

**Two harnesses were removed rather than moved**: `twm_breakdown` (uncited; TWM
closed with ADR 0006 and `engine dump` already dumps the per-frame scorer) and
`joint_b_refine_diagnostic` ([ADR 0006](0006-discovery-refinement-validation.md)
records it tested and rejected). Git history is the archive.

## Consequences

- **No measured figure moved.** All 51 documented invocations ran from the clean
  `HEAD` tree, then again after, diffed byte for byte across all four capture
  sets: 29 identical outright, and every difference in the other 22 falls into
  four named classes — the banner below, E9's departure, the output *paths*
  passed to three modes (whose WAVs, `curve_report.json` and `panel.json` are
  themselves identical), and `engine mobo`'s two wall-clock lines. The anchors
  hold: `engine lock` piano-1 **81/87**, MAT **76/87** and **77/87**, `strobe
  replay` E1–E5 and E10–E12, `curve compare`'s goldens, `mat offset`, `engine
  from-onset`, `gates ambient`'s three-population run, and the dump modes' CSVs
  including the 5761-line `goertzel.csv`. No runtime moved past the ±25 % two
  runs of identical work show on this machine.
- **E9 left `strobe replay` for a bench.** It printed wall-clock microseconds —
  the one block that could never be diffed; two runs of the *unmodified* harness
  disagreed by 3×. Removing it made E1–E12 byte-reproducible for the first time.
- **Three ADR figures gained assertions** (ADR 0006's recovery rows and seed
  cliff, ADR 0012 §4's resolution law), sized to the claim rather than the sweep:
  7.3 s on a plain `cargo test`, against 48 s had the sweeps moved wholesale. The
  sweeps stay in the lab — an assertion is not an instrument.
- **One header moved, in 15 runs.** `pitch_ground_truth` printed its
  "app vs truth vs yin" banner *before* dispatching, so all eighteen of its modes
  carried a preamble describing only the default one. It now belongs to the mode
  it describes; those diffs are that banner and nothing else.
- **A second alias, `cargo lab-telemetry`, was forced by the first**: everything
  after `lab`'s `--` goes to the binary, so `--features` cannot ride on it, and
  `engine dump` writes `goertzel.csv` only with the feature compiled in.
- **Reproduce lines changed shape** in 16 places across the ADRs, 9 in
  `scripts/`, and the internals docs; the map is below.
  [`tuner-lab/README.md`](../../tuner-lab/README.md) replaces
  `tuner-core/examples/README.md`, and `tuner-core/examples/` is removed.

## Rename map

Reference for the migration, not part of the decision: every old
invocation and what replaces it.

| Old | New |
| --- | --- |
| `--example validate_engine_lock -- [DIR]` | `cargo lab engine lock [DIR]` |
| `--example validate_engine_lock -- DIR --from-onset` | `cargo lab engine from-onset DIR` |
| `--example diagnose_engine -- FILE [flags]` | `cargo lab engine dump FILE [flags]` |
| `--example diagnose_engine -- FILE --config "p q r rho l"` | `cargo lab engine dump FILE --config "p q r rho l"` |
| `--example pitch_reach_sweep [-- name q r rho]` | `cargo lab engine reach [name q r rho]` |
| `--example mobo_evaluator [-- --serve]` | `cargo lab engine mobo [--serve]` |
| `--example diagnose_gatekeeper -- FILE` | `cargo lab gatekeeper dump FILE` |
| `--example sparsity_ab [-- DIR]` | `cargo lab gatekeeper sparsity [DIR]` |
| `--example validate_mat [-- DIR]` | `cargo lab mat validate [DIR]` |
| `--example validate_mat -- DIR --offset-ms 0,116,300` | `cargo lab mat offset DIR --offsets 0,116,300` |
| `--example repeat_noise -- P.json` | `cargo lab mat repeats P.json` |
| `--example regenerate_partials [-- DIR]` | `cargo lab mat regen [DIR]` |
| `--example mat_b_recovery` | `cargo lab mat recovery` (assertions: `cargo test -p tuner-core --test mat_b_recovery`) |
| `--example curve_compare -- P.json [--json OUT]` | `cargo lab curve compare P.json [--json OUT]` |
| `--example auralize -- P.json [--out DIR]` | `cargo lab curve auralize P.json [--out DIR]` |
| `--example strobe_replay [-- DIR]` | `cargo lab strobe replay [DIR]` |
| `--example strobe_replay` (E6) | unchanged, plus `cargo test -p tuner-core --test unison_resolution` |
| `--example strobe_replay` (E9) | `cargo bench -p tuner-core --bench strobe_cost` |
| `--example isolation -- R.json DIR [--json OUT] [--all] [--noise-floor V]` | `cargo lab strobe isolation R.json DIR [--json OUT] [--all] [--noise-floor V]` |
| `--example pitch_ground_truth -- DIR` | `cargo lab strobe truth DIR` |
| `--example pitch_ground_truth -- --selftest` | `cargo lab strobe selftest` |
| `--example pitch_ground_truth -- --inharm` | `cargo lab strobe inharm` |
| `--example pitch_ground_truth -- --detune [--span C] [--min-bins B] [--flank-hz H]` | `cargo lab strobe detune [--span C] [--min-bins B] [--flank-hz H]` |
| `--example pitch_ground_truth -- DIR --alias` | `cargo lab strobe alias DIR` |
| `--example pitch_ground_truth -- DIR --window` | `cargo lab strobe window DIR` |
| `--example pitch_ground_truth -- DIR --readout` | `cargo lab strobe readout DIR` |
| `--example pitch_ground_truth -- DIR --chatter` | `cargo lab strobe chatter DIR` |
| `--example pitch_ground_truth -- DIR --policy [--measured-b]` | `cargo lab strobe policy DIR [--measured-b]` |
| `--example pitch_ground_truth -- DIR --measured-b` | `cargo lab strobe fixed-n DIR` |
| `--example pitch_ground_truth -- DIR --bass-partials` | `cargo lab strobe bass-partials DIR` |
| `--example pitch_ground_truth -- DIR --reach` | `cargo lab strobe reach DIR` |
| `--example pitch_ground_truth -- DIR --cfar-profile [--max-n N]` | `cargo lab gates profile DIR [--max-n N]` |
| `--example pitch_ground_truth -- DIR --pfa --fft 8192` | `cargo lab gates pfa DIR --fft 8192` |
| `--example pitch_ground_truth -- DIR --refset [--keys K,…]` | `cargo lab gates refset DIR [--keys K,…]` |
| `--example pitch_ground_truth -- DIR --verify-shipped` | `cargo lab gates verify DIR` |
| `--example pitch_ground_truth -- DIR --gate-ab [--partial N]` | `cargo lab gates ab DIR [--partial N]` |
| `--example pitch_ground_truth -- --np-set L R.json DIR …` | `cargo lab gates ambient --set L R.json DIR …` |
| `--example pitch_ground_truth -- --np-live 20 --np-set …` | `cargo lab gates ambient --live 20 --set …` |
| `--example twm_breakdown` | **removed** (ADR 0016) |
| `--example joint_b_refine_diagnostic` | **removed** (ADR 0016) |

`--keys`, `--span`, `--min-bins` and `--fft` are unchanged in name and default
wherever they applied before.
