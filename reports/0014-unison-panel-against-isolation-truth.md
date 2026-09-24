# report 0014 — the unison panel against isolation truth

The pre-registered criteria C1–C3 were fixed before any measurement ran, and the
outcome is the one a pre-registration is *for*: **C1 could not be scored**,
because its population turned out to be one key — this piano's as-found unisons
are mostly tighter than the panel can resolve. A criterion that cannot be
evaluated is reported as inconclusive rather than quietly replaced with one the
data can answer.

Everything outside C1–C3 is exploratory and did not move the outcome.

Two claims here have been superseded by [report 0015](retiring/0015-ambient-sigma-gates-measured.md)
and are struck through where they appear (staleness pass S1): §8a's in-note noise
figure, and §8b's framing of the mechanism as temporal.

## Where this stands

**MEASURED 2026-08-20.** The criteria below were **pre-registered
before any measurement ran** — the rule is
[the methods standard](README.md),
and report 0010 and report 0011 already worked this way. Everything outside C1–C3 is
reported as exploratory and did not move the outcome.

**Outcome: WAIT.** C2 passes, C3 is partial, and **C1 is inconclusive because
its pre-registered population is one key** — this piano's as-found unisons are
mostly tighter than the panel can resolve, so the set cannot score the criterion
that decides accuracy. The panel ships unchanged; the fate decision goes to the
post-tuning detuning ladder, which is the only instrument that
can place splits at chosen sizes — but piano #2 is not available for that,
so §9 routes it to an as-found isolation pass on piano #1 instead.

No behaviour changes. `examples/isolation` and `examples/common/` were new;
nothing in `tuner-core/src` or `tuner-gui/src` was touched.

**Amended 2026-08-21** with 30 further captures (six treble notes, open, 5 s).
They change §8's conclusion and are the first direct measurement of one of the
two gates then carried as un-measured in `suspected-issues.md` (since retired;
now [report 0015](retiring/0015-ambient-sigma-gates-measured.md)) — see §8a/§8b.

**Amended 2026-08-22 by [report 0015](retiring/0015-ambient-sigma-gates-measured.md).** §8a's
*direction* — the gate is what closes the top octave — is confirmed and
strengthened. Its **in-note noise trajectory is superseded**: "real noise falls
13–56× across the note" is measured at **2.2–2.9×** on these same six keys, and
§8b's framing of the mechanism as primarily temporal is superseded by a
primarily **spectral** one. See §8c.

## Context

Unison assist (report 0012) shipped with an acknowledged hole: **there was no
ground truth for any split.** Every capture of a multi-strung note is a blend,
and a reference DFT of the same 1.5 s hits the same `2/T` wall the estimator
does, so "clean" and "out of resolution" were indistinguishable. report 0013 closed
the bass-attribution question as far as single-struck captures allow and named
the mute test as the decisive next experiment.

That session happened (2026-08-15/16, piano #2, **as found, before tuning**):
555 captures, eight complete isolation sets, every capture carrying an operator
declaration of which strings sounded. This report is its analysis.

**What isolation buys that nothing else does:** a true split as the difference
of two independently measured f₀, which is not resolution-bound; per-string `B`,
the discriminator's own untested premise; and a **false-beat positive control**,
since a solo capture that still resolves two lines is a false beat by
construction. report 0013 D3 noted the project had none of these. It now has **193**
— 78 single-strung keys that are solos by construction, plus 115 muted solos.

## 1. The screen: `capture-sets.md`'s partial-count tell over-rejects, and B is sharper

A muted bass string can be quiet enough that MAT locks onto something else.
`capture-sets.md` documents the tell as a partial count far below the key's open captures.
Measured, that rule is both too weak and too strong:

| capture | partials | f₀ vs siblings | `B` | vs key's solo median | vs Rigaud prior |
| --- | --- | --- | --- | --- | --- |
| `key_015_C2_…4963` | 20 | −85 ¢ | 6.9e−3 | **30×** | **101×** |
| `key_015_C2_…4969` | 17 | −166 ¢ | 2.1e−2 | **90×** | **307×** |
| good C2 solos (10) | 32 | — | 2.3e−4 | 1.00 ± 0.004 | 3.3–3.4× |
| `key_029_D3_…2679` | **24** | +0.1 ¢ | 1.6e−4 | 1.01 | 1.20 |

The D3 capture has a low count and is **good** — its `f₀` and `B` agree with its
siblings. A count threshold rejects it. The failures instead announce themselves
in `B`, disagreeing with their own siblings by 30–90× where good captures agree
to a fraction of a percent.

**The screen used throughout is within-key agreement of `B`**, cut at 3×. The
cut sits inside a chasm — survivors within 0.4 %, failures beyond 30× — so its
placement is not load-bearing, which is the point of stating the margin.

Its effect is not cosmetic: C2 string 1's f₀ repeatability reads **56.6 ¢**
unscreened and **0.078 ¢** screened.

## 2. Per-string truth

**f₀ repeatability**, per string, across repeat strikes:

| key | strings | repeatability (¢) | partials |
| --- | --- | --- | --- |
| C2, F2, C3, D3 | 2 each | 0.005–0.258 | 30–32 |
| A#3 | 3 | 0.332–0.448 | 23 |
| A4, C5 | 3 each | 0.044–0.153 | 10–13 |
| C6 | 3 | **1.679–4.251** | 11 |

Median 0.114 ¢, and **below C5 the worst string repeats to 0.45 ¢** — 2–20×
finer than a sub-1-¢ tolerance, so a solo capture's f₀ is a usable per-string
tuning target. C6 is not: `capture-sets.md` records the estimator as bistable there, and the
repeat scatter confirms it.

`capture-sets.md` claims 0.04–0.16 ¢ "from the bass through the upper mid". The honest range
is **0.005–0.45 ¢**; A#3 sits at 0.33–0.45, above the published band.

**As-found splits** ran 0.09–18.5 ¢, and the register pattern is the opposite of
comfortable: the bass and tenor unisons are *well set* (0.88–3.67 ¢) while A4
upward are not (9.2–18.5 ¢).

## 3. The operating regime, in beats — and a prediction that failed

report 0012 §5 forbids a key threshold, so the regime must fall out of a measured
quantity. **In beats it is one number for the whole compass**: `2/T` = 1.538 Hz
at the ring cap, the slowest beat a 1.30 s record can show. The register
dependence enters only through *which partial is watched*, since a pair beating
at `r` at the fundamental beats at `n·r` at partial `n`.

`capture-sets.md`'s cents figures were computed at the **fundamental** and are therefore
pessimistic by exactly `n*`:

| key | n\* | floor at n\* | floor at f₁ (as `capture-sets.md` states it) |
| --- | --- | --- | --- |
| C2 | 6 | **6.74 ¢** | 40.20 ¢ |
| F2 | 6 | 5.06 ¢ | 30.25 ¢ |
| C3 | 4 | 5.08 ¢ | 20.27 ¢ |
| A#3 | 4 | 2.85 ¢ | 11.38 ¢ |
| A4 / C5 / C6 | 1 | 6.07 / 5.11 / 2.58 ¢ | identical |

**The prediction this report set out to test was that correcting to the displayed
partial would move the "three of eight keys resolvable" conclusion. It does
not.** Three of eight resolve either way, and the *same* three. The mechanism is
real and its size is exactly as predicted — but the keys that gain the `n*`
multiplier (bass, tenor) have splits far too small to benefit, and the keys with
splits wide enough to resolve are all `n* = 1`, where there is no multiplier.
Recorded because a prediction that fails is worth as much as one that holds.

Across the whole compass with the default table, the floor runs **0.64 ¢ (C8) to
16.06 ¢ (A0)** and is **not monotonic**: it improves with pitch inside each
`n*` band and jumps coarser at every break — B2 3.59 ¢ → C3 5.08 ¢, B3 2.69 ¢ →
C4 5.08 ¢, G#4 3.20 ¢ → A4 6.04 ¢. Three sawtooth discontinuities where the
panel abruptly halves in precision as the tuner moves *up* the keyboard. The
display table is chosen for the strobe band (`curves::default_display_partials`),
not for this consumer.

**The scoping comparator already exists and is already wired.** It is
`UNISON_SPAN_LADDER[0]` = 3.0 ¢, "a unison being finished" (`app.rs:392`), and
`main_view.rs:583` already turns the resolution figure amber past it. With the
default table that flag is lit on **59 of 88 keys** — the panel declares itself
insufficiently precise across two-thirds of the compass, using a rule that
requires no new constant and no key index. Only A#3–B3 and A#5–C8 clear it.

## 4. C2 — the false-beat positive control: **PASS**

193 solo captures, 192 of which published a record.

| population | captures | ≥2 lines | ≥3 | `Unison` | `FalseBeat` |
| --- | --- | --- | --- | --- | --- |
| single-strung (no mute needed) | 78 | **65 %** | 32 % | 5.1 % | 44 % |
| muted solo | 115 | 37 % | 11 % | 2.6 % | 18 % |
| all solos | 193 | 48 % | 19 % | — | 28 % |

**A single string resolves two lines on 65 % of captures.** That is report 0013's
phenomenon confirmed against ground truth for the first time: the second line is
real, it is not a second string, and the project can now say so from a capture
where only one string could possibly sound.

The criterion is the *assertion*, not the lines, and what reaches the user
depends on the layout — `main_view.rs` prints the verdict only when the
displayed rows carry ≥2 lines, and "the displayed rows" is the `n*` row alone in
`Displayed` mode but every resolved partial in `AllPartials`:

| layout | solos called `Unison` | rate | bar 5 % |
| --- | --- | --- | --- |
| `Displayed` | 4 / 192 | **2.1 %** | PASS |
| `AllPartials` | 7 / 192 | **3.6 %** | PASS |

Both pass. **The two shipped layouts disagree about whether a unison is
asserted**, on 3 of 192 captures — which is evidence for the open "which layout
to keep" question in `TODO.md` that nothing else has supplied.

## 5. C1 — accuracy against truth: **INCONCLUSIVE**

Comparing the panel's reported *span* to the widest true pair is wrong, and
measuring it that way first is how the error was found: at C5 the panel reports
≈2.4 Hz where the widest pair is 3.27 Hz, because with three strings and two
lines resolved the span is between whichever two it separated. report 0012 §4's own
accuracy figure is a **position** error for the same reason. Each reported line
is therefore matched to its nearest solo partial.

| key | truth / `2/T` | captures | matched lines | median \|err\| Hz |
| --- | --- | --- | --- | --- |
| A4 | 1.75 | 8 | 16 | **0.208** |
| C5 | 1.64 | 8 | 16 | **0.265** |
| C6 | 7.17 | 12 | 32 | 0.611 |
| C3 | 0.74 | 1 | 2 | 0.400 |
| A#3 | 0.34 | 6 | 12 | 1.200 |
| C2 / F2 | 0.17 / 0.28 | 1 each | 2 each | 1.81 / 2.25 |
| D3 | 0.66 | 11 | 22 | **8.805** |

**The pre-registered population — truth above 2 × `2/T` — contains one key.**
C6 qualifies; A4 (1.75) and C5 (1.64) fall just under. Scored as written, C1
**fails**: median \|err\| 0.611 Hz against a derived tolerance of **0.372 Hz**
(§4's 0.085 Hz systematic + 3σ, plus §6's measured 0.26 Hz coupling bound), with
22 % of lines inside it.

That result should not be leaned on, for a reason that is not the panel's fault:
**the one qualifying key is the one `capture-sets.md` documents as bistable**, and §2 measures
its repeat scatter at 1.7–4.3 ¢ — the truth itself is shaky there. At report 0012
§4's own "exact above 1.6 ×" boundary the population becomes A4, C5 and C6, and
the first two pass comfortably at 0.208 and 0.265 Hz.

**Both are reported and the pre-registered one stands as the formal answer.**
Moving the bound from 2.0 to 1.6 after seeing which keys landed where is exactly
the goalpost-move pre-registration exists to prevent. The honest statement is
that **the criterion is underpowered on this instrument**, because its
population is "unisons wider than the panel's floor" and this piano's are
mostly tighter — five of eight keys sit below it. That is a defect in the
pre-registration meeting reality, not a measurement of the panel, and it is why
the outcome is Wait rather than Retire.

### What the panel does below its floor — the sharpest thing this set shows

| key | truth / `2/T` | opens | of those, ≥2 lines | nearest string to a reported line |
| --- | --- | --- | --- | --- |
| C2 | 0.17 | 8 | **1** | 1.81 Hz |
| F2 | 0.28 | 8 | **1** | 2.03 Hz |
| C3 | 0.74 | 8 | **1** | 0.40 Hz |
| A#3 | 0.34 | 13 | 6 | 1.20 Hz |
| D3 | 0.66 | 11 | **11** | **17.49 Hz** |
| A4 / C5 / C6 | 1.64–7.17 | 8 / 8 / 12 | 8 / 8 / 12 | 0.21 / 0.26 / 0.59 Hz |

Where it resolves, it finds the **actual strings**, to 0.21–0.59 Hz. Where it
cannot, it mostly shows **one line** — it does not invent a split. D3 is the
exception and it is not a near-miss: its extra line sits 17.5 Hz from either
string, which is report 0013's bass extra line caught for the first time on a note
whose string positions are independently known.

## 6. Q3 — the Weinreich coupling shift: below measurement error

Not open-f₀ minus solo-mean, which is confounded — a blend's fitted f₀ tracks
whichever strings dominate. The weight-free test: for each partial, the solos
define a *span*, and a plain superposition of the same strings must fall inside
it. Where it falls inside is set by relative strike strength, which is
unrecoverable; falling **outside** requires something to have moved.

| key | median span | open partials inside the span |
| --- | --- | --- |
| A4 | 15.8 Hz | **96 %** |
| C5 | 15.9 Hz | **98 %** |
| C6 | 37.5 Hz | 77 % |
| C3 / D3 / F2 | 5.1–6.5 Hz | 70–76 % |
| A#3 | 1.6 Hz | 44 % |
| C2 | 0.9 Hz | 47 % |

Pooled: 67 % of 1 681 open partials inside; **median excursion when outside
0.258 Hz**, p90 3.44 Hz.

**The "inside" fraction tracks span width, which is the signature of measurement
error rather than coupling** — a narrow span is crossed by noise alone, and
where the span is wide enough to test cleanly (A4, C5) 96–98 % fall inside. So
**no coupling shift is detectable above measurement error**. The pooled median
excursion, 0.26 Hz, is the bound, and it is what enters C1's tolerance — a
measured quantity rather than a rounded one (the anchor-a-threshold rule in
[`CONTRIBUTING.md`](../CONTRIBUTING.md)).

## 7. Q2 — the strings of one note differ in stiffness, and only partly because they are out of tune

The test fits `ln Δ = ln a + p·ln f` and asks whether `p = 1`, which is exact
only if a note's strings share `B`. They do not. Scored as a separation in
combined standard errors of each string's mean (repeat strikes, after the §1
screen):

| key | string `B` values | separation | Δ`B` | verdict |
| --- | --- | --- | --- | --- |
| C2 | 2.320 / 2.238 e−4 | **27.4 σ** | 3.6 % | real |
| A4 | 7.731 / 7.650 / 7.602 e−4 | **16.4 σ** | 1.7 % | real |
| F2 | 1.681 / 1.827 e−4 | **16.0 σ** | 8.3 % | real |
| D3 | 1.595 / 1.713 e−4 | **11.8 σ** | 7.1 % | real |
| C5 | 9.729 / 9.635 / 9.491 e−4 | **11.0 σ** | 2.5 % | real |
| C3 | 1.495 / 1.566 e−4 | **5.7 σ** | 4.7 % | real |
| A#3 | 2.929 / 2.940 / 2.935 e−4 | **5.0 σ** | 0.4 % | real |
| C6 | 3.001 / 2.692 / 2.858 e−3 | 1.1 σ | 10.8 % | **not separable** |

**Real on seven of eight keys.** (An earlier cruder test — comparing spread
ranges rather than standard errors — put it at five; the σ form is the correct
one and is what the reproduction script computes.) C6 fails for the reason §9
gives: its own repeat scatter swamps everything.

### Two controls, because this claim carries weight elsewhere

**It is not unequal partial counts.** A muted string is quieter, so MAT might fit
`B` over a shorter partial range and produce a spurious difference. Refuted three
ways: C2 and F2 show 27 σ and 16 σ separations with **identical** partial counts
(32 vs 32); the correlation between a pair's partial-count difference and its `B`
difference is **−0.25**, near zero and the wrong sign; and A#3, whose three
strings all read 23–24 partials, still separates at 5 σ.

**It is not mute placement.** C2's string 1 was recorded in two sessions ~45
minutes apart, so the mute was set twice. Between sessions it reproduces to
**0.13 %**; between strings it differs by **3.63 %** — 28× larger. The difference
is a property of the string, not of how the felt happened to sit.

### How much of it is just being out of tune

`B ∝ 1/T` — inharmonicity falls as tension rises — and two strings at different
pitches are at different tensions by construction. Since `f ∝ √T`, a split of `Δ`
cents forces `ΔB/B = 2Δ/1731`. That is a floor under the effect, not an
explanation of it, and the two registers answer differently:

| key | split | Δ`B` predicted by tension | Δ`B` measured | measured / predicted |
| --- | --- | --- | --- | --- |
| A4 | 10.70 ¢ | 1.24 % | 1.70 % | **1.4 ×** |
| C5 | 10.87 ¢ | 1.26 % | 2.12 % | **1.7 ×** |
| C6 | 18.52 ¢ | 2.14 % | 5.93 % | 2.8 × |
| C3 | 3.67 ¢ | 0.42 % | 5.02 % | 11.8 × |
| C2 | 1.45 ¢ | 0.17 % | 3.68 % | 21.9 × |
| D3 | 2.78 ¢ | 0.32 % | 8.03 % | 25.0 × |
| F2 | 0.96 ¢ | 0.11 % | 8.50 % | **76.4 ×** |

**In the plain-wire treble, tension is nearly the whole story** (1.4–1.7 ×): those
strings are effectively identical and differ in `B` because they differ in pitch.
Tune the unison and the `B` difference very largely goes with it.

**In the wound bass it explains almost nothing** (12–76 ×). Those strings differ
in `B` as manufactured, which is physically unsurprising — `B` goes as the fourth
power of the core diameter, so a 1 % core difference is a 4 % `B` difference, and
bass strings add a copper winding that contributes mass without stiffness. A
bass unison's `B` spread is a property of the wire, and tuning will not remove it.

### What it costs the discriminator

A `B` difference puts a systematic `866·(B₁−B₂)·n²` term in the split, so the
split in cents is not constant across partials. Measured from partial 1 to
partial 6 it moves by **3–48 %** of itself (F2 48 %, C6 25 %, C2 18 %, D3 15 %,
A4/C5 4–6 %).

**This vindicates report 0012 §6's fix with ground truth.** The design note's χ²
tested against the estimator's own σ (≈0.07 Hz per split) and called 87 % of
tenor unisons false beats; the shipped test estimates the standard error **from
the residuals**, which is exactly what absorbs a tilt of this size. The mechanism
§6 inferred from a failure is now measured directly.

It also names one contributor to §6's near-silence: the tilt is systematic rather
than random, so a residual-based SE absorbs it by *inflating*, which widens
`Undetermined`. Lever arm remains the dominant cause.

### What this does and does not disturb elsewhere

`B` is measured and stored **per key**, never per string, and every consumer —
the tuning curve, the strobe references, discovery — reads a key's `B` from an
*open* capture, which is the blend the tuner actually plays. So nothing shipped
assumes the strings share a `B`; the assumption lives in exactly one place, the
unison discriminator, and that is the place this section measures. What changes
is the *interpretation* of a key's stored `B` in the bass: it is an average over
strings that genuinely differ by up to 8 %, not a property they share. report 0009's
σ_lnB — repeat noise of **0.4 %** in the bass and mid — is measured from repeat
strikes of the same open note and is unaffected, but it is now clear that it
measures reproducibility, not how well one number describes three strings.

## 8. Availability: record length below C7, the D3 gate above it

| register | record length | captures | published |
| --- | --- | --- | --- |
| treble | ≥ 1.5 s | 31 | **100 %** |
| treble | 0.6–1.45 s | 62 | 44 % |
| treble | < 0.6 s | 42 | **0 %** |
| high 76–87 | < 0.6 s (all of them) | 48 | **0 %** |
| bass / tenor | any | 363 | 98–100 % |

Treble notes decay fast and the shipped decay stop cut 104 of them below the
`UNISON_MIN_BINS` floor, at 0.42–0.56 s. Where the treble was given a full
record it published **every time**.

As first recorded, this set could **not** separate report 0012 §7's attribution of
the high-treble failure to the D3 gate from simple truncation: all 48 of its
high-treble captures were too short for the ring to fill regardless of gating.
Six long captures were taken to break that confound, and §8a does — the answer
is *both*, split at about C7.

### What the treble actually needs, which is less than the cap

The floor is a **statistical** requirement, not a resolution one: `UNISON_MIN_BINS`
= 25 hops comes from Rohling §V solved for record length (report 0012 §3). The
*resolution* requirement runs the other way, because a treble split is wide in Hz
— C6's needs 8 hops, C7's 4, C8's 2. Above ~C5 the panel is held at 0.58 s by its
own admission test while the split it looks for needs a fifth of that.

### 8a. Six long captures, and the answer is the gate — **measured 2026-08-21**

Six notes were re-recorded open at 5 s (**E6, A6, C#7, F7, A7, C8**, 2–8 strikes
each, 30 captures) specifically to separate "the note ended" from "our capture
ended". With the decay stop bypassed (`pipeline.rs:1117` — it only applies at or
below the 1.5 s default) every file is exactly 5.00 s.

| note | captures | published | record achieved (hops) |
| --- | --- | --- | --- |
| E6 | 10 | **10/10** | 43–56 |
| A6 | 8 | **8/8** | 31–51 |
| C#7 | 3 | 2/3 | 0, 51, 56 |
| F7 | 3 | 2/3 | 0, 26, 28 |
| A7 | 3 | **0/3** | 0, 0, 0 |
| C8 | 3 | **0/3** | 0, 0, 0 |

**E6–C#7 was our capture stopping early** — those notes published nothing before
and publish every time now. **A7 and C8 fail with five seconds in hand**, so for
them the question becomes why.

**It is the D3 gate — and the defect is that the gate is static while the noise
is not.** The gate is `amplitude < noise_floor · K`, where `noise_floor` is the
calibrated ambient-silence RMS (`gatekeeper.rs`; measured at startup over 2 s of
room, max RMS × 1.5). That threshold is **6.55e−4** on these captures and it does
not move for the life of the note. The noise actually present at the partial does
— measured at three off-partial frequencies **in the same window, hop by hop**:

| note | gate ÷ real noise at the peak | at the hop it closes | at the note's end | partial's SNR when it closes |
| --- | --- | --- | --- | --- |
| E6 | **0.8×** | 11.9× | 10.6× | 5.1× |
| A6 | **1.7×** | 13.6× | 79.0× | 13.4× |
| C#7 | **0.9×** | 19.0× | 48.4× | 14.8× |
| F7 | **0.8×** | 8.0× | 25.9× | 6.6× |
| A7 | **2.1×** | 13.1× | 43.9× | 10.0× |
| C8 | **1.4×** | 7.4× | 32.2× | 3.0× |

**At the strike the calibrated threshold is right** — within a factor of two of
the true local noise on all six notes, which is the calibration doing its job.
Then the note's own leakage subsides: the real noise falls **13–56×** over the
note while the signal falls 400–7000×, and the fixed threshold is left stranded
7–19× too high by the time it closes. It shuts off a partial still sitting
**3–15×** above the noise beside it.

So this is not "ambient σ is the wrong number". The ambient σ is a *correct*
measurement of the room, and the room is briefly the right reference. The defect
is that **a static threshold is being asked to track a quantity that moves one to
two orders of magnitude inside a single note** — and, per report 0011's deep-bass
control, moves the other way in a register where 32 partials leak into each
other. That is why a startup measurement of any refinement, per-bin included,
cannot close it: nothing measured before the note begins can follow the note.

**Confirmed by sweeping the threshold** rather than by inference
(`--noise-floor`):

| `noise_floor` | E6 | A6 | C#7 | F7 | A7 | C8 |
| --- | --- | --- | --- | --- | --- | --- |
| 3e−3 (shipped) | 10/10 | 8/8 | 2/3 | 2/3 | **0/3** | **0/3** |
| 3e−4 | 10/10 | 8/8 | 3/3 | 3/3 | **2/3** | 0/3 |
| 1e−4 | 10/10 | 8/8 | 3/3 | 3/3 | 2/3 | **1/3** |

3e−4 is still ~20× above the measured local noise and it recovers C#7, F7 and
A7. **Only C8 is genuinely marginal**: even against real noise its partial holds
for ~19 hops against the 25-hop floor.

### 8b. This measures a gate the σ-misspecification entry called unmeasured — and the sign is the opposite

The Neyman–Pearson entry names two shipped gates and says their exposure is
"confirmed by analogy only". This is a direct measurement of one of them, the
strobe's D3 gate, and it refines the entry rather than confirming it:

- **The entry predicts under-rejection**: during a sustain, leakage from a note's
  other partials raises the real noise *above* ambient, so an ambient-σ threshold
  is too low and dead partials pass. report 0011's control demonstrated exactly that
  in the deep bass — 100 % of ±400 ¢ garbage admitted.
- **The treble does the reverse.** At `n* = 1` a treble note is nearly a pure
  tone: few partials, little leakage, so the real per-bin noise is far *below*
  ambient and the same threshold is 6–44× too **high**. Live partials are dropped.

One misspecification, opposite signs, split by register — and the unifying
statement is **static threshold, dynamic noise**. The in-note noise at a partial's
bin is dominated by leakage from the note's own other partials, so it rises with
the strike and falls with the decay. A leakage-rich bass note keeps it above
ambient for the whole note (under-rejection); a near-pure-tone treble note lets it
fall far below ambient within a second (over-rejection). Same fixed threshold,
both failures.

Corrected 2026-08-21: an earlier revision of this section quoted "6–44× above the
real noise" and "7–32× SNR at closure" from noise medianed over the whole record,
which is biased low by the quiet tail. The same-hop figures above supersede them;
the conclusion is unchanged and the mechanism is better stated.

### 8c. Superseded by report 0015: the noise trajectory, and what it means

**Added 2026-08-22.** [report 0015](retiring/0015-ambient-sigma-gates-measured.md) re-measured
this gate across 986 383 (capture, hop, partial) rows on three populations,
including **these same six keys and these same 30 captures**. What it confirms and
what it overturns are different halves of §8a/§8b:

**Confirmed, and strengthened.** The gate is what closes the top octave; the
threshold is far above the noise present there; the failure is bidirectional with
the deep bass. report 0015 puts the top-octave miss rate at 56–65 % of partials
standing ≥ 3× above their own local noise, on both piano #2 sets, and adds a third
site sharing the same σ (`engine.rs:252`, discovery's peak floor).

**Superseded: "the real noise falls 13–56× over the note".** Measured with a probe
that excludes both the target's and the neighbours' Hann main lobes, and resolved
at the gate's own 23 ms window, `σ_local` on these six keys falls **2.9× (treble)
and 2.2× (high octave)** from the first 50 ms to the tail — not 13–56×. Three
checks support the smaller figure:

- the same probe on **pre-onset silence** agrees with the note's own tail within
  0.86–1.35× on these keys, so the tail has reached the room floor and cannot fall
  further;
- two independent probe windows agree to 1–3 % once past the attack;
- on **AWGN of known σ** the probe recovers that σ to 4 %.

**The likely cause of the discrepancy, offered as a hypothesis.** §8a's three
off-partial probes are not in the repository — commit `f578aa7` added only
`--noise-floor` — so they cannot be diffed. A probe inside the target's own main
lobe (±86 Hz at the 1024 window) measures the *signal's* skirt, which falls with
the signal (400–7000×) and would yield a figure of exactly §8a's order. report 0015
demonstrates the mechanism in its **own** harness: its 1024-sample probe reads
**1.26× high** in the first bucket from live-partial sidelobes, and that bias is
removed from the figures above. This is evidence that the mechanism is real and
available; it is not proof that §8a's probe suffered from it.

**Superseded: "static threshold, dynamic noise" as the primary framing.** The
temporal term is the smaller one. The σ that would correctly specify the threshold
spans **21–56× across registers at one instant** against **1.4–4.7× within a
note** — because the broadband ambient RMS is loaded by low-frequency room rumble
and then applied flat. The sharper statement is **one flat threshold against a
spectrum that is not flat**, with a secondary decay term.

That distinction is not cosmetic: it is what decides whether a startup per-bin
spectral floor can help. §8b said nothing measured before the note begins can
follow it; on the revised figures such a floor addresses the dominant term and
leaves a residual concentrated in the first ~0.5 s. See report 0015 §8.

Both directions are fixed by the same change, and report 0011 already shipped it for
the coarse readout: an ordered-statistic CFAR gate against *local* reference
cells admitted 0 % of the deep-bass junk **and** lifted C8 availability from 42 %
to 100 %. That second half is this same effect, in a different consumer.

**So the ambient-σ work's value is raised by this set after all — but not for the reason
report 0012 §7 gave.** §7 guessed the D3 gate blocked the high treble; that guess is
now measured true, with a mechanism and a magnitude, on notes that decay too fast
for the ambient floor to be a sane reference.

## Decisions

### D1 — The panel ships unchanged, and the fate decision waits

C2 passes, C3 is partial, C1 is inconclusive for a reason the set cannot fix.
Per the pre-registered outcome table that is **Wait**. Nothing measured here
argues for removing the panel: where it resolves it finds the real strings to
0.21–0.59 Hz, and where it cannot it mostly says one line rather than inventing
a split.

### D2 — No new scoping constant; the existing comparator is the right shape

`UNISON_SPAN_LADDER[0]` against the row's own `resolution_cents` is already a
derived, key-index-free rule and it already flags 59 of 88 keys. Whether 3.0 ¢
is the right value is a question for the ladder, not for this set.

### D3 — `capture-sets.md` is corrected, not merely annotated

The isolation-set count (eight, not seven), the f₀ repeatability range
(0.005–0.45 ¢), and the per-fundamental resolution figures are wrong as
published. The screen description gains the `B`-agreement rule.

## Consequences

- Two new harnesses: `examples/isolation` (the shipped panel over the set) and
  `examples/common/` (the shared loader), now `cargo lab strobe isolation` and
  the lab's `capture`/`raw`/`regen` modules. The
  truth-side statistics are `tuner-lab/scripts/isolation_truth.py`, the same split as
  `tuner-lab/scripts/audit_captures.py`.
- `common/` carries a second copy of `strobe_replay`'s `run_unison`. They must
  stay behaviourally identical until one of them is retired one under its byte-diff
  protocol.
- report 0012 §§5–6 gain pointers here: §5's bass lines now have a ground-truth
  positive control, and §6's residual-based SE is vindicated by measurement.
- report 0013 D3's mute test is **discharged**: the single-strung 65 % figure is
  the control it asked for.
- **The ambient-σ work gains a measured motivation** (§8a/§8b): the strobe's D3 gate is
  6–44× above the real local noise in the treble and drops partials at 7–32×
  SNR, which is the *opposite sign* to the under-rejection
  the σ-misspecification entry predicts and is fixed by the same CFAR port. Superseded
  note follows.
- ~~**One motivation for the ambient-σ work weakens; its main case is untouched.**~~ report 0012
  §7 attributed the high-treble failure to the D3 gate, and §8 here shows the
  blocker in this set is record length, with the two confounded. That was only
  ever the *unison-availability* argument for O. The σ misspecification's larger
  exposure is the **engine's** per-partial tracking gate (`engine.rs`), which
  decides which partials the phase vocoder keeps and so feeds `measured_f0`, the
  MAT seed and the M-of-N lock — nothing here bears on that, and it remains the
  reason that entry existed; it is measured in [report 0015](retiring/0015-ambient-sigma-gates-measured.md) §6.

## 9. What the next session must record

The capture plan moved to [`TODO.md`](../TODO.md) item 0, so it sits with the
backlog rather than inside the evidence. Piano #2 is not to be deliberately
detuned; piano #1 is where a wider-unison pass would come from.

## Limitations / threats to validity

- **One instrument, as found.** Validation-only per `capture-sets.md`; this cannot select a
  configuration.
- **C1's population is one key**, and that key is bistable. The criterion is
  underpowered, which is the finding rather than a caveat on it.
- **C3 is confounded with the instrument's state.** The five unresolved keys
  have well-set unisons; the panel's *capability* is the floor in §3, and
  whether that is useful depends on how mistuned a piano is. On a piano with
  10 ¢ bass unisons the same panel would resolve them.
- **Eight keys chosen by hand**, not sampled — they cannot support a
  compass-wide claim about splits.
- **Coupling and estimator error are not separable** in C1's residual; §6 bounds
  their sum.
- **§6 measures same-note string coupling only** — Weinreich's mechanism, three
  strings on one bridge pin exchanging energy. It says nothing about the other
  coupling a tuner meets: tuning a note changes the load on the plate and
  soundboard and shifts notes already set. Nothing in these captures can measure
  that — they are single notes on an instrument nobody tuned — and it is the
  reason a tuning pass is not one-and-done.
- **C6's scatter is not "bistable" as `capture-sets.md` describes it.** Measured across
  repeat strikes of a single solo string, its values spread continuously over
  16.5 ¢ rather than falling into two clusters. The description in `capture-sets.md` came from
  open captures; the solo behaviour is broad scatter.
- **The two MAT failures** (A7, B7 open captures at 0.28 and 0.32 s) are
  excluded, not explained.

## Artifacts & reproduction

```bash
cargo lab mat regen <dump_dir> > iso.json
cargo lab strobe isolation iso.json <dump_dir> --json panel.json
python3 tuner-lab/scripts/isolation_truth.py iso.json panel.json
```

The set is not in the repository: it lives in the app's per-user dump directory
under the instrument's `identity.id` ([`../internals/capture-sets.md`](../docs/internals/capture-sets.md)).

## References

- Weinreich, G. (1977). *Coupled piano strings.* JASA 62(6). — the coupling §6
  bounds, and why a unison's beat is not stationary.
- [report 0012](0012-unison-line-estimator.md) — the estimator, its resolution law
  (§4) and the discriminator (§6) this report tests against truth.
- [report 0013](0013-bass-extra-lines-attribution.md) — the bass extra lines, whose
  positive control (D3) §4 discharges.
- [report 0009](0009-repeat-capture-noise-decomposition.md) — σ_lnB, the repeat
  noise §7's spread is measured against.
