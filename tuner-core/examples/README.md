# Developer harnesses

Offline tools for exercising the DSP without launching the GUI. These are
**development instruments, not user features** — they read the diagnostic files
a capture leaves on disk and print or dump what the algorithms actually saw.

Run them with `--release`: debug builds drop audio and change every availability
figure. Some read the capture sets described in
[`../../docs/internals/06-capture-sets.md`](../../docs/internals/06-capture-sets.md),
whose consumption rules are binding — in particular, piano #2 must be consumed
through `regenerate_partials`, never through raw `analysis.json`.

## What a capture writes

Every capture produces three files in its own subdirectory of `diagnostics/`,
named `key_<index>_<note>_<timestamp>` (e.g. `diagnostics/key_001_A#0_1752264903/`).
The timestamp suffix means repeat captures of one key are all retained rather
than overwriting each other, which is what the repeat-capture experiments
consume.

| File | Contents |
| --- | --- |
| `audio.raw` | The strictly causal, stable audio buffer that triggered the capture — raw `f32` samples, no header. |
| `audio_full_event.raw` | The non-causal diagnostic buffer: ~348 ms of pre-roll, the hammer strike, and the decay. |
| `analysis.json` | The Worker's analytical telemetry for that capture. |

Offline tools discover dumps by the `key_` prefix and read the key identity from
`analysis.json`, so the directory name is not load-bearing.

`analysis.json`'s `metadata.sounding_strings` carries the operator's declaration of which
of the key's strings were sounding, or `null` where none was made — which is
every capture outside the mute-isolation set. `regenerate_partials` passes it
through.

## The harnesses

### Engine and discovery

- **`diagnose_engine`** — replays a `.raw` capture through the engine and dumps
  what the STFT, peak extractor and TWM scorer saw at every frame into
  `spectrum.csv` and `peaks.csv`. With `--features telemetry` it also writes
  `goertzel.csv` (per-partial amplitude and Neyman–Pearson threshold per
  tracking frame). Drives the engine in *manual* mode.

  ```bash
  cargo run --release --example diagnose_engine -- diagnostics/key_001_A#0_1752264903/audio_full_event.raw
  ```

- **`validate_engine_lock`** — the same path in *auto* mode: end-to-end
  validation of the shipped discovery lock, including the M-of-N rule.
- **`twm_breakdown`** — decomposes a TWM score into its forward, reverse and
  normalized error terms for one candidate.
- **`pitch_reach_sweep`** — the 1 ¢-resolution detuning sweep behind ADR 0006's
  pitch-raise-reach figures.
- **`mobo_evaluator`** — the synthetic dataset generator and discovery fitness
  harness from the MOBO parameter-tuning effort (ADR 0001).
- **`joint_b_refine_diagnostic`** — offline test of asymmetric per-candidate
  (f₀, B) refinement (ADR 0006 fix-path step 3; refuted on real data).

### Measurement

- **`validate_mat`** — replays the Worker's MAT (f₀, B) estimator over every
  capture and prints measured inharmonicity against the Rigaud prior for both
  trajectory orders, plus a ground-truth-free goodness-of-fit comparison.

  ```bash
  cargo run --release --example validate_mat
  ```

- **`regenerate_partials`** — re-derives per-key partials from the kept audio
  with the current estimator, emitting one JSON dump. **This is the required
  entry point for piano #2 data**; most curve tooling consumes its output rather
  than `analysis.json`.

  ```bash
  cargo run --release --example regenerate_partials -- diagnostics > p2.json
  ```

- **`mat_b_recovery`** — characterises MAT against *known* synthetic B, the
  ground-truth stress test behind the deep-bass measurement argument.
- **`repeat_noise`** — the repeat-capture noise decomposition (σ_lnB, ρ
  reproducibility, strike strength) behind ADR 0009.
- **`pitch_ground_truth`** — computes an independent hi-res DFT pitch truth per
  capture. This, not `measured_f0`, is the reference for estimator-accuracy work.

### Curve and strobe

- **`curve_compare`** — runs all four curve engines on a regenerated-partials
  dump and prints the full diagnostic set: stretch tables, implied beat rates,
  leave-one-key-out prediction error, Giordano cross-scoring, curvature, and
  detector flag counts. The curve-side goldens.

  ```bash
  cargo run --release --example curve_compare -- p2.json
  ```

- **`strobe_replay`** — runs the shipped `Strobe` bank over real captures and
  reports rotation fidelity: detuning coherence (E1), the bass-window A/B (E2),
  per-hop delta noise behind the readable-range margin (E3), fit-window jitter
  versus motion lag (E4), and the shipped rate against an independent refit (E5).
  Also has `--chatter` and `--refset` modes.

  **E6–E9 are unison assist** (ADR 0012): the estimator against synthetic truth
  — resolution law, accuracy, the level/separation surface and the false-split
  null (E6); availability, repeat
  reproducibility and the discriminator's verdict on real captures (E7); every
  reported line matched against a full-rate DFT of the identical span, to
  attribute the unexplained ones (E8); and per-hop cost in `--release` against
  the callback budget (E9). E6 and E9 are synthetic and run whatever directory
  is passed.

  **E10–E12 are the bass attribution** (ADR 0013): the null, the resolution law
  and the baseband noise correlation in the deep bass's own 4096-sample window,
  the folded interferer's strength, and a register sweep of neighbour leakage
  (E10); whether the extra lines recur at fixed absolute frequencies across
  different keys (E11); and every extra line against the families that could
  have produced it — the struck key's own partials, the neighbouring keys',
  Conklin's mixing products — each scored against a permutation null, plus the
  fitted `Δ ∝ f^p` law and the flanking pair's symmetry (E12). E10 is synthetic.

  E1–E5 are ADR 0011's and must not move: a change to the strobe that shifts
  them has changed the band-slope or coarse readout, which the unison work is a
  tap alongside rather than a stage within.

  ```bash
  cargo run --release --example strobe_replay -- diagnostics
  ```

- **`auralize`** — renders each candidate tuning curve to a loudness-matched WAV
  by offline additive resynthesis (`tuner_core::synth`), so a stretch can be
  judged by ear before tuning a piano to it. Cold-path: it opens no audio device
  and never touches the real-time pipeline.

  ```bash
  cargo run --release --example auralize -- p2.json --out auralize_out
  ```

### Gatekeeper

- **`diagnose_gatekeeper`** — replays a capture through the 5-state validator and
  dumps its per-frame metrics to CSV.
- **`sparsity_ab`** — head-to-head between our spectral-sparsity gate and
  faithful Mounir NINOS² variants (faithfulness audit 05).
