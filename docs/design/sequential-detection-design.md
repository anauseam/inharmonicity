# Decision-level sequential detection — design note (exploratory, build nothing)

**Status:** exploratory, **not built** — written per the ADR 0006 item-5 decision gate
(the flicker-vs-stable diagnostic found real headroom, so the evidence-accumulation
idea is not closed; per the gate, this note frames it faithfully and nothing more).

**⚠️ [2026-07-05] The measured problem below predates the jacobsen peak-estimator
fix (`faithfulness-audit-03-jacobsen.md`) — the exact key lists are stale.**
Prompt A′ (ADR 0006 item 5, 2026-07-05 entry) re-ran this classification on fixed
peaks: discrete 7 early-wrong-lock keys (000/004/080/081/082/084/085), refined 6
(000/001/002/080/082/087) — vs. the 6 below. **The conclusion strengthens** —
headroom is real and larger (gross plurality-flip ceilings: discrete 76→83,
refined 77→81; refined 000 is a dead tie and 001 flips the *wrong* way by one
frame, so those two need windowed/score-level rules, not plain plurality) — and
one premise below is now **WRONG on fixed peaks**: the class is **no longer
bass-free**. Discrete 000 (A0) and 004 (C#1), and refined 002 (B0), are deep-bass
true-plurality early-wrong-locks (refined 000/001 near-tied races), so the
"headroom … none bass — this is not a bass lever" line no longer holds. The
Decision section still holds (build nothing; offline M-of-N replay first), and
its gate 1 is now *degenerately resolved*: no constants candidate survived A′
(t1898 refuted — see ADR item 2), so "whichever config survives" = the shipped
default, and the re-classification under it is the A′ Task-3 data. The t1898
"levers overlap" framing below is superseded.
Distinct from [`temporal-integration-design.md`](temporal-integration-design.md),
which integrates at the **signal** level (partial tracking across frames); this note
is about the **decision** level — how per-frame TWM verdicts are fused into a lock.
The two are independent levers and could coexist.

## The measured problem (2026-07-02 diagnostic, ADR 0006 item 5)

The production lock rule is **first key to win 3 consecutive stable frames**. On the
13 keys the shipped 74/87 config fails, the per-frame stable-winner sequences
(`peaks.csv` × `gatekeeper.csv`) split three ways:

| class | n | keys | decision-level headroom |
| --- | --- | --- | --- |
| stable-wrong | 5 | 000, 001, 005, 010, 012 (all deep bass) | **none** — wrong winner ≥71% of all frames |
| **early-wrong-lock** | 6 | 034, 070, 080, 081, 084, 085 | **real** — wrong key wins only the attack transient (first 3-run at stable index 2–17), then the true key takes the plurality (38–89%, runs of 5–39) |
| genuine flicker | 2 | 006, 086 | uncertain — true key wins 10–29% of frames, never 3 straight; wrong plurality |

The failure mode of the current rule is precise: **first-to-3 is a race, and the
attack transient runs first.** Full-window plurality voting alone would flip all six
early-wrong-lock keys. Two caveats bound the prize: (a) headroom is ≤6 keys (+2
uncertain), **none bass** — this is not a bass lever; (b) the pinned arm-6 candidate
constants (seed-7 trial 1898, ADR 0006 item 2) already fix 3 of the 6 (070/084/085)
plus 005, so the constants lever and this lever **overlap** — but not fully: the
same classification run under t1898's 10 failures still shows **4 early-wrong-lock
keys with true-key plurality (034/080/081/082, including 082, the key t1898
breaks — wrong lock at stable index 13, then a 27-frame true-key run)**. So ~4–6
keys of headroom survive either config; the levers are complementary, not
redundant. Re-measure under whichever config survives second-instrument
validation before sizing this.

## Faithful framings (published methods only)

Per the standing faithful-ports principle, the candidate mechanisms are published
sequential-detection rules, not bespoke assemblies:

1. **Binary integration / M-of-N detection** (Schwartz 1956; Shnidman 1998 — the
   radar "M out of N" rule). Declare a lock when the same key wins **M of the last
   N** stable frames. Consumes only the per-frame *winner votes* — no likelihood
   model needed, so it is faithful as-is on top of the existing pipeline. The
   current rule is the degenerate M=N=3 consecutive-runs case; M<N versions
   tolerate transient interruptions instead of restarting the count. This is the
   **minimal faithful upgrade** and is directly testable **offline from the
   existing CSVs** (pure post-processing of the winner sequences — no engine
   change) before any code is touched.

2. **Wald SPRT** (Wald 1945; optimality Wald & Wolfowitz 1948) and its
   multi-hypothesis form **MSPRT** (Baum & Veeravalli 1994; Veeravalli & Baum
   1995). Accumulate per-frame **log-likelihoods** per candidate; stop when the
   leading hypothesis's posterior crosses a threshold. This is the principled
   version of "average per-candidate TWM scores across stable frames before the
   argmin", and it buys the optimal latency/error tradeoff — the thing the current
   rule gets wrong (it commits at fixed, minimal latency regardless of evidence
   quality).

## The faithfulness gap (why MSPRT is not a drop-in)

- **TWM error is not a likelihood.** SPRT/MSPRT consume log-likelihood ratios; a
  bespoke `exp(−score)` mapping is exactly the kind of unpublished assembly this
  project's record says underperforms. A published bridge exists — Duan, Pardo &
  Zhang 2010's spectral likelihood model — but note its *scoring proxy* was already
  rejected on real captures (ADR 0006). Using it as a *temporal fusion* weight is a
  different role, untested, and would need its own validation.
- **Frames are not independent.** Hop-overlapped analysis frames violate the iid
  assumption behind SPRT's guarantees; a faithful port must either decimate to
  effectively-independent frames or cite a correlated-observation treatment.
- **M-of-N has neither problem** — it is nonparametric over winner votes. Its cost
  is latency: N must outlast the attack transient (the measured wrong-locks happen
  at stable index 2–17), so a rule that spans it means locking tens of stable
  frames after onset instead of ~3. The latency budget is a UX question (the tuner
  should feel instant) and must be decided before any parameter is chosen; N and M
  would otherwise become two new magic numbers tuned on n=1.

## Decision

**Build nothing now.** Gates, in order:

1. Second-instrument validation resolves which constants ship (ADR 0006 item 2);
   re-run the flicker classification under that config — the residual early-lock
   class may shrink to ≤3 keys.
2. If headroom survives, the first experiment is **offline M-of-N replay** over the
   existing `diagnostics/` CSVs (zero engine changes): sweep (M, N), measure flips
   and added latency in stable frames, and check nothing currently-passing breaks
   (a longer window also gives *impostors* more chances to accumulate on the
   currently-passing keys — the same symmetric-exposure trap as K=88 and the
   deadzone; the replay prices it for free).
3. MSPRT/score-averaging only if M-of-N's winner votes prove insufficient (the
   2 genuine-flicker keys are the only class that would justify it) — and only with
   a published likelihood bridge, not a bespoke score transform.

## References

- Wald, A. (1945). Sequential Tests of Statistical Hypotheses. Ann. Math. Statist. 16(2).
- Wald, A. & Wolfowitz, J. (1948). Optimum Character of the Sequential Probability
  Ratio Test. Ann. Math. Statist. 19(3).
- Baum, C. W. & Veeravalli, V. V. (1994). A Sequential Procedure for Multihypothesis
  Testing. IEEE Trans. Inf. Theory 40(6).
- Schwartz, M. (1956). A coincidence procedure for signal detection. IRE Trans. Inf.
  Theory 2(4) — the M-of-N / binary-integration rule.
- Shnidman, D. A. (1998). Binary Integration for Swerling Target Fluctuations. IEEE
  Trans. AES 34(3).
- Duan, Z., Pardo, B. & Zhang, C. (2010). Multiple Fundamental Frequency Estimation
  by Modeling Spectral Peaks and Non-Peak Regions. IEEE TASLP 18(8).
