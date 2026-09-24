# report 0012 — the unison line estimator

The case the methods standard cites for **a model's numbers being provisional
until the port reproduces them**. Porting `report 0012` appendix to Rust
overturned three of its published figures — the resolution law is a sharp
transition rather than a smooth curve, weak-second-string sensitivity is
separation-limited rather than level-limited, and bass repeat reproducibility is
1 % rather than 11 %, which removed one of the three legs an earlier conclusion
stood on.

§6 is the sharpest of them. The design note's χ² discriminator was built exactly
as specified and **measured wrong**, rejecting 87 % of genuine tenor unisons.
Building it as written is how that was found.

## Where this stands

**IMPLEMENTED 2026-08-07.** An offline measurement phase ran first
(2026-08-05, against all three capture sets) and settled the estimator's shape
before any hot-path code was written; its derivation trail is
[`report 0012` appendix](0012-unison-line-estimator.md#appendix--the-design-notes-derivation-trail), kept
as the record of what was considered and rejected. This report carries the shipped
decisions and the numbers the Rust port itself produces.

Shipped: `algorithms::peaks::resolve_lines` (stateless), `strobe::unison` (the
per-reference ring and the discriminator), a widened crossing #2, and a GUI
panel. The port is validated by `cargo lab strobe replay` experiments **E6–E8**
(E6's law is also asserted in `tuner-core/tests/unison_resolution.rs`) and by
`cargo bench -p tuner-core` for E9's per-hop cost,
run on both pianos. The band-slope and coarse readouts are **bit-identical**
before and after (E1–E5 diffed against `HEAD`), which is the property the tap
doctrine requires.

Three things changed against the design note as a direct result of porting it,
and each is a measurement rather than a preference: the discriminator's
statistics (§6), the estimator's usable record floor (§3), and the accuracy
claim near the resolution limit (§4). The bass stays out of scope (§5).

## Context

Setting a multi-strung note's strings to zero-beat against each other is the one
genuine tuning need the app did not serve — the README said so, and the strobe
design note §7.4 proposed an **envelope-beat** readout for it: watch the Goertzel
*magnitude*, which swells at |f₁ − f₂|, and estimate that periodicity.

§7.4's physics is right and its conclusion is wrong. The magnitude route discards
the phase, and the phase carries the **sign** of each offset and the **positions**
of the strings; it also collapses a three-string note's three pair-beats into one
real signal and needs a decay detrend that is itself an error source. Both routes
hit the same ≈2/T resolution wall, so it buys nothing for what it gives up.

Resolving the individual strings is also what the modern field ships — TuneLab's
Spectrum Display, PianoMeter's Peak View, pianoscope 4.0 — which the §12 survey
missed entirely.

## The estimator

Per reference *i*, one curve-target frequency the `Strobe` already evaluates.

**It is a zoom FFT** (Lyons ch. 13) whose front end is already running. The
strobe's Hann-windowed Goertzel at `f_ref` is the mixer and anti-alias filter;
taking one output per hop is the decimator. Keeping the amplitude *with* the
phase — which the bank computes and then throws away — gives a complex baseband
sampled at 43.07 Hz, unambiguous over ±21.53 Hz, in which each string is a
phasor turning at its own offset from the target. It buys the resolution of a
~57 000-point DFT of the raw audio for about a thousandth of the work.

The demodulation needs no new state: the bank's accumulated beat angle already
*is* the reference rotation removed. It differs from the textbook
`φ_h − 2π·f_ref·h·H/f_s` by the run's first phase and a whole number of turns,
i.e. by one constant rotation of the whole record, which changes neither `|Z|`
nor the Candan ratio.

Then, per hop: Hann-window the record, take an `N`-point complex DFT at
**natural Fourier bins**, take circular local maxima of `|Z|` in descending
magnitude, refine each sub-bin by Candan Eq. 1 on the complex bins, reject any
candidate within 2 bins of an accepted stronger one, and admit by an
ordered-statistic CFAR gate. Candidates are magnitude-sorted, so the first
rejection ends the list.

No zero-padding, deliberately: padded bins are interpolated rather than
independent, and the CFAR null assumes independence.

## The measurements behind the decisions

Decided in report 0012.

### 1. The window stays 1024 samples

Decimation theory says otherwise and was overruled by the null. Taking one
Goertzel output per hop *is* a decimation, so alias-free operation needs the Hann
half main lobe below the baseband Nyquist, i.e. `N > 4H` — 4096, not 1024. The
4096 window also buys 7–9 points of real treble availability.

But its 75 % overlap makes consecutive baseband samples correlated, and
correlated reference cells are exactly what a CFAR threshold assumes away:
**45 % false second lines** at SNR 15 and a 30-hop record (design note E-U). The
hypothesis that aliasing was what produced the unexplained lines was tested
separately and **refuted** — the window barely moves the unmatched rate (E-T).

So the theoretical argument and the availability measurement both favoured 4096
and the null test refutes it. Revisitable only by re-deriving the effective
independent reference count for a 75 %-overlapped baseband (≈N/4, not the N/2 the
Hann-correlation halving assumes) and re-running that table. Not needed for v1.

### 2. The CFAR reference geometry is the *opposite* of `coarse_read`'s

`coarse_read` draws its reference cells from flanks **outside** the search band
and has no guard cells, both for reasons that are correct there and wrong here.
Outside-flank references exclude the dominant line's own skirt, so a secondary
maximum riding that skirt is compared against distant background and passes.
Measured on a *single* synthetic string: 7.5 % false second lines at 56 hops,
**20.8 %** at 86, **26.7 %** with a fast decay — worse at high SNR and longer
records, the signature of a deterministic artefact rather than noise.

The shipped geometry is a **sliding local window with main-lobe guard cells**:
guard 2 bins either side, 16 reference cells per side. That gives **0 false second
lines in 2 160 null trials** across SNR 6–40 dB, decay 0.15–1.5 s and 30–86 hops
(design note), and the Rust port reproduces it — E6c is 0 % in every cell.

The rank is the paper's own median, `q = 0.50`, and here Rohling §V binds from
the usual side rather than the inverted one that forced
`COARSE_CFAR_QUANTILE = 0.25`: two interfering lines occupy `2 × 5 = 10` cells of
the reference window against the `N_ref − k = 16` the criterion allows.
`q = 0.75` — the paper's worked value — loses a three-string unison outright
(detection 100 % → 0 %), the reference window being signal-dominated at the upper
quantile with three lines present.

The Hann halvings and the `m_eff` search-loss divisor follow `coarse_read`'s
calibrated pattern, because this detector also takes an argmax (report 0011 §5) —
over the whole record here, there being no bounded band.

### 3. The record has a floor, and it is Rohling §V solved for length

`UNISON_MIN_BINS = 25` hops (0.58 s) is not a display policy. Below it the
reference window is signal-dominated whenever a second string is present, so
"one line" stops meaning "no second line" and starts meaning "the detector is
blind" — which is the one thing the panel must never say silently.

It is the same criterion as the rank, solved for the record instead: two
interferers × one main lobe each (`2·GUARD + 1 = 5` cells) must fit inside
`N_ref − k = N_ref/2`, so `N_ref ≥ 20`; the cell under test and its guard cost
5 bins; hence 25. It lands within a hop of the 20 the offline harness had picked
by hand, which is a coincidence worth noting and not evidence.

Below 37 bins the "16 per side" window naturally degrades to *every* bin outside
the guard, which is the same thing said with fewer cells.

**There is no matching ceiling, deliberately.** How long a record is worth
keeping is the caller's policy — Weinreich coupling, §4 — and the estimator holds
no opinion on it. An earlier scan retired examined bins in a `u64` bitmask, which
put an accidental 64-bin ceiling on the algorithm; advancing a cursor through a
strict total order on `(magnitude, bin)` does the same job with no bound at all,
and the two are behaviourally identical (E6a and E6c unchanged, E1–E5 still
byte-identical).

`candan_bias_correction`'s 2.0 fallback is a **2.4 % scale error** on every
reported offset at these lengths (`c_N` = 2.050 at N = 56, 2.117 at N = 25), so
Candan Eq. 12 is now evaluated numerically — `spectral::candan_c_n`, the paper's
own prescribed route — once per length at startup. The same function reproduces
the shipped 2048/8192 table to six decimals, which retro-validates constants that
had been carried as bare numbers since audit 03.

### 4. Resolution, and the honesty it forces

`P(two lines resolved)`, 40 trials per cell, two equal strings, τ 1.5 s, SNR
40 dB, driven as **audio** through the shipped Goertzel front end:

| split Hz | 0.46 s | 0.65 s | 0.93 s | 1.30 s |
| --- | --- | --- | --- | --- |
| 0.7 | — | 100 % | 0 % | 0 % |
| 1.0 | — | 0 % | 0 % | 0 % |
| 1.5 | — | 0 % | 0 % | 100 % |
| 2.0 | — | 0 % | 100 % | 100 % |
| 3.0 | — | 100 % | 100 % | 100 % |
| 5.0 | — | 100 % | 100 % | 100 % |
| *2/T* | *4.31* | *3.08* | *2.15* | *1.54* |

The 0.46 s column is **withheld, not failed**: 20 hops is under the §3 floor.

Two things this table says that the design note's smoother version did not.

**The transition is sharp, not gradual.** At 40 dB the outcome is essentially
deterministic in the split — the design note's intermediate percentages do not
reproduce. A pair resolves when its separation clears `2/T` and not before.

**"Two lines" and "the right two lines" are different claims.** The split the
estimator *reported* where it resolved:

| split Hz | 0.65 s | 0.93 s | 1.30 s |
| --- | --- | --- | --- |
| 0.7 | **3.48** | — | — |
| 1.5 | — | — | **1.90** |
| 2.0 | — | **2.92** | 1.92 |
| 3.0 | 3.46 | 2.57 | 2.99 |
| 5.0 | 4.97 | 5.01 | 5.00 |

Sorted by the ratio of the true split to `2/T`, the pattern is sharp:

- **At or below 1.0 ×** the reported split collapses onto the limit itself —
  1.13, 1.36, 1.23 and 1.12 × `2/T` for the four such cells, whatever the truth
  was. A 0.7 Hz pair is reported at 3.48 Hz. This is survivorship: the pair is
  only *seen* in the realisations where it happened to look at least `2/T` wide,
  and those are the only ones the median is taken over.
- **Between 1.0 and ~1.6 ×** it is unreliable in **both** directions — +46 %
  (2.0 Hz at 0.93 s) and −14 % (3.0 Hz at 0.93 s).
- **Above ~1.6 ×** it is exact, to the systematic −0.085 Hz below.

**This is the whole argument for publishing `2/T` and putting it on screen**, and
it is why the panel states "resolved to ±x ¢" rather than "clean". It also says
where a *stronger* display rule could sit if one is wanted: the boundary at which
the number becomes trustworthy is derived, not chosen.

Accuracy where it resolves cleanly, 1.30 s record, 2.0 Hz split: σ 0.000 /
0.003 / 0.009 Hz at SNR 40 / 15 / 6 — the estimator's own scatter is
noise-limited and tiny — with a systematic **−0.085 Hz** bias, four times the
design note's model figure. Position error 0.042 Hz.

**Sensitivity to a weak second string is separation-limited, not level-limited**,
and the trade is a surface rather than a number. `P(two lines)` at the 56-hop
record, separation in units of that record's own `2/T` = 1.54 Hz:

| 2nd string | 1.0 × | 1.3 × | 1.6 × | 2.0 × | 2.6 × | 3.3 × |
| --- | --- | --- | --- | --- | --- | --- |
| 0 dB | 100 % | 100 % | 100 % | 100 % | 100 % | 100 % |
| −6 dB | 100 % | 100 % | 100 % | 100 % | 100 % | 100 % |
| −12 dB | **0 %** | **0 %** | 100 % | 100 % | 100 % | 100 % |
| −20 dB | **0 %** | **0 %** | 100 % | 100 % | 100 % | 100 % |
| −26 dB | **0 %** | 100 % | 100 % | 100 % | 100 % | 100 % |

Two strings **within 6 dB of each other resolve at the geometric limit itself**,
at any separation the record can see. The dynamic-range limit bites only in a
narrow band — separations between 1.0 and 1.6 × `2/T` — and only once the second
string is 12 dB or more down, where it is a shoulder on the strong line's main
lobe and never becomes a local maximum of its own. (The −26 dB row recovering at
1.3 × is consistent with that mechanism: too weak to fuse with the main lobe, it
keeps its own maximum.) The design note's flat "−26 dB" therefore holds nearly
everywhere and fails in one measured corner.

The same corner takes the split-decay case (τ 1.5/0.4 s, 2.0 Hz = 1.3 ×) to 0 %:
after 1.3 s the second string is ~28 dB down at 1.3 × the limit, which is exactly
the cell above.

**What the panel does not say, and why it stays that way.** The resolution it
states is a *geometric* limit, and in that corner the real limit is dynamic range;
it over-promises there by up to 1.6 ×. It is not conditioned on the level ratio
because **the condition is unobservable exactly when it applies**: the corner is
the one where a second line is *not* detected, so the panel has one line, no
second amplitude, and no way to tell a genuinely single string from a pair with a
quiet member. Stating 1.6 × `2/T` always would be honest in that corner and
pessimistic by 60 % in the overwhelming majority, where the strings are within
6 dB. Closing it properly is an estimator change — fitting a single Hann kernel
and testing the residual for an asymmetric shoulder — not a display change.

**Both boundaries in §4 land at ≈1.6 × `2/T`**: the separation above which the
reported split is exact, and the separation above which a 12 dB-down string is
found at all. That is one mechanism, not two — main-lobe overlap — and it is the
same mechanism a shoulder detector would attack.

### 5. The bass is out of v1, and the discriminator is what keeps it out

Keys 0–27 produce a second line on essentially every capture of both instruments
— including keys 0–11, which are **single-strung**. Three independent facts agree
that these are not unisons: the splits are not constant in cents (spread across
partials 33–51 % against 12–17 % in the tenor), they reproduce far worse across
repeat strikes, and they occur where there is only one string. What they *are* is
unestablished; the obvious artefact is ruled out (they are not aliased
neighbouring partials), and the candidates — polarization false beats,
longitudinal modes, sympathetic resonance from a neighbouring key, soundboard
coupling — were the subject of a separate investigation.

**[report 0014](0014-unison-panel-against-isolation-truth.md) §4 supplies the
positive control this section lacked.** On 78 single-strung keys — where only one
string can possibly sound — the estimator resolves two lines on **65 %** of
captures, which settles by construction that the bass second line is not a second
string. The discriminator asserts `Unison` on 2.1–3.6 % of all 193 solo captures.

**[report 0013](0013-bass-extra-lines-attribution.md) supersedes this section's
scope wording.** The register ships — the estimator is sound there and the lines
are not a second string — while what they *are* stays open. The scoping
mechanism below is unchanged and is why it can ship at all.

**There is no key threshold anywhere**, and there must not be. The scoping
mechanism is the discriminator: measured over both instruments, the panel asserts
"unison" in the bass on **0 %** (piano 1) and **4 %** (piano 2) of captures. The
top octave excludes itself for a different reason — the strobe's D3 gate never
opens there, so the ring never fills (availability 8 % / 17 %), which is the same
outcome a register constant would give without the constant report 0011 §7 warns
against.

### 6. The discriminator: the design note's test was mis-specified, and measuring it is how we know

Under the unison hypothesis two strings' partial *n* sit at frequencies whose
ratio is the same for every *n*, so their separation is proportional to the
partial frequency — constant in cents. The design note proposed testing that as a
χ² goodness-of-fit against **the estimator's own measured line σ** (≈0.05 Hz per
line ⇒ ≈0.07 Hz per split).

Built exactly as specified, it called **87 % of piano-1 tenor captures false
beats.** The mechanism is not subtle once measured: the *physical* scatter of the
split across partials runs several times the estimator's precision — the strings
of a unison differ slightly in B as well as in f₀, and Weinreich coupling moves
them — so a null built from the instrument's precision rejects almost everything.
This is the same class of defect as the Neyman–Pearson σ entry (then in
`suspected-issues.md`, since retired; now [report 0015](retiring/0015-ambient-sigma-gates-measured.md)):
a correctly-derived threshold against the wrong noise.

The shipped test avoids assuming σ at all. Both hypotheses are members of one
family,

```text
    ln Δ = ln a + p·ln f       p = 1 unison,  p = 0 fixed in Hz
```

so the exponent is fitted, its standard error is **estimated from the residuals**
(which absorbs the physical scatter) and floored at the estimator's own precision
(which no fit can beat), and a verdict is returned only when `p̂` sits within
`UNISON_FIT_SIGMAS = 3` standard errors of one hypothesis and further than that
from the other. Three partials minimum: the free slope costs two parameters.

Measured, over the best record each capture reached:

| set | register | unison | false beat | undetermined |
| --- | --- | --- | --- | --- |
| piano 1 | bass | **0 %** | 36 % | 64 % |
| piano 1 | tenor | 9 % | 22 % | 70 % |
| piano 1 | treble | 0 % | 0 % | 100 % |
| piano 2 | bass | **4 %** | 38 % | 58 % |
| piano 2 | tenor | 3 % | 10 % | 87 % |
| piano 2 | treble | 0 % | 0 % | 100 % |

**[report 0014](0014-unison-panel-against-isolation-truth.md) §7 confirms the
mechanism with ground truth.** A note's strings really do differ in `B` — by
1.7–8.5 %, exceeding within-string repeat noise by 3.7–8.5× on five of eight
isolation keys — which tilts the split across partials by 3–48 % of itself. That
is exactly the physical scatter a σ-based null cannot survive and a
residual-estimated one absorbs, so the fix below was made for the right reason.
It also names a second contributor to the near-silence: the tilt is systematic
rather than random, so absorbing it *inflates* the standard error.

It is **conservative to the point of near-silence**, and that is the deliberate
trade: `Undetermined` leaves the panel showing its per-partial splits, which is
what a tuner would read anyway, while a wrong verdict is an assertion about the
instrument. The dominant reason for `Undetermined` is lever arm — a fit over
three neighbouring partials barely separates a slope of 0 from a slope of 1 — and
the fix is more resolved partials, i.e. longer records, not a looser test.

### 7. Availability, and reproducibility as the truth-free check

At the displayed partial n\*, which is what the panel shows:

| set | register | captures | ≥1 line | ≥2 | ≥3 |
| --- | --- | --- | --- | --- | --- |
| piano 1 | tenor | 23 | 100 % | 70 % | 17 % |
| piano 1 | treble | 24 | 96 % | 88 % | 62 % |
| piano 1 | high 76–87 | 12 | 8 % | 8 % | 0 % |
| piano 2 | tenor | 216 | 100 % | 69 % | 17 % |
| piano 2 | treble | 168 | 97 % | 90 % | 40 % |
| piano 2 | high 76–87 | 69 | 17 % | 17 % | 6 % |

Over *every* reference in the bank the figures are much lower (tenor 58–69 %
≥1 line, treble 15–26 %) because most of a key's twelve partials are dead or
below the gate. That is the honest denominator for "how much of the bank
resolves", and n\* is the honest one for "does the panel work".

A third instrument checks the null from the other side. The guitar set is one
string per course, so the panel must never report a pair — and at the displayed
partial it reports **≥2 lines on 0 % of captures** while still finding the single
line on 75 % of them. Six captures is not a result, but a false-split rate that
happened to be nonzero there would have been one.

**Reproducibility** is the strongest evidence available, because there is no
truth at the splits that matter — any reference DFT sees the same 1.3 s and so
has the same resolution limit. If the estimator measures the instrument rather
than noise, independent strikes must agree. Piano 2, keys with ≥3 strikes:

| register | keys | median split | median MAD across repeats | relative |
| --- | --- | --- | --- | --- |
| bass | 9 | 21.48 ¢ | 0.15 ¢ | 1 % |
| tenor | 17 | 7.96 ¢ | 0.11 ¢ | 1 % |
| treble | 23 | 7.20 ¢ | 0.08 ¢ | 1 % |

0.08–0.15 ¢ across independent strikes. Note this **diverges from the design
note**, which put bass reproducibility at 11 % relative and used it as one of
three arguments that the bass lines are not unisons; the shipped estimator
reproduces the bass as well as it does the tenor. The other two arguments (the
splits are not constant in cents, and they occur on single-strung keys) are
untouched, so §5's conclusion stands — but this one should not be repeated.

### 8. The unexplained lines: real energy, near the limit

The design note left ≈5–6 % of tenor/treble lines with no counterpart in a
full-rate DFT, and the open question was whether the estimator was inventing
them. The investigation matched every reported line against a full-rate DFT of
the **identical span** with an **uncapped** peak picker, and then asked the
question a peak picker cannot: is there energy at that frequency at all?
"Excess" below is the reference DFT's magnitude at the reported frequency over
the median of its own band.

| set | register | rank | lines | unmatched | med excess | excess > 2 |
| --- | --- | --- | --- | --- | --- | --- |
| piano 1 | tenor | 1 | 190 | 3.7 % | 0.8 | 14 % |
| piano 1 | tenor | 2 | 138 | 19.6 % | 15.7 | 70 % |
| piano 1 | tenor | 3 | 61 | 24.6 % | 16.2 | 80 % |
| piano 1 | treble | 2 | 37 | 10.8 % | 103.7 | 75 % |
| piano 1 | treble | 3 | 26 | 11.5 % | 11.5 | 100 % |
| piano 2 | tenor | 1 | 1493 | 0.7 % | 237.6 | 100 % |
| piano 2 | tenor | 2 | 1080 | 18.6 % | 33.5 | 77 % |
| piano 2 | tenor | 3 | 440 | 24.5 % | 4.9 | 61 % |
| piano 2 | treble | 2 | 268 | 14.6 % | 41.6 | 87 % |
| piano 2 | treble | 3 | 125 | 25.6 % | 30.6 | 100 % |

**The residue is not fabricated.** On both instruments, 61–100 % of the unmatched
lines sit on real spectral energy — 3× to 240× the band median — that the
reference's local-maximum picker does not list, because it is a *shoulder* rather
than a separate maximum. The zoom's effective window is the Goertzel's Hann
convolved with the record's own, so the two views differ near the limit and the
zoom sometimes separates what the reference renders as one broadened peak.

The separation evidence agrees where it is legible: on piano 1 the unmatched
lines sit measurably closer to their sibling than lines in general (tenor rank 3,
1.55 × 2/T against 2.37; treble rank 3, 1.36 against 1.99), which is the
signature of a barely-separated pair. On piano 2 the effect is present in the
treble (1.48 against 1.81) and absent in the tenor.

Taken with §4 — separations within 2 × 2/T are inflated — the attribution is
**position error near the resolution limit, not spurious detection**. It is
worst in the weakest of three lines, which is why the panel draws that marker as
provisional; the residual genuinely unattributed fraction is the ~39 % of
piano-2 tenor rank-3 unmatched lines with no energy excess, i.e. ≈10 % of third
lines.

The **bass behaves differently** and is not explained by this: its unmatched
lines carry little excess (median 1.4–3.5, only 21–55 % over 2) and sit *further*
from their siblings, not nearer. That is consistent with §5 — whatever the bass
lines are, they are not shoulders of a barely-resolved unison — and it is
the bass second-lines problem.

### 9. Cost

Measured in `--release`, all twelve references live and every ring at the cap —
a worst case a real capture does not reach, since a key's upper partials gate out
and stop transforming:

| quantity | measured | against |
| --- | --- | --- |
| whole bank, per hop | median 87 µs, p90 95 µs, max 105 µs | 23.2 ms callback (0.4 %) |
| `resolve_lines` × 12 alone | 45 µs | 0.19 % of the callback |
| ring memory touched per hop | 5.2 KB | `FrameOutput` already carries 16 KB |

The design note's operation count (≈10 % of one 8192-point FFT) was optimistic by
roughly 2× against the whole-bank figure, and irrelevant either way: the
transform decimation it offered as an escape hatch is not needed, so the stride
was never derived and no such constant exists.

## Consequences

- `FrameOutput` widens (crossing #2): per-reference lines as **signed Hz offsets**
  plus relative amplitudes, the per-reference Goertzel **amplitude** that strobe
  design R2 specified and was never built, the ring's current resolution, and the
  discriminator's verdict. Hz never cents — the frontend owns the reference a
  number is displayed against.
- `strobe.rs` splits into `strobe.rs` + `strobe/{band_slope,unison}.rs`, following
  the parent-file-beside-its-directory pattern the workspace already uses. The
  band-slope tests move with their subject, unchanged.
- The `Strobe` remains a **tap**: nothing in the chain consumes its output, and
  E1–E5 are bit-identical against `HEAD`.
- The panel is gated on the strobe's existing debounced `out_of_range` verdict.
  Past ±21.5 Hz the baseband folds, so the lines would be real content at
  fictitious places, and that flag already computes exactly this question.
- Two panel layouts ship behind a toggle — the displayed partial alone, and every
  resolved partial stacked. Which reads better in use is not a question the
  captures can answer.

## Limitations / threats to validity

- **Two instruments, both out of tune.** Per the n = 2 rule these captures
  validate; they cannot select a configuration.
- **The reference is assumed correct.** Every real-capture run used the *measured*
  partial frequency as `f_ref`, which centres the baseband. In the app `f_ref` is
  the curve target and an out-of-tune string sits off-centre; past ±21.5 Hz the
  lines fold. The reference-offset sweep was not run — the `out_of_range` gate is
  what stands in for it.
- **Unison state is unrecorded, and 1.5 s cannot supply it.** Nothing in the
  capture sets says how far apart any note's strings actually were. Nor can it be
  recovered: 1.5 s bounds *any* method to ≈1.33 Hz, which at C#4's 2nd partial is
  4.2 ¢ — enough for the working range but not for the endgame, where a set
  unison is well under 1 ¢. So the captures cannot distinguish "clean" from "out
  of resolution", which is the one distinction the display must be honest about.
  The fix is longer recordings and a mute test, not better estimation.
- **Non-stationarity and coupling are unmodelled** in the synthetic trials: real
  strings' pitch falls in the first tens of ms, and Weinreich coupling exchanges
  energy between strings over the note. The synthetic sources are independent
  damped exponentials.
- **The bass ring runs a 4096-sample Goertzel** (the existing R3 rule), so its
  baseband is 4× oversampled and its noise correspondingly correlated — the
  assumption §1's null rests on. **Measured since (report 0013 §1):** the null does
  break there, by too little to account for §5, and the overlap costs exactly the
  2× that §1's `N_ref/4` conjecture assumed.
- **The discriminator is near-silent** (§6) and its `Undetermined` rate is
  dominated by lever arm, not by the instrument.
- **≈10 % of third lines remain unattributed** (§8).
- The guitar set is out of scope for this feature (single strings, six captures).

## Artifacts & reproduction

```bash
cargo lab strobe replay diagnostics_piano_1
cargo lab strobe replay diagnostics_piano2
```

E6 is synthetic (resolution law, accuracy, null); E7 is availability,
reproducibility and the verdict distribution; E8 is the unexplained-line
investigation; E9 is cost. E1–E5 are report 0011's and must not move: they were
diffed against a `HEAD` worktree and are byte-identical.

Piano #2's cached deep-bass partials predate the `MAT_SEED_TOLERANCE` fix, so the
harness drops any capture whose fundamental is beyond ±200 ¢ of ET rather than
consuming it ([`capture-sets.md`](../docs/internals/capture-sets.md)).

## References

- Rohling, H. (1983). *Radar CFAR Thresholding in Clutter and Multiple Target
  Situations.* IEEE Trans. AES-19(4). — the admission gate; §V sets both the rank
  and the record floor. Ported in `peaks.rs`; audit 13.
- Candan, Ç. (2015). *Fine resolution frequency estimation from three DFT
  samples: case of windowed data.* Signal Processing 114. — Eq. 1 the refiner,
  Eqs. 6/10/12 the bias factor now evaluated numerically. Audit 03.
- Lyons, R. (2010). *Understanding Digital Signal Processing*, ch. 13, "The Zoom
  FFT". — the demodulate-decimate-transform structure.
- Weinreich, G. (1977). *Coupled piano strings.* JASA 62(6). — why unisons beat,
  why the beat is not stationary, and why the record has a cap at all.
- Conklin, H. A. (1996). *Design and tone in the mechanoacoustic piano.* JASA —
  longitudinal modes, a candidate for the bass lines.

## Appendix — the measurements the design note left behind

The design note this feature came from is superseded by the report above and by
the shipped code; git history holds it in full. What is kept here is the subset
of its measurement phase that something still points at: the resolution law and
accuracy bounds (E-A, E-B), the decimation finding that governs the bass window
(E-Q), the folded-interferer reference it is scored against (E-R), the window
question settled against the theory (E-T/E-U), and the partial-folding null that
report 0013 builds on (E-N, inside E-J/E-L).

### 5. The measurement phase (2026-08-05)

Method: a Python model mirroring the above, run against all three capture sets.
Piano #2 consumed through `regenerate_partials` per
[`capture-sets.md`](../docs/internals/capture-sets.md). The model's numerical
calibration of `c_N` reproduces the shipped Rust table to six decimals
(2.001325 vs 2.001329 at N=2048; 2.000331 vs 2.000332 at N=8192).

**What counts as truth.** On real captures there is none at the splits that
matter: any reference DFT sees the same 1.3 s and so has the same resolution
limit. Real-capture work therefore validates *implementation* and *consistency*;
every **accuracy** claim rests on synthetic signals, which are rendered as audio
and pushed through the same Goertzel front end so the window, the decay and the
strike are in the loop.

### E-A — resolution law (synthetic, exact truth)

P(two lines resolved); two equal strings, τ 1.5 s, SNR 40 dB, 40 trials/cell:

| split Hz | 0.46 s | 0.65 s | 0.93 s | 1.30 s | 2.00 s | 3.00 s |
| --- | --- | --- | --- | --- | --- | --- |
| 0.7 | 8 % | 18 % | 20 % | 15 % | 40 % | **100 %** |
| 1.0 | 12 % | 15 % | 15 % | 28 % | **88 %** | 100 % |
| 1.5 | 10 % | 20 % | 42 % | **57 %** | 100 % | 100 % |
| 2.0 | 5 % | 25 % | 57 % | **100 %** | 100 % | 100 % |
| 3.0 | 30 % | **68 %** | 100 % | 100 % | 100 % | 100 % |
| 5.0 | **100 %** | 100 % | 100 % | 100 % | 100 % | 100 % |
| *2/T floor* | *4.31* | *3.08* | *2.15* | *1.54* | *1.00* | *0.67* |

**50 % detection at ≈ 2/T, 100 % at ≈1.3–1.4 × 2/T.** `2/T` is therefore the
honest number for the display to state as its current resolution.

### E-B — accuracy where it resolves (synthetic, 1.30 s)

| case | P(2) | split bias | split σ | position error |
| --- | --- | --- | --- | --- |
| equal, 2.0 Hz, SNR 40 | 100 % | +0.022 Hz | 0.046 | 0.050 |
| equal, 2.0 Hz, SNR 15 | 100 % | +0.008 | 0.057 | 0.057 |
| equal, 2.0 Hz, SNR 6 | 100 % | +0.002 | 0.053 | 0.053 |
| second string −20 dB | 100 % | +0.003 | 0.023 | 0.020 |
| fast decay τ 0.4 s | 100 % | +0.011 | 0.052 | 0.053 |
| split decay 1.5/0.4 s | 100 % | +0.004 | 0.023 | 0.016 |

Bias ≤ 0.02 Hz, σ ≤ 0.06 Hz, essentially SNR-independent down to 6 dB.
Sensitivity reaches a second string **26 dB below** the first at ≥1.3 s.

### E-J / E-L — the discriminator, and the bass

| set | register | captures ≥3 partials | median split | spread across n | "constant" (<25 %) |
| --- | --- | --- | --- | --- | --- |
| piano1 | tenor | 24 | 4.18 ¢ | 26 % | 46 % |
| piano1 | treble | 11 | 3.59 ¢ | 15 % | 64 % |
| piano2 | tenor | 214 | 4.24 ¢ | 17 % | 67 % |
| piano2 | treble | 125 | 4.04 ¢ | 12 % | 63 % |
| piano1 | **bass** | 27 | **24.56 ¢** | **33 %** | 30 % |
| piano2 | **bass** | 137 | **32.46 ¢** | **51 %** | 19 % |

Null bands, where a second line cannot be a unison:

| band | cases | ≥2 lines | median split | spread across n |
| --- | --- | --- | --- | --- |
| piano keys 0–11 (single-strung) | 72 | **100 %** | 28–51 ¢ | 40–51 % |
| piano keys 12–27 (bichord) | 97 | 94–100 % | 17–28 ¢ | 29–54 % |

**The bass produces second lines essentially always and they are not unisons.**
No piano has bass unisons 25–50 ¢ apart, and keys 0–11 are single-strung. Three
independent facts agree: the splits are not constant in cents (33–51 % spread vs
12–17 % in the tenor), they reproduce only to 11 % across repeats vs 1–2 %, and
they occur where there is only one string. **E-N** rules out the obvious
artefact: only 3–22 % of second lines land within 1 Hz of where a neighbouring
partial would fold after decimation, and the median split (3.3–4.5 Hz) is nowhere
near the median fold (7.4–17 Hz).

What they *are* is not established — see §7.

### E-Q — the 1024-sample window is under-filtered for a 1024-sample hop

Taking one Goertzel output per hop *is* a decimation, so the window is the
anti-alias filter. Alias-free decimation by `H` needs the Hann half main lobe
`2·f_s/N` below the baseband Nyquist `f_s/(2H)`, i.e. **N > 4H**:

| N | half main lobe | vs 21.53 Hz Nyquist |
| --- | --- | --- |
| 1024 | 86.1 Hz | **aliases** |
| 2048 | 43.1 Hz | **aliases** |
| 4096 | 21.5 Hz | critically filtered |

So the treble path (1024 window, 1024 hop) is **4× under-filtered**: content
21.5–86 Hz from the reference folds back essentially unattenuated. At C#4's 2nd
partial a semitone is 32 Hz, so a neighbouring key sounding lands squarely in the
fold zone. Confirmed: an interferer at +30 Hz produced a spurious line within
1 Hz of its predicted fold (−13.07 Hz) in 100 % of trials.

The 4096 window is the theoretically correct filter — its first null sits exactly
at the baseband Nyquist — but it did **not** remove the spurious line in the same
test (Hann sidelobes still admit a strong out-of-band interferer). Only presence
was measured, not the folded line's *strength*, which is what decides whether it
out-ranks a genuine string.

**Closed 2026-08-12 by [report 0013](0013-bass-extra-lines-attribution.md)
§2**, which measured the strength: the fold is severe at the 1024 window and
suppressed at 4096, which is what makes R3's long window load-bearing for this
consumer too. The window choice itself does not move.

### E-R — erroneous lines, matched against the full-rate DFT

Every zoom line matched to the full-rate DFT's peaks by nearest neighbour; a line
with no full-rate peak within 0.5 Hz is unexplained:

| set | register | lines | matched | unmatched | median \|d\| |
| --- | --- | --- | --- | --- | --- |
| piano1 | tenor | 416 | 88 % | 12 % | 0.039 Hz |
| piano1 | treble | 219 | 95 % | 5 % | 0.049 |
| piano2 | tenor | 3976 | 88 % | 12 % | 0.048 |
| piano2 | treble | 2039 | 91 % | 9 % | 0.063 |
| piano2 | bass | 2133 | 78 % | **22 %** | 0.020 |
| piano2 | high | 164 | 79 % | **21 %** | 0.097 |

**5–12 % of lines in the feature's home register are unexplained**, and about a
quarter in the bass and top octave. Matched lines agree to 0.02–0.06 Hz. This is
an upper bound on the zoom's error rate — the full-rate reference used a crude
local-max picker capped at three peaks, so some "unmatched" lines may be real
peaks its own picker missed.

### E-T / E-U — the window question, settled against the theory

E-Q's decimation argument says the 4096-sample window is the correct anti-alias
filter and 1024 is 4× under-filtered. Two measurements were run to act on that.

**E-T — is aliasing what produces the unexplained lines?** Tenor and treble,
both windows, scored on the E-R unmatched rate:

| set | band | window | lines | unmatched | ungated hops |
| --- | --- | --- | --- | --- | --- |
| piano2 | tenor | 1024 | 1762 | 6 % | 98 % |
| piano2 | tenor | 4096 | 1977 | 6 % | 99 % |
| piano2 | treble | 1024 | 1486 | 6 % | 58 % |
| piano2 | treble | 4096 | 1552 | 5 % | **67 %** |
| piano1 | treble | 1024 | 176 | 3 % | 56 % |
| piano1 | treble | 4096 | 196 | 2 % | **63 %** |

**Hypothesis refuted:** the window barely moves the unmatched rate, so aliasing
is *not* what produces the unexplained lines. It does buy real availability —
7–9 points more ungated hops in the treble, from the halved Neyman–Pearson
threshold a longer window earns.

This run also corrects E-R: allowing the full-rate reference five peaks instead
of three roughly halves the unmatched rate (12 % → 6 % in the tenor). **Most
"unmatched" lines were peaks the crude reference picker had not listed**, so the
estimator's true error rate is nearer 5–6 %.

**E-U — the null in the 4096 configuration**, which had never been tested:

| case | want | 1024 @30 | 1024 @56 | 4096 @30 | 4096 @56 |
| --- | --- | --- | --- | --- | --- |
| null, τ 1.5 s, SNR 40 | 2 | 0 % | 0 % | 0 % | 0 % |
| null, τ 0.4 s, SNR 40 | 2 | 0 % | 0 % | 0 % | 2 % |
| **null, τ 1.5 s, SNR 15** | 2 | **0 %** | **0 %** | **45 %** | **4 %** |
| 2 strings 2.0 Hz | 2 | 34 % | 100 % | 38 % | 100 % |
| 3 strings 0/2.0/4.5 | 3 | 0 % | 100 % | 0 % | 100 % |

**The 4096 window breaks the null** at short rings and modest SNR — 45 % false
second lines. Its 75 % overlap makes consecutive baseband samples correlated, and
correlated reference cells are exactly what the CFAR threshold assumes away.

**Decision: keep the 1024 window.** The theoretical argument and the availability
measurement both favoured 4096 and the null test refutes it. The failure is
diagnosable rather than fundamental — with 75 % overlap the effective independent
reference count is ≈N/4, not the N/2 the Hann-correlation halving assumes — so
4096 could be revisited by re-deriving that factor and re-running this table. It
is not free, and it is not needed for v1.
