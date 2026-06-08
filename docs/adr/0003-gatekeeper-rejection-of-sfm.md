# Gatekeeper: Removal of ΔSFM Plateau Detection from State 2

## Status

Accepted

## Context

The Gatekeeper state machine uses a 5-state model to protect the Pitch Engine from
receiving audio during the chaotic hammer-strike transient of a piano note. State 2
(TRANSIENT) was originally designed around the hypothesis that Spectral Flatness Measure
(SFM) would gradually settle over multiple FFT frames after an onset, and that the
frame-to-frame derivative |ΔSFM| could be used to detect when this plateau was reached.

The original State 2 exit condition was:

```rust
if (delta_sfm <= sfm_settling_derivative   // 0.05
    && current_sfm <= sfm_tonal_threshold) // 0.5
    || transient_timeout_counter > transient_timeout_frames
```

The preceding Gatekeeper architecture (prior to the overhaul) used a static 10-frame
(~464ms) blind wait. State 2's ΔSFM plateau logic was intended to replace this with a
dynamic, signal-driven equivalent.

## Investigation

Empirical diagnostics were run across all 87 keys of a real acoustic piano (A0–C8)
recorded in a typical room. The `diagnose_gatekeeper` tool generated per-key CSV
telemetry including `sfm` and `delta_sfm` on every frame. Findings:

- In **86 of 87 keys**, `delta_sfm` contributed **zero frames** of additional hold time
  beyond what NHWRSF alone provided.
- In all 87 keys, `current_sfm` fell below the `sfm_tonal_threshold` of 0.5 **on the
  very first onset frame itself**, before State 2 was ever evaluated.
- The State 2 exit condition resolved unconditionally and instantly on the first frame
  that NHWRSF dropped below its threshold, making it structurally equivalent to a no-op.

A timing budget analysis confirmed the total gate hold time across the keyboard:

| Contributor            | Frames | Time       |
|------------------------|--------|------------|
| NHWRSF > 0.5           | 2      | ~92ms      |
| ΔSFM plateau (State 2) | 0      | ~0ms       |
| NINOS2 ramp-up         | 3      | ~139ms     |
| **Total**              | **5**  | **~232ms** |

## Root Cause

The hypothesis underlying State 2 was physically incorrect given our FFT parameters.

Piano hammer-string contact lasts 4ms (bass) to <1ms (treble). A 20–25ms broadband
touch precursor (keybed mechanical noise) precedes the hammer contact. Our FFT frame is
**46.4ms** (2048 samples @ 44,100 Hz).

Because the entire transient event (touch precursor + hammer contact + early string
resonance onset) is shorter than a single FFT frame, the 46.4ms analysis window
captures both the brief broadband chaos AND the dominant harmonic sustain within the
same frame. The harmonic energy is so much larger in magnitude that the geometric mean
in the SFM formula is already driven to near-zero in Frame 1.

This is a direct consequence of the Gabor Limit: the FFT window cannot simultaneously
achieve the temporal resolution needed to observe the ms-scale transient evolution AND
the spectral resolution needed to resolve low bass partials. At 44.1kHz with a 2048
FFT, the temporal resolution constraint dominates, making multi-frame SFM settling
physically unobservable.

This finding was validated against the literature:

> "The observed ΔSFM plateau is frequently an artificial manifestation of signal
> processing latency rather than a true representation of the acoustic boundary between
> mechanical hammer impact and stable harmonic sustain." — Deep Research Brief,
> Piano Transient Gatekeeper (2026)

The same research validated that NHWRSF (half-wave rectified spectral flux) is the
robust, geometrically stable onset detector for this application, and that NINOS2
(spectral sparsity via L2/L4 norm ratio) is the correct stability gatekeeper.

## Decision (Updated: Total Purge)

Originally, the decision was to only remove `sfm_settling_derivative` from State 2's exit condition.

However, upon further review, it was recognized that the remaining SFM metrics (`sfm_tonal_threshold`, `sfm_noise_threshold`, `sfm_decay_derivative`) were also redundant or addressing problems outside the scope of piano tuning:

1. `sfm_tonal_threshold` was already empirically dead (SFM drops below 0.5 instantly).
2. The damper-drop detector (`sfm_noise_threshold` & `sfm_decay_derivative`) was an over-optimization designed to instantly terminate the RMS EMA upon key release. However, in a tuning context, the natural ~200ms decay of the EMA is imperceptible, and any subsequent rapid strike is robustly caught by the NHWRSF onset detector anyway.

Therefore, the decision was expanded to **completely eradicate SFM from the Gatekeeper**.

- Removed `calculate_sfm` entirely.
- Added a failsafe: if NHWRSF spikes > threshold (`is_new_onset`), the capture state is unconditionally reset to prevent mixing rapid strikes.
- Renamed the transient tracking flag to `transient_active` (formerly `sfm_has_settled`).

## Consequences

- The Gatekeeper architecture is radically simplified and relies purely on:
  1. **RMS EMA** (Am I loud?)
  2. **NHWRSF** (Am I a hammer strike?)
  3. **NINOS2** (Am I a stable tone?)
- The mathematical overhead of calculating the power spectrum's log-domain geometric mean is eliminated, saving CPU cycles.
- The `is_dead` damper-drop logic is removed, allowing the EMA to naturally decay to silence, which simplifies the state machine.
- The old 10-frame static wait (~464ms) is confirmed as a conservative but not incorrect design. The new NHWRSF+NINOS2 architecture achieves ~232ms dynamically, adapting to each note's register and transient character rather than using a blind stopwatch or unobservable SFM derivatives.
