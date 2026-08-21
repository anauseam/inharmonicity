# ADR 0014 — The Unison Panel Against Isolation Truth

## Status

**MEASURED 2026-08-20 (Prompt AD).** The criteria below were **pre-registered
before any measurement ran** — the rule is
[`../internals/07-evidence-and-methodology.md`](../internals/07-evidence-and-methodology.md),
and ADR 0010 and ADR 0011 already worked this way. Everything outside C1–C3 is
reported as exploratory and did not move the outcome.

**Outcome: WAIT.** C2 passes, C3 is partial, and **C1 is inconclusive because
its pre-registered population is one key** — this piano's as-found unisons are
mostly tighter than the panel can resolve, so the set cannot score the criterion
that decides accuracy. The panel ships unchanged; the fate decision goes to the
post-tuning detuning ladder (Prompt W part 3), which is the only instrument that
can place splits at chosen sizes — but piano #2 is not available for that,
so §9 routes it to an as-found isolation pass on piano #1 instead.

No behaviour changes. `examples/isolation` and `examples/common/` are new;
nothing in `tuner-core/src` or `tuner-gui/src` was touched.

**Amended 2026-08-21** with 30 further captures (six treble notes, open, 5 s).
They change §8's conclusion and are the first direct measurement of one of the
two gates `suspected-issues.md` carries as un-measured — see §8a/§8b.

## Context

Unison assist (ADR 0012) shipped with an acknowledged hole: **there was no
ground truth for any split.** Every capture of a multi-strung note is a blend,
and a reference DFT of the same 1.5 s hits the same `2/T` wall the estimator
does, so "clean" and "out of resolution" were indistinguishable. ADR 0013 closed
the bass-attribution question as far as single-struck captures allow and named
the mute test as the decisive next experiment.

That session happened (2026-08-15/16, piano #2, **as found, before tuning**):
555 captures, eight complete isolation sets, every capture carrying an operator
declaration of which strings sounded. This ADR is its analysis.

**What isolation buys that nothing else does:** a true split as the difference
of two independently measured f₀, which is not resolution-bound; per-string `B`,
the discriminator's own untested premise; and a **false-beat positive control**,
since a solo capture that still resolves two lines is a false beat by
construction. ADR 0013 D3 noted the project had none of these. It now has **193**
— 78 single-strung keys that are solos by construction, plus 115 muted solos.

## 1. The screen: `06`'s partial-count tell over-rejects, and B is sharper

A muted bass string can be quiet enough that MAT locks onto something else.
`06` documents the tell as a partial count far below the key's open captures.
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
tuning target. C6 is not: `06` records the estimator as bistable there, and the
repeat scatter confirms it.

`06` claims 0.04–0.16 ¢ "from the bass through the upper mid". The honest range
is **0.005–0.45 ¢**; A#3 sits at 0.33–0.45, above the published band.

**As-found splits** ran 0.09–18.5 ¢, and the register pattern is the opposite of
comfortable: the bass and tenor unisons are *well set* (0.88–3.67 ¢) while A4
upward are not (9.2–18.5 ¢).

## 3. The operating regime, in beats — and a prediction that failed

ADR 0012 §5 forbids a key threshold, so the regime must fall out of a measured
quantity. **In beats it is one number for the whole compass**: `2/T` = 1.538 Hz
at the ring cap, the slowest beat a 1.30 s record can show. The register
dependence enters only through *which partial is watched*, since a pair beating
at `r` at the fundamental beats at `n·r` at partial `n`.

`06`'s cents figures were computed at the **fundamental** and are therefore
pessimistic by exactly `n*`:

| key | n\* | floor at n\* | floor at f₁ (as `06` states it) |
| --- | --- | --- | --- |
| C2 | 6 | **6.74 ¢** | 40.20 ¢ |
| F2 | 6 | 5.06 ¢ | 30.25 ¢ |
| C3 | 4 | 5.08 ¢ | 20.27 ¢ |
| A#3 | 4 | 2.85 ¢ | 11.38 ¢ |
| A4 / C5 / C6 | 1 | 6.07 / 5.11 / 2.58 ¢ | identical |

**The prediction this ADR set out to test was that correcting to the displayed
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

**A single string resolves two lines on 65 % of captures.** That is ADR 0013's
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
lines resolved the span is between whichever two it separated. ADR 0012 §4's own
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
**the one qualifying key is the one `06` documents as bistable**, and §2 measures
its repeat scatter at 1.7–4.3 ¢ — the truth itself is shaky there. At ADR 0012
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
string, which is ADR 0013's bass extra line caught for the first time on a note
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
measured quantity rather than a rounded one (`07` §2).

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

**This vindicates ADR 0012 §6's fix with ground truth.** The design note's χ²
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
strings that genuinely differ by up to 8 %, not a property they share. ADR 0009's
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

As first recorded, this set could **not** separate ADR 0012 §7's attribution of
the high-treble failure to the D3 gate from simple truncation: all 48 of its
high-treble captures were too short for the ring to fill regardless of gating.
Six long captures were taken to break that confound, and §8a does — the answer
is *both*, split at about C7.

### What the treble actually needs, which is less than the cap

The floor is a **statistical** requirement, not a resolution one: `UNISON_MIN_BINS`
= 25 hops comes from Rohling §V solved for record length (ADR 0012 §3). The
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

**It is the D3 gate, and the margin is not subtle.** The gate is
`amplitude < noise_floor · K` where `noise_floor` is the *ambient-silence* RMS
(`pipeline.rs:927`), giving a threshold of 6.55e−4 on these captures. Measured
against the noise actually present at three off-partial frequencies in the same
window:

| note | real local noise | the gate sits | partial's SNR when the gate closes |
| --- | --- | --- | --- |
| E6 | 4.0–5.2e−5 | 6× above it | **7×** |
| A6 | 2.3–4.8e−5 | 11× above it | **26×** |
| C#7 | 2.0–3.0e−5 | 18× above it | **24×** |
| F7 | 1.6–2.4e−5 | 29× above it | **26×** |
| A7 | 1.3–1.7e−5 | 44× above it | **32×** |
| C8 | 1.4–2.1e−5 | 40× above it | **18×** |

At the hop the gate declares the partial "below the noise floor", the partial is
**7–32× above the noise measurably present in that same window**. Three
independent off-partial probes agree to within 2×, and all sit clear of the
1024-point Hann main lobe (±86 Hz), so leakage would only *inflate* the noise
estimate and make this conservative.

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

### 8b. This measures a gate `suspected-issues.md` calls unmeasured — and the sign is the opposite

The Neyman–Pearson entry names two shipped gates and says their exposure is
"confirmed by analogy only". This is a direct measurement of one of them, the
strobe's D3 gate, and it refines the entry rather than confirming it:

- **The entry predicts under-rejection**: during a sustain, leakage from a note's
  other partials raises the real noise *above* ambient, so an ambient-σ threshold
  is too low and dead partials pass. ADR 0011's control demonstrated exactly that
  in the deep bass — 100 % of ±400 ¢ garbage admitted.
- **The treble does the reverse.** At `n* = 1` a treble note is nearly a pure
  tone: few partials, little leakage, so the real per-bin noise is far *below*
  ambient and the same threshold is 6–44× too **high**. Live partials are dropped.

One misspecification, opposite signs, split by register — because the quantity
being compared is a broadband time-domain RMS against a single-bin amplitude, and
which way that lands depends on how much of the note's energy sits near the bin.
(That dimensional reading is inference; the ratios above are measurement.)

Both directions are fixed by the same change, and ADR 0011 already shipped it for
the coarse readout: an ordered-statistic CFAR gate against *local* reference
cells admitted 0 % of the deep-bass junk **and** lifted C8 availability from 42 %
to 100 %. That second half is this same effect, in a different consumer.

**So Prompt O's value is raised by this set after all — but not for the reason
ADR 0012 §7 gave.** §7 guessed the D3 gate blocked the high treble; that guess is
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

### D3 — `06` is corrected, not merely annotated

The isolation-set count (eight, not seven), the f₀ repeatability range
(0.005–0.45 ¢), and the per-fundamental resolution figures are wrong as
published. The screen description gains the `B`-agreement rule.

## Consequences

- Two new harnesses: `examples/isolation` (the shipped panel over the set) and
  `examples/common/` (the shared loader Prompt AF sweeps the rest onto). The
  truth-side statistics are `scripts/isolation_truth.py`, the same split as
  `scripts/audit_captures.py`.
- `common/` carries a second copy of `strobe_replay`'s `run_unison`. They must
  stay behaviourally identical until Prompt AF retires one under its byte-diff
  protocol.
- ADR 0012 §§5–6 gain pointers here: §5's bass lines now have a ground-truth
  positive control, and §6's residual-based SE is vindicated by measurement.
- ADR 0013 D3's mute test is **discharged**: the single-strung 65 % figure is
  the control it asked for.
- **Prompt O gains a measured motivation** (§8a/§8b): the strobe's D3 gate is
  6–44× above the real local noise in the treble and drops partials at 7–32×
  SNR, which is the *opposite sign* to the under-rejection
  `suspected-issues.md` predicts and is fixed by the same CFAR port. Superseded
  note follows.
- ~~**One motivation for Prompt O weakens; its main case is untouched.**~~ ADR 0012
  §7 attributed the high-treble failure to the D3 gate, and §8 here shows the
  blocker in this set is record length, with the two confounded. That was only
  ever the *unison-availability* argument for O. The σ misspecification's larger
  exposure is the **engine's** per-partial tracking gate (`engine.rs`), which
  decides which partials the phase vocoder keeps and so feeds `measured_f0`, the
  MAT seed and the M-of-N lock — nothing here bears on that, and it remains the
  reason `suspected-issues.md` carries the entry.

## 9. What the next session must record

**Neither piano has been tuned**, so the as-found state still exists and is
destroyed by the next tuning.

**Piano #2 will not be deliberately detuned** (decided 2026-08-20): it is an
instrument in use whose only goal is to be properly tuned. That constraint is
what shapes the list below — and it is why item 1 changed.

### 1. An as-found isolation pass on **piano #1**, no detuning required

C1 needs unisons wider than the panel's floor. Piano #2 could not supply them
because its bass and tenor were already well set. **Piano #1 may supply them for
free**: it is the guinea-pig instrument, it has never been isolation-recorded,
and it sits further from equal temperament than piano #2 (median +4.4 ¢, p10–p90
−7.1…+17.6, against −0.8 ¢ and −12.0…+3.5 — `06`). That is not a unison-spread
measurement, but it is the only prior available and it points the right way.

The protocol is exactly what was done on piano #2 — solos then open, ~4 strikes
each — on eight or so keys spread across the compass. **If piano #1's unisons are
wider, C1 becomes scorable with no detuning anywhere.** If they are also tight,
that is itself the answer: the panel's floor sits above where real unisons live,
and the feature's case is much weaker.

A deliberate detuning ladder stays available on piano #1 in the far future, and
only there. It is no longer the primary route.

### 2. Long **open** captures above C6 — the cheapest item on this list

Six notes, open only, 5 s each (Settings → Advanced → Capture Duration):
**E6, A6, C#7, F7, A7, C8.** No mutes, no solos.

The question is only whether the note sustains 0.58 s, which an open capture
answers as well as a solo does — so this is six strikes, not six isolation sets.
Every capture we have above C6 is 0.37–0.56 s, and the panel needs 0.58 s;
whether that is the note or our decay stop is currently untestable and these six
captures settle it.

### 3. Repeat C6, and add a neighbour

C6's strings scatter **16.5 ¢, 6.3 ¢ and 4.2 ¢** across repeat strikes, against
0.11–0.45 ¢ at C5 — a scatter nearly as wide as the unison being measured. Take
~8 strikes per string there, and record **A5 or C#6** as an isolation set too, so
the register has one key that is not the unreliable one. C6 was the *only* key
that qualified for C1 this round, which is precisely why C1 returned nothing.

### 4. If a bass unison is ever found genuinely out, record it

Not a request to create one. Every bass key sampled was within 4 ¢, so there is
no evidence at all about the panel's bass behaviour when a unison is really
spread — which is the register where watching partial 6 should help most. If one
turns up on piano #1, or on any instrument, that key is worth an isolation set.

## Limitations / threats to validity

- **One instrument, as found.** Validation-only per `06`; this cannot select a
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
- **C6's scatter is not "bistable" as `06` describes it.** Measured across
  repeat strikes of a single solo string, its values spread continuously over
  16.5 ¢ rather than falling into two clusters. The description in `06` came from
  open captures; the solo behaviour is broad scatter.
- **The two MAT failures** (A7, B7 open captures at 0.28 and 0.32 s) are
  excluded, not explained.

## Artifacts & reproduction

```bash
cargo run --release --example regenerate_partials -- <dump_dir> > iso.json
cargo run --release --example isolation -- iso.json <dump_dir> --json panel.json
python3 scripts/isolation_truth.py iso.json panel.json
```

The set is not in the repository: it lives in the app's per-user dump directory
under the instrument's `identity.id` ([`../internals/06-capture-sets.md`](../internals/06-capture-sets.md)).

## References

- Weinreich, G. (1977). *Coupled piano strings.* JASA 62(6). — the coupling §6
  bounds, and why a unison's beat is not stationary.
- [ADR 0012](0012-unison-line-estimator.md) — the estimator, its resolution law
  (§4) and the discriminator (§6) this ADR tests against truth.
- [ADR 0013](0013-bass-extra-lines-attribution.md) — the bass extra lines, whose
  positive control (D3) §4 discharges.
- [ADR 0009](0009-repeat-capture-noise-decomposition.md) — σ_lnB, the repeat
  noise §7's spread is measured against.
