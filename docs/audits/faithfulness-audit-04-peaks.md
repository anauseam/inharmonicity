# Faithfulness audit 04 — `peaks.rs` (`extract_peaks`, `mask_peaks`) vs their cited bases

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 4 of 8.
**Date:** 2026-07-04.
**Sources checked:**

- Gómez, E. (2006). "Tonal Description of Music Audio Signals." PhD thesis,
  MTG-UPF — **primary source obtained** (full text, tdx.cat) and searched.
- Cano, P. (1998). "Fundamental Frequency Estimation in the SMS Analysis,"
  DAFx-98 — **primary source obtained** (user-supplied,
  `resources/engine/CAN65.PS.pdf`) and read; §4.3 settles the dynamic-range
  attribution (see finding 5, resolved).
- Kay, S.M. (1998). Fundamentals of Statistical Signal Processing (the
  caller-side threshold's citation) — checked against the in-comment derivation.
- ADR 0002 (`0002-twm-peak-masking-validation.md`, 2026-05-28) — the mask's
  actual empirical provenance.

**Scope:** `tuner-core/src/algorithms/peaks.rs` and the `min_magnitude`
contract at its caller (`engine.rs` Discovery). **No behavior changed; no code
comments changed yet** — the reclassification below is queued for user review.

## Context: the handoff premise was wrong

The inventory said "peaks.rs vs its cited basis (Miron 2014 per
docs/internals/04)". **"Miron 2014" is a phantom citation**: it appears only in
internals/04's *example sentence* about citing sources and in records derived
from that sentence — nothing in the codebase cites it, and no Miron-based
algorithm exists here. The actual cited bases in the code are **Gómez 2006
§3.1.2.2 + Cano 1998** (on `mask_peaks`) and **Kay 1998** (on the caller's
threshold). The user's intuition ("isn't this mostly an amplitude gate?") is
correct for `extract_peaks`.

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | `extract_peaks`: local-maxima walk + absolute gate + Candan sub-bin + sort | (a) textbook/SMS-consistent; makes no paper claim — correct posture |
| 2 | Caller threshold: Neyman–Pearson AWGN magnitude gate (Kay 1998 eq. 7.26) | (b) documented analytical adaptation — derivation verified |
| 3 | `mask_peaks` **Gómez citation is false** (primary source) | (c) **misattribution** — thesis has no §3.1.2.2 and no masking at all |
| 4 | `mask_peaks` doc says −40 dB; constant is −30 dB | (c) internal inconsistency — the **comments** are wrong (ADR 0002 validated −30) |
| 5 | Cano 1998 "SMS dynamic range" attribution | **resolved** — Cano §4.3 does specify 40 dB; ours is a documented 40→30 dB adaptation (ADR 0002) |
| 6 | `mask_peaks` mechanism itself | **(b′) validated bespoke heuristic** — reclassify honestly, keep (ADR 0002: 8/8, replaced the failed geometric gate) |
| 7 | "Miron 2014" in internals/04 example list | (c) phantom citation — records fix |

## Findings

**1. `extract_peaks` (peaks.rs:36–88) — no paper claim, none needed.** Local
maxima (`mag[i] > both neighbours`), absolute magnitude gate, sub-bin
refinement via the (just-audited-and-fixed) Candan estimator, magnitude-sort.
Structurally identical to the SMS peak detection Gómez actually describes
(§3.2.4: local maxima over a threshold, sub-bin interpolation — she uses
parabolic-in-dB; our Candan estimator is the better-founded equivalent and is
separately audited). The doc-comment claims only "local maxima + threshold +
Jacobsen", which is accurate. Minor notes: the 128-entry temp cap and the
`frequency > 0.0` admission check are ours and self-evident; boundary bins
excluded (consistent with any three-point method).

**2. Caller-side `min_magnitude` (engine.rs, Discovery block) — verified.**
The threshold is not a tuned constant: it is a Neyman–Pearson false-alarm
threshold for AWGN, `T = sqrt(−p_bin·ln P_fa)` with `p_bin = σ²·Σw²`,
`Σw² = 0.375·N` (exact for Hann), `P_fa = 10⁻³`, citing Kay 1998 eq. 7.26.
Checked: the magnitude of a windowed-DFT bin of white noise is Rayleigh with
power `σ²Σw²`, giving exactly `P_fa = exp(−T²/p_bin)`. Faithful analytical
adaptation, fully documented in place. (Where `σ` = `self.noise_floor` comes
from is gatekeeper telemetry — that lineage belongs to audit item 5.)

**3. The Gómez citation on `mask_peaks` is false — primary source checked.**
The doc-comment cites "Gómez (2006), Section 3.1.2.2" for the masking rule and
labels the constants "Canonical Gómez/Essentia Defaults". The thesis (full
text searched):

- has **no section 3.1.2.2** (the TOC goes 3.1, 3.2, 3.2.2.1, 3.2.2.2, …);
- contains **no peak-masking procedure anywhere** — the word "mask" does not
  appear in the document;
- its peak selection (§3.2.4–3.2.5) is: SMS local maxima, parabolic
  interpolation, a **−100 dB-of-maximum** magnitude threshold, and a
  **[100, 5000] Hz** band filter. No −30 dB relative threshold, no 20 %
  proportional bandwidth, no dominance ordering.

None of `mask_peaks`' three constants trace to the cited source.

**4. Internal inconsistency: −40 dB (comments) vs −30 dB (code).** The
doc-comment says peaks "outside the 40 dB dynamic range of the global maximum"
are discarded, and the inline comment says "-40 dB from the global maximum" —
but `GLOBAL_THRESHOLD_DB = 0.0316` is **−30 dB** (as its own name-line comment
says). ADR 0002 records the mask as introduced and validated at **−30 dB**, so
the constant is the load-bearing, validated value and the prose is wrong.
Queued fix: correct the comments; do **not** touch the constant.

**5. Cano 1998 — RESOLVED (user supplied the PDF same session).** §4.3 "Peak
selection" reads: *"in order for a peak to be accepted it has to be less than
40 dB below the highest peak and it has to have a minimum bandwidth"* (plus an
optional phase-slope criterion). So the −40 dB prose in the old doc-comment
was accurately describing **Cano's rule**, while the code ships the **−30 dB**
that ADR 0002 validated. Final classification of the global gate: **(b)
deliberate adaptation of Cano §4.3** with a documented value change
(40 → 30 dB, empirically validated). Cano's *other* criteria (minimum
bandwidth, phase slope) are different mechanisms and are not implemented —
and §4.3 contains **no dominance-masking procedure**, confirming the masking
itself is ours (finding 6). Cano §4.2's candidate-search optimization and
§4.4's F0 tracking are unrelated to `mask_peaks` (and, incidentally, our
88-key candidate set is a stronger form of his candidate restriction).

**6. Reclassification: `mask_peaks` is a *validated bespoke heuristic* — keep
it, relabel it.** The mechanism (dominance-ordered masking within a
**proportional 20 % band** at a **−30 dB relative threshold**, plus a −30 dB
global dynamic-range gate) does not come from the cited papers, but it:

- has a real empirical record: **ADR 0002** (2026-05-28) — introduced to
  replace the failed geometric gate; 8/8 keys, zero false locks on the
  first real captures; known SNR-floor limitation documented there;
- satisfies internals/04's *allowed heuristic* rules: fully scale-invariant
  (dimensionless dB ratios; percentage-of-frequency bandwidth — the 20 % band
  is recognisable as the textbook critical-band approximation CB ≈ 0.2·f
  above ~500 Hz (Zwicker), which is *inspiration*, not a port);
- is load-bearing in the shipped Discovery chain (engine.rs runs it on every
  discovery frame).

This audit therefore does **not** propose removing or changing the mechanism —
only labeling it truthfully: the project's faithful-ports principle counts
wins by lineage, and `mask_peaks` currently sits in the "faithful port" column
when it is actually the codebase's one **empirically-validated bespoke**
component. (A useful nuance for the principle itself: the failed bespoke
assemblies were *scoring-objective* bolt-ons; this is a *pre-filter* validated
in isolation.)

**7. Phantom "Miron 2014" (records).** Remove or replace the reference in
internals/04's example sentence (it names a paper nothing uses); the handoff
and audit-01 table entries derived from it are corrected as part of this
audit's bookkeeping.

## Resolution (2026-07-04, same session — all fixes APPLIED on user go-ahead)

User confirmed the reclassification ("this is the codebase's own heuristic
until we can find a better way for peak detection") and supplied Cano 1998.
Applied, all comment/records-only, zero behavior change:

1. `mask_peaks` doc-comment rewritten: provenance section (ours, ADR 0002
   empirical basis); global gate credited to Cano §4.3 with the documented
   40 → 30 dB adaptation; masking marked ours with the critical-band
   approximation as inspiration; the false Gómez §3.1.2.2 citation removed
   (with a note recording that it was wrong, so it can't silently return).
2. −40 dB prose corrected to −30 dB at both sites; constants unchanged and
   now labeled "OURS, ADR 0002-validated".
3. internals/04: phantom "Miron 2014" replaced with Short & Garcia 2006, and
   the citation guidance strengthened (verify against the actual source; if
   the mechanism is ours, cite the validating ADR instead).
4. `extract_peaks` doc: `min_magnitude` contract note added (Neyman–Pearson
   threshold computed by the caller, Kay 1998 — see engine.rs).

## Audit series status

Item 4 complete (this doc). Running status table: `faithfulness-audit-01-twm.md`.
Next: item 5, `metrics.rs` vs the
gatekeeper papers (PDFs already in `resources/gatekeeper/`).
