# Decision-level sequential detection — design note (exploratory, build nothing)

**Status:** **implemented-v1 (2026-07-20, Prompt M)** — the acquisition-only
M-of-N lock now ships in the hot path (`Engine::record_stable_winner`, refined
default (M, N) = (7, 8)); the release/re-lock hysteresis half is deliberately
deferred to a second design (see ADR 0010 Limitations). This note is retained as
the derivation trail. Path to here: written per the ADR 0006 item-5 decision gate
(the flicker-vs-stable diagnostic found real headroom, so the evidence-
accumulation idea was not closed); **[2026-07-19] Gate 2 (offline M-of-N replay)
executed** — protocol in the "Replay protocol" section below, results in
`docs/adr/0010-m-of-n-lock-rule-replay.md`; **[2026-07-20] hot-path implementation
landed** (engine + the `validate_config.py` / `test_engine_all.py` known-answer
replicas, `--lock-m/--lock-n`, defaults 7/8); **[2026-07-22] gate 3 (score-level
fusion / MSPRT) empirically refuted and CLOSED** via a one-off offline probe —
see "Score-level fusion: tested and refuted" below (method + numbers retained
there; the probe itself was not kept). The decision-level lever is
now fully explored: M-of-N is the shipped win; score fusion has no headroom; the
residuals are B-limited, not decision-limited.

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
   **→ Gate 3 CLOSED (2026-07-22, empirically refuted): see below.**

## Score-level fusion: tested and refuted (2026-07-22)

Before investing in a likelihood bridge, the cheap version of gate 3 was run
directly by a one-off offline probe (not retained — this section is the record):
all 88 keys were scored per stable frame (via `twm::score_candidate` — the same
atomic scorer discovery uses) and fused across the stable window two ways —
**mean-score** (lowest mean TWM error) and **rank-score** (lowest Borda rank-sum,
a magnitude-robust control). Both are compared to plurality (winner-voting) —
reconstructable from this description if ever needed — against folder-name
ground truth, on both
instruments and in discrete and refined scoring. Correctness gate: the discrete
plurality lands on the ADR 0010 ceiling — P1 **83/87** (strict = loose, exact)
and P2 **575/595** = the ties-as-pass ceiling (strict is 573; the probe's
tie-break resolves 2 tied captures toward the true key). Landing precisely on the
loose ceiling confirms the recomputed winner sequences are faithful.

**The discrete run is the primary evidence.** Score fusion is only well-defined
at the discrete level, because a full comparable 88-key score vector exists every
frame there; production's refined path refines only the top-3, so a refined
"score to average" for all 88 does not exist in the real algorithm. The `--refine`
run below refines all 88 as a stress test — but that lets distant impostors cheat
a favorable scale, so its plurality baseline is artifact-depressed (P1 75 vs the
production 81 ceiling) and its apparent "fixes" are mostly recoveries of that
self-inflicted damage (P1 refined mean-score "fixes" 016/018/027 — mid-bass keys
the artifact broke — none of them the B-limited residuals).

| | P1 discrete | P1 refined | P2 discrete | P2 refined |
| --- | --- | --- | --- | --- |
| plurality (vote) | 83 | 75* | 575 | 566* |
| mean-score | 81 | 74 | 563 | 559 |
| rank-score | 70 | 69 | 527 | 542 |

`*` refined plurality is below production's top-3-refined ceiling (81/578)
because the probe refines all 88 candidates, letting distant impostors cheat a
favorable scale — a probe artifact; the fusion-vs-vote comparison is unaffected
(both use the same scores). Discrete plurality: 83 is strict; 575 is ties-as-pass
(573 strict).

**Verdict — score-level fusion has no headroom here, so this branch is closed.**
In the primary (discrete) test, fusion is strictly worse and fixes exactly zero.
Across all four configurations it never nets a win over voting (mean net −1 to −7;
rank strictly worse), and in every mode **the stable-wrong residuals 000/010/012
are never fixed** (the only refined "fixes" are mid-bass artifact recoveries).
The mechanism is diagnostic: bass/mid are saturated (fusion neither helps nor
hurts), and the loss is entirely in the **treble**, where high partials die
intermittently — the true key wins the *plurality* of frames but has scattered
catastrophic frames that any temporal *average* (mean or rank) lets poison it,
while voting is immune because it only counts wins. This both vindicates M-of-N
(a voting rule) as the correct temporal integrator for this signal and warns
that a full MSPRT — which also integrates per-frame evidence — would inherit the
same treble vulnerability unless its likelihood model explicitly down-weights
dead-partial frames. Given two simple fusions regressing on two instruments in
both modes, and the residuals being independently B-limited (ADR 0006 item 5),
the burden of proof for the heavy likelihood-bridge version is not met. **Do not
reopen without a specific new reason** (e.g. a future instrument whose residuals
are shown to be genuine flicker, not stable-wrong/B-limited).

## Replay protocol (decided 2026-07-19 — gate 2 executed)

All parameters below were fixed **before any sweep surface was computed** (the
selection criterion is baked into `scripts/replay_lock_rules.py`'s defaults);
the three user decisions (latency budget, data protocol, execution scope) are
on record from 2026-07-19. Results: `docs/adr/0010-m-of-n-lock-rule-replay.md`.

**Rule under test.** M-of-N binary integration (Schwartz 1956; Shnidman 1998)
over the *stable-frame winner sequence* — the identical merge
`validate_config.py` uses (`peaks.csv` `key_idx` × `gatekeeper.csv`
`state_name == "Stable"`, frame order). At stable index *t* the window is the
last min(*t*+1, N) winners (partially filled at the start, so clean evidence
still locks early); lock fires at the first *t* where some key holds ≥ M votes.
**M > N/2 required** — majority makes the winner unique per frame, no tie rule
needed. M = N = 3 reduces *exactly* to the production 3-consecutive rule and is
the harness-correctness gate.

**Grid & budget.** Full surface N ∈ 3..43, M ∈ ⌊N/2⌋+1..N, computed for the
record; **selection is restricted to N ≤ 21** (user decision: ≈ 0.49 s
worst-case added latency at the 23.2 ms stable-frame hop; the current rule
commits ≈ 70 ms after stability). Added latency is reported in stable frames;
the ms conversion assumes contiguous stability (approximate — the gatekeeper
can interleave non-Stable frames).

**Datasets & config.** Piano-1 = `diagnostics_piano_1/` (87 keys × 1 capture);
piano-2 = `diagnostics/` (595 repeat dumps, 88 keys, ≥ 5 each; folder-name
ground truth, wrong-strike dumps already discarded — the audio is genuine even
where early `analysis.json` files are not, and the replay consumes only audio).
CSVs regenerated fresh under the shipped `TwmConfig::default()` (telemetry
build, mirroring `validate_config.py`); both discovery modes (discrete,
refined) swept independently, refined = the production-weighted mode. No
constants sweep (Prompt A′: no candidate beats the default).

**Metrics per (M, N, mode, instrument).** Pass (lock == folder key) total +
per register (bass ≤ 26 / mid 27–59 / treble ≥ 60); fail mode (wrong lock vs
no-lock); fixed/broken lists vs the M = N = 3 baseline; added-latency
median/p95/max over keys both rules lock correctly. Piano-2 additionally:
per-key dump-consistency (a key's repeats should agree; mixed outcomes are
reported, and per-dump counting is the primary score).

**Pre-registered selection criterion (two-instrument concordance).** Per mode:
plateau₁ = pairs within **1 key** of the piano-1 surface max (N ≤ 21);
plateau₂ = pairs within **7 dumps** of the piano-2 max (≈ 1 key-equivalent —
595/88 ≈ 6.8 repeats per key); candidate region = plateau₁ ∩ plateau₂.
Nonempty ⇒ recommend min N, then min M (lowest latency, most
interruption-tolerant), and require the recommendation's grid neighbors to sit
in or within one key of the region (no isolated spikes — the t1898 lesson).
Empty ⇒ no transferable rule at this budget; record and close per the gates
below.

**Harness-correctness gates (both passed 2026-07-19).** (1) M = N = 3
reproduces `validate_config.py` exactly on piano-1: discrete 76/87
(b21/m33/t22) and refined 77/87 (b20/m33/t24) with the A′ failure lists
key-for-key. (2) The full-window plurality ceiling recomputed from the cache
matches A′'s gross ceilings (discrete 83; refined 81 strict — key 000's dead
tie appears exactly as A′ recorded it).

**Outcome gates.** *Support*: a concordant (M, N) improves or holds the total
on **both** instruments with ≥ 2 net fixes on at least one and no register
collapse ⇒ status here moves to "replay supports M-of-N"; the hot-path design
note is the next step, still nothing built without user approval. Known
non-targets stay non-targets: the two genuine-flicker keys and the refined
000/001 near-tie races are outside what plain winner votes can fix. *Refute*:
no concordant pair beats both baselines within budget ⇒ close the M-of-N
branch in ADR 0006 item 5. Either way **MSPRT stays gated** behind its
likelihood-bridge problem — this replay neither authorizes nor needs it.

**Artifacts.** Harness: `scripts/replay_lock_rules.py` (kept — standing,
stdlib-only). Caches/surfaces: `replay_cache/` (gitignored, regenerable).
Results record: ADR 0010. Nothing committed (standing rule).

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
