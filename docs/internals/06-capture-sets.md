# Capture Sets — the validation data

Every empirical claim in this project rests on three sets of real recordings.
They are kept on disk and **never committed** (`.gitignore`: `diagnostics/`,
`diagnostics_piano*/`) — they are large, regenerable-in-principle, and
instrument-specific. This file records what they are, what state the
instruments were in, and the rules for consuming them, because none of that is
recoverable from the audio.

## The sets

| Directory | Instrument | Captures | Keys | Captured for |
| --- | --- | --- | --- | --- |
| `diagnostics/` | Steel-string guitar, standard tuning | 6 | 6 (E2 A2 D3 G3 B3 E4 = keys 19/24/29/34/38/43) | The guitar-strobe frequency audit; the cross-instrument check for anything piano-tuned |
| `diagnostics_piano_1/` | Upright piano #1 | 87 | 87 (one per key; **key 21 / F#2 is absent**) | The original discovery/TWM validation set — the "87 captures" every lock-accuracy number cites |
| `diagnostics_piano2/` | Upright piano #2 | 595 | 88 (≥ 5 repeats per key) | Prompt H's repeat-capture noise decomposition (ADR 0009); the project's first two-instrument evidence base |

Note the naming inconsistency: piano **1** has an underscore before the index,
piano **2** does not. This is why `.gitignore` uses `diagnostics_piano*/` — an
earlier `diagnostics_piano_*/` silently failed to ignore 315 MB of piano-2
captures.

## Instrument state — read this before interpreting any number

**Both pianos are out of tune.** That is not a defect in the data, it is the
point: this app targets the out-of-tune and pitch-raise regime, so captures of
a freshly-tuned instrument would test the easy case only. It does, however,
constrain what the sets can prove:

- There is **no trusted `B` reference** on either instrument. This is the
  standing gate behind `APPLY_MEASURED_B_TO_DISCOVERY` and the ADR 0006
  measured-B pathway — see ADR 0006 and `pipeline.rs`.
- Lock-accuracy scores are *relative* measures. A config scoring 77/87 is
  better than one scoring 74/87 **on this instrument**; neither number is an
  accuracy claim about pianos.

Measured deviation from equal temperament (median per key, from
`analysis.json`; keys whose seeds are known-bad excluded):

| Set | median | p10 … p90 | full range |
| --- | --- | --- | --- |
| piano #1 | +4.4 ¢ | −7.1 … +17.6 | −24 … +44 |
| piano #2 | −0.8 ¢ | −12.0 … +3.5 | (see defect below) |
| guitar | −2.7 ¢ | −8.9 … −0.8 | −9 … −1 |

**Do not read those as tuning quality.** A correctly tuned piano deviates from
ET by design — the Railsback stretch reaches tens of cents at both extremes —
so cents-vs-ET conflates intended stretch with mistuning. The table
characterises *the detuning regime the captures represent*, which is what
matters when choosing a search span or an unwrap range. The evidence for actual
tuning state is qualitative and lives in ADR 0006 and the strobe design note
§15 ("every capture we have is of a detuned piano").

## Consumption rules

**Validation only.** These captures may not select a configuration. With n = 1
instrument (or 2), a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence. Report per-register counts and which keys moved; do not tune on them,
and do not recalibrate the synthetic generator to match them.

**Piano #2 must be consumed through `regenerate_partials`, never through raw
`analysis.json`.** The deep-bass entries were written before the
`worker::MAT_SEED_TOLERANCE` fix and carry rumble-seeded garbage — measured:
**35 of 595 entries** land beyond ±200 ¢, all in the deep bass, with A0
"measured" at 7–14.7 Hz against an ET 27.5 Hz. The *audio* is genuine; only the
cached analysis is wrong. So:

```bash
cargo run --release --example regenerate_partials -- diagnostics_piano2 > p2.json
python3 scripts/audit_captures.py p2.json      # consumes the regen, not analysis.json
```

**Prefer independent truth to the cached fields.** `examples/pitch_ground_truth.rs`
computes a zero-padded hi-res DFT truth per capture; that is the reference for
estimator accuracy work, not `measured_f0`.

**Release builds.** DSP must be exercised with `--release`; debug builds drop
audio and change availability figures.

## Where the numbers ended up

- ADR 0006 — discovery/TWM lock accuracy, the measured-B gate (piano #1).
- ADR 0009 — σ_lnB, ρ reproducibility, strike strength (piano #2 repeats).
- ADR 0010 — M-of-N lock rule, concordance across both pianos.
- ADR 0011 — the coarse readout: CFAR profile, P_fa calibration, n\* selection
  (both pianos **and** the guitar — the cross-instrument disagreement at n = 5
  is what fixed n\* = 4).
