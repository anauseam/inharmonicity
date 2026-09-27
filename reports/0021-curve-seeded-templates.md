# report 0021 — discovery's templates, from the instrument's fitted B curve

Discovery scores every hop against 88 templates, one stiff-string partial series
per key, each stretched by a B. Until this change every template carried the
Rigaud prior (`models::get_expected_beta`), and a pathway gated off since
[report 0006](0006-discovery-refinement-validation.md) would have replaced a
measured key's template with its own measured B and left every other key on the
prior. This report measures what replaced it and is the record the change ships
on: **every template takes one curve, Rigaud's whole-compass B_ξ̂(m) with the bass
pair fitted to the instrument**, on the keys the measurement itself places on the
bass bridge, with the default pair below two such keys and the slope held at the
default below twelve keys of them. The fit reads each key's newest trusted capture,
as every product consumer of the profile does, and no template is a key's own
measurement. The Worker computes the 88 values with every curve bundle; the
frontend pushes them whole over the template channel; the per-key pushes, the flag
and the startup read of a pre-library profile file go.

What it leaves alone: discovery's scoring, the picker, the gate, the vote and the
hot path, which still swaps templates in at the top of a hop as it always did; the
strobe, which still builds each key's targets from its own measured B; and the
tuning-curve engines, whose own fit (`curves::instrument_b_fit`) still reads one
capture per key, so no curve number moves.

**The instrument's own B fixes bass locks wherever the bass sits off the prior**,
and one curve through it keeps that without a per-key table's failure: a table with
holes lets a measured key outbid its unmeasured neighbour. The fitted curve was
non-inferior in every register with the bass benefit on both reads of piano #2 at
sixteen phases of the hop grid, and then **confirmed on the three RWC pianos**,
instruments never read for the question, each under its own scatter. The research
fitted each key's median of agreeing repeats; the shipped source fits each key's
newest trusted capture, so its pairs are not the scored ones. The two curves are
within 0.6 % of each other at every key on four of the five reads and within 2.7 %
on the RWC Steinway, where the shipped admission keeps a key the research's screen
had dropped. At phase 0 the shipped path reproduces the research's locks capture for
capture on three reads and moves one capture of piano #2 and two of the Steinway.
Scored afterwards at sixteen phases on piano #2, its extended captures and the
Steinway, the shipped source is inside each read's scatter in every register; on
piano #2 its treble sits nearer the edge of that scatter than the scored curve's.

## Pre-registered decision rule

Two rules, each fixed before its candidate was scored and applied as written.

**On piano #2, which decides, then its extended captures.** The null is the hop
grid: where a strike lands inside a 1 024-sample hop is an accident, and the
baseline replayed at sixteen phases of the grid, nothing else changed, is the
margin ([report 0020](0020-vote-from-onset.md), *The yardstick*). Per capture, the
mean over the sixteen phases of pass and of wrong; a cell's count is the sum.
**Non-inferior**: candidate mean pass at least the baseline's minus σ, and mean wrong
at most the baseline's plus σ, overall and in bass (keys 0–26), mid (27–59) and
treble (60–87), a register's wrong using that register's pass σ. **The bass
benefit**: the candidate's mean bass wrong below the baseline's minus the bass σ.
**ADVANCE**: non-inferior with the bass benefit on piano #2, and then, scored only
after the piano-#2 text was frozen, on the extended captures.

**On an instrument never read for the question.** The change ships only when the
source it implements is confirmed on such an instrument, at sixteen phases under
that instrument's own across-phase σ, computed from its baseline alone and frozen
before any candidate cell existed: non-inferior overall and in every register. The
bass benefit is asked where the baseline leaves room for it. A source is confirmed
on RWC when it is confirmed on every RWC piano.

Both rules scored the vote on `Stable` hops only, the lock as it was before
[report 0020](0020-vote-from-onset.md). The engine now votes from the onset; the
same cells under that vote are reported beside the decision, descriptively.

The labels in the tables are the runs' own: **prior** is the shipped template,
**curve** or **fit** the fitted curve in all 88, **holes** or **screened** the per-key
table (each measured key its own B, the prior elsewhere), **blend** the precision blend
the curve engines compute; **S** is the vote on `Stable` hops and **O** the vote from
the onset. U1 and U2 are the confirmation's two rules, the fit and the blend; PF1, PF2
and PF3 are RWC's own names for its Yamaha, Bösendorfer and Steinway.

## Population

| read | instrument | captures scored | keys | source | licence |
| --- | --- | --- | --- | --- | --- |
| piano #2 | upright piano #2, out of tune (median −0.8 ¢ from equal temperament) | 595 | 88, 5–16 captures each | the project's own captures | — |
| piano #2, extended | the same upright in its as-found state, before tuning | 472: every open or undeclared capture in the `Piano2_extended` store, its isolation captures excluded | 88, 4–17 each | the project's own captures | — |
| Iowa | Steinway & Sons model B grand, recorded 2001; channel 0 is a microphone 8 inches over the bass strings | 260 single notes at pp, mf and ff (pp lacks A0; mf lacks A0, A♯0 and F♯7); 26 pp notes have no measured onset and are placed by their loudest hop | 88 | University of Iowa Electronic Music Studios, Musical Instrument Samples, <https://theremin.music.uiowa.edu/MISpiano.html> | "downloaded and used for any projects, without restrictions" |
| MAPS, ambient | Yamaha Disklavier Mark III (an upright), omnidirectional microphones 3–4 m away | 179 of the 264 normally played notes, both pedal states, three dynamics; set apart: 79 whose measured onset is more than 100 ms from the set's annotation and 6 with no measured onset | 87 (no B4) | MAPS `ENSTDkAm`, Zenodo record 18160555 (v0.4) | CC BY-NC-SA 2.0 France |
| MAPS, close | the same Disklavier, microphones about 50 cm away | 257 of 264; set apart: 3 by the 100 ms rule and 4 with no measured onset | 88 | MAPS `ENSTDkCl`, same record | CC BY-NC-SA 2.0 France |
| RWC PF1 | a Yamaha piano | 264 normal-style notes, three dynamics | 88 | RWC Instruments Database, Zenodo record 17170844 | CC BY-NC 4.0 |
| RWC PF2 | a Bösendorfer piano | 259; set apart: 5 soft notes, F5 to A7, that the gate never takes to `Stable` | 88 | the same | CC BY-NC 4.0 |
| RWC PF3 | a Steinway piano | 264 | 88 | the same | CC BY-NC 4.0 |

Piano #2's two sets are one instrument, read twice, and neither satisfies the
two-instrument rule ([`CONTRIBUTING.md`](../CONTRIBUTING.md)). Its captures are
consumed through `cargo lab mat regen`; the external sets' B were measured with the
same estimator on the converted audio, 116 ms after the measured onset, where the
app's own capture would begin. Every external instrument was read from a published
recording, once, never through the app's microphone.

## Method

**The template source, as shipped** (`curves::template_b_fit`):

1. A key is read through its newest trusted capture (`InharmonicityProfile::active`),
   the one capture the curve input holds for it and every product consumer reads. An
   older capture of the key is not read, nor an auto-mode capture or a solo recorded
   after it.
2. A key's point is that capture's B, when the capture carries at least eight
   partials. The research's points were medians of agreeing repeats instead (*The
   admission*, below); the two differ in the pairs and in one capture of piano #2
   (*Re-baselining*).
3. The fit's points are the keys whose B is at least twice the universal treble
   asymptote e^(S_T m + Y_T) (Rigaud, David & Daudet 2013, §III.B.1): the keys the
   measurement places on the bass bridge. Eq. 29 is not identifiable from keys off it:
   fitted on the Disklavier's mid keys alone it put A0 at 25 times the default, on
   piano #2's treble keys alone at 224 times, and on its extended captures' at 0.08.
4. With two or more points covering at least twelve keys, lowest to highest
   inclusive, the bass pair is fitted by Eq. 29, least absolute deviation in ln B;
   covering fewer, the intercept alone at the default slope −0.066; with fewer than
   two points the default pair (−0.066, −7.891) stands, which is the prior at every
   key to f32 precision.
5. Every template takes B_ξ̂(m) = e^(s_B m + y_B) + e^(S_T m + Y_T) at its key.

**The research's form.** The research took its points from a screen instead: at
least two captures of a key with eight or more partials, all within a factor of two
of each other, and their median. It fitted the bass-bridge keys among them with the tree's
`rigaud::fit_b_xi`, with the default pair below two of them and no slope guard; the
guard was added after the partial-profile study (below) and is part of no scored
rule.

**The replay.** Each capture's audio was cut at sixteen phases of the hop grid, k =
0, 64, …, 960 samples removed from its head, and dumped with the shipped front end
under each template table; the engine's own state machine then scored the dumps
under each vote. A capture's outcome is its first commit against the key it was
captured as: pass, wrong or no lock. At phase 0 `cargo lab engine lock` with each
table reproduced the replay capture for capture on piano #2, its extended captures,
Iowa and the RWC pianos.

**Through the shipped path.** `cargo lab engine lock --regen <dump>` builds the
instrument's profile from a `mat regen` dump the way the app holds one (each entry
recorded as a named capture with its declaration, at most eight trusted per key, the
lab's ±200 ¢ plausibility rule applied) and pushes the templates the shipped source
makes from it. Entries are recorded in the dump's order, so a key's newest capture
is its last: on piano #2 the last struck; on the RWC pianos, whose dumps list each
key's strikes soft, medium, loud, the loud one on 81 to 86 keys of each piano and an
earlier strike where the dump holds no plausible loud one. The RWC measurements were
rewritten in that dump's schema by the same estimator at the same 116 ms, and
matched the research's measurement bit for bit in B and partial count on every one
of 758 captures before use.

## Results

### The mechanism: a table with holes

On the MAPS Disklavier's close read, a per-key table that measured C1 and D1 and left
C♯1 and D♯1 on the prior committed four captures a semitone low. C♯1's template
carried the prior, 8.4 × 10⁻⁵, beside C1 measured at 3.13 times its own prior, 2.75
× 10⁻⁴: the neighbour's stretched template predicted the true key's upper partials
better than the true key's own did. Every semitone-low commit the table made there:

| setting | capture | true key | committed under the screened table | phases | true key in the table | committed key in the table |
| --- | --- | --- | --- | --- | --- | --- |
| ENSTDkCl | `key_004_C#1_1002` | 4 C#1 | 3 C1 (-1) | 2 | prior | measured, 3.13× prior |
| ENSTDkCl | `key_004_C#1_1163` | 4 C#1 | 3 C1 (-1) | 12 | prior | measured, 3.13× prior |
| ENSTDkCl | `key_006_D#1_1164` | 6 D#1 | 5 D1 (-1) | 11 | prior | measured, 2.54× prior |
| ENSTDkCl | `key_006_D#1_1216` | 6 D#1 | 5 D1 (-1) | 3 | prior | measured, 2.54× prior |

The same signature appeared on piano #2 when its profile was cut to random partial
subsets: five F1 captures committed to E1, with E1 measured and F1 on the prior.
In a session the profile is partial by construction, so the gated pathway was that
table. Under the fitted curve all four of the Disklavier's semitone-low captures pass at
phase 0, the one phase that check scored:

| setting | capture | key | S scored it | prior | holes (S screened) | curve |
| --- | --- | --- | --- | --- | --- | --- |
| ENSTDkCl | `key_006_D#1_1216` | D#1 | yes | pass | pass | pass |
| ENSTDkCl | `key_004_C#1_1163` | C#1 | yes | pass | wrong → C1 | pass |
| ENSTDkCl | `key_006_D#1_1164` | D#1 | yes | pass | pass | pass |
| ENSTDkCl | `key_004_C#1_1002` | C#1 | yes | pass | wrong → C1 | pass |

### Piano #2, which decides

Over the sixteen phases, the prior in every template:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 595 | 564–571 (567.19 ± 2.04) | 8–14 (11.75 ± 1.71) | 13–19 (16.06 ± 1.95) |
| bass (0–26) | 137 | 133–135 (133.56 ± 0.70) | 0–0 (0.00 ± 0.00) | 2–4 (3.44 ± 0.70) |
| mid (27–59) | 298 | 296–297 (296.31 ± 0.46) | 0–0 (0.00 ± 0.00) | 1–2 (1.69 ± 0.46) |
| treble (60–87) | 160 | 135–140 (137.31 ± 1.53) | 8–14 (11.75 ± 1.71) | 8–14 (10.94 ± 1.64) |
| treble low (60–74) | 85 | 85–85 (85.00 ± 0.00) | 0–0 (0.00 ± 0.00) | 0–0 (0.00 ± 0.00) |
| treble high (75–87) | 75 | 50–55 (52.31 ± 1.53) | 8–14 (11.75 ± 1.71) | 8–14 (10.94 ± 1.64) |

The fitted curve in every template:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 595 | 565–571 (567.75 ± 1.79) | 9–14 (11.62 ± 1.54) | 13–18 (15.62 ± 1.62) |
| bass (0–26) | 137 | 134–136 (134.56 ± 0.70) | 0–0 (0.00 ± 0.00) | 1–3 (2.44 ± 0.70) |
| mid (27–59) | 298 | 296–297 (296.31 ± 0.46) | 0–0 (0.00 ± 0.00) | 1–2 (1.69 ± 0.46) |
| treble (60–87) | 160 | 135–139 (136.88 ± 1.32) | 9–14 (11.62 ± 1.54) | 9–14 (11.50 ± 1.27) |
| treble low (60–74) | 85 | 85–85 (85.00 ± 0.00) | 0–0 (0.00 ± 0.00) | 0–0 (0.00 ± 0.00) |
| treble high (75–87) | 75 | 50–54 (51.88 ± 1.32) | 9–14 (11.62 ± 1.54) | 9–14 (11.50 ± 1.27) |

The margins, as applied, with piano #2's σ (overall pass 2.04, wrong 1.95; bass
0.70, mid 0.46, treble 1.53):

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 595 | 567.19 | 567.75 | 565.15 | ✓ | 16.06 | 15.62 | 18.01 | ✓ |
| bass (0–26) | 137 | 133.56 | 134.56 | 132.86 | ✓ | 3.44 | 2.44 | 4.14 | ✓ |
| mid (27–59) | 298 | 296.31 | 296.31 | 295.85 | ✓ | 1.69 | 1.69 | 2.15 | ✓ |
| treble (60–87) | 160 | 137.31 | 136.88 | 135.78 | ✓ | 10.94 | 11.50 | 12.47 | ✓ |

| test | result |
| --- | --- |
| non-inferior, all | ✓ pass 567.75 ≥ 565.15, wrong 15.62 ≤ 18.01 |
| non-inferior, bass (0–26) | ✓ pass 134.56 ≥ 132.86, wrong 2.44 ≤ 4.14 |
| non-inferior, mid (27–59) | ✓ pass 296.31 ≥ 295.85, wrong 1.69 ≤ 2.15 |
| non-inferior, treble (60–87) | ✓ pass 136.88 ≥ 135.78, wrong 11.50 ≤ 12.47 |
| the bass benefit | ✓ 2.44 < 2.74 (baseline 3.44 − σ 0.70) |

**Non-inferior with the bass benefit.** The benefit is one capture,
`key_003_C1_1783694464`, which passes at every phase under the curve and at none
under the prior. The cost is in the top octave and inside σ: `key_076_C#7_1783697222`
commits to A♯0 on six phases, a bottom key whose template B is 4.73 times the prior.

### The extended captures, scored after the freeze

The same cells on the same piano in its as-found state, σ overall pass 1.36 and
wrong 1.14; bass 0.48, mid 0.24, treble 1.32:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 472 | 446.12 | 448.94 | 444.76 | ✓ | 9.06 | 6.00 | 10.20 | ✓ |
| bass (0–26) | 144 | 140.62 | 144.00 | 140.15 | ✓ | 3.38 | 0.00 | 3.85 | ✓ |
| mid (27–59) | 164 | 163.94 | 163.94 | 163.70 | ✓ | 0.06 | 0.06 | 0.30 | ✓ |
| treble (60–87) | 164 | 141.56 | 141.00 | 140.24 | ✓ | 5.62 | 5.94 | 6.95 | ✓ |

| test, extended store | result |
| --- | --- |
| non-inferior, all | ✓ pass 448.94 ≥ 444.76, wrong 6.00 ≤ 10.20 |
| non-inferior, bass (0–26) | ✓ pass 144.00 ≥ 140.15, wrong 0.00 ≤ 3.85 |
| non-inferior, mid (27–59) | ✓ pass 163.94 ≥ 163.70, wrong 0.06 ≤ 0.30 |
| non-inferior, treble (60–87) | ✓ pass 141.00 ≥ 140.24, wrong 5.94 ≤ 6.95 |
| the bass benefit | ✓ 0.00 < 2.90 (baseline 3.38 − σ 0.48) |

The benefit is three captures of G♯1, which pass at every phase under the curve and
at none under the prior. The cost is one top-octave capture, `key_084_A7_1787288143`,
which commits to A0 on eight phases. **ADVANCE on both reads.**

The table below reads every cell against the same baseline and σ; the bass benefit
is meaningful only for the S cells. On a complete profile the curve and the per-key
table do the same thing in the bass and the mid, and differ in the treble by less
than one capture's mean pass.

Piano #2:

| cell | against | mean pass | mean wrong | bass wrong | non-inferior | bass benefit | latency: faster / slower / equal | median Δ ms | p |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| prior · S (the baseline) | — | 567.19 | 16.06 | 3.44 | — | — | — | — | — |
| curve · S | prior · S | 567.75 | 15.62 | 2.44 | yes | yes | 62 / 71 / 457 | 0.0 | 0.488 |
| holes · S | prior · S | 567.56 | 15.88 | 2.44 | yes | yes | 60 / 64 / 466 | 0.0 | 0.788 |
| prior · O | prior · S | 566.94 | 16.31 | 3.44 | yes | **no** | 428 / 0 / 163 | -40.6 | 2.89e-129 |
| curve · O | prior · S | 567.62 | 15.75 | 2.44 | yes | yes | 442 / 45 / 103 | -47.9 | 4.96e-83 |
| holes · O | prior · S | 567.31 | 16.12 | 2.44 | yes | yes | 437 / 44 / 109 | -49.3 | 1.82e-82 |
| curve · O | prior · O | 567.62 | 15.75 | 2.44 | yes | yes | 122 / 180 / 288 | 0.0 | 0.00101 |
| holes · O | prior · O | 567.31 | 16.12 | 2.44 | yes | yes | 121 / 184 / 285 | 0.0 | 0.000369 |

The extended captures:

| cell | against | mean pass | mean wrong | bass wrong | non-inferior | bass benefit | latency: faster / slower / equal | median Δ ms | p |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| prior · S (the baseline) | — | 446.12 | 9.06 | 3.38 | — | — | — | — | — |
| curve · S | prior · S | 448.94 | 6.00 | 0.00 | yes | yes | 72 / 73 / 317 | 0.0 | 1 |
| holes · S | prior · S | 449.44 | 5.75 | 0.00 | yes | yes | 72 / 70 / 321 | 0.0 | 0.933 |
| prior · O | prior · S | 449.75 | 7.44 | 1.75 | yes | yes | 362 / 0 / 101 | -47.9 | 2.13e-109 |
| curve · O | prior · S | 450.94 | 6.00 | 0.00 | yes | yes | 362 / 50 / 50 | -53.7 | 1.8e-59 |
| holes · O | prior · S | 451.38 | 5.75 | 0.00 | yes | yes | 361 / 48 / 54 | -56.6 | 1.83e-60 |
| curve · O | prior · O | 450.94 | 6.00 | 0.00 | yes | yes | 120 / 119 / 224 | 0.0 | 1 |
| holes · O | prior · O | 451.38 | 5.75 | 0.00 | yes | yes | 121 / 116 / 227 | 0.0 | 0.795 |

### Partial profiles

A session measures some keys, not all. Piano #2's measured keys were drawn at random
in subsets of 1 to 32 (five draws each), plus the bass alone and the temperament
octave alone, and each subset scored at phase 0 as a per-key table and as the curve.
A neighbour commit is a wrong commit within two semitones: for the table, one where
the true key is unmeasured and the committed key measured. The baseline alone is
568 / 13 / 14, with bass wrong 3.

| n | design | draws | curve fallbacks | pass (sum) | wrong (sum) | bass wrong (sum) | draws better than σ somewhere | draws worse than σ somewhere | neighbour commits (sum) |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | holes | 5 | 0 | 2841 | 69 | 14 | 1 | 0 | 0 |
| 1 | curve | 5 | 5 | 2840 | 70 | 15 | 0 | 0 | 0 |
| 2 | holes | 5 | 0 | 2841 | 69 | 14 | 1 | 0 | 0 |
| 2 | curve | 5 | 4 | 2838 | 72 | 14 | 1 | 1 | 1 |
| 4 | holes | 5 | 0 | 2840 | 70 | 15 | 0 | 0 | 0 |
| 4 | curve | 5 | 5 | 2840 | 70 | 15 | 0 | 0 | 0 |
| 8 | holes | 5 | 0 | 2836 | 74 | 15 | 0 | 2 | 0 |
| 8 | curve | 5 | 0 | 2834 | 74 | 10 | 5 | 4 | 0 |
| 16 | holes | 5 | 0 | 2839 | 71 | 11 | 4 | 2 | 1 |
| 16 | curve | 5 | 0 | 2836 | 74 | 10 | 5 | 2 | 0 |
| 32 | holes | 5 | 0 | 2832 | 78 | 14 | 2 | 3 | 4 |
| 32 | curve | 5 | 0 | 2837 | 73 | 10 | 5 | 2 | 0 |
| bass | holes | 1 | 0 | 567 | 15 | 2 | 1 | 1 | 0 |
| bass | curve | 1 | 0 | 568 | 14 | 2 | 1 | 0 | 0 |
| temper | holes | 1 | 0 | 568 | 14 | 3 | 0 | 0 | 0 |
| temper | curve | 1 | 1 | 568 | 14 | 3 | 0 | 0 | 0 |

The tables make five neighbour commits, every one an F1 capture committing to E1. The
curve with four or more points makes none. **Two points close together fit a bad
slope**: two adjacent bass keys, C1 and C♯1, fitted s_B = −0.0141 and lost three treble
captures; two keys four apart, A1 and C♯2, fitted −0.0994 and left two bass captures
without a lock. The fitted slopes on every complete profile read lie between −0.0095
and −0.14, with the default −0.066 inside the range. That is the slope guard: below
twelve covered keys the slope is held at the default and only the intercept is fitted.
Both two-point subsets above now fit the intercept alone.

Two points an octave apart were never drawn in the study. They were scored afterwards
through the shipped path, exploratory and at phase 0 only: for each of the thirteen
pairs of piano #2's bass-bridge keys twelve apart, the regen dump cut to every capture
of the two keys, with the prior and every bass-bridge key beside them:

| points | pass / 595 | bass | treble | wrong | s_B | off the complete fit's slope |
| --- | --- | --- | --- | --- | --- | --- |
| none (the prior) | 568 | 134 | 138 | 14 | -0.066 | — |
| every bass-bridge key | 567 | 135 | 136 | 15 | -0.0614 | — |
| A0–A1 | 567 | 135 | 136 | 15 | -0.0446 | 27 % |
| A#0–A#1 | 568 | 135 | 137 | 14 | -0.0424 | 31 % |
| B0–B1 | 568 | 135 | 137 | 14 | -0.0594 | 3 % |
| C1–C2 | 568 | 135 | 137 | 14 | -0.0570 | 7 % |
| C#1–C#2 | 568 | 135 | 137 | 14 | -0.0572 | 7 % |
| D1–D2 | 567 | 135 | 136 | 15 | -0.0550 | 10 % |
| D#1–D#2 | 567 | 135 | 136 | 15 | -0.0648 | 6 % |
| E1–E2 | 568 | 135 | 137 | 14 | -0.0613 | 0 % |
| F1–F2 | 566 | 135 | 135 | 16 | -0.0690 | 13 % |
| F#1–F#2 | 568 | 135 | 137 | 14 | -0.0895 | 46 % |
| G1–G2 | 568 | 135 | 137 | 14 | -0.0878 | 43 % |
| G#1–G#2 | 568 | 135 | 137 | 14 | -0.0844 | 38 % |
| A1–A2 | 568 | 135 | 137 | 14 | -0.0776 | 27 % |

The guard fits both parameters there, as the unguarded rule did, and the slopes run
from 0 % to 46 % off the complete fit's. The locks do not follow the slope: every pair
keeps the complete fit's bass count, and none differs from the complete fit by more than
one capture in any register, where the treble's σ across the hop grid is 1.53.

### Confirmed on the RWC pianos

Each piano read once, with the rules fixed, at sixteen phases under its own σ:

| rule | PF1 | PF2 | PF3 | on RWC |
| --- | --- | --- | --- | --- |
| **U1, the fit** | CONFIRMED | CONFIRMED | CONFIRMED | **CONFIRMED on RWC** |
| **U2, the blend** | CONFIRMED | CONFIRMED | CONFIRMED | **CONFIRMED on RWC** |

| the bass benefit | PF1 | PF2 | PF3 |
| --- | --- | --- | --- |
| fit · S, mean bass wrong | 1.19 → 1.19 (not met; threshold 0.80) | 5.38 → 4.38 (met; threshold 4.89) | 1.19 → 1.12 (not met; threshold 0.80) |
| blend · S, mean bass wrong | 1.19 → 1.00 (not met; threshold 0.80) | 5.38 → 4.25 (met; threshold 4.89) | 1.19 → 1.12 (not met; threshold 0.80) |
| fitted pair (s_B, y_B) against (−0.066, −7.891) | (-0.0814, -6.670) | (-0.0095, -9.903) | (-0.1397, -5.982) |

The Yamaha, the fit against the prior:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 249.81 | 249.81 | 248.93 | ✓ | 14.19 | 14.19 | 15.07 | ✓ |
| bass (0–26) | 81 | 79.81 | 79.81 | 79.42 | ✓ | 1.19 | 1.19 | 1.58 | ✓ |
| mid (27–59) | 99 | 98.00 | 98.00 | 98.00 | ✓ | 1.00 | 1.00 | 1.00 | ✓ |
| treble (60–87) | 84 | 72.00 | 72.00 | 71.13 | ✓ | 12.00 | 12.00 | 12.87 | ✓ |

The Bösendorfer:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 259 | 236.19 | 237.19 | 235.38 | ✓ | 12.81 | 11.81 | 13.62 | ✓ |
| bass (0–26) | 81 | 75.62 | 76.62 | 75.14 | ✓ | 5.38 | 4.38 | 5.86 | ✓ |
| mid (27–59) | 98 | 91.00 | 91.00 | 91.00 | ✓ | 7.00 | 7.00 | 7.00 | ✓ |
| treble (60–87) | 80 | 69.56 | 69.56 | 69.07 | ✓ | 0.44 | 0.44 | 0.93 | ✓ |

The Steinway:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 241.69 | 241.69 | 240.29 | ✓ | 12.88 | 12.81 | 14.33 | ✓ |
| bass (0–26) | 81 | 79.81 | 79.88 | 79.42 | ✓ | 1.19 | 1.12 | 1.58 | ✓ |
| mid (27–59) | 99 | 92.00 | 92.00 | 92.00 | ✓ | 7.00 | 7.00 | 7.00 | ✓ |
| treble (60–87) | 84 | 69.88 | 69.81 | 68.34 | ✓ | 4.69 | 4.69 | 6.22 | ✓ |

**Confirmed on RWC, for the fit and for the blend.** The bass benefit was askable on
the Bösendorfer alone, where the prior leaves 5.38 of 81 bass captures wrong, and both
sources met it. The Yamaha and the Steinway leave 1.19, which leaves the test nothing
to find. In each piano's mid the baseline commits identically at all sixteen phases,
so its σ there is zero and the candidate could lose nothing in that register; none did.

Under the vote from the onset, the engine as it now ships, the fit against the prior
(descriptive; "against σ" reads each difference against the piano's σ). The Yamaha:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 249.75 | 250.25 | +0.50 | 14.25 | 13.75 | -0.50 | inside σ |
| bass (0–26) | 81 | 79.88 | 80.31 | +0.44 | 1.12 | 0.69 | -0.44 | outside σ, better |
| mid (27–59) | 99 | 98.00 | 98.00 | +0.00 | 1.00 | 1.00 | +0.00 | inside σ |
| treble (60–87) | 84 | 71.88 | 71.94 | +0.06 | 12.12 | 12.06 | -0.06 | inside σ |

The Bösendorfer:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 259 | 241.31 | 241.94 | +0.62 | 10.06 | 9.69 | -0.38 | inside σ |
| bass (0–26) | 81 | 78.31 | 78.69 | +0.38 | 2.69 | 2.31 | -0.38 | inside σ |
| mid (27–59) | 98 | 91.06 | 91.06 | +0.00 | 6.94 | 6.94 | +0.00 | inside σ |
| treble (60–87) | 80 | 71.94 | 72.19 | +0.25 | 0.44 | 0.44 | +0.00 | inside σ |

The Steinway:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 244.25 | 244.12 | -0.12 | 10.38 | 10.44 | +0.06 | inside σ |
| bass (0–26) | 81 | 80.56 | 80.50 | -0.06 | 0.44 | 0.50 | +0.06 | inside σ |
| mid (27–59) | 99 | 93.81 | 93.81 | +0.00 | 5.19 | 5.19 | +0.00 | inside σ |
| treble (60–87) | 84 | 69.88 | 69.81 | -0.06 | 4.75 | 4.75 | +0.00 | inside σ |

### Six instruments' bass against the prior

The instruments read for the question put their bass at very different places against
the prior's default pair. In the research's form (per key, the median of each capture's
B over the prior, then the register's median), piano #2's two reads sit at 4.21 and
4.18 times the prior, the Disklavier's at 2.20 and 1.83, the Iowa Steinway's at 1.77,
and the RWC pianos as below:

| piano | register | keys | q1 | median | q3 | min – max | above the prior |
| --- | --- | --- | --- | --- | --- | --- | --- |
| PF1 | bass | 27 | 1.40 | **1.49** | 2.01 | 1.08 – 2.47 | 27 / 27 |
| PF1 | keys 0-6 | 7 | 2.14 | **2.27** | 2.37 | 1.97 – 2.47 | 7 / 7 |
| PF1 | keys 7-13 | 7 | 1.48 | **1.53** | 1.90 | 1.44 – 2.04 | 7 / 7 |
| PF1 | keys 14-26 | 13 | 1.31 | **1.40** | 1.47 | 1.08 – 1.49 | 13 / 13 |
| PF1 | mid | 33 | 1.05 | **1.07** | 1.10 | 0.89 – 1.32 | 26 / 33 |
| PF1 | treble | 14 | 1.22 | **1.26** | 1.28 | 1.02 – 1.46 | 14 / 14 |
| PF2 | bass | 27 | 0.62 | **0.74** | 1.01 | 0.55 – 1.61 | 7 / 27 |
| PF2 | keys 0-6 | 7 | 0.56 | **0.58** | 0.60 | 0.55 – 0.61 | 0 / 7 |
| PF2 | keys 7-13 | 7 | 0.71 | **0.74** | 0.75 | 0.62 – 0.77 | 0 / 7 |
| PF2 | keys 14-26 | 13 | 0.89 | **1.05** | 1.21 | 0.65 – 1.61 | 7 / 13 |
| PF2 | mid | 33 | 0.88 | **0.95** | 0.99 | 0.80 – 1.11 | 5 / 33 |
| PF2 | treble | 13 | 0.94 | **0.97** | 0.99 | 0.88 – 1.08 | 3 / 13 |
| PF3 | bass | 27 | 0.81 | **0.86** | 0.89 | 0.66 – 1.41 | 5 / 27 |
| PF3 | keys 0-6 | 7 | 0.95 | **1.04** | 1.29 | 0.85 – 1.41 | 5 / 7 |
| PF3 | keys 7-13 | 7 | 0.71 | **0.77** | 0.88 | 0.66 – 0.89 | 0 / 7 |
| PF3 | keys 14-26 | 13 | 0.81 | **0.84** | 0.88 | 0.70 – 0.91 | 0 / 13 |
| PF3 | mid | 33 | 0.98 | **1.02** | 1.04 | 0.91 – 1.13 | 21 / 33 |
| PF3 | treble | 18 | 1.03 | **1.09** | 1.16 | 0.49 – 1.81 | 17 / 18 |

At the bottom of the compass that is 0.58 times the prior (the Bösendorfer, keys 0–6)
to 4.8 (piano #2's A♯0, 4.77 and 4.81 on its two reads): a factor of eight, with the
default inside it. No single default pair sits well in that spread, which is why the
source is a per-instrument fit and not a better constant. The research's fitted pairs,
with the curve over the prior at probe keys, and beside each the pair `instrument_b_fit`
gets from every measured key:

| fit | points | point keys | closest point / closest miss (B / 2T) | s_B | y_B | A0 | C#1 | E1 | A#1 | F#2 | B2 | D#3 | F4 | A5 | C7 | C8 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| curve, piano 2 | 25 | 0–24 | G#2 1.214 / A#2 0.984 | -0.0608 | -6.373 | 4.75 | 4.59 | 4.37 | 3.61 | 2.34 | 1.74 | 1.43 | 1.06 | 1.00 | 1.00 | 1.00 |
| tree fit, piano 2 | 60 | 0–57, 62, 64 | every measured key | -0.0667 | -6.217 | 4.91 | 4.64 | 4.33 | 3.48 | 2.20 | 1.64 | 1.37 | 1.04 | 1.00 | 1.00 | 1.00 |
| curve, extended | 25 | 0–24 | A2 1.210 / A#2 0.988 | -0.0614 | -6.360 | 4.76 | 4.59 | 4.35 | 3.59 | 2.32 | 1.73 | 1.43 | 1.05 | 1.00 | 1.00 | 1.00 |
| tree fit, extended | 62 | 0–61 | every measured key | -0.0681 | -6.185 | 4.92 | 4.62 | 4.30 | 3.43 | 2.16 | 1.61 | 1.35 | 1.04 | 1.00 | 1.00 | 1.00 |
| curve, Iowa (Q table) | 20 | 1–6, 8–21 | E2 1.210 / G2 0.991 | -0.0653 | -6.981 | 2.40 | 2.31 | 2.21 | 1.91 | 1.45 | 1.24 | 1.14 | 1.02 | 1.00 | 1.00 | 1.00 |
| curve, ENSTDkAm (S table) | 12 | 4, 7–8, 10–11, 14–19, 21 | F#2 1.082 / C3 0.658 | -0.0810 | -6.331 | 3.27 | 2.95 | 2.68 | 2.09 | 1.44 | 1.21 | 1.11 | 1.01 | 1.00 | 1.00 | 1.00 |
| tree fit, ENSTDkAm | 29 | 4, 7–8, 10–11, 14–19, 21, 27–30, 32–33, 35–36, 38, 44–45, 47, 51, 55, 57, 59, 62 | every measured key | -0.0917 | -5.977 | 3.71 | 3.20 | 2.81 | 2.08 | 1.37 | 1.16 | 1.07 | 1.00 | 1.00 | 1.00 | 1.00 |
| curve, ENSTDkCl (S table) | 12 | 0, 2–3, 5, 11–12, 14, 16–19, 21 | F#2 1.056 / G#2 0.935 | -0.0874 | -6.149 | 3.43 | 3.01 | 2.68 | 2.03 | 1.38 | 1.17 | 1.08 | 1.00 | 1.00 | 1.00 | 1.00 |
| tree fit, ENSTDkCl | 24 | 0, 2–3, 5, 11–12, 14, 16–19, 21, 23, 27–28, 34, 36–37, 39, 41–42, 53, 57, 59 | every measured key | -0.0880 | -6.127 | 3.46 | 3.03 | 2.69 | 2.04 | 1.38 | 1.17 | 1.08 | 1.00 | 1.00 | 1.00 | 1.00 |

| fit | points | point keys | closest point / closest miss (B / 2T) | s_B | y_B | A0 | C#1 | E1 | A#1 | F#2 | B2 | D#3 | F4 | A5 | C7 | C8 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| curve, PF1 | 21 | 0–20 | E2 1.020 / F#2 0.983 | -0.0814 | -6.670 | 2.34 | 2.12 | 1.95 | 1.60 | 1.22 | 1.10 | 1.05 | 1.00 | 1.00 | 1.00 | 1.00 |
| tree fit, PF1 | 61 | every measured key | — | -0.0807 | -6.683 | 2.34 | 2.13 | 1.96 | 1.61 | 1.23 | 1.10 | 1.05 | 1.00 | 1.00 | 1.00 | 1.00 |
| curve, PF2 | 15 | 0–11, 17–19 | G#1 1.024 / A1 0.977 | -0.0095 | -9.903 | 0.48 | 0.61 | 0.72 | 0.95 | 1.12 | 1.14 | 1.12 | 1.04 | 1.01 | 1.00 | 1.00 |
| tree fit, PF2 | 57 | every measured key | — | -0.0525 | -8.868 | 0.54 | 0.60 | 0.64 | 0.76 | 0.90 | 0.96 | 0.98 | 1.00 | 1.00 | 1.00 | 1.00 |
| curve, PF3 | 11 | 0–4, 6–11 | G#1 1.126 / B1 0.899 | -0.1397 | -5.982 | 1.40 | 1.06 | 0.89 | 0.74 | 0.80 | 0.88 | 0.93 | 0.99 | 1.00 | 1.00 | 1.00 |
| tree fit, PF3 | 60 | every measured key | — | -0.1270 | -6.324 | 1.30 | 1.04 | 0.90 | 0.77 | 0.82 | 0.89 | 0.93 | 0.99 | 1.00 | 1.00 | 1.00 |

The rows marked Q table and S table are Iowa's and the Disklavier's points under the
research's earlier screens, at four partials and at eight.
Slopes run from −0.0095, the Bösendorfer's nearly flat bass, to −0.14, the RWC
Steinway's. The residual of each measured key from its curve, in ln B:

| fit | bass: n, median, min, max | mid | treble |
| --- | --- | --- | --- |
| curve, piano 2 | 27, -0.006, -0.174, +0.208 | 31, -0.008, -0.396, +0.080 | 2, +0.008, -0.071, +0.008 |
| tree fit, piano 2 | 27, +0.002, -0.112, +0.246 | 31, +0.002, -0.350, +0.091 | 2, +0.009, -0.070, +0.009 |
| curve, extended | 27, -0.003, -0.167, +0.214 | 33, -0.002, -0.372, +0.072 | 2, -0.012, -0.037, -0.012 |
| tree fit, extended | 27, +0.000, -0.095, +0.260 | 33, +0.009, -0.319, +0.081 | 2, -0.010, -0.036, -0.010 |
| curve, Iowa (Q table) | 25, +0.000, -0.277, +0.243 | 33, -0.058, -0.199, +0.099 | 12, -0.029, -0.158, +0.182 |
| curve, ENSTDkAm (S table) | 12, +0.008, -0.110, +0.133 | 16, -0.121, -0.252, +0.071 | 1, -0.186, -0.186, -0.186 |
| curve, ENSTDkCl (S table) | 13, +0.007, -0.139, +0.207 | 11, -0.121, -0.164, -0.020 | 0 keys |

| fit | bass | mid | treble |
| --- | --- | --- | --- |
| curve, PF1 | 27, +0.004, -0.190, +0.298 | 33, +0.061, -0.157, +0.200 | 1, +0.182, +0.182, +0.182 |
| curve, PF2 | 27, -0.045, -0.450, +0.418 | 30, -0.075, -0.322, +0.079 | 0 keys |
| curve, PF3 | 26, +0.007, -0.126, +0.128 | 33, +0.040, -0.046, +0.157 | 1, +0.007, +0.007, +0.007 |

The two-asymptote form holds the uprights, the Disklavier and two of the RWC pianos'
bass keys within about 0.3 in ln B; it misses the Bösendorfer's by up to ±0.45, a factor
of 1.5 either way, and the fit was still non-inferior there.

### The admission: eight partials, one capture per key

Report 0009 measured how far repeat captures of one key scatter in ln B, against the
partial count the capture persisted:

| partials n | keys | median σ_lnB | as % of B |
| --- | --- | --- | --- |
| 28–32 | 36 | 0.0036 | 0.4 % |
| 20–27 | 8 | 0.0033 | 0.3 % |
| 14–19 | 5 | 0.0050 | 0.5 % |
| 10–13 | 8 | 0.0128 | 1.3 % |
| 8–9 | 2 | 0.0215 | 2.2 % |
| 6–7 | 8 | 0.0778 | 8.1 % |
| 4–5 | 18 | 0.2393 | 27 % (max 1.44 ≈ ×4) |

Below eight partials a key's repeats disagree about B by as much as moves a lock. The
research's first screen, at four partials, missed piano #2's treble margins by 0.28
passes and 0.6 wrong commits, on three top-octave keys (G7, D♯7, B7) whose repeats
agreed within the screen's factor of two; with the floor at eight, where the scatter
falls to 2.2 %, the treble drops out on its own. Where the floor lands on each read,
under the research's screen:

| read | entries considered | qualifying captures | measured keys | bass | mid | treble | no qualifying capture | n < 2 | ratio > 2 |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| piano 2 | 595 | 440 | 60 | 27 | 31 | 2 | 22 | 6 | 0 |
| the extended store | 470 | 325 | 62 | 27 | 33 | 2 | 23 | 1 | 2 |

| piano | bass | mid | treble |
| --- | --- | --- | --- |
| PF1 | 80 of 80 (median 32 partials) | 96 of 99 (median 17 partials) | 3 of 70 (median 3 partials) |
| PF2 | 80 of 80 (median 32 partials) | 77 of 88 (median 16 partials) | 3 of 65 (median 3 partials) |
| PF3 | 81 of 81 (median 32 partials) | 92 of 97 (median 16 partials) | 5 of 71 (median 4 partials) |

On piano #2 the screen kept every key through F♯5 and two treble keys above it; on the
extended captures every key through A♯5.

**The shipped admission reads one capture per key**: a key's point is its newest trusted
capture with eight or more partials, as every product consumer reads the profile. The
research's points were medians of agreeing repeats: at least two captures of a key with
eight or more partials, all within a factor of two of each other, and their median.
Neither the ratio test nor the median ships. Both pool a key's repeats, and
[`layering.md`](../docs/internals/layering.md) binds the product to a key's newest
trusted capture and keeps pooling with the validation statistics; the template source is
part of the product, so it reads one capture per key, as the curve engines and the
strobe do.

The two admissions differ at their edges and in each point's value. On piano #2 the
shipped rule admits A5 and D♯6, whose newest capture is their only one with eight
partials or more, and not B5, whose newest carries six where four earlier ones qualify.
On the other reads it admits one to six keys the screen did not (*Re-baselining*). Every
such key is off the bass bridge but one: the RWC Steinway's D1, which the ratio test
dropped because one of its three captures sat 618 times off the other two. The lab's
plausibility rule drops that one, and D1's newest capture is one of the two that agree.
On the bass bridge the newest capture sits within 3 % of the research's median at every
point of every read, piano #2's A0 the farthest at 1.025 times.

### The three decisions, and what each displaced

1. **The source is the fit alone.** Every template takes B_ξ̂(m); none is a key's own
   measurement. The treble is therefore the universal asymptote by construction, which
   is where the treble cannot be measured reliably. **Displaced: the precision blend**
   the curve engines already compute, each measured key at w · ln B + (1 − w) · ln B_ξ̂
   with w = σ_p² / (σ_p² + σ_m²), the fit elsewhere. It was confirmed on RWC beside the
   fit and equalled or bettered it on every piano (the tables above), keeps the per-key
   values that pass three Iowa bass captures the fit loses (scored as the per-key table),
   and holds where the two-asymptote form misfits. It was
   not chosen because it reads a per-key value into the templates, exposing a template to
   one capture, and because its weights rest on a modelled σ_m that the repeats
   contradict by up to 23 times in the treble. It is the recorded alternative, revisited
   after two investigations: per-key σ_m measured from repeats, and a bass model with
   more freedom than two asymptotes.
2. **The slope guard is the octave rule.** Points covering at least twelve keys fit both
   parameters; fewer fit the intercept at the default slope; fewer than two points keep
   the default pair. The span is counted in keys covered, lowest to highest inclusive, so
   points eleven apart fit both parameters. Measured instead as the distance between the
   outermost points, the rule would have fitted the RWC Steinway's complete bass bridge,
   A0 to G♯1, by the intercept alone, within 0.3 % of the default pair, not the fit
   confirmed on it; the guard is for few or clustered points, and counted this way it
   changes no complete profile read. **Displaced: penalising the slope toward the
   default**, as the tuning-curve fit penalises its octave type, which needs a weight and
   pulls the two grands whose slopes lie farthest from the default the wrong way at every
   span; **and no guard**, the rule as scored, whose two-point failures are above.
3. **The tuning-curve engines' fit is unchanged.** `instrument_b_fit` fits every measured
   key; the template source fits its bass-bridge points. Both read the same capture per key,
   and two fits of it are accepted for now so that no curve number moves in this change.
   **Displaced: one fit for both**, the engines taking the template source's points, which
   moves every curve number; planned as its own change after this one.

**Also considered.** The fit on every measured key, `instrument_b_fit` as it stands: within
7.5 % of the bass-bridge fit on piano #2's complete profiles, but not identifiable from keys
off the bass bridge (*Method*). A register cap on where measured B applies: blunter than the
partial floor, and a register table, which the project's direction excludes. Raising the
default bass pair: no single constant sits well inside a factor-of-eight spread. Leaving the
prior in the templates: it forgoes the bass benefit wherever an instrument's bass sits off
the prior.

**The contract that shaped them** is `layering.md`'s: a profile holds repeats, the product
reads one, and a statistic reads all. The template source is part of the product, so it
reads each key's newest trusted capture, and pooling repeats stays with the validation
statistics, where the research used it. A research design that needed the contract
amended was the wrong design.

### Re-baselining in the tree

`cargo lab engine lock` before and after the change, and with `--regen` from each read's
regenerated measurements:

| capture set | before the change | after, no regen | failures that changed | with `--regen` | failures that changed |
| --- | --- | --- | --- | --- | --- |
| piano #2 | 568 / 595, 14 wrong (bass 134/137, mid 296/298, treble 138/160) | 568 / 595, 14 wrong (bass 134/137, mid 296/298, treble 138/160) | none | 567 / 595, 15 wrong (bass 135/137, mid 296/298, treble 136/160) | `key_003_C1_1783694464`: wrong, G2 → pass; `key_076_C#7_1783697222`: pass → wrong, A#0; `key_086_B7_1783697415`: pass → wrong, A6 |
| piano #2, extended | 449 / 472, 7 wrong (bass 142/144, mid 164/164, treble 143/164) | 449 / 472, 7 wrong (bass 142/144, mid 164/164, treble 143/164) | none | 450 / 472, 5 wrong (bass 144/144, mid 164/164, treble 142/164) | `key_011_G#1_1787072516`: wrong, G#2 → pass; `key_011_G#1_1787072520`: wrong, G#2 → pass; `key_083_G#7_1787074396`: wrong, G7 → no lock; `key_084_A7_1787288143`: pass → wrong, A0 |
| piano #1 | 82 / 87, 5 wrong (bass 22/26, mid 33/33, treble 27/28) | 82 / 87, 5 wrong (bass 22/26, mid 33/33, treble 27/28) | none | — (no regen dump) | — |
| RWC Yamaha | — | 251 / 264, 13 wrong (bass 80/81, mid 98/99, treble 73/84) | — | 251 / 264, 13 wrong (bass 80/81, mid 98/99, treble 73/84) | none |
| RWC Bösendorfer | — | 240 / 264, 10 wrong (bass 78/81, mid 91/99, treble 71/84) | — | 240 / 264, 10 wrong (bass 78/81, mid 91/99, treble 71/84) | none |
| RWC Steinway | — | 245 / 264, 9 wrong (bass 81/81, mid 94/99, treble 70/84) | — | 245 / 264, 11 wrong (bass 81/81, mid 94/99, treble 70/84) | `key_065_D6_2090`: no lock → wrong, C#1; `key_086_B7_2287`: no lock → wrong, C1 |

Without `--regen` nothing moves: the prior is still the template when nothing is
measured. With it, the shipped path against the research's phase-0 cell under the vote
from the onset:

| read | `engine lock --regen` | the replay, phase 0 | captures that differ |
| --- | --- | --- | --- |
| piano #2 | 567/595, wrong 15, no-lock 13 | 568/595, wrong 14, no-lock 13 | 1: `key_076_C#7_1783697222` pass → wrong, A#0 |
| piano #2, extended | 450/472, wrong 5, no-lock 17 | 450/472, wrong 5, no-lock 17 | 0 |
| RWC Yamaha | 251/264, wrong 13, no-lock 0 | 251/264, wrong 13, no-lock 0 | 0 |
| RWC Bösendorfer | 240/264, wrong 10, no-lock 14 | 240/264, wrong 10, no-lock 14 | 0 |
| RWC Steinway | 245/264, wrong 11, no-lock 8 | 245/264, wrong 9, no-lock 10 | 2: `key_065_D6_2090` no lock → wrong, C#1; `key_086_B7_2287` no lock → wrong, C1 |

The shipped points against the research's, key for key. The farthest fit point is the
bass-bridge key whose newest capture sits farthest from the research's median, as their
ratio; the curves' largest difference is between the two pairs' B_ξ̂ over the 88 keys:

| read | keys admitted: shipped (research) | fit points: shipped (research), keys covered | pair shipped | pair research | fit point farthest from the research's | the curves' largest difference | admitted by one rule only |
| --- | --- | --- | --- | --- | --- | --- | --- |
| piano #2 | 61 (60) | 25 (25), 25 | (-0.0614, -6.359) | (-0.0608, -6.373) | A0, ×1.025 | 0.6 %, at G#2 | shipped: A5, D#6; research: B5 |
| piano #2, extended | 63 (62) | 25 (25), 25 | (-0.0612, -6.365) | (-0.0614, -6.360) | C#2, ×1.019 | 0.3 %, at F#2 | shipped: A6 |
| RWC Yamaha | 62 (61) | 21 (21), 21 | (-0.0813, -6.672) | (-0.0814, -6.670) | B0, ×0.989 | 0.1 %, at A0 | shipped: A#5 |
| RWC Bösendorfer | 63 (57) | 15 (15), 20 | (-0.0089, -9.922) | (-0.0095, -9.903) | A#0, ×0.995 | 0.5 %, at A0 | shipped: F#5, G5, G#5, A5, A#5, B5 |
| RWC Steinway | 64 (60) | 12 (11), 12 | (-0.1378, -6.050) | (-0.1397, -5.982) | E1, ×1.004 | 2.7 %, at A0 | shipped: D1, A#5, B5, C6 |

The fit's points are the research's keys on every read except for the Steinway's added
D1 (*The admission*); every other key one rule admits and the other does not is off the
bass bridge. Otherwise what moves the pairs is each point's value, the newest capture in
place of the median: the two curves stay within 0.6 % of each other at every key on four
reads, and within 2.7 % on the Steinway, at A0, where D1 enters. The pairs are close to
the scored ones and not equal to them. At phase 0 the locks are the research's capture for capture
on the extended captures, the Yamaha and the Bösendorfer. On piano #2 one capture moves:
`key_076_C#7_1783697222` commits to A♯0, as it did under the research's curve on six of
sixteen phases. On the Steinway two treble no-locks, D6 and B7, commit instead to C♯1
and C1.

The shipped source was then scored at sixteen phases through the tree's own engine under
the vote from the onset, on piano #2, its extended captures and the Steinway, each against
the prior with the read's own σ. This is exploratory: it was scored after the decision and
the confirmation, on pianos already read, and it decides nothing.

| read | cell | all: pass / wrong | bass | mid | treble |
| --- | --- | --- | --- | --- | --- |
| piano #2 | prior · O, the tree without `--regen` | 566.94 / 16.31 | 133.56 / 3.44 | 296.19 / 1.81 | 137.19 / 11.06 |
|  | the shipped source · O, the tree with `--regen` | 566.88 / 16.44 | 134.56 / 2.44 | 296.25 / 1.75 | 136.06 / 12.25 |
|  | the research's fitted curve · O, its replay | 567.62 / 15.75 | 134.56 / 2.44 | 296.31 / 1.69 | 136.75 / 11.62 |
| | σ, report 0020's yardstick | 2.04 / 1.95 | 0.70 | 0.46 | 1.53 |
| piano #2, extended | prior · O, the tree without `--regen` | 449.75 / 7.44 | 142.25 / 1.75 | 163.94 / 0.06 | 143.56 / 5.62 |
|  | the shipped source · O, the tree with `--regen` | 450.94 / 6.00 | 144.00 / 0.00 | 163.94 / 0.06 | 143.00 / 5.94 |
|  | the research's fitted curve · O, its replay | 450.94 / 6.00 | 144.00 / 0.00 | 163.94 / 0.06 | 143.00 / 5.94 |
| | σ, report 0020's yardstick | 1.36 / 1.14 | 0.48 | 0.24 | 1.32 |
| RWC Steinway | prior · O, the tree without `--regen` | 244.25 / 10.38 | 80.56 / 0.44 | 93.81 / 5.19 | 69.88 / 4.75 |
|  | the shipped source · O, the tree with `--regen` | 244.12 / 10.56 | 80.50 / 0.50 | 93.81 / 5.19 | 69.81 / 4.88 |
|  | the research's fitted curve · O, its replay | 244.12 / 10.44 | 80.50 / 0.50 | 93.81 / 5.19 | 69.81 / 4.75 |
| | σ, its own, from prior · S | 1.40 / 1.45 | 0.39 | 0.00 | 1.54 |

- piano #2: the shipped source differs from the research's cell on 13 of 595 × 16
  capture-phases; the prior run equals the research's prior cell on every one.
- piano #2, extended: the shipped source differs from the research's cell on 0 of 472 × 16
  capture-phases; the prior run equals the research's prior cell on every one.
- RWC Steinway: the shipped source differs from the research's cell on 2 of 264 × 16
  capture-phases; the prior run equals the research's prior cell on every one.

Every register of every read is inside σ of the prior. On the extended captures the
shipped source is the research's cell at every capture and phase; on the Steinway it
differs from the confirmed pair only in the two phase-0 commits above. On piano #2 it keeps
the bass benefit and pays more in the treble than the scored curve did: three top-octave
captures commit to A♯0 on 17 capture-phases between them, `key_076_C#7_1783697222` on
nine, `key_078_D#7_1783697264` on six and `key_085_A#7_1783697399` on two, where under the
research's curve the C♯7 capture alone did, on six. The treble's mean pass sits 1.125 below
the prior's, against a σ of 1.53. A♯0's template B differs between the two curves by
0.22 %; the A0 attractor (*Exploratory*) decides these captures on the hop grid under any
measured bass B, and a difference that small moves it.

The octave rule changes no complete profile read: every read's points cover at least
twelve keys (piano #2 and its extended captures 25, the Yamaha 21, the Bösendorfer 20,
the Steinway 12; on the research's points, the Iowa Steinway and the Disklavier 18 to 22).

### Cost

| what | where it runs | median |
| --- | --- | --- |
| discovery, refined, one frame (unchanged) | the audio thread, every hop | 22.937 µs |
| the templates' fit, a complete synthetic profile | the Worker, once per curve bundle | 2.0380 ms |
| 88 `KeyProfile::new`, one refresh | the frontend, once per curve bundle | 8.9202 µs |
| the whole curve bundle, piano #2's profile | the Worker | 1.150 s |
| of it, engine (c) alone | the Worker | 1.078 s |
| of it, the templates' fit | the Worker | 1.858 ms |

The criterion benches started at a load average of 0.92, the bundle's timing at 0.94.

Nothing new runs on the hot path. The template channel already swapped each update into
the live table at the top of a hop; a refresh is now all 88 at once, committed to the
ring together, so the hop that drains it moves 88 templates in place and discovery's
per-frame cost does not change. The fit adds about 2 ms to a curve bundle that takes
about a second, and the 88 templates cost the frontend under 10 µs per bundle.

## Exploratory

**Iowa, beside the decision** (one phase, the research's four-partial table): the fitted
curve helps the Steinway B's bass less than its own per-key values. Three bass captures
that pass under the table commit an octave high under the curve, where the fundamental is
weak and a 25 % residual of the two-asymptote model is enough to tip the octave rival.

| cell | overall | bass | mid | treble |
| --- | --- | --- | --- | --- |
| prior · S | 213/15/32 | 56/0/22 | 96/0/3 | 61/15/7 |
| holes · S | 218/15/27 | 61/0/17 | 96/0/3 | 61/15/7 |
| curve · S | 215/15/30 | 58/0/20 | 96/0/3 | 61/15/7 |

- **The bass at a distance and the mid on grands.** What remains in the bass commits an
  octave high where the fundamental is weak, and no B table moves the octave-high commits
  in the mid of two RWC grands. That is the estimator on a weak fundamental, not the
  template.
- **The A0 attractor.** With the instrument's bass B in the templates, A0 and A♯0 carry B
  near five times the prior on piano #2, and one top-octave capture per read commits to
  them on six to nine of sixteen phases, under the curve and the per-key table alike,
  inside the grid's σ. Which captures it takes is decided on the hop grid and is fragile:
  under the shipped source, whose A♯0 template differs from the research curve's by
  0.22 %, three piano-#2 captures commit to A♯0 in place of one (*Re-baselining*).
- **Latency.** Under the curve the bass commits earlier on both piano-#2 reads and the mid
  and the treble slightly later, by less than a hop in mean; under the fit on the RWC
  Steinway fifteen captures commit later and two earlier, with a median difference of
  zero. Latency enters no rule.

## Limitations

- **The treble is the prior by construction.** The curve's treble is the universal
  asymptote, and above the bass bridge the templates follow the instrument only through
  its bass pair.
- **Top-octave captures drift to A0 or A♯0** under any measured bass B, inside the
  scatter: one per read under the research's curve, three on piano #2 under the shipped
  source.
- **The two-asymptote form misfits the Bösendorfer's bass** by up to ±0.45 in ln B. The fit
  was non-inferior there; the blend, which keeps the measured values, is the recorded
  alternative.
- **The mid had no margin on any RWC piano**: its baseline committed identically at every
  phase, so each piano's mid σ was zero. The candidate lost nothing there.
- **The confirmations were scored under the vote on `Stable` hops.** The engine now votes
  from the onset; the O cells beside them are descriptive.
- **The slope guard entered after the scored rules**, from the partial-profile study, and
  was scored nowhere; on complete profiles it binds on none of the reads.
- **The shipped points are not the research's.** Each is a key's newest trusted capture
  with eight or more partials, where the research's were medians of agreeing repeats, so no
  shipped pair is a scored one: the curves are within 0.6 % of the scored ones on four
  reads and within 2.7 % on the RWC Steinway. The shipped source was scored at sixteen
  phases only after the fact, on piano #2, its extended captures and the Steinway; on the
  Yamaha and the Bösendorfer at phase 0 alone.
- **In a session the profile starts empty.** Until two bass keys are measured the templates
  are the prior; a session that measures the temperament octave first keeps them there, which
  the partial-profile draws measured as the baseline.
- **Both of the project's pianos are out of tune**, the regime the app targets, and a lock
  score is relative to its instrument
  ([`capture-sets.md`](../docs/internals/capture-sets.md)).

## Reproduce

- `cargo lab engine lock diagnostics_piano2` gives 568 / 595, `diagnostics_piano_1` 82 / 87,
  the extended store's 472 open and undeclared captures 449 / 472, unchanged by this change.
  With `--regen` and each read's `cargo lab mat regen` dump, piano #2 gives 567 / 595 with
  15 wrong and the extended captures 450 / 472 with 5 wrong, and each prints the fitted pair
  on a `[TEMPLATES]` line.
- `cargo test -p tuner-core --lib -- template slope_guard bass_bridge no_hole newest_trusted`:
  the fit reproduces the recorded pairs bit for bit on their points, below two bass-bridge
  keys every template is the prior, a key is a point from twice the treble term up, one
  measured bass key leaves no hole, the slope guard
  counts covered keys, the fit reads each key's newest trusted capture and not an older
  one, an auto-mode capture or a solo, and a whole refresh crosses the ring in one piece.
- `cargo bench -p tuner-core --bench discovery_cost`.
- The sixteen-phase replays, the partial-profile draws and the external reads were made with
  research tooling kept outside the repository; every table above is copied from its output
  files.

## References

- Rigaud, F., David, B. & Daudet, L. (2013). A parametric model and estimation techniques for
  the inharmonicity and tuning of the piano. *JASA* 133(5), 3107–3118.
  doi:10.1121/1.4799806 — Eqs. 7–8 (the whole-compass model), Eq. 29 (the bass pair's fit),
  §III.B.1 (the universal treble pair).
- Emiya, V., Badeau, R. & David, B. (2010). Multipitch estimation of piano sounds using a new
  probabilistic spectral smoothness principle. *IEEE Transactions on Audio, Speech, and
  Language Processing* 18(6), 1643–1654. doi:10.1109/TASL.2009.2038819 — the citation MAPS
  asks for.
- Goto, M., Hashiguchi, H., Nishimura, T. & Oka, R. (2003). RWC Music Database: Music Genre
  Database and Musical Instrument Sound Database. *Proc. ISMIR 2003*, 229–230.
- Goto, M., Balke, S. & Müller, M. (2026). RWC Instruments Database. Zenodo.
  doi:10.5281/zenodo.17170844.
- University of Iowa Electronic Music Studios. Musical Instrument Samples: piano (recorded
  2001). <https://theremin.music.uiowa.edu/MISpiano.html>
