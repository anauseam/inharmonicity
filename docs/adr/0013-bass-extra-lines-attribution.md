# ADR 0013 — The Bass Extra Lines: Attribution, and the Window They Are Measured Through

## Status

**INVESTIGATION COMPLETE 2026-08-12 (Prompt T). No code change.** The
experiments are `examples/strobe_replay` **E10–E12**, run on both pianos; they
join E1–E9 as standing regression material.

ADR 0012 §5 kept the bass out of unison assist's claimed scope because keys 0–27
produce a second spectral line on essentially every capture of both instruments
— including keys 0–11, which are **single-strung** — and what those lines are was
unestablished. This ADR settles what can be settled from the captures we have,
and says plainly what cannot.

Three questions were open. Two are now closed:

- **Are they a detector artefact of the 4096-sample bass window?** Partly, and
  the part is small: 0–20 % of trials against a 45–72 % prevalence (§1).
- **Is the 1024-sample window's under-filtering (design note E-Q) doing damage?**
  Its *strength* is now measured rather than its presence, and it is severe —
  which is what vindicates R3's long window in the bass and bounds where the
  short one is safe (§2).

The third — **what the lines are** — is not settled, and the measurements say
why it cannot be settled from these captures (§3–§5). Every attribution family
that predicts a *sparse* set of frequencies sits at its own permutation null on
both instruments; the families that hit anything are dense enough to hit by
chance. What survives is a structural fact, reproducible across both
instruments, that excludes the two leading candidates (§4).

## Context

The measurement phase (design note §5) established that the bass lines are real
spectral content, are not aliased neighbouring partials (E-N), and are not
constant in cents across partials. ADR 0012 §8 added that ~78 % of bass rank-2
lines match a peak in a full-rate DFT of the identical span, so they are not
fabricated by the zoom; but unlike the tenor and treble, the bass residue does
**not** look like the shoulder of a barely-separated pair — it carries little
energy excess and sits *further* from its sibling, not nearer.

The candidates carried forward, none excluded: polarization false beats;
longitudinal string modes (Conklin); sympathetic resonance from a neighbouring
key, which in the deep bass is a devastating confound because a semitone at
55 Hz is ~3.3 Hz and lands *inside* the ±21.5 Hz baseband; soundboard or bridge
mode coupling; or genuinely wide bichords on two neglected instruments.

The method is the one the measurement makes possible: every transverse partial
of every key is predictable from the measured `(f₀, B)`, so each candidate family
can be *predicted* and what is left over classified.

## 1. The bass configuration's own null — the artefact is real and bounded

Every synthetic trial behind ADR 0012 ran the treble's 1024-sample window, whose
baseband is critically sampled. The deep bass runs `goertzel_bass` (R3) against
the same 1024-sample hop, so its window overlaps 75 %, its baseband is
oversampled 4× and consecutive samples are correlated — the independence a CFAR
threshold assumes away. **E10a** runs the null there, driving synthetic audio
through the shipped bank at key 7 (E1, f₁ = 41.2 Hz, `B` the Rigaud medium prior),
probed at the bass's own displayed partial n = 6:

| case | 1024 @30 | 1024 @56 | 4096 @30 | 4096 @56 |
| --- | --- | --- | --- | --- |
| isolated line, SNR 40 | 0 % | 0 % | 0 % | 0 % |
| isolated line, SNR 15 | 0 % | 0 % | **10 %** | **8 %** |
| isolated line, SNR 6 | 0 % | 0 % | **18 %** | 0 % |
| whole partial series, SNR 40 | 10 % | **100 %** | 0 % | 8 % |
| whole partial series, SNR 15 | 0 % | **100 %** | **18 %** | 5 % |
| whole partial series, SNR 6 | 0 % | **100 %** | **20 %** | 5 % |

The A/B is the *same audio* through two reference sets that differ only in what
selects the window, so nothing but the window moves.

**The null does break in the bass configuration, at 5–20 % once SNR drops to 15
or below.** That is a genuine defect and it is the one the design note's
limitation predicted. It is also far too small to be the explanation: the panel
resolves ≥2 lines at the displayed partial on 72 % (piano 1, 25 captures) and
45 % (piano 2, 114) of bass captures, and on 63 % / 54 % of bass references over
the whole bank. Taken with ADR 0012 §8's 78 % full-rate match rate, **the bass
lines are not manufactured by the detector.** The artefact exists, it is bounded,
and it is worst on short records — which is consistent with `UNISON_MIN_BINS`
having been derived (§3 of ADR 0012) from an independence that does not hold
here.

**E10c** measures the mechanism directly. Complex correlation of the baseband on
noise-only input, pooled over 64 runs of 56 hops:

| window | ρ₁ | ρ₂ | ρ₃ | ρ₄ | N/N_eff |
| --- | --- | --- | --- | --- | --- |
| 1024 | 0.126 | 0.126 | 0.114 | 0.104 | 1.11 |
| 4096 | **0.654** | 0.211 | 0.150 | 0.154 | **2.01** |

`|ρ̂|` of independent samples estimates ≈1/√N = 0.134, not 0, so the 1024 row is
consistent with zero throughout — as it must be, its windows being disjoint. The
4096 row's ρ₁ = 0.654 is the textbook 75 %-overlap Hann correlation (0.659), and
the variance inflation is **2.0×**. ADR 0012 §1 conjectured that the effective
independent reference count in an oversampled baseband is "≈N/4, not the N/2 the
Hann-correlation halving assumes". That conjecture is now **measured and
confirmed**: the overlap costs exactly the extra factor of two.

**E10b** answers the question that decides whether the bass is servable at all.
Genuine pairs at the bass reference, 56-hop record:

| split Hz | 1024 P(2) | 1024 split | 4096 P(2) | 4096 split |
| --- | --- | --- | --- | --- |
| 0.7 | 0 % | — | 0 % | — |
| 1.0 | 0 % | — | 0 % | — |
| 1.5 | 100 % | 1.90 | 100 % | 1.67 |
| 2.0 | 100 % | 1.92 | 100 % | 2.08 |
| 3.0 | 100 % | 2.99 | 100 % | 2.99 |
| 5.0 | 100 % | 5.00 | 100 % | 5.00 |

**The bass configuration resolves real pairs exactly as well as the treble one
does**, at the same `2/T` limit and with the same accuracy above ≈1.5 × it. There
is no estimator-side reason to withhold the register.

## 2. The window question (design note E-Q), settled by strength

E-Q showed that taking one Goertzel output per hop is a decimation, that
alias-free operation needs `N > 4H` — 4096, not the shipped 1024 — and that an
interferer at +30 Hz duly produced a spurious line at its predicted fold.
E-T then refuted the hypothesis that aliasing was what produced the unexplained
lines, and E-U refused the 4096 window because it broke the null. What was never
measured is the folded interferer's **amplitude**, which is what decides whether
it out-ranks a genuine string. **E10e**, one true line plus one equal-amplitude
interferer δ Hz above it, 56-hop record, SNR 40:

| δ Hz | folds to | 1024 spur | 1024 amp | 4096 spur | 4096 amp |
| --- | --- | --- | --- | --- | --- |
| 5 | +5.0 | 100 % | −1 dB | 100 % | −2 dB |
| 10 | +10.0 | 100 % | −1 dB | 100 % | −5 dB |
| 15 | +15.0 | 100 % | −1 dB | 100 % | −13 dB |
| 21 | +21.0 | 100 % | −2 dB | 100 % | **−41 dB** |
| 30 | −13.1 | 100 % | **−3 dB** | 100 % | **−40 dB** |
| 43 | −0.1 | 0 % | — | 0 % | — |
| 55 | +11.9 | 100 % | −11 dB | 75 % | −61 dB |
| 65 | −21.1 | 100 % | −16 dB | 0 % | — |
| 86 | −0.1 | 0 % | — | 0 % | — |
| 110 | −19.2 | 100 % | −34 dB | 0 % | — |
| 150 | +20.8 | 100 % | −43 dB | 0 % | — |

**The fold is not weak.** At the 1024 window anything within the Hann main lobe
— ±86 Hz, i.e. ±2 bins — arrives at −1 to −16 dB and lands at an *arbitrary*
place in the baseband, wherever `δ mod f_hop` puts it. Only past the main lobe
does it fall to −34 dB and below. The 4096 window is the anti-alias filter
decimation theory says it is: −40 dB at the baseband edge and nothing admitted
at all past ±65 Hz.

The two empty rows are structural and worth recording: the 1024-sample window's
bin width is `f_s/1024` = 43.07 Hz, which **is** the hop rate, so its Hann nulls
sit exactly on the offsets that fold to zero. That coincidence protects δ ≈ k·43
Hz and nothing else.

**E10d** takes the same test across the compass — one string, its whole partial
series, 56 hops, SNR stated at the probed partial:

| key | note | f₁ Hz | n\* | spacing | folds to | R3 | 1024 @40 | @15 | 4096 @40 | @15 | rel amp |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 7 | E1 | 41.2 | 6 | 41.4 | −1.7 | 4096 | 100 % | 100 % | 2 % | 10 % | 0.507 |
| 12 | A1 | 55.0 | 6 | 55.2 | +12.2 | 4096 | 100 % | 100 % | 0 % | 2 % | 0.265 |
| 19 | E2 | 82.4 | 6 | 82.8 | −3.3 | 4096 | 100 % | 18 % | 0 % | 5 % | 0.014 |
| 24 | A2 | 110.0 | 6 | 110.7 | −18.5 | **1024** | **100 %** | **100 %** | 0 % | 0 % | 0.022 |
| 26 | B2 | 123.5 | 6 | 124.3 | −4.9 | **1024** | **100 %** | 0 % | 0 % | 2 % | 0.005 |
| 27 | C3 | 130.8 | 4 | 131.3 | +2.1 | 1024 | 0 % | 0 % | 0 % | 0 % | — |
| 31 | E3 | 164.8 | 4 | 165.6 | −6.7 | 1024 | 28 % | 0 % | 0 % | 0 % | 0.003 |
| 36 | A3 | 220.0 | 4 | 221.6 | +6.3 | 1024 | 0 % | 0 % | 0 % | 2 % | — |
| 43 | E4 | 329.6 | 2 | 331.0 | −13.6 | 1024 | 0 % | 0 % | 0 % | 0 % | — |
| 48 | A4 | 440.0 | 1 | 440.9 | +10.3 | 1024 | 0 % | 0 % | 0 % | 2 % | — |

Two conclusions, one reassuring and one not.

**R3's long window is what makes the deep bass usable at all**, for this consumer
as well as for the band it was derived for: without it a single string's own
neighbouring partials fold in and are reported as a second line in **100 %** of
trials, at up to −6 dB. The feature's home register is clear from key 27 up.

**Keys ~20–26 are a gap.** Their f₁ clears the 86 Hz R3 boundary, so they ship
the 1024 window, while the display table still puts the panel on partial **6** —
whose neighbours sit at ±110–124 Hz, just outside the main lobe but well inside
what the CFAR admits, and fold to −18.5 and −4.9 Hz. Synthesis puts a false
second line there in 100 % of trials at SNR 40. **This is a shipped
configuration, not a hypothetical**, and it is a plausible contributor for the
top of the bass band. It is *not* the deep bass, which runs 4096 and shows 0–2 %.
No change is proposed here: R3's boundary was derived for the band-slope readout
against a stated criterion (ADR 0011), and moving it for a second consumer needs
its own re-validation of E1–E5. Recorded as the next lever if the bass panel is
ever to assert anything.

## 3. Attribution by frequency coincidence fails, and the null is why

**E12** predicts each candidate family from the instrument's own measured
`(f₀, B)`, admits a candidate only if the analysis window's main lobe reaches it,
folds it into the baseband, and asks whether it lands within 0.5 Hz of the
reported line. The null repeats the identical test with the offsets **permuted
between lines of the same register**, which keeps the reference layout and the
offsets' own distribution and destroys only which line carries which. That null
is load-bearing: against a uniform redraw the Conklin family appeared to explain
+9 points, and the permutation shows most of that was the difference between two
distributions rather than a coincidence rate.

Excess over the permutation null, in points:

| family | P1 bass 4096 | P1 bass 1024 | P1 tenor | P2 bass 4096 | P2 bass 1024 | P2 tenor |
| --- | --- | --- | --- | --- | --- | --- |
| own partials (E-N) | +1 | −5 | −1 | +1 | +5 | +4 |
| neighbour ±1 semitone | +0 | +8 | −1 | −1 | +3 | −0 |
| neighbour ±2 semitones | −2 | +9 | −2 | −2 | +2 | −1 |
| neighbour ±12 (octave) | −6 | +8 | −9 | −1 | +3 | +7 |
| any key on the piano | −4 | +6 | −1 | +3 | −0 | +2 |
| Conklin mixing, any fᵢ ± fⱼ | +4 | +8 | −5 | +12 | +6 | +8 |
| phantom at this partial | +5 | −5 | +2 | +10 | +9 | +6 |

**Nothing replicates.** The largest excesses are +8 to +12 points against
absolute rates of 20–40 %, and no family is above its null on both instruments in
the same band. Two specific readings:

- **E-N is confirmed with the shipped estimator.** The struck key's own partials,
  folded, explain 1–5 % of extra lines against a null of 0–5 %. The design note's
  Python-model rule-out holds.
- **The dense families are uninformative by construction.** "Any key on the
  piano" explains 33–90 % of lines — and its null explains 30–83 %. A piano's
  spectrum, seen through a ±86 Hz window and then folded, has a candidate almost
  everywhere. This is the honest reason the sympathetic-resonance candidate
  cannot be settled from captures of single struck notes: *the prediction is not
  falsifiable at this density.*

The near-neighbour case can be settled without statistics, and is: at the bass's
displayed partial n = 6, a semitone away is ≈ 6 × f₀ × 0.0595 ≈ **15 Hz**, while
the observed extra lines sit at a median **|δ| of 5.3–5.5 Hz**. The neighbouring
key's partial is simply not where the line is. Sympathetic resonance from a
*freely ringing* neighbour is what that test excludes; a neighbour driven through
the bridge radiates at the driving frequency, not its own, and so produces no
second line at all.

## 4. What does replicate: the split is fixed in Hz, not proportional to it

**E12d** reports the discriminator's own fit — `ln Δ = ln a + p·ln f`, where
`p = 1` is a unison and `p = 0` a separation fixed in Hz — as a distribution
rather than a verdict:

| set | register | splits used | captures | median p̂ | p10 | p90 | \|p̂−1\|<3σ | \|p̂\|<3σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| piano 1 | bass | all | 25 | **0.09** | −0.50 | 0.71 | 56 % | **100 %** |
| piano 1 | bass | > 2 × 2/T | 25 | **0.12** | −0.17 | 0.69 | 56 % | 96 % |
| piano 1 | tenor | all | 22 | 0.28 | −0.91 | 1.04 | 64 % | 91 % |
| piano 2 | bass | all | 110 | **−0.02** | −0.46 | 0.65 | 36 % | **93 %** |
| piano 2 | bass | > 2 × 2/T | 108 | **−0.02** | −0.43 | 0.59 | 31 % | 95 % |
| piano 2 | tenor | all | 201 | 0.33 | −0.33 | 1.15 | 78 % | 87 % |
| piano 2 | treble | all | 21 | **1.00** | −0.06 | 1.38 | 76 % | 33 % |

There is a register gradient in the fitted law and it holds on both instruments:
**treble p̂ ≈ 1.0, tenor ≈ 0.3, bass ≈ 0.0.** The treble — where the feature is
most available and the strings are most nearly identical — fits the unison
hypothesis exactly. The bass does not, and is consistent with a separation fixed
in Hz on 93–100 % of captures.

The obvious threat to that reading is survivorship: a split at the record's own
limit is reported *at* the limit whatever the truth was (ADR 0012 §4), and the
limit is the same for every partial, which would manufacture `p̂ = 0` on its own.
The second row per register admits only splits wider than 2 × `2/T`, where §4
measures the reported separation to be exact. **The result does not move.**

This excludes both leading candidates for the bass:

- **A second string is excluded**, which is what ADR 0012 §5 already concluded
  from three other legs. A unison requires `p = 1`; the bass is 3–4 σ from it on
  most captures.
- **Phantom partials are excluded.** Conklin's nonlinear mixing puts a product at
  `fᵢ + fⱼ`, which lands *below* the transverse partial `n = i+j` by
  `f_n − (fᵢ+fⱼ) = (3/2)·B·f₀·i·j·n`, i.e. `Δ ∝ n³` for the dominant pair, i.e.
  `p ≈ 3`. **E12b** kills it independently: a phantom *must* sit below its
  partial, and the extra lines sit below on 59 % (piano 1) and 61 % (piano 2) of
  cases — a coin flip. **E12c** kills it a third time: predicted against observed
  offsets per partial, the ratio scatters over 0.9–22 and the within-0.5 Hz rate
  is 0–38 %, no better than the coincidence rates in §3. The free longitudinal
  series is untestable from `(f₀, B)` alone — it needs the string's length and
  `√(E/ρ)` — so what is excluded is Conklin's *predictable* half.

What is left is a family whose separation is constant in Hz across a note's
partials. That is the signature of a **modulation** — anything varying the
string's amplitude or frequency at a fixed rate puts sidebands at ±ν around
*every* partial — or of a false beat whose split happens not to scale.
**E12e** tests the modulation reading where three lines resolve, by asking
whether the two weaker ones are a symmetric pair about the strongest:

| set | register | triples | median \|d₁\| | median \|d₂\| | asymmetry | null |
| --- | --- | --- | --- | --- | --- | --- |
| piano 1 | bass | 71 | 3.83 | 5.92 | **0.45** | 0.83 |
| piano 1 | tenor | 61 | 3.18 | 5.97 | 0.76 | 1.00 |
| piano 2 | bass | 320 | 3.66 | 7.14 | **0.59** | 0.80 |
| piano 2 | tenor | 440 | 3.12 | 6.21 | 0.80 | 1.00 |

`0` is a perfectly symmetric pair, `1` two lines on the same side. The bass is
measurably more symmetric than its null on both instruments and the tenor is not
— but 0.45–0.59 is a long way from 0, so this is a tendency, not a mechanism.
**Recorded as suggestive, not as a finding.**

## 5. Fixed absolute frequencies: a positive control that fires, and nothing else

**E11** is Prompt T's second experiment and Prompt E's own cheap test: an
instrument or room resonance sits where it sits whatever is struck, so a line it
caused must reappear at the same **absolute** frequency under *other keys*.

| set | register | lines | shared | null | top bin | null bin |
| --- | --- | --- | --- | --- | --- | --- |
| piano 1 | bass | 260 | 38 % | 23 % | 7 keys | 3 |
| piano 1 | tenor | 199 | 23 % | 15 % | 3 | 2 |
| piano 2 | bass | 1165 | 42 % | **55 %** | 10 keys | 6 |
| piano 2 | tenor | 1520 | 35 % | **53 %** | 4 | 5 |
| piano 2 | treble | 393 | 27 % | 23 % | 3 | 3 |

The aggregate excess on piano 1 does not replicate — piano 2 is *below* its own
null. But the most-crowded bins are informative, and they carry the reference
partial each line came from:

- piano 2: **59.9 Hz × 10 keys × n∈{1,2}**, **60.4 × 10 × n{1,2}**, 53.5 × 9 ×
  n{1,2}, then 145.6 and 146.1 × 8 × n{2,3,4,5}
- piano 1: 170.7 × 7 × n{2..6}, 265.2 × 6 × n{3..8}, then **60.0 × 4 × n{1,2}**

**Mains hum is the positive control this test needed, and it fires**: a bin at
mains frequency, seen only through the n = 1 and n = 2 references — the only ones
whose window reaches 60 Hz — on both instruments. The test can see a
fixed-frequency interferer when one is there. That makes the absence of anything
comparable at the *displayed* partial the meaningful result: a handful of
identifiable lines at n = 1–2 are room-fixed, and the register-wide behaviour is
not.

The one loose end is piano-1's 170.7 Hz and piano-2's 145.6 Hz, each seen through
four or five different partials of many keys, both above their nulls. Those are
the shape a soundboard resonance would take, and they are exactly what Prompt E's
fingerprint experiment is for. They are too few to explain the register.

## Decisions

### D1 — The bass ships in v1, unchanged, and the wording changes

There is **no key threshold anywhere** in the code and there must not be
(ADR 0012 §5), so "shipping the bass" was never about removing a constant — it is
about what the panel is allowed to say there. Ship it:

- The estimator is sound in the bass configuration: genuine pairs resolve at the
  same limit and the same accuracy as in the treble (§1, E10b), so keys 12–27
  (bichord) are servable if a real unison is what is there.
- The discriminator already withholds the claim: measured over both instruments
  it asserts "unison" on 0 % (piano 1) and 4 % (piano 2) of bass captures, and
  §4 now explains *why* — the fitted exponent really is ≈0 there.
- The markers themselves are honest: real spectral content (§1), reproduced
  across independent strikes to 1 % (ADR 0012 §7), with the record's own
  resolution stated beside them.

What changes is only the language — `strobe_replay`'s register label drops "(out
of scope)" — because the register is not out of scope. Its *unison
interpretation* is what the discriminator withholds, per key, at runtime.

### D2 — The window stays 1024, and R3 stays as it is

ADR 0012 §1's decision survives with a better argument on both sides. The fold is
strong, not weak (§2), so the 1024 window really is exposed to anything within
±86 Hz of a reference; and the 4096 window really does break the null, now
measured in the Rust port at 5–20 % rather than the Python model's 45 %, with the
mechanism quantified (ρ₁ = 0.654, N/N_eff = 2.0). Neither number is small enough
to move the other. The revisit path is unchanged and now has its constant: a CFAR
re-derivation for a 75 %-overlapped baseband should use **N_ref/4**, and §1
measures the factor of two that justifies it.

### D3 — What the lines are stays open, and the next experiment is not an analysis

The decisive experiment is the **mute test** — record a bass note with one string
muted, then open — which is Prompt T's experiment 3 and Part 2 of **Prompt W**.
Nothing in the captures we have can replace it: §3 shows that a piano's own
spectrum is dense enough to "explain" any line by coincidence, and §5 shows that
the only fixed-frequency structure we can find is mains hum. A capture that
resolves two lines with **one string sounding** is a false beat by construction,
and there is not one of those in the project.

Two things Prompt W should now carry that it did not:

- **Single-strung keys 0–11 are the sharpest case**, not an afterthought: a solo
  capture there is already the positive control, no mute required.
- **Long records are the other half.** §4's law was fitted over partials at one
  record length; a 8–10 s capture both sharpens `2/T` by 6–8× and lets the fit
  see whether `p̂` stays at 0 when the estimator can no longer be blamed.

## Consequences

- No behaviour changes. `strobe_replay` gains E10–E12; ADR 0012's E1–E9 are
  untouched and still reproduce.
- ADR 0012 §5's separate investigation resolves here, and the design note's E-Q
  closes by measurement (§2) rather than staying "an open measurement".
- A **shipped-configuration gap** is on record at keys ~20–26 (§2) with no change
  proposed, because R3's boundary is the band-slope's constant and moving it
  needs E1–E5 re-validated.
- Prompt E gains a starting point it did not have: 170.7 Hz (piano 1) and
  145.6 Hz (piano 2), each seen through several partials of many keys.

## Limitations / threats to validity

- **Two instruments, both out of tune**, and the bass of both is in unknown
  unison condition. Per the n = 2 rule these captures validate; they cannot
  select a configuration.
- **The synthetic sources are independent damped exponentials.** Non-stationarity
  and Weinreich coupling are unmodelled, which is exactly the physics a
  modulation hypothesis (§4) would live in, so §1's null is a lower bound on what
  a real string can do to the detector.
- **The attribution's admission rule is the main lobe.** Content beyond it enters
  through sidelobes at −34 dB and below (§2) and was not offered as a candidate;
  widening the rule would raise every family's rate *and* its null.
- **`(f₀, B)` for the neighbour prediction comes from each set's own captures**,
  median per key, and piano 2's deep-bass entries predating the
  `MAT_SEED_TOLERANCE` fix are dropped by the harness's ±200 ¢ sanity check
  rather than regenerated (`06-capture-sets.md`).
- **E12d's fit has almost no lever arm**, as ADR 0012 §6 says: a handful of
  neighbouring partials barely separates a slope of 0 from a slope of 1. The
  p10–p90 spread of ±0.5 is the honest width, and the claim rests on the median
  agreeing across two instruments and on the treble landing at exactly 1.00.
- **E12e's symmetry statistic assumes the strongest line is the carrier.** With
  three lines that is an assumption, not a measurement.

## Artifacts & reproduction

```bash
cargo run --release --example strobe_replay -- diagnostics_piano_1
cargo run --release --example strobe_replay -- diagnostics_piano2
```

E10 is synthetic and runs whatever directory is passed; E11–E12 are real. E1–E5
are ADR 0011's, E6–E9 ADR 0012's, and none of them move.

## References

- Rohling, H. (1983). *Radar CFAR Thresholding in Clutter and Multiple Target
  Situations.* IEEE Trans. AES-19(4). — the admission gate whose independence
  assumption §1 measures.
- Welch, P. D. (1967). *The use of Fast Fourier Transform for the estimation of
  power spectra.* IEEE Trans. AU-15(2). — the overlapped-segment correlation
  §1 quantifies.
- Harris, F. J. (1978). *On the use of windows for harmonic analysis with the
  Discrete Fourier Transform.* Proc. IEEE 66(1). — Hann main lobe and overlap
  correlation; the ±2-bin lobe §2 turns on.
- Conklin, H. A. (1999). *Generation of partials due to nonlinear mixing in a
  stringed instrument.* JASA 105(1). — the phantom-partial family §4 excludes.
- Conklin, H. A. (1996). *Design and tone in the mechanoacoustic piano.* JASA —
  longitudinal modes.
- Weinreich, G. (1977). *Coupled piano strings.* JASA 62(6). — coupling and
  non-stationary beats, the physics behind the modulation reading.
