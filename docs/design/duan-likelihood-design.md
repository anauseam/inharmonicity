# Design Note: Duan Peak/Non-Peak Likelihood for Discovery

Status: **Scoping** (not yet implemented). Feeds ADR 0006. Motivated by the
deadzone n-kernel failure derived there.

## Why this, why now

The optimization program (ADR 0006) hit a ceiling: constant-tuning tops out at
74/87 on the worst-case instrument, and the residuals split into octave/sub-harmonic
confusion (near the TWM scoring limit) and the **dense-bass attractor / sub-harmonic
steals**. The 0006 derivation pinned the attractor pathology precisely: candidate
error is too *forgiving* of partials a candidate predicts where no peak exists (the
`N_gap` channel). Every tolerance-style fix (`/N` averaging, K=88, the deadzone)
made it worse, because forgiveness scales with partial count and rewards the densest
(lowest-f₀) candidates.

**Duan, Pardo & Zhang (2010)** is the principled *inverse*: it models both spectral
peaks and **non-peak regions**, and explicitly *charges* a candidate for predicting
a harmonic where no peak was observed. That penalty is exactly the `N_gap` quantity,
turned from a rebate into a cost. It is the right tool for the attractor channel.
(It will not help the octave residuals — those need high-partial resolution, an
information limit — so success here means recovering the sub-harmonic steals, not the
octave errors.)

## Reference

Duan, Z., Pardo, B., & Zhang, C. (2010). "Multiple Fundamental Frequency Estimation
by Modeling Spectral Peaks and Non-Peak Regions." IEEE TASLP 18(8).
DOI: 10.1109/TASL.2010.2042119. (We already cite its Eq. 3 bound for the M→P
ceiling; this note adopts its non-peak *region* idea, which we currently do not use.)

## The core formulation we adopt

A candidate's score becomes a per-frame log-likelihood with two terms:

- **Peak term** (we already have this — the reverse error `Err_{m→p}`): each observed
  peak should be explained by a nearby predicted partial.
- **Non-peak term** (NEW): each *predicted* partial that falls in a region with **no
  observed peak** incurs a penalty — the harmonic "should" have produced a detectable
  peak and didn't. This replaces the current forward error's *forgiving* `/N`-diluted
  `Δf·f^{-p}` treatment of absent partials with a *charging* one.

In log-domain (for the real-time hot path) this is an additive penalty, not a
product of Gaussians — Duan's full GMM/likelihood is too heavy and not auditable; we
adopt the *structure* (charge absent-but-expected partials) as a bounded additive
term, MOBO-weighted.

## The make-or-break tension: missing fundamentals

A blanket "penalize every predicted-but-absent partial" **cannot ship** — it would
crush true bass notes, whose partials 1–3 are legitimately absent (soundboard
impedance). This is the exact concern already flagged in `twm.rs`'s comment about
Duan's Eq. 7 cliff. The non-peak penalty MUST distinguish:

- **Attractor gaps (penalize):** a predicted partial in the *active* spectral range —
  there is energy (other peaks) around it at the wrong spacing, so its absence is
  real evidence against the hypothesis.
- **Missing fundamentals (do NOT penalize):** a predicted partial *below the lowest
  observed peak*, in a genuinely quiet region — its absence is expected physics.

**Proposed gate (auditable, no magic amplitude table):** apply the non-peak penalty
only to predicted partials whose frequency lies **within the observed active band**
`[min_obs_freq, max_obs_freq]`. Below the lowest strong peak = the missing-fundamental
zone → no penalty. This mirrors the existing upper "dynamic bandwidth cap"
(`max_obs_freq + f0_et`) with a symmetric lower bound, and it is a *topological*
constraint (a band on the search space), not a fragile amplitude threshold — passes
the `04-algorithms` Scrutiny Test. A secondary refinement (optional): weight the
penalty by the local spectral context (energy near the predicted frequency) so an
absent partial surrounded by strong peaks costs more than one in a sparse region.

## Integration

- Add a default-off `TwmConfig` field (e.g. `nonpeak_penalty: f32`, default 0.0) so
  the regression test stays byte-identical and the term is MOBO-tunable. At 0 it is
  the current behavior.
- Slot the non-peak term into `score_candidate`'s forward pass: for each predicted
  partial with no peak within a match tolerance AND inside `[min_obs, max_obs]`, add
  `nonpeak_penalty` (× any context weight) instead of the diluted distance term.
- Keep it in the **shared discovery module path** (engine + evaluator + diagnose),
  per the single-implementation rule.
- Reconsider the `/N` normalization in light of this: the non-peak term is a *count*
  (it should scale with the number of bad predictions), so it likely should NOT be
  `/N`-averaged — but that interacts with cross-candidate comparability (the very
  thing `/N` protects). This is an open design question (below).

## Validation plan (per the lessons learned)

1. **Cheap look first**, on real captures with the conservative config + the non-peak
   term at a derived weight: the decisive check is **two-sided** —
   (a) does it suppress the dense-bass/sub-harmonic steals (treble→bass, the
   `N_gap` channel)? AND (b) does it *preserve* bass missing-fundamentals (bass
   register pass-rate must NOT drop)? Both must hold; a one-sided win is a fail.
2. If promising, **co-tuned MOBO arm** (tune the non-peak weight with q/r/ρ on the
   tuning-state bench) — because, like every error-landscape change, frozen-constant
   results are only a filter, not a verdict.
3. **Real-data gate**: beat 74/87 with **no bass regression** (the failure mode of
   every prior attempt). Held-out: the three known sub-harmonic pairs.

## Risks & kill criteria

- **Crushes bass (missing-fundamental):** if the `[min_obs, max_obs]` gate doesn't
  spare legitimately-absent low partials, bass pass-rate drops → reject or refine the
  gate.
- **Doesn't net-suppress the attractor:** if the peak term's gains are cancelled by
  collateral damage, reject (it's the same dual-failure shape as the deadzone).
- **Normalization re-opens the dense-candidate bias** (see open question) → the very
  pathology we're fixing could reappear through the wrong normalization.

## Open design questions

- **Normalization of the non-peak term** (count vs averaged) and its interaction with
  the existing `/N` forward and `/K` reverse normalizers — the crux, since
  normalization choice is what created the `/N` laundering in the first place.
- **Match tolerance** defining "no peak nearby" (reuse the masking/critical-band
  width? a fixed cents window?).
- **Context weighting** (uniform penalty vs local-energy-weighted) — start uniform
  (simplest, auditable), add context only if needed.
- Whether to *replace* the forgiving forward distance term entirely or *augment* it.
