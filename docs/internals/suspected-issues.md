# Suspected Issues — Descriptive Context

This file documents **suspected-but-unreproduced** hazards in the codebase and the defensive code that addresses them. Unlike the rules in `docs/internals/`, this file is mainly descriptive and provides historical context for certain workarounds.

---

## CPAL/ALSA shutdown-segfault workaround

**Location:** `tuner-gui/src/app.rs` (the exit-on-close-request path) and `tuner-core/src/audio.rs`.

**What we suspect:** Shutting down the CPAL/ALSA audio stream without a clean drop can trigger a segmentation fault during application exit.

**Why we suspect it:** During early development, forcing an exit without cleanly joining/dropping the stream threads caused a segfault on Linux under ALSA. The GUI's exit-on-close-request path was added defensively to ensure the audio pipeline drops completely before the main process exits.

**Reproduction status:** Unverified against current CPAL/ALSA versions. The fault may no longer reproduce.

**Action if refuted:** If a future maintainer can verify that closing the app via standard means (without the dedicated exit-on-close-request path) no longer triggers a segfault under ALSA, the explicit drop workaround can be removed.

---

## Neyman–Pearson partial gate: σ is mis-specified during sustain

**Location:** `tuner-core/src/engine.rs` (`NEYMAN_PEARSON_K`, the tracker's
per-partial amplitude gate) and `tuner-core/src/strobe.rs`
(`neyman_pearson_k(n)`, the strobe bank's D3 gate — same derivation, same σ).

**What we suspect:** The gate's threshold `T = noise_floor · K` uses the
**ambient-silence** RMS (the calibrated silence threshold) as the noise σ in
Kay's detection formula. That is the correct H₀ only when the room is quiet.
During an active sustain, the effective noise at a partial's bin — spectral
leakage from the note's other partials plus its broadband decay residue — is
far above the ambient floor, so the threshold is too low in exactly the
regime where the gate runs. Consequence: the gate under-rejects during
sustain (a dead partial whose bin is fed by neighbor leakage still passes),
and its statistical guarantee (P_fa = 0.001) holds only against ambient
noise, not against the note itself. Audit-08 verified the *constant's
derivation given σ*, not the appropriateness of σ under sustain — this gap
was spotted in review (2026-07-19), not by the audits.

**What the gate still does:** true-silence rejection (moot engine-side, where
the Gatekeeper's `is_silence` resets first, but live for the strobe bank,
which runs unconditionally), and dropping long-dead partials whose bins have
no strong leakage neighbors (e.g. widely-spaced treble partials during a
bass-dominated sustain). It is a floor, not the advertised detector.

**Reproduction status: the MECHANISM is confirmed; these two gates are still
un-measured (2026-07-25, ADR 0011).** Be precise about what was shown. The
coarse-readout study ran the same ambient-σ threshold as a *control* against
FFT bins in a bounded search, and it admitted **100 % of ±400 ¢ deep-bass
garbage** — the predicted under-rejection, at full strength — while an
ordered-statistic CFAR gate against *local* reference cells admitted 0 % of the
same junk at unchanged median accuracy and lifted C8 availability from 42 % to
100 %.

That confirms the σ-misspecification mechanism and demonstrates the fix. It
does **not** measure the two gates named above: they threshold a *Goertzel*
amplitude (adaptive centre in the engine, fixed reference in the strobe bank),
not an FFT bin, so their exposure is confirmed by analogy only. Dead-partial
pass rates during sustain for the engine tracker and the strobe bank remain
unmeasured.

**Action: queued as Prompt O** (`docs/design/next-chats-handoff.md`) — measure
these two gates directly, then decide the port. The fix this entry anticipated
("a local noise reference (off-partial bins around the target) in place of the
ambient σ") now exists and ships, but **only in the coarse readout**
(`peaks::coarse_read`, ADR 0011). The engine tracker and strobe-bank gates are
**deliberately untouched**: they are load-bearing for the 87-capture discovery
baselines, and swapping their gate demands revalidation first. Until then, treat
`strobe_gated` as a coarse decay indicator, not a calibrated detector.

---

## Coarse readout — bounded, measured caveats (not suspicions)

**Location:** `tuner-core/src/algorithms/peaks.rs` (`coarse_read` and its
constants), `pipeline.rs` step 5b.

Unlike the rest of this file these are **measured** limits, recorded here
because each is a trap a future change could walk into. Full numbers: ADR 0011.

- **The argmax costs a search loss the radar framing does not cover.** Rohling's
  P_fa is for *one* cell under test; taking the argmax over a band of M cells
  gives M chances to false-alarm. Measured: realized AWGN rate 0.0386 against a
  nominal 0.001 (39×), and collapsing the band to a single bin brought it to
  0.0012 — i.e. the per-cell calibration was exactly right and the whole excess
  was the search. Corrected by a per-cell budget `P_fa / M_eff` with
  `M_eff` = band width halved (Hann correlation); realized 0.00097. **The lesson
  generalizes:** any future detector that scans-then-thresholds needs this
  correction, and its absence is invisible in per-cell verification.
- **Below ≈ F3 at 2048 the gate is a ratio test, not a P_fa test.** At that size
  the ±43 Hz Hann lobes tile the low-mid spectrum, so no inter-partial valleys
  exist and the reference cells measure lobe skirts rather than noise. The gate
  degenerates to peak-versus-lobe-cusp ratio and its P_fa semantics are void
  *while a note sounds* (it stays sound on dead notes). Do not quote a
  false-alarm figure for that register at 2048; the tier-1 rule prefers 8192
  there in any case. This is the same inequality as the window rule
  N > 4·fs/f₀ — resolution and noise-estimation validity fail together.
- **One P_fa bucket is slightly permissive.** Per-band-width audit at 8192:
  the 17–32-bin bucket realizes 0.0027, i.e. **2.7× nominal**. Bounded, and only
  ever evaluated outside `Silence`. At 2048 every bucket is ≤ 0.0017. Quote
  these with their trial counts, and re-measure if the band geometry changes.
- **Room noise is not a valid H₀ for this gate** and must not be used as one:
  colored rumble *is* narrowband energy, so it scores P_fa ≈ 0.34 at any
  quantile. This is not a defect — the coarse read is skipped during `Silence`,
  and calibration puts the silence threshold above ambient, so 95 % of
  pre-onset frames are `Silence` by construction. **If step 5b is ever made to
  run during `Silence`, it will read rumble as a partial.**
- **Motion tail.** Median error under fast detuning is the analysis window's own
  group delay, but p90 tail errors reach 82 ¢ at 200 ¢/s and 131 ¢ at 400 ¢/s,
  with 4–6 % mixed-source churn from the dual-window selection. Absorb this in
  *display* smoothing; it is not a DSP defect to chase.

**Retired assumptions (do not reinstate).** Round-1 measurement produced three
conclusions that later rounds overturned: that duration kills the 8192 window in
the treble (an ambient-gate artifact — 8192 dominates statically everywhere,
including C8); that the analysis size should be split by *register* (the real
split is motion); and that n\* = 3 is the best coarse partial in the bass (that
came from an ambient-gate aggregate, and the shipping gate behaves entirely
differently — the answer is a fixed n\* = 4).

**α = 0.05 on the engine tracker is load-bearing — do not raise it.** It is the
pitch-raise follower and the lobe-centering loop at once. α = 1 at N = 4096 gives
|z| = √2, i.e. an unstable re-centering loop, and the re-centering transient gain
is (N−1)/(2·HOP) ≈ 2. Its cost is on record instead: the EMA's τ ≈ 0.46 s is
outrun by a fast peg (measured tracker aliasing of 152–387 ¢ at 200–400 ¢/s),
which is precisely why the coarse readout exists rather than a faster tracker.
