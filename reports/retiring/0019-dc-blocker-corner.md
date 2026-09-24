# report 0019 — the DC blocker's corner

Measured 2026-07; recorded in `ARCHITECTURE.md` until the documentation refactor moved
decisions and their evidence into their own homes.

This is a short body for a decision that took two rounds to settle, and it is
worth reading for the shape rather than the numbers: a documented figure was
found to be wrong by a factor of ten, the obvious fix was measured and **made
things worse**, a better-designed fix was then measured and changed nothing that
mattered — so the outcome is a documentation change and no code change. The
reopen condition is stated because the null result is contingent on who consumes
the bottom octave, and nothing does today.

### Why the DC blocker corner sits above A0

The input conditioner is a one-pole high-pass with α = 0.995, whose −3 dB corner
is `(1−α)·fs/2π` ≈ **35 Hz**. That is _above_ A0's 27.5 Hz fundamental, which it
attenuates by 4.2 dB (3.3 dB at C1, 1.5 at A1, 0.4 by A2). For a tuner that
sets out to capture the whole bass register that looks wrong, so it was measured
rather than argued, and the corner is kept.

- **Restoring a 3.5 Hz corner (α = 0.9995) buys nothing and costs accuracy.** The
  bass fundamental gains 2–4 dB but remains 24–41 dB below the note's strongest
  partial — still under the −30 dB masking gate on the same 7 of 9 bass keys, so
  discovery sees no new partials. The missing bass fundamental is acoustic, not
  filter-induced. Meanwhile the CFAR reference cells that set the coarse readout's
  local noise estimate include this band (its deep-bass lower flank clamps at bin
  1), so the threshold rises 1–3 dB while the read's own reference partial at
  110 Hz gains nothing: measured, coarse availability falls 93.3 % → 87.4 % and
  error worsens 0.70 ¢ → 1.85 ¢.
- **A steeper filter is the better lever, and still not worth it.** Order — not α
  — is the axis that escapes the trade: a 3rd-order Butterworth at 25 Hz recovers
  2.4 dB at A0 while admitting slightly _less_ rumble, for 9 µs per 23 ms callback
  (0.04 % of one core), which is affordable. It was rejected on outcome: MAT's
  measured `B` moves by a median of **0.00 %** across 87 keys, and the coarse read
  by +0.3 points of availability. MAT tracks 30+ partials and the deep-bass
  fundamental was never in its fit, so the filter only shapes spectrum the
  estimator already ignores.

**What would reopen it.** A consumer that actually uses the bottom octave's
fundamental — the per-bin/per-octave noise floor in [TODO.md](../../TODO.md),
or an instrument that genuinely radiates it (both validation pianos are uprights,
the weak case). If that happens, change the **order**, not α, and note three
things: cascaded biquads are needed rather than one pole; conditioning at
`fc/fs ≈ 5.7e-4` puts the poles at radius ≈ 0.9965, where f32 direct-form I is
marginal (use transposed direct-form II or f64 state); and more filter state
multiplies the single-state stereo defect recorded in [TODO.md](../../TODO.md).

Re-validating any change is possible **without re-recording**: the one-pole
inverts exactly (`x[n] = y[n] + x[n−1] − α·y[n−1]`, round-tripping to 1e-15
relative in f64 on real captures), so a candidate filter can be applied to the
existing capture sets by inverting this one first.
