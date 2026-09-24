# `tuner-lab` — the measurement instruments

Offline harnesses that replay captured audio through the shipped algorithms and
report what they saw. Development instruments, not user features; they ship in
nothing.

```bash
cargo lab --help                    # the six subsystems
cargo lab strobe --help             # one subsystem's modes
cargo lab engine lock               # a mode
```

`cargo lab` runs in `--release`: a debug build drops audio and changes every
availability figure. `cargo lab-telemetry` adds `--features telemetry`, which
`engine dump` needs for `goertzel.csv`; the two share one binary path, so re-run
it after any plain `cargo lab`.

A mode that draws writes PNG in the platform's own sans-serif font. On Linux the
lab needs fontconfig's development files to build, beyond what the app needs
(`libfontconfig1-dev` on Debian and Ubuntu, `fontconfig-devel` on Fedora).

Most modes read the capture sets in
[`../docs/internals/capture-sets.md`](../docs/internals/capture-sets.md),
whose consumption rules bind: piano #2 is consumed through `mat regen`, never
raw `analysis.json`. `regen::load` applies the ±200 ¢ plausibility rule.

Why a crate and why six subcommands: the three-homes rule in
[`style.md`](../docs/internals/style.md). The old-invocation rename map is
below; the move itself is in git history (2026-09).

## What is *not* here

A mode that **asserts** a number a report states is an integration test in
[`../tuner-core/tests/`](../tuner-core/tests); a mode that **times** the hot
path is a criterion bench in
[`../tuner-core/benches/`](../tuner-core/benches). Only a mode that *reports*
belongs here.

| Where | What | Reproduces |
| --- | --- | --- |
| `tuner-core/tests/mat_b_recovery.rs` | B recovered from known synthetic truth; the ±10 % seed cliff and the octave trap | report 0006 |
| `tuner-core/tests/unison_resolution.rs` | the `2/T` resolution law, sharp transition, exact above ~1.6× | report 0012 §4 (E6) |
| `tuner-core/benches/strobe_cost.rs` | per-hop bank cost and `resolve_lines` at the ring cap, against the 23.2 ms budget | report 0012 (E9) |

## `engine` — discovery

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `lock [ROOT]` | end-to-end auto-lock validation, M-of-N included | report 0010; piano-1 **81/87** | a capture set |
| `from-onset [ROOT]` | Stage-A's winner on every hop from the onset, gate ignored — what the `Stable` wait buys, per register | report 0003 (2026-09-02 amendment) | a capture set (needs `audio_full_event.raw`) |
| `dump <AUDIO>` | per-frame STFT, peaks and TWM scores to `spectrum.csv` / `peaks.csv`; `goertzel.csv` under `cargo lab-telemetry` | report 0002 | one `.raw` |
| `reach [NAME Q R RHO]…` | 1 ¢-resolution detuning reach of the lock | report 0006 pitch-raise reach (canonical 78 ¢, conservative 69 ¢) — re-run after the jacobsen fix reproduced both exactly, so the harness is peak-estimator-independent | none (synthetic) |
| `nsga2 [--serve]` | the synthetic dataset generator and discovery fitness harness | report 0001; `--serve` is `tuner-lab/scripts/optimize_twm.py`'s protocol | none (synthetic) |

### Running the NSGA-II sweep

The methodology, the synthetic signal model, and the threats-to-validity audit
are [`reports/nsga2/README.md`](../reports/nsga2/README.md); this is how to
drive it.

```bash
# 1. Build the evaluator (release; the sweep is hours even parallelised).
cargo build --release -p tuner-lab

# 2. Run the 5-arm sweep. Refuses to clobber an existing db (see resume guard).
python3 tuner-lab/scripts/optimize_twm.py            # fresh run (errors if twm_nsga2.db exists)
python3 tuner-lab/scripts/optimize_twm.py --resume   # continue existing studies

# 3. Validate a candidate config on the REAL captures (the decision gate).
python3 tuner-lab/scripts/validate_config.py --refine --config "0.5 3.88 1.426 0.298 18"
```

The evaluator can also be driven directly (one trial per stdin line):

```bash
./target/release/tuner-lab engine nsga2 --serve
# then write:  "<mode> <p> <q> <r> <rho> <lambda>\n"   mode ∈ {refine, discrete}
# reads back:  {"objA":..., "objB":..., "fl_bass":..., ...}
```

### Reproduction checklist

- [ ] Evaluator rebuilt (`cargo build --release -p tuner-lab`).
- [ ] Dataset fingerprint == `e11fea90889dee30` (run `cargo lab engine nsga2`).
- [ ] Sanity: M&B default → overall prod_fl ≈ 0.308 (asserted by the orchestrator).
- [ ] `twm_nsga2.db` absent (fresh) or `--resume` intended.
- [ ] Python deps: `pip install optuna==4.9.0` (Optuna pinned) in the .venv.
- [ ] Final config decided on **real** captures (overall pass-count, no register
      regressed), not synthetic hypervolume.

## `gatekeeper` — the signal validator

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `dump <AUDIO>` | per-frame validator metrics to `gatekeeper.csv` | report 0003 | one `.raw` |
| `plot <PATH>` | the gate's verdict drawn per capture to `gatekeeper.png` — the waveform shaded by state; NHWRSF, RMS and sustain stability against their thresholds — and the wait from onset to `Stable` per register | — | one capture or a set; `--profile`, `--silence`, `--nhwrsf`, `--sustain` and `--out` |
| `sparsity [ROOT]` | our sustain-stability gate against faithful Mounir NINOS² variants | faithfulness audit 05 | a capture set |

The third gatekeeper measurement — what the `Stable` wait actually buys — is
`engine from-onset`, which needs the engine's own Stage-A scan.

## `mat` — the Worker's (f₀, B) estimator

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `validate [ROOT]` | measured B against the Rigaud prior for both trajectory orders, plus goodness-of-fit | report 0006; discrete **76/87**, refined **77/87** | a capture set |
| `offset [ROOT] --offsets <MS>` | each capture re-measured at offsets from the *physical* onset, paired and read against the set's repeat scatter | report 0009 analysis 9 | a capture set (needs `audio_full_event.raw`) |
| `repeats <REGEN>` | repeat-capture noise decomposition: σ_lnB, ρ reproducibility, strike strength | report 0009 | a `mat regen` dump |
| `regen [ROOT]` | per-key partials re-derived from the kept audio, one JSON dump on stdout | **the required entry point for piano #2** (`capture-sets.md`) | a capture set |
| `recovery` | MAT against known synthetic B, swept 1×–25× the prior, with and without a fundamental | report 0006 (its assertion is `tuner-core/tests/`) | none (synthetic) |

## `curve` — the tuning-curve engines

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `compare <REGEN> [--json OUT]` | every engine: stretch tables, implied beat rates, leave-one-key-out error, Giordano cross-scoring, curvature, flag counts | reports 0007–0009; the curve-side goldens | a `mat regen` dump |
| `auralize <REGEN> [--out DIR]` | each candidate curve rendered to a loudness-matched WAV by offline additive resynthesis | the perceptual selection evidence | a `mat regen` dump |

## `strobe` — the bank, and the number the panel displays

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `replay [ROOT]` | rotation fidelity E1–E5, unison assist E6–E8, bass attribution E10–E12 | report 0011 (**E1–E5 must not move**), report 0012, report 0013 | a capture set |
| `isolation <REGEN> <ROOT>` | the panel against isolation truth: false-beat control, availability, per-capture line positions (`--json`) | report 0014 §§3–5; truth side is `tuner-lab/scripts/isolation_truth.py` | the mute-isolation set, which must carry `metadata.sounding_strings` |
| `truth [ROOT]` | the shipped readout against the hi-res DFT truth and YIN, per capture | the guitar strobe frequency audit | a capture set, or one capture |
| `selftest` | bias of the reference estimators themselves, on synthetic tones | calibrates the instrument; characterisation | none (synthetic) |
| `inharm` | YIN's sharpness against B and partial richness | the mechanism behind the audit's verdict | none (synthetic) |
| `detune` | the bounded read swept across detuning | characterisation | none (synthetic) |
| `alias [ROOT]` | per-hop reading either side of the phase-vocoder alias boundary | report 0011 | a capture set |
| `window [ROOT]` | longest ungated run and the band read, per fit-window length | report 0011 | a capture set |
| `readout [ROOT]` | tracker as-is / tracker + Defect-1 window / bounded spectral peak | report 0011 | a capture set |
| `chatter [ROOT]` | whether the band/coarse regime switch chatters near its boundary | report 0011 (T6, `READOUT_SWITCH_HOPS`) | a capture set |
| `policy [ROOT] [--measured-b]` | fixed n* (register table) against strongest-margin-per-hop | report 0011 | a capture set |
| `fixed-n [ROOT]` | the register table re-run on the capture's own measured B | report 0011 | a capture set |
| `bass-partials [ROOT]` | partial-centered bass read at each partial's prior-B target, against a pre-registered criterion | report 0011 | a capture set |
| `reach [ROOT]` | how far off pitch the coarse read still reads | report 0011 | a capture set |

## `gates` — the detection thresholds

`config.silence_threshold` is calibrated once and thresholded at three hot-path
detectors, two in `engine.rs` and one in `strobe.rs`
([report 0015](../reports/retiring/0015-ambient-sigma-gates-measured.md) §Context); the
OS-CFAR family that could replace it is measured against the same reference.

| Mode | What it reports | Reproduces | Input |
| --- | --- | --- | --- |
| `ambient --set <LABEL> <REGEN> <ROOT>…` | per hop and per partial, signal against the noise beside it in the *same* window, all three gates over a σ sweep. Repeatable, so populations are directly comparable | report 0015 | one or more (dump, set root) pairs |
| `profile [ROOT]` | per-key × per-partial profile under the settled gate, and how near each cell is to flipping | audit 13 (T1) | a capture set |
| `pfa [ROOT]` | realized false-alarm rate on signal-free input | audit 13 (T3) | a capture set |
| `refset [ROOT]` | reference-set anatomy: valley cell or weak partial's lobe, and what the guard buys (Rohling §V) | audit 13 (T5) | a capture set |
| `verify [ROOT]` | the shipped gate against the harness's replica of it | report 0015 §4 | a capture set |
| `ab [ROOT] [--partial N]` | the same bounded read under the shipped ambient-σ gate and four OS-CFAR variants | report 0011 §4, report 0015 | a capture set |

`ambient`'s AWGN calibration row (report 0015 §4) needs a synthetic dump directory
that is not in the repository and has no generator in the tree; it is the one
reproduce line here that cannot be run from a clean checkout.

## What a capture writes

Every capture produces three files in a subdirectory named
`key_<index>_<note>_<timestamp>` (e.g. `key_001_A#0_1752264903/`). The timestamp
suffix means repeat captures of one key are all retained rather than overwriting
each other, which is what the repeat-capture experiments consume.

Those sit under the instrument's own dump directory — `diagnostics/<instrument
id>/`, keyed on the opaque `identity.id`, so two instruments never share a
directory and renaming an instrument moves nothing. Each carries an
`instrument.json` (id, name, make, model, serial, kind).

| File | Contents |
| --- | --- |
| `audio.raw` | The strictly causal, stable audio buffer that triggered the capture — raw `f32` samples, no header. 1.5 s unless the session raised the capture duration, and shorter where the note decayed first. |
| `audio_full_event.raw` | The non-causal diagnostic buffer: ~348 ms of pre-roll, the hammer strike, and the decay. Always 1.5 s — the capture-duration control moves the stable record only. |
| `analysis.json` | The Worker's analytical telemetry for that capture. |

A longer `audio.raw` is **stored** audio, not measured audio: the Worker
analyses the first `CAPTURE_ANALYSIS_SAMPLES` (1.5 s) whatever the record's
length, so measurements stay comparable across the sets. `mat regen` bounds
itself the same way, deliberately — reproducing the shipped measurement is its
job. A harness that wants the whole record (decay τ, deep-bass resolution) reads
the file directly and says so.

`analysis.json`'s `metadata.sounding_strings` carries the operator's declaration
of which of the key's strings were sounding, or `null` where none was made —
which is every capture outside the mute-isolation set. `mat regen` passes it
through.

`metadata.noise_floor`, `nhwrsf_threshold` and `sustain_stability_threshold` are
the gate's three thresholds as the capture ran. Captures taken before the last
two were logged carry only `noise_floor`, so `gatekeeper plot` fills the others
from `--profile` or the gate's defaults and prints which it used.

## Shared plumbing

| Module | Owns |
| --- | --- |
| `capture` | finding captures (either layout), key identity from `analysis.json`, the register labels |
| `figure` | the PNG canvas, fonts, colours and time charts a drawing mode uses |
| `raw` | headerless `f32` dumps |
| `regen` | the `mat regen` schema, and the piano-2 ±200 ¢ rule as a function the caller cannot forget |
| `truth` | the hi-res DFT reference, YIN, synthetic tones, and the gate models read through them |

Three register splits exist and each names its consumer:
`capture::strobe_register` (four-band, the unison summaries),
`capture::curve_register` (three-band, the curve tables), and a third private
to `gatekeeper::sparsity`'s AUC table. Two `median` conventions likewise —
averaging the middle pair (`curve compare`, `strobe isolation`) versus the upper
element (everything else) — each private to its module.

The ±200 ¢ rule guards a *cached* `measured_f0`, so it lives in `regen::load`,
the one path that reads cached values. `mat validate` seeds MAT from the ET
frequency and `gatekeeper sparsity` reads audio and a key index, so neither
consumes the field it guards.

## Rename map — the old `--example` invocations

Kept because a reader coming from a pre-2026-09 record, script or note needs
to find the mode again.

Reference for the migration, not part of the decision: every old
invocation and what replaces it.

| Old | New |
| --- | --- |
| `--example validate_engine_lock -- [DIR]` | `cargo lab engine lock [DIR]` |
| `--example validate_engine_lock -- DIR --from-onset` | `cargo lab engine from-onset DIR` |
| `--example diagnose_engine -- FILE [flags]` | `cargo lab engine dump FILE [flags]` |
| `--example diagnose_engine -- FILE --config "p q r rho l"` | `cargo lab engine dump FILE --config "p q r rho l"` |
| `--example pitch_reach_sweep [-- name q r rho]` | `cargo lab engine reach [name q r rho]` |
| `--example nsga2_evaluator [-- --serve]` | `cargo lab engine nsga2 [--serve]` |
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
| `--example twm_breakdown` | **removed** in the 2026-09 crate move |
| `--example joint_b_refine_diagnostic` | **removed** in the 2026-09 crate move |

`--keys`, `--span`, `--min-bins` and `--fft` are unchanged in name and default
wherever they applied before.
