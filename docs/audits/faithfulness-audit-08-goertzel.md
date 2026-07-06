# Faithfulness audit 08 — Goertzel + phase-vocoder tracking in `engine.rs`

**Series:** Prompt B faithfulness audits (status table in `faithfulness-audit-01-twm.md`), item 8
of 8 — **the series is complete with this audit.**
**Date:** 2026-07-04.
**Scope:** `spectral.rs::goertzel` and the tracking/decision logic around it
in `engine.rs` (per the handoff: "textbook algorithm; verify the tracking
window/decision logic around it is documented as ours"). Bases: Goertzel 1958
(textbook recurrence), Sysel & Rajmic 2012 (non-integer-frequency use),
McAulay & Quatieri 1986 + Dolson 1986 (cited in-code for the phase tracking),
Kay 1998 (the amplitude gate — same framework audit 04 verified for
Discovery).

## Verdict summary

| # | Item | Classification |
| --- | ---- | -------------- |
| 1 | Goertzel recurrence + finalization | (a) textbook-correct (derivation verified) |
| 2 | Phase semantics at non-integer bins | (a) sound **for the engine's differencing use**; constraint now documented |
| 3 | 4/N amplitude normalization; Hann window | (b) ours, correct (coherent gain 0.5 × single-sided 2), documented |
| 4 | `NEYMAN_PEARSON_K = 0.201184` | (a/b) verified exact against its stated Kay derivation; consistent with Discovery's threshold |
| 5 | Phase-vocoder f_inst (MQ 1986) + unwrap | (a) faithful to the cited basis; ±21.5 Hz unwrap range comment verified |
| 6 | Adaptive re-centering (0.95/0.05, gate-first) | (b) ours (Dolson cited as concept), documented; transient bias analyzed below — benign |
| 7 | Partial-1-only cent meter; warmup hop; refined-series seeding | (b) ours, each documented with rationale in place |
| 8 | Missing citation on `goertzel` itself | (c) fixed — Goertzel 1958 + Sysel & Rajmic 2012 added |

## Findings

**1–2. The Goertzel core is textbook-correct, and the phase subtlety is now
stated.** Verified: the recurrence `q0 = 2cosω·q1 − q2 + x` with finalization
`real = q1 − q2·cosω, imag = q2·sinω` computes `s[N−1] − e^(−jω)s[N−2]` =
e^(jω(N−1))·X(ω) — i.e. the DTFT value times a constant phase factor. At
integer bins the factor vanishes (classic Goertzel); at the engine's
non-integer targets it does not. **Magnitude is exact regardless; absolute
phase is offset by a constant per target.** The engine only ever uses
hop-to-hop phase *differences* at a fixed target, where the offset cancels
exactly — so the tracking is sound — but a future consumer reading `phase` as
the DTFT phase would be wrong. That constraint (plus the missing citations)
is now one short block in the doc-comment.

**3. Normalization/window: ours, correct.** 4/N = (1/Hann coherent gain 0.5)
× (2 single-sided) / N — amplitudes come out in physical time-domain units,
which is what makes the Kay threshold below dimensionally coherent.
`HANN_1024` uses the same [0, N−1] convention as `fft`.

**4. The amplitude gate constant verifies exactly.** T_amp = σ·K with
K = (4/HOP)·√(0.375·HOP·(−ln 10⁻³)) = 0.201184 — recomputed, matches to all
printed digits. This is precisely Discovery's Neyman–Pearson Rayleigh
threshold (audit 04 verified that derivation) rescaled into `goertzel`'s
normalized-amplitude units. Same P_fa = 0.1 %, same Kay citation, consistent
framework across both consumers of the noise floor.

**5. Phase-vocoder instantaneous frequency: faithful to its cited basis.**
`f_inst = f_target + wrap(φ_n − φ_{n−1} − 2πf_target·t_hop)/(2π·t_hop)` is
the McAulay–Quatieri phase-difference estimator (cited in-code); the
principal-value wrap via `rem_euclid` is correct; the ±1/(2·t_hop) = ±21.5 Hz
unwrap range quoted in the lock-seeding comment is exact, and seeding the
trackers from the *refined* (scale-corrected) series rather than ET —
precisely to stay inside that range at high n — is ours and documented.

**6. Adaptive re-centering: ours, benign — transient bias derived.** The 5 %
EMA of the evaluation center toward f_inst (gated on SNR survival) is our
mechanism; Dolson 1986 is cited as the concept source, fairly. One subtlety
the audit derived: when the target moves by Δf between hops, the two phases
entering the difference were measured at *different* ω (both the window's
linear phase and the finalization offset shift), producing a one-hop f_inst
bias ≈ +Δf·(N−1)/(2·HOP) ≈ 0.5·Δf. With the 5 % step and the ±21.5 Hz range,
that is ≤ ~0.5 Hz for one hop, decays with the step size, and is zero in
steady tracking (Δf → 0). It mildly increases the effective adaptation gain
(still ≪ 1, stable). Not worth hot-path correction or a code comment beyond
this record.

**7. Decision logic around the tracker: all ours, all documented in place.**
The 3-consecutive-stable-frame lock gate (Discovery side), refined-series
tracker seeding, one-hop warmup before the first derivative, the
amplitude-squared weighting, and the Partial-1-only cent-meter policy (with
its B-immunity rationale and the deliberate None fallback for dead bass
fundamentals) — none claims a paper basis, each carries its engineering
rationale. This is the correct posture; nothing to reclassify.

**8. Fix applied (comment-only):** `goertzel` doc-comment now cites Goertzel
1958 and Sysel & Rajmic 2012, marks the window/normalization as ours, and
states the phase-offset constraint. 28/28 lib tests pass.

## Series wrap-up

All eight inventory items are complete. Tally of the series' substantive
findings: one real numerical bug (jacobsen, −2.5δ bins — fixed, baselines
moved 74/87 → 76/87 discrete / 77/87 refined), five citation defects (a
fabricated Gómez section, a phantom "Miron 2014", a false "Rigaud Fig. 3"
σ attribution, a misattributed NINOS², a wrong "§7"), two stale-doc classes
(Hamming/Hann, Simultaneous-default), one measured reclassification
(mask_peaks and ninos2 as validated bespoke heuristics, with the sparsity
A/B quantifying where each beats the faithful alternative), and confirmation
that the load-bearing ports (TWM, CSPE, MAT, Rigaud's treble universals, the
Kay thresholds) are faithful — several verified to the last constant. The
running status table lives in `faithfulness-audit-01-twm.md`.
