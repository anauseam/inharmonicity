# Multi-frame temporal integration — design note (exploratory)

**Status:** exploratory, not built. Captured so the one genuinely *information-adding*
signal-evidence lever isn't lost. Deprioritized behind a second instrument and manual-mode
finalization. No code committed against this yet.

## Why this is the lever (and transform swaps are not)

Two ways to "use more time":

- **Reallocate** the single-frame time-frequency budget (sliCQ/NSGT, reassignment,
  synchrosqueezing). A constant-Q transform genuinely integrates *more time at low
  frequencies* (window length ∝ 1/f), but it is still one linear transform of one stretch
  of signal, bound by the uncertainty principle, and imposes **no model of how partials
  behave over time**. It spends a fixed budget differently; it adds no information. ADR 0005
  rejected this family on exactly that basis.
- **Add** information with a **temporal-continuity model** across many successive frames
  (a partial is born, persists smoothly, dies). This is nonlinear and prior-driven, and it
  is the only way to *reject transient confusers* and *track-average* a partial beyond what
  any single transform can do. This note is about that.

## The actual data flow (so the integration point is correct)

Two separate paths — and **CSPE is not in the real-time path**:

- **Real-time discovery (hot path, per hop):** FFT → magnitude spectrum → `extract_peaks`
  (sub-bin via `spectral::jacobsen`, Candan 2015) → TWM `score_candidate` over the 88
  templates. Deliberately **stateless** per frame. No CSPE, no MAT, no Goertzel seed here.
- **Worker (async, post-lock capture):** receives a `CapturePayload` (1.5 s stable buffer +
  Goertzel `measured_f0` seed) and runs CSPE (two FFTs, 1-sample shift) + MAT to produce the
  measured (f₀, B). Latency-tolerant by construction.

So temporal integration, if added, sits on top of the **per-frame peak lists** — not CSPE.
Two candidate homes, with very different stakes:

### A. Discovery pipeline (real-time)

Track peaks across hop-advanced frames; feed **persistence-filtered / track-averaged** peaks
to TWM instead of the raw single-frame peak list. Upside: rejects transient and
sympathetic-resonance peaks that don't persist; stabilizes peak frequencies.
**Costs:** (1) **latency** — a track needs a few frames to be confirmed before discovery can
trust it; (2) **state** — this re-introduces temporal state into a path ADR 0005
deliberately kept stateless, and must avoid the **full-sequence Viterbi / path-persistence**
mechanism ADR 0005 rejected.

### B. Worker (post-capture)

Run multi-frame tracking *within* the captured buffer to stabilize MAT's partial set before
the median solve. Latency is free (already async), stakes are lower (the key is already
identified), and it cannot regress discovery. The cheaper, safer first experiment if this is
ever pursued.

## Methods — faithful ports only

Our load-bearing wins are all faithful ports (TWM = Maher & Beauchamp, MAT = Hodgkinson,
CSPE = Short & Garcia); our bespoke assemblies underperformed. Apply the same rule here:

- **McAulay–Quatieri (1986)** — frame-to-frame partial linking by *greedy nearest-peak
  continuation* (births/deaths/continuations decided locally). **Not Viterbi.** The baseline.
- **Lagrange, Marchand & Rault — "Enhanced partial tracking using linear prediction"** —
  predict each partial's next frequency/amplitude by LP over its recent trajectory; purely
  recursive, forward-only. The natural "short-horizon confirmation" form.
- **Particle / Kalman recursive trackers** (Dubois & Davy SMC; Extended Complex Kalman
  Filter) — recursive Bayesian, multi-hypothesis carried forward, **no Viterbi**.

**Explicitly NOT** pYIN-style full-sequence Viterbi smoothing — cited earlier only for its
*candidate-distribution + keep-genuine-estimates* principle, not its decode.

## Conflicts / constraints (must be respected)

1. **ADR 0005 statelessness + Viterbi rejection.** Use short-horizon, recursive confirmation
   — never a global path optimization.
2. **Latency budget.** Confirmation costs frames of delay before a discovery lock; the
   discovery path is latency-sensitive in a way the worker is not.
3. **Process-model mismatch.** A smooth-evolution prior fits sustained tones, not struck
   transients/attack. Mitigation: **loose coupling** — the tracker consumes the existing
   per-frame point estimates as noisy *observations* and only decides persistence/continuity;
   it does **not** replace the frequency estimator (Jacobsen in discovery, CSPE in the worker).

## Noise-source taxonomy — what this lever can and cannot fix

| Confuser | Temporal behavior | Multi-frame helps? |
| --- | --- | --- |
| Sympathetic resonance (other strings ringing) | Builds *after* onset, decays with driver, weaker | **Yes** — onset-timing/persistence separates it |
| Attack broadband (treble) | Transient, dies fast | **Yes** |
| Random spectral noise peaks | Non-persistent | **Yes** |
| Longitudinal / phantom partials | Arrive *with* the onset | **No** — not time-separable |
| Wound-string stiff-law breakdown (bass) | Not a peak-detection issue | **No** — model error, not noise |
| Octave / sub-harmonic identity (the bass bottleneck) | Structural identity in the harmonic series | **No** — perfect noise rejection leaves it untouched |

## Verdict

Worth doing for **Category-3 signal evidence** — rejecting sympathetic-resonance and
transient confusers, stabilizing peaks — which could lift all registers' auto-detection.
It is **not** a bass-octave fix (that's Category 2, a structural identity no amount of
noise rejection resolves). If pursued, start in the **worker** (latency-free, can't regress
discovery). Gated behind the second instrument and manual-mode work.

## References

- McAulay, R. & Quatieri, T. (1986). Speech Analysis/Synthesis Based on a Sinusoidal
  Representation. IEEE TASSP.
- Lagrange, M., Marchand, S. & Rault, J.-B. Enhanced partial tracking using linear
  prediction. <https://hal.science/hal-00308184v1/document>
- Dubois, C. & Davy, M. Tracking of time-frequency components using particle filtering.
  <https://www.researchgate.net/publication/4137213_Tracking_of_time-frequency_components_using_particle_filtering>
- Arulampalam, S. et al. A Tutorial on Particle Filters for Online Nonlinear/Non-Gaussian
  Bayesian Tracking.
