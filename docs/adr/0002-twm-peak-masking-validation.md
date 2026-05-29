# Engine Validation — 2026-05-28

## Purpose

This document records the first full-compass diagnostic run of the TWM engine
following two major architectural changes made during this session:

1. **Removal of the Geometric Gate** — the `E_win < 0.5 × E_chromatic` gate was
   proven to be structurally incompatible with bass strings (its threshold was
   below the physical error floor of an inharmonic string) and was removed.
2. **Introduction of the Gómez/Cano `-30 dB` Peak Mask** (`mask_peaks` in
   `peaks.rs`) — replaces the geometric gate as the primary mechanism for
   rejecting sympathetic tonal noise before TWM evaluation.

## Method

`cargo run --example diagnose_engine` was run against all 8 real acoustic piano
captures stored in `diagnostics/`. Each run replays the raw audio frame-by-frame
through the full peak extraction → `mask_peaks` → TWM scoring → Viterbi tracking
chain, identical to what the live engine executes.

Because these captures pre-date the noise-floor telemetry field added to
`analysis.json`, the diagnostic harness defaulted to a conservative noise floor
of `0.001` (-60 dBFS). The actual recording environment had an estimated SNR of
~29–32 dB based on RMS measurements observed in the output.

## Results

| Sample        | Key Index | Frames Tested | Correct Locks | False Locks |
| ------------- | --------- | ------------- | ------------- | ----------- |
| `key_001_A#0` | 1         | 57            | 57            | 0           |
| `key_003_C1`  | 3         | 57            | 57            | 0           |
| `key_015_C2`  | 15        | 57            | 57            | 0           |
| `key_027_C3`  | 27        | 57            | 57            | 0           |
| `key_039_C4`  | 39        | 57            | 57            | 0           |
| `key_051_C5`  | 51        | 57            | 57            | 0           |
| `key_063_C6`  | 63        | 57            | 57            | 0           |
| `key_075_C7`  | 75        | 31            | 31            | 0           |

**8/8 samples. 0 false locks across the entire compass.**

## Significance

Prior to the geometric gate removal, C2 was reliably locking to C1
(sub-harmonic false lock) because the gate's threshold of `0.5` was
below the baseline TWM error floor for an inharmonic string (~0.7–1.0),
causing the engine to mathematically prefer the higher partial grouping of C1.

The `-30 dB` relative mask strips the sympathetic room resonance from the
peak list before TWM ever evaluates it, leaving only the structural harmonics
of the struck string. This is sufficient to give TWM unambiguous input across
the full bass-to-treble compass at the SNR levels present in these recordings.

## Known Limitation

The `-30 dB` threshold is a relative mask (30 dB below the frame's global
spectral maximum). It is scale-invariant and therefore hardware-agnostic.
However, if the SNR of the recording environment drops below ~30 dB (e.g. a
loud rehearsal room), sympathetic noise could survive the mask and re-introduce
instability. See `README.md` Known Issues for more detail.

## Next Steps

- MOBO parameter tuning (`q`, `r`, `ρ`) against a synthetic dataset to further
  tighten the error margin between the winning key and the runner-up.
  See [`mobo-tuning.md`](mobo-tuning.md) for the methodology.
- Capture samples from non-piano instruments (ukulele, bass guitar) to verify
  the `-30 dB` threshold generalises across different tonal SNR profiles.
