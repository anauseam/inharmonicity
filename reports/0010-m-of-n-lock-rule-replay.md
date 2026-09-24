# report 0010 — offline M-of-N lock-rule replay

This is the project's cleanest confirmatory measurement, and the one the methods
standard in [`README.md`](README.md) points at first: the replay protocol, the
support outcome gate, the concordance criterion, **and what M-of-N was not
expected to fix** were all written into
[`report 0010` appendix](0010-m-of-n-lock-rule-replay.md#appendix--the-design-notes-derivation-trail)
while the design was still "build nothing", then measured on two instruments.

The concordance criterion is the part worth borrowing. It exists because of a
specific earlier failure — a best-of-N configuration (trial 1898) chosen on one
instrument's pass count, which did not survive a bug fix. Requiring a *broad
plateau whose interior is smooth on both instruments*, rather than an argmax, is
the anti-t1898 test.

## Where this stands

**Measurement on record (2026-07-19). Acquisition-only rule IMPLEMENTED
(2026-07-20).** The v1 hot-path lock (`Engine::record_stable_winner`,
refined default (M, N) = (7, 8)) and the matching `validate_config.py` /
`test_engine_all.py` known-answer replicas (`--lock-m/--lock-n`) now ship and
reproduce the numbers below exactly; the standing build-nothing gate was cleared
by the user directing the implementation. Lock-release/re-lock hysteresis remains
the deferred second design (see Limitations). This report
executes gate 2 of
[`report 0010` appendix](0010-m-of-n-lock-rule-replay.md#appendix--the-design-notes-derivation-trail)
(the "Replay protocol" section there carries the pre-registered decisions;
this report carries the numbers). The pre-registered *support* outcome gate is
**met on both discovery modes and both instruments**: a concordant (M, N)
region exists in which M-of-N binary integration fixes the early-wrong-lock
class with small, priced regressions and bounded latency. The next step is a
hot-path design note (lock acquisition *and* release semantics — see
Limitations); **nothing ships from this report alone**, per the standing
build-nothing-without-approval rule. MSPRT/score-level fusion remains gated
behind its likelihood-bridge problem and is neither authorized nor needed by
this result. **(Update 2026-07-22: the cheap version of that branch — mean and
rank-sum fusion of per-frame TWM scores — was run offline on both instruments
(a one-off probe, not retained; method and numbers in the design note) and
REFUTED: it never beats winner-voting and
fixes none of the B-limited residuals. Score-level fusion is closed; see
`report 0010` appendix.)**

## Context

The production Discovery lock rule is *first key to win 3 consecutive stable
frames*. The 2026-07-02/07-05 failure taxonomy (report 0006 items 2/5)
showed its precise failure mode: **first-to-3 is a race the attack transient
runs first** — on the failing keys the wrong key wins the first stable frames,
then the true key takes the plurality of the note body. Measured gross
plurality ceilings (full-window vote): discrete 76 → 83, refined 77 → 81 on
the 87 piano-1 captures. The design note's gate 2 prescribed exactly this
experiment: an offline replay of the published **M-of-N binary-integration
rule** (Schwartz 1956; Shnidman 1998) over the existing winner sequences,
before any engine code is touched.

Three decisions were made by the user on 2026-07-19 before execution:
latency cap **N ≤ 21** (~0.49 s worst-case window at the 23.2 ms stable-frame
hop) applied at selection time only; **both instruments swept, concordance as
the evidence standard** (the 595 piano-2 repeat dumps have genuine audio and
folder-name ground truth, making this the project's first two-instrument
lock-accuracy experiment); full execution in-session. The selection criterion
(plateau tolerances 1 key / 7 dumps ≈ 1 key-equivalent, region intersection,
min-N-then-min-M, neighbor spike guard) was fixed in the harness defaults
before the first surface was computed.

## Method

Harness: `tuner-lab/scripts/replay_lock_rules.py` (kept; stdlib-only, three
subcommands `cache`/`sweep`/`concord`). The cache phase regenerates
`gatekeeper.csv`/`peaks.csv` per capture under the shipped
`TwmConfig::default()` (telemetry build — mirroring `validate_config.py`
exactly) and extracts the stable-frame winner sequence with the identical
merge. The rule: at stable index t, lock the first key holding ≥ M votes in
the last min(t+1, N) winners; M > N/2 enforced (majority ⇒ unique winner, no
tie rule; partial windows let clean evidence lock early). **M = N = 3 is
exactly the production rule** and is the correctness gate.

**Harness-correctness gates — both passed.**

1. M = N = 3 reproduces `validate_config.py` on piano-1 exactly: discrete
   **76/87** (b21/m33/t22), refined **77/87** (b20/m33/t24), failure lists
   key-for-key identical to the 2026-07-05 re-derivation (discrete
   000/004/005/010/012/080/081/082/084/085/086; refined
   000/001/002/005/010/012/080/082/086/087).
2. The cache-recomputed full-window plurality ceilings match A′: discrete
   **83/87**; refined **81/87** strict, 82 with ties-as-pass — key 000's dead
   tie appears exactly as A′ recorded it.

## Results

### Baselines (first piano-2 discovery numbers on record)

| mode | piano-1 (87) | piano-2 (595 dumps) | piano-2 % |
| --- | --- | --- | --- |
| discrete | 76/87 | 533/595 | 89.6 % |
| refined | 77/87 | 530/595 | 89.1 % |
| plurality ceiling (strict) | 83 / 81 | 573 / 578 | 96.3 / 97.1 % |

The two instruments' per-capture accuracies are in family (87–90 %), and the
plurality ceiling shows the same ~7 pp of decision-level headroom on the new
instrument — the early-wrong-lock phenomenon **transfers across instruments**.
Piano-2's failures concentrate in the same registers (extreme treble ≥ key 75,
a deep-bass/low-mid cluster, plus a mid-register C#5 impostor case).

### Concordance selections (pre-registered criterion, N ≤ 21)

**Refined (the production-weighted mode): recommendation (M = 7, N = 8)** —
lock on 7 votes of the last 8 stable frames (tolerates one dissenting frame).
Candidate region 38 pairs; neighbors smooth (no spike).

| | baseline | (7, 8) | fixed | broken | added latency (stable frames, med/p95/max) |
| --- | --- | --- | --- | --- | --- |
| piano-1 | 77/87 | **81/87** (b21/m33/t27) | 001, 002, 080, 082, 087 | 003 | 4 / 12 / 15 |
| piano-2 | 530/595 | **568/595** (b134/m296/t138) | 44 dumps | 6 dumps | 4 / 11 / 22 |

**Discrete: recommendation (M = 8, N = 8)** (8 consecutive — the same rule
shape as production, longer). Region 29 pairs.

| | baseline | (8, 8) | fixed | broken | added latency |
| --- | --- | --- | --- | --- | --- |
| piano-1 | 76/87 | **82/87** (b22/m33/t27) | 004, 080, 081, 082, 084, 085 | — | 5 / 12 / 14 |
| piano-2 | 533/595 | **561/595** (b136/m296/t129) | 35 dumps | 7 dumps | 5 / 14 / 29 |

Seven pairs sit in **both** modes' regions — minimal member **(M = 10,
N = 14)** — so a single mode-agnostic rule exists if the design phase wants
one; the per-mode recommendations above are the latency-optimal choices.

Median added latency 4–5 stable frames ≈ **93–116 ms** (clean-signal bound:
the (7,8) rule fires after 7 straight, i.e. +4 frames over baseline —
matching the observed median exactly). p95 ≈ 280–330 ms; worst observed single
capture +22–29 frames (~0.5–0.7 s) — those are dirty sequences where the old
rule locked early *wrong* on siblings of the same key.

### Latency–accuracy tradeoff across (M, N) — the low-latency alternatives

The shipped **(7, 8)** is the *robustness-optimal* concordance pick (broadest
two-instrument plateau), **not** the latency-optimal one. Most of the accuracy
gain is available for far less added latency; these points (refined mode, from
`replay_cache/p{1,2}_refined_surface.csv`, 1 stable frame ≈ 23 ms) are on record
so a live latency-budget decision can be made against data rather than by feel.
"Added latency" is the extra stable frames to lock vs the old rule, on notes
both rules lock correctly (median; p95 in the last column).

| (M, N) | P1 /87 | P2 /595 | added lat. median | p95 |
| --- | --- | --- | --- | --- |
| (3, 3) old rule | 77 | 530 | 0 | 0 |
| (4, 4) | 80 | 550 | **+1f (≈23 ms)** | +8f |
| (5, 6) | 80 | 562 | **+2f (≈46 ms)** | +3f |
| **(7, 8) shipped** | **81** | **568** | +4f (≈93 ms) | +12f |
| (7, 13) | 81 | 573 | +4f (≈93 ms) | +8f |
| (8, 14) | 81 | 575 | +5f (≈116 ms) | +9f |

Reading it: **(4, 4)** buys +3 P1 keys / +20 P2 dumps for a single extra frame;
**(5, 6)** captures nearly the whole win (P1 80, P2 562) at +2 frames and, by
tolerating one dissenting frame, has a *tighter tail* than the zero-tolerance
(4, 4) (p95 +3 vs +8 frames). (7, 8) spends ~47 ms median beyond (5, 6) for the
last +1 P1 key (its strict plurality ceiling) and +6 P2 dumps; P1 has plateaued
by (7, 8), and larger N only trades a little more P2 for more latency. The
`(M, N)` are named constants in `engine.rs` — dropping to **(5, 6)** is a
one-line change and the recommended latency-first alternative if the cumulative
pipeline budget is the binding constraint. (These are still the same two
uprights; the post-tuning re-replay refreshes the numbers.)

### Per-key movement highlights (refined @ (7,8), piano-2 repeats)

- **key 81 F#7: 0/5 → 5/5**, key 52 C#5: 9/16 → 16/16, key 66 D#6: 3/6 → 6/6,
  key 83 G#7: 3/5 → 5/5, key 59 G#5: 3/5 → 5/5 — whole keys move from broken
  to consistently correct, not just individual dumps.
- Deep bass moves too: keys 3/4/13/17 each gain a dump; on piano-1 the fixes
  include **001 (A#0) and 002 (B0)** — confirming A′'s finding that the
  early-wrong-lock class includes deep bass on fixed peaks.
- **Regressions (priced, not hidden):** key 87 C8 2/5 → 0/5 (the last key;
  its true-key evidence is so thin that a longer window lets an impostor
  accumulate — and piano-1's (7,8) *fixes* its 087, so this is
  instrument-specific evidence thinness, not a systematic top-octave law),
  key 57 F#5 5/5 → 3/5, key 82 G7 0/6 → 2/6 (partial recovery), piano-1 003
  C1 breaks. Net: every register improves or holds on both instruments;
  the symmetric-exposure trap (longer windows helping impostors) is real
  but small — 0–7 broken vs 35–50 fixed.

### What M-of-N does not fix (as pre-registered)

Piano-1 refined 000 (A0, dead-tie plurality) and 010/012 (stable-wrong
mid-bass) stay failed — winner votes cannot flip a key that never out-votes
the impostor. Notably, piano-1 refined at (7,8) scores 81/87 = **the strict
plurality ceiling itself**: within budget, M-of-N collects *all* of the
plurality-flip headroom on that surface. The residual gaps elsewhere
(piano-2 refined 568 vs ceiling 578) are stable-wrong/near-tie cases, which
belong to the constants/B-template levers, not this one.

## Interpretation

1. **The mechanism is confirmed, on a second instrument.** The attack
   transient wins first-to-3 races; a window that outlasts it (≈ 8 stable
   frames ≈ 190 ms) recovers the note-body plurality. The effect size is
   +4–6 keys/87 and +28–38 dumps/595 — the largest single lever measured on
   the discovery side, delivered by a published nonparametric rule with two
   integer parameters.
2. **Concordance held.** The plateaus are broad (29–47 pairs per surface),
   intersect widely, and the recommendations sit mid-plateau with smooth
   neighbors — the anti-t1898 test this protocol was designed around.
3. **Latency is inside the user's budget.** Median ≈ 100 ms added; the rule
   only spends latency where evidence is actually contested.

## Limitations / threats to validity

- **Acquisition-only.** The replay models the *initial lock* decision on
  isolated single-note captures. Production adds lock-release, re-lock during
  continuous playing, and interaction with capture arming. The hot-path
  design note must specify M-of-N acquisition *plus* release/hysteresis
  semantics before any implementation.
- Both instruments are uprights, neither in tune; the post-tuning re-replay
  (cheap: rerun `cache` + `concord` on new dumps) is the planned refresh.
- Piano-2 repeat dumps are correlated within keys (same instrument state,
  same session); the 7-dump plateau tolerance approximates 1 key-equivalent
  rather than treating dumps as independent.
- The ms conversions assume contiguous stable frames (the gatekeeper can
  interleave non-Stable frames; stable-frame counts are the exact metric).
- Sequences come from offline replay of captured audio through the identical
  engine code (`diagnose_engine`), not from live runs.

## Artifacts & reproduction

- Harness: `tuner-lab/scripts/replay_lock_rules.py` (standing).
- Caches + full surfaces: `replay_cache/` (gitignored, regenerable):
  `p{1,2}_{discrete,refined}.csv` + `*_surface.csv`.
- Reproduce:
  `python3 tuner-lab/scripts/replay_lock_rules.py cache --base diagnostics_piano_1 --out replay_cache/p1_refined.csv --refine`
  (etc.), then
  `python3 tuner-lab/scripts/replay_lock_rules.py concord --p1 replay_cache/p1_refined.csv --p2 replay_cache/p2_refined.csv`.

## References

Schwartz 1956 (IRE Trans. IT 2(4), the coincidence/M-of-N procedure);
Shnidman 1998 (IEEE Trans. AES 34(3), binary integration); full list in
`report 0010` appendix.

## Appendix — the pre-registration, and the branch it closed

The design note this rule came from is superseded by the report above; git
history holds it in full. What is kept is the part the methods standard points
at — the replay protocol, whose decisions and concordance criterion were written
while the design was still "build nothing" — plus why MSPRT stays gated and why
score-level fusion was closed.

### The faithfulness gap (why MSPRT is not a drop-in)

- **TWM error is not a likelihood.** SPRT/MSPRT consume log-likelihood ratios; a
  bespoke `exp(−score)` mapping is exactly the kind of unpublished assembly this
  project's record says underperforms. A published bridge exists — Duan, Pardo &
  Zhang 2010's spectral likelihood model — but note its *scoring proxy* was already
  rejected on real captures (report 0006). Using it as a *temporal fusion* weight is a
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

### Decision

**Build nothing now.** Gates, in order:

1. Second-instrument validation resolves which constants ship (report 0006 item 2);
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


### Score-level fusion: tested and refuted (2026-07-22)

Before investing in a likelihood bridge, the cheap version was run as a one-off
offline probe: all 88 keys scored per stable frame with the same
`twm::score_candidate` discovery uses, then fused across the stable window two
ways — mean-score, and Borda rank-sum as a magnitude-robust control — against
plurality voting, on both instruments in discrete and refined scoring. The probe
was not retained; this is the record, and it is reconstructable from this
description.

**Neither fusion ever beat voting, and both regressed.** The mechanism is that a
dead or mis-measured partial poisons a frame's *score* while voting is immune,
because voting only counts wins. That vindicates M-of-N as the correct temporal
integrator for this signal, and warns that a full MSPRT would inherit the same
treble vulnerability unless its likelihood explicitly down-weights dead-partial
frames. With two simple fusions regressing on two instruments in both modes, and
the residuals independently B-limited (report 0006 item 5), the burden of proof
for the heavy likelihood-bridge version is not met. **Do not reopen without a
specific new reason** — for instance a future instrument whose residuals are
shown to be genuine flicker rather than stable-wrong or B-limited.

### Replay protocol (decided 2026-07-19 — gate 2 executed)

All parameters below were fixed **before any sweep surface was computed** (the
selection criterion is baked into `tuner-lab/scripts/replay_lock_rules.py`'s defaults);
the three user decisions (latency budget, data protocol, execution scope) are
on record from 2026-07-19. Results: `reports/0010-m-of-n-lock-rule-replay.md`.

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
constants sweep (the 2026-07-05 re-derivation: no candidate beats the default).

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
branch in report 0006 item 5. Either way **MSPRT stays gated** behind its
likelihood-bridge problem — this replay neither authorizes nor needs it.

**Artifacts.** Harness: `tuner-lab/scripts/replay_lock_rules.py` (kept — standing,
stdlib-only). Caches/surfaces: `replay_cache/` (gitignored, regenerable).
Results record: report 0010. Nothing committed (standing rule).
