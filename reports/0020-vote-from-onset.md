# report 0020 — the acquisition lock, voting from the onset

The M-of-N acquisition lock ([report 0010](0010-m-of-n-lock-rule-replay.md))
recorded a discovery winner only on hops the Gatekeeper called `Stable`:
`Engine::process`, unlocked and in auto mode, returned early on every other hop
and left the vote window as it was. This report measures the change that removes
that return and nothing else, and it is the record the change ships on. An
`Unstable` hop after an onset now runs the same peak extraction, masking and
discovery scan a `Stable` hop runs, and its winner enters the same window.

What it leaves alone: the Gatekeeper, with its states, onset flag, thresholds and
four-hop stable run; the rule, 7 of the last 8; the hops that clear the window,
`Silence` and the onset's bypass; manual mode, which never took the return; and
the capture path, which still starts its measured record on the first `Stable`
hop. The `is_stable` argument the return read had no other reader, and
`Engine::process` no longer takes it.

**Every commit measured still lands on a `Stable` frame**: 16 647 of 16 647 over
sixteen phases of the hop grid on both reads of piano #2, and 245 of 245 on the
Iowa Steinway. Seven votes cannot be collected in the four hops between the
gate's last onset hop and its first `Stable` verdict, so the window reaches
`Stable` already partly full and the commit lands a median 4 hops after that
verdict instead of 6. On piano #2 that is a median 42 ms sooner per capture on
one read and 48 ms on the other, and no capture commits later.

The figures quoted as this tree's are from `4c53fc0` with this change and without
the extraction-cap change: the picker still keeps the first 128 maxima in bin
order. The two changes together are measured below (*With the extraction-cap
change*), and on piano #2 they do not meet this report's rule.

## Pre-registered decision rule

Fixed before the replay was scored, and applied as written. Nothing was tuned:
the candidate is the shipped engine with one early return removed.

**The null is the hop grid.** Where a hammer strike lands inside a 1 024-sample
hop is an accident, and every lock figure is one draw of it. The shipped rule
replayed at sixteen phases of the grid, with nothing else changed, ranges over
564–571 passes on piano #2 (*The yardstick*, below). A difference inside that
scatter is not a difference, so the margin is the baseline's own across-phase
population σ.

**Population and statistic.** Every capture of a read: 595 on piano #2, 472 on
its extended captures. Per capture, over the sixteen phases, the mean of pass
(0 or 1) and the mean of wrong; a cell's count is the sum over captures, which
equals the mean over phases of that phase's count.

**Non-inferior** on a read: the candidate's mean pass count is at least the
baseline's minus σ, and its mean wrong count at most the baseline's plus σ,
overall and in each of bass (keys 0–26), mid (27–59) and treble (60–87). A
register's wrong count uses that register's pass σ.

| read | overall pass / wrong | bass | mid | treble |
| --- | --- | --- | --- | --- |
| piano #2 | 2.04 / 1.95 | 0.70 | 0.46 | 1.53 |
| extended | 1.36 / 1.14 | 0.48 | 0.24 | 1.32 |

**Latency.** Per capture, the median commit time over the phases both rules
pass; the exact two-sided sign test on candidate minus baseline, ties dropped,
α = 0.05. It counts for the candidate when it rejects with the candidate lower.

**Outcomes.** ADVANCE: non-inferior on piano #2 with the latency test counting
for the candidate there, and then, scored only after the piano-#2 results were
frozen, the same on the extended captures. NO MEASURED BENEFIT: non-inferior on
both, the latency test not counting on one. STOP: not non-inferior on either.

The confirmation on the MAPS Disklavier was fixed the same way before that
instrument was read: the same test under the Disklavier's own σ, computed from
its baseline alone and frozen before any candidate cell existed. The ambient read
decides, the close read confirms, and the rule is confirmed on the instrument
only if both reads confirm it. The Iowa and RWC reads decide nothing: one read of
one instrument is exploratory ([`CONTRIBUTING.md`](../CONTRIBUTING.md)).

## Population

Every set is in the app's capture format: mono `f32` at 44.1 kHz holding the
note's whole event, which the gate and the engine read as they read the
microphone. Piano #2's two sets are the project's own and are described in
[`capture-sets.md`](../docs/internals/capture-sets.md). The external sets were
converted from their published audio for this measurement: channel 0 at unit
gain, each note cut to 64 hops with its onset at hop 15, and one silence
threshold per set, derived from the set's own audio.

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

The RWC set names each piano's maker and not its model. Piano #1 — an upright, 87
captures, one per key, F♯2 absent — enters only the re-baselining.

What the population cannot support: piano #2's two sets are one instrument, and
every external instrument was read once, from a published recording rather than
the app's microphone and room. RWC's notes are edited into silences of digital
zeros, and 115 of the Iowa notes needed zeros prepended to put their onset at
hop 15.

## Method

**The replay.** Each capture's audio was cut at sixteen phases of the hop grid —
k = 0, 64, …, 960 samples removed from its head — and each phase dumped with the
shipped front end, which writes every hop's discovery winner and the gate's
verdict (`cargo lab engine dump --refine`, `cargo lab gatekeeper dump`). The
engine's own state machine then ran over the dumps under each rule:

| hop | shipped rule | vote from the onset | in `Engine::process` |
| --- | --- | --- | --- |
| `Silence` | clear the window, unlock | the same | resets and returns |
| the onset's bypass | clear the window, unlock | the same | resets and returns |
| `Stable`, unlocked | vote | vote | records the winner |
| `Unstable`, unlocked | no vote, window untouched | vote | the removed early return |

A key commits on the first hop where it holds 7 of the last 8 votes. A capture's
outcome is its first commit against the key it was captured as: pass, wrong, or
no lock.

**The integration check.** The replay is a model of the engine, so the engine was
built both ways and run through `cargo lab engine lock` at phase 0. The unchanged
binary reproduced the shipped rule's replay and the binary with the return
removed reproduced the candidate's, capture for capture and committed key for
committed key, on both reads: 568 / 595 with the same 27 failures under both
rules on piano #2, 447 / 472 and 449 / 472 on the extended captures. This tree
gives the same figures (*Re-baselining*, below).

**The clock.** Latency is the commit's position on the original recording's
timeline minus the onset's, the same for every phase, in milliseconds; on Iowa,
in hops from the onset hop.

**The tables** below are copied from the runs' own output files and keep their
labels. `S` is the shipped rule and `O` the vote from the onset; `base` and
`prior` are the shipped engine otherwise, and `patched` is the extraction-cap
change. The baseline is `base · S` (or `prior · S`), the candidate `base · O`.

## Results

### Piano #2, which decides

Pass / no-lock / wrong at each phase:

| phase k (samples) | shipped rule: pass / no-lock / wrong | vote from the onset |
| --- | --- | --- |
| 0 | 568 / 13 / 14 | 568 / 13 / 14 |
| 64 | 570 / 12 / 13 | 570 / 12 / 13 |
| 128 | 568 / 13 / 14 | 568 / 13 / 14 |
| 192 | 569 / 12 / 14 | 569 / 12 / 14 |
| 256 | 565 / 14 / 16 | 565 / 14 / 16 |
| 320 | 564 / 14 / 17 | 564 / 14 / 17 |
| 384 | 565 / 12 / 18 | 565 / 12 / 18 |
| 448 | 564 / 13 / 18 | 564 / 13 / 18 |
| 512 | 566 / 11 / 18 | 565 / 11 / 19 |
| 576 | 566 / 11 / 18 | 565 / 11 / 19 |
| 640 | 567 / 9 / 19 | 566 / 9 / 20 |
| 704 | 569 / 8 / 18 | 569 / 8 / 18 |
| 768 | 567 / 11 / 17 | 566 / 11 / 18 |
| 832 | 567 / 14 / 14 | 567 / 14 / 14 |
| 896 | 569 / 11 / 15 | 569 / 11 / 15 |
| 960 | 571 / 10 / 14 | 571 / 10 / 14 |

Over the sixteen phases, the shipped rule:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 595 | 564–571 (567.19 ± 2.04) | 8–14 (11.75 ± 1.71) | 13–19 (16.06 ± 1.95) |
| bass (0–26) | 137 | 133–135 (133.56 ± 0.70) | 0–0 (0.00 ± 0.00) | 2–4 (3.44 ± 0.70) |
| mid (27–59) | 298 | 296–297 (296.31 ± 0.46) | 0–0 (0.00 ± 0.00) | 1–2 (1.69 ± 0.46) |
| treble (60–87) | 160 | 135–140 (137.31 ± 1.53) | 8–14 (11.75 ± 1.71) | 8–14 (10.94 ± 1.64) |
| treble low (60–74) | 85 | 85–85 (85.00 ± 0.00) | 0–0 (0.00 ± 0.00) | 0–0 (0.00 ± 0.00) |
| treble high (75–87) | 75 | 50–55 (52.31 ± 1.53) | 8–14 (11.75 ± 1.71) | 8–14 (10.94 ± 1.64) |

The vote from the onset:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 595 | 564–571 (566.94 ± 2.16) | 8–14 (11.75 ± 1.71) | 13–20 (16.31 ± 2.23) |
| bass (0–26) | 137 | 133–135 (133.56 ± 0.70) | 0–0 (0.00 ± 0.00) | 2–4 (3.44 ± 0.70) |
| mid (27–59) | 298 | 295–297 (296.19 ± 0.53) | 0–0 (0.00 ± 0.00) | 1–3 (1.81 ± 0.53) |
| treble (60–87) | 160 | 135–140 (137.19 ± 1.59) | 8–14 (11.75 ± 1.71) | 8–14 (11.06 ± 1.75) |
| treble low (60–74) | 85 | 84–85 (84.88 ± 0.33) | 0–0 (0.00 ± 0.00) | 0–1 (0.12 ± 0.33) |
| treble high (75–87) | 75 | 50–55 (52.31 ± 1.53) | 8–14 (11.75 ± 1.71) | 8–14 (10.94 ± 1.64) |

The margins, as applied:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 595 | 567.19 | 566.94 | 565.15 | ✓ | 16.06 | 16.31 | 18.01 | ✓ |
| bass (0–26) | 137 | 133.56 | 133.56 | 132.86 | ✓ | 3.44 | 3.44 | 4.14 | ✓ |
| mid (27–59) | 298 | 296.31 | 296.19 | 295.85 | ✓ | 1.69 | 1.81 | 2.15 | ✓ |
| treble (60–87) | 160 | 137.31 | 137.19 | 135.78 | ✓ | 10.94 | 11.06 | 12.47 | ✓ |

**Non-inferior overall and in every register.** The largest shortfall is a
quarter of a capture overall, against a margin of 2.04, and no capture's
sixteen-phase pass rate moved by more than 0.125.

Latency, per capture over the phases both rules pass:

| group | captures | candidate lower | higher | tied | median Δ | p |
| --- | --- | --- | --- | --- | --- | --- |
| all | 576 | **414** | **0** | 162 | **−42.09 ms** | 4.73 × 10⁻¹²⁵ |
| bass (0–26) | 135 | 119 | 0 | 16 | −60.95 ms | 3.01 × 10⁻³⁶ |
| mid (27–59) | 297 | 249 | 0 | 48 | −50.79 ms | 2.21 × 10⁻⁷⁵ |
| treble (60–87) | 144 | 46 | 0 | 98 | 0.00 ms | 2.84 × 10⁻¹⁴ |

No capture commits later. The treble's median is zero because two thirds of its
captures do not move: the gate reaches `Stable` there before the window could
have filled anyway. The latency test counts for the candidate, and piano #2 meets
the first half of ADVANCE.

### The extended captures, scored after the freeze

The same upright in its as-found state: a second read of one instrument, not a
second instrument.

Pass / no-lock / wrong at each phase:

| phase k (samples) | shipped rule: pass / no-lock / wrong | vote from the onset |
| --- | --- | --- |
| 0 | 447 / 17 / 8 | 449 / 16 / 7 |
| 64 | 447 / 18 / 7 | 449 / 17 / 6 |
| 128 | 446 / 17 / 9 | 449 / 15 / 8 |
| 192 | 446 / 17 / 9 | 450 / 14 / 8 |
| 256 | 450 / 13 / 9 | 452 / 12 / 8 |
| 320 | 446 / 18 / 8 | 450 / 16 / 6 |
| 384 | 447 / 16 / 9 | 450 / 15 / 7 |
| 448 | 446 / 16 / 10 | 451 / 14 / 7 |
| 512 | 446 / 15 / 11 | 451 / 13 / 8 |
| 576 | 446 / 19 / 7 | 450 / 17 / 5 |
| 640 | 447 / 16 / 9 | 451 / 13 / 8 |
| 704 | 445 / 16 / 11 | 449 / 14 / 9 |
| 768 | 444 / 18 / 10 | 447 / 16 / 9 |
| 832 | 445 / 18 / 9 | 449 / 15 / 8 |
| 896 | 444 / 18 / 10 | 449 / 15 / 8 |
| 960 | 446 / 17 / 9 | 450 / 15 / 7 |

Over the sixteen phases, the shipped rule:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 472 | 444–450 (446.12 ± 1.36) | 13–19 (16.81 ± 1.42) | 7–11 (9.06 ± 1.14) |
| bass (0–26) | 144 | 140–141 (140.62 ± 0.48) | 0–0 (0.00 ± 0.00) | 3–4 (3.38 ± 0.48) |
| mid (27–59) | 164 | 163–164 (163.94 ± 0.24) | 0–0 (0.00 ± 0.00) | 0–1 (0.06 ± 0.24) |
| treble (60–87) | 164 | 139–145 (141.56 ± 1.32) | 13–19 (16.81 ± 1.42) | 3–7 (5.62 ± 1.05) |
| treble low (60–74) | 98 | 98–98 (98.00 ± 0.00) | 0–0 (0.00 ± 0.00) | 0–0 (0.00 ± 0.00) |
| treble high (75–87) | 66 | 41–47 (43.56 ± 1.32) | 13–19 (16.81 ± 1.42) | 3–7 (5.62 ± 1.05) |

The vote from the onset:

| group | n | pass min–max (mean ± σ) | no-lock min–max (mean ± σ) | wrong min–max (mean ± σ) |
| --- | --- | --- | --- | --- |
| all | 472 | 447–452 (449.75 ± 1.15) | 12–17 (14.81 ± 1.38) | 5–9 (7.44 ± 1.06) |
| bass (0–26) | 144 | 141–143 (142.25 ± 0.56) | 0–0 (0.00 ± 0.00) | 1–3 (1.75 ± 0.56) |
| mid (27–59) | 164 | 163–164 (163.94 ± 0.24) | 0–0 (0.00 ± 0.00) | 0–1 (0.06 ± 0.24) |
| treble (60–87) | 164 | 142–146 (143.56 ± 1.00) | 12–17 (14.81 ± 1.38) | 3–7 (5.62 ± 1.05) |
| treble low (60–74) | 98 | 98–98 (98.00 ± 0.00) | 0–0 (0.00 ± 0.00) | 0–0 (0.00 ± 0.00) |
| treble high (75–87) | 66 | 44–48 (45.56 ± 1.00) | 12–17 (14.81 ± 1.38) | 3–7 (5.62 ± 1.05) |

The margins, as applied:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 472 | 446.12 | 449.75 | 444.76 | ✓ | 9.06 | 7.44 | 10.20 | ✓ |
| bass (0–26) | 144 | 140.62 | 142.25 | 140.15 | ✓ | 3.38 | 1.75 | 3.85 | ✓ |
| mid (27–59) | 164 | 163.94 | 163.94 | 163.70 | ✓ | 0.06 | 0.06 | 0.30 | ✓ |
| treble (60–87) | 164 | 141.56 | 143.56 | 140.24 | ✓ | 5.62 | 5.62 | 6.95 | ✓ |

Latency:

| group | captures | candidate lower | higher | tied | median Δ | p |
| --- | --- | --- | --- | --- | --- | --- |
| all | 451 | **333** | **0** | 118 | **−47.89 ms** | 1.14 × 10⁻¹⁰⁰ |
| bass (0–26) | 141 | 121 | 0 | 20 | −47.89 ms | 7.52 × 10⁻³⁷ |
| mid (27–59) | 164 | 144 | 0 | 20 | −68.21 ms | 8.97 × 10⁻⁴⁴ |
| treble (60–87) | 146 | 68 | 0 | 78 | 0.00 ms | 6.78 × 10⁻²¹ |

**Non-inferior in every group, and above the baseline in three of the four**:
3.63 more passes and 1.62 fewer wrong commits per phase, the bass gaining 1.63
passes. Five captures' sixteen-phase pass rates moved by more than a quarter, all
toward the candidate: two of G♯1 — `key_011_G#1_1787072524`, wrong at every phase
under the shipped rule and right at fifteen of sixteen under the candidate, and
`key_011_G#1_1787072516`, +0.312 — and three of A7, +0.625, +0.625 and +0.500.
No capture moved against the candidate by more than a quarter on either read.

### The decision: ADVANCE

Both halves hold: non-inferior on piano #2 with the latency test counting (414
captures earlier, 0 later, p = 4.7 × 10⁻¹²⁵), then non-inferior on the extended
captures with it counting there too (333 earlier, 0 later, p = 1.1 × 10⁻¹⁰⁰).

### Confirmed on the MAPS Disklavier

Read once, at two distances, with the rules fixed and the Disklavier's σ taken
from its own baseline. At 3–4 m, 179 captures, the margins and then the latency:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 179 | 149.56 | 151.50 | 148.39 | ✓ | 27.56 | 25.62 | 28.50 | ✓ |
| bass (0–26) | 65 | 38.00 | 39.94 | 37.29 | ✓ | 27.00 | 25.06 | 27.71 | ✓ |
| mid (27–59) | 61 | 61.00 | 61.00 | 61.00 | ✓ | 0.00 | 0.00 | 0.00 | ✓ |
| treble (60–87) | 53 | 50.56 | 50.56 | 49.86 | ✓ | 0.56 | 0.56 | 1.27 | ✓ |

| group | captures with a shared pass | lower | higher | tied | median Δ ms | p (two-sided sign) |
| --- | --- | --- | --- | --- | --- | --- |
| all | 154 | 96 | 0 | 58 | -31.2 | 2.52e-29 |
| bass (0–26) | 41 | 33 | 0 | 8 | -48.6 | 2.33e-10 |
| mid (27–59) | 61 | 53 | 0 | 8 | -49.3 | 2.22e-16 |
| treble (60–87) | 52 | 10 | 0 | 42 | 0.0 | 0.00195 |

At about 50 cm, 257 captures, the same:

| group | n | baseline pass | candidate pass | floor (base − σ) | pass | baseline wrong | candidate wrong | ceiling (base + σ) | wrong |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 257 | 237.00 | 237.88 | 236.06 | ✓ | 10.38 | 9.50 | 11.23 | ✓ |
| bass (0–26) | 79 | 70.06 | 70.94 | 69.51 | ✓ | 8.94 | 8.06 | 9.49 | ✓ |
| mid (27–59) | 98 | 98.00 | 98.00 | 98.00 | ✓ | 0.00 | 0.00 | 0.00 | ✓ |
| treble (60–87) | 80 | 68.94 | 68.94 | 68.28 | ✓ | 1.44 | 1.44 | 2.10 | ✓ |

| group | captures with a shared pass | lower | higher | tied | median Δ ms | p (two-sided sign) |
| --- | --- | --- | --- | --- | --- | --- |
| all | 240 | 190 | 0 | 50 | -53.0 | 1.27e-57 |
| bass (0–26) | 72 | 65 | 0 | 7 | -59.9 | 5.42e-20 |
| mid (27–59) | 98 | 92 | 0 | 6 | -64.6 | 4.04e-28 |
| treble (60–87) | 70 | 33 | 0 | 37 | 0.0 | 2.33e-10 |

**Confirmed on both reads, so confirmed on the instrument**: non-inferior in every
register on each and faster on each — 96 captures earlier and 58 tied at 3–4 m,
median −31 ms; 190 earlier and 50 tied at 50 cm, median −53 ms — with no capture
later on either. The mid's margin is zero on both reads, because the baseline
commits the mid identically at every phase; the candidate lost nothing there.

### Where the commits land

Piano #2:

| cell | commits | on a `Stable` frame | median hops after the gate's first `Stable` | min | max |
| --- | --- | --- | --- | --- | --- |
| base · S (baseline) | 9 332 | 9 332 | 6 | 6 | 38 |
| base · O (candidate) | 9 332 | 9 332 | 4 | 3 | 38 |

The extended captures:

| cell | commits | on a `Stable` frame | median hops after the gate's first `Stable` | min | max |
| --- | --- | --- | --- | --- | --- |
| base · S (baseline) | 7 315 | 7 315 | 6 | 6 | 31 |
| base · O (candidate) | 7 315 | 7 315 | 4 | 3 | 31 |

No commit of either rule lands on or before the gate's first `Stable` verdict, in
any of the 9 332 and 7 315 (capture, phase) pairs that commit. The gap from the
gate's last flagged onset hop to its first `Stable` hop is 4 hops in 9 505 of
piano #2's 9 520 (capture, phase) pairs, 5 in 12 and 6 in 3; on the extended
captures, 4 in 7 486 of 7 552, 5 in 52 and 6 in 14; on Iowa, 4 in all 260
captures, whose 245 commits all land on a `Stable` frame a median 4 hops after
the gate's verdict against 6.

So what the change buys is a window already partly filled when `Stable` arrives,
not an earlier kind of commit. Measured from the gate's verdict the commit moves
from 6 hops to 4, 46.4 ms; the per-capture median gain on piano #2, 42.09 ms, is
a little less, because 162 of its 576 captures do not move.

### The yardstick

The margin is the baseline's own scatter over the hop grid. At sixteen phases the
shipped rule passes 564–571 captures on piano #2 (mean 567.19, σ 2.04), with 13–19
wrong, and 444–450 on the extended captures (mean 446.12, σ 1.36), with 7–11
wrong. On piano #2, keys 60–74 never move — 85 of 85 at every phase — while keys
75–87 range over 5 passes, the bass over 2 and the mid over 1. 558 captures pass
at every phase and 19 at none; of the 18 the grid decides, 15 are in keys 75–87.
Two phases of one unchanged build disagree about a median of 9 captures on
piano #2 and 6 on the extended captures. The recorded 568 / 595 is one draw from
that range, near its middle.

### Cost

The change runs `discover()` on the `Unstable` hops between an onset and `Stable`
that the return skipped. A hop's cost does not change: at most one peak
extraction, masking and refined discovery scan per hop, what a `Stable` hop has
always cost. `cargo bench -p tuner-core --bench discovery_cost` puts the scan at
21.9 µs refined (Stages A and B; 12.7 µs for Stage A alone), under a tenth of a
percent of the 23.2 ms hop. The bench scores one synthetic 24-partial frame; the
extraction and masking before the scan are not in it. It was run at a load
average of 0.99.

Per capture the number of scans barely moves, because the earlier commit ends
them sooner. Over the phase-0 dumps, under the engine's state machine, which
reproduces `engine lock`'s outcomes and failure lists under both rules on both
reads:

| read | scans per capture, shipped rule: median [range] | vote from the onset | difference | of which on `Unstable` hops |
| --- | --- | --- | --- | --- |
| piano #2 | 7 [7, 41] | 8 [7, 44] | +1 [−9, +8] | 3 [3, 11] |
| extended | 7 [7, 32] | 8 [7, 35] | +1 [−9, +9] | 3 [3, 9] |

The `Unstable` hops that now run a scan number three per capture at the median
and at the least: the hops between a strike's two onset hops and the gate's first
`Stable` one. The treble, where commits come late or not at all, scans a median
three more per capture (piano #2 12 → 15, extended 9 → 12).

### Re-baselining in the tree

`cargo lab engine lock`, before and after the change, on all three capture sets:

| capture set | captures | before | after | failures that changed |
| --- | --- | --- | --- | --- |
| piano #2 | 595 | 568 / 595 (bass 134/137, mid 296/298, treble 138/160) | 568 / 595 (bass 134/137, mid 296/298, treble 138/160) | none |
| piano #2, extended | 472 | 447 / 472 (bass 141/144, mid 164/164, treble 142/164) | 449 / 472 (bass 142/144, mid 164/164, treble 143/164) | `key_011_G#1_1787072524`: locked 23 → pass; `key_084_A7_1787074433`: never locked → pass |
| piano #1 | 87 | 81 / 87 (bass 21/26, mid 33/33, treble 27/28) | 82 / 87 (bass 22/26, mid 33/33, treble 27/28) | `key_003_C1`: locked 4 → pass |

Piano #2 is unchanged, with the same 27 failures, and the extended captures gain
the two passes the replay predicted, with the 23 failures it predicted. Piano #1
was never replayed. It gains one capture, C1, which the shipped rule locked a
semitone high; [report 0010](0010-m-of-n-lock-rule-replay.md) lists C1 as the one
piano-#1 capture its (7, 8) rule broke against the first-to-3 rule before it.

## Exploratory

None of this moves the decision.

### Iowa, beside the decision

| cell | pass | no-lock | wrong | latency: lower / higher / tied | median Δ | p |
| --- | --- | --- | --- | --- | --- | --- |
| base · S (baseline) | 213 | 15 | 32 | — | — | — |
| **base · O (candidate)** | **213** | **15** | **32** | 140 / 0 / 73 | **−2 hops** | 1.43 × 10⁻⁴² |

One phase, so no margin. The outcomes are identical capture for capture, and 140
captures commit earlier, none later, a median 2 hops; by register the bass and the
mid move a median 3 hops and the treble none.

### The RWC pianos

One read each, sixteen phases, each piano under its own σ; the vote from the onset
(`cand`) against the shipped rule (`base`). PF1, the Yamaha:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 249.81 | 249.75 | -0.06 | 14.19 | 14.25 | +0.06 | inside σ |
| bass (0–26) | 81 | 79.81 | 79.88 | +0.06 | 1.19 | 1.12 | -0.06 | inside σ |
| mid (27–59) | 99 | 98.00 | 98.00 | +0.00 | 1.00 | 1.00 | +0.00 | inside σ |
| treble (60–87) | 84 | 72.00 | 71.88 | -0.12 | 12.00 | 12.12 | +0.12 | inside σ |

PF2, the Bösendorfer:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 259 | 236.19 | 241.31 | +5.12 | 12.81 | 10.06 | -2.75 | outside σ, better |
| bass (0–26) | 81 | 75.62 | 78.31 | +2.69 | 5.38 | 2.69 | -2.69 | outside σ, better |
| mid (27–59) | 98 | 91.00 | 91.06 | +0.06 | 7.00 | 6.94 | -0.06 | outside σ, better |
| treble (60–87) | 80 | 69.56 | 71.94 | +2.38 | 0.44 | 0.44 | +0.00 | outside σ, better |

PF3, the Steinway:

| group | n | base pass | cand pass | Δ pass | base wrong | cand wrong | Δ wrong | against σ |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| all | 264 | 241.69 | 244.25 | +2.56 | 12.88 | 10.38 | -2.50 | outside σ, better |
| bass (0–26) | 81 | 79.81 | 80.56 | +0.75 | 1.19 | 0.44 | -0.75 | outside σ, better |
| mid (27–59) | 99 | 92.00 | 93.81 | +1.81 | 7.00 | 5.19 | -1.81 | outside σ, better |
| treble (60–87) | 84 | 69.88 | 69.88 | +0.00 | 4.69 | 4.75 | +0.06 | inside σ |

Non-inferior in every register on all three, and better outside σ on the
Bösendorfer (5.12 more passes of 259: bass 2.69, treble 2.38) and the Steinway
(2.56 of 264: mid 1.81); inside σ everywhere on the Yamaha. The vote from the
onset has now been read on six instruments: piano #2 twice, Iowa once, the
Disklavier once at two distances, and the three RWC pianos once each.

### The Disklavier's other playing modes

Long notes (LG) and staccato (ST), which the rules did not score, at phase 0 on
their own populations, pass / no-lock / wrong. At 3–4 m:

| mode | population | cell, phase 0 | all | bass | mid | treble | pass by pedal (S0 · S1) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| LG | 136 | prior · S | 118/4/14 of 136 | 24/0/14 of 38 | 46/0/0 of 46 | 48/4/0 of 52 | S0 110/128 · S1 8/8 |
| LG | 136 | prior · O | 120/4/12 of 136 | 26/0/12 of 38 | 46/0/0 of 46 | 48/4/0 of 52 | S0 112/128 · S1 8/8 |
| ST | 226 | prior · S | 187/26/13 of 226 | 65/0/8 of 73 | 69/15/5 of 89 | 53/11/0 of 64 | S0 102/134 · S1 85/92 |
| ST | 226 | prior · O | 189/23/14 of 226 | 64/0/9 of 73 | 71/13/5 of 89 | 54/10/0 of 64 | S0 103/134 · S1 86/92 |

At 50 cm:

| mode | population | cell, phase 0 | all | bass | mid | treble | pass by pedal (S0 · S1) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| LG | 254 | prior · S | 236/8/10 of 254 | 74/0/7 of 81 | 97/0/0 of 97 | 65/8/3 of 76 | S0 111/118 · S1 125/136 |
| LG | 254 | prior · O | 236/8/10 of 254 | 74/0/7 of 81 | 97/0/0 of 97 | 65/8/3 of 76 | S0 111/118 · S1 125/136 |
| ST | 257 | prior · S | 206/36/15 of 257 | 69/3/6 of 78 | 75/17/7 of 99 | 62/16/2 of 80 | S0 89/125 · S1 117/132 |
| ST | 257 | prior · O | 218/30/9 of 257 | 71/3/4 of 78 | 85/11/3 of 99 | 62/16/2 of 80 | S0 101/125 · S1 117/132 |

The staccato notes pass more often from the onset on the close read, 206 → 218 of
257; one phase, descriptive.

### With the extraction-cap change

The picker change that keeps the 128 largest maxima instead of the first 128 in
bin order, which lands separately, was replayed beside this one (`patched`; the
`-abstain` variant makes a hop with nothing to score cast no vote). On piano #2
the pair fails this report's rule:

| cell | mean pass | mean wrong | non-inferior | latency: lower / higher / tied | median Δ | p | role |
| --- | --- | --- | --- | --- | --- | --- | --- |
| base/S (baseline) | 567.19 | 16.06 | — | — | — | — | the baseline |
| base/O | 566.94 | 16.31 | yes | 414 / 0 / 162 | -42.1 ms | 4.73e-125 | **the candidate** |
| patched/S | 566.81 | 17.69 | **no** | 65 / 7 / 504 | 0.0 ms | 6.96e-13 | secondary |
| patched/O | 563.56 | 21.12 | **no** | 473 / 1 / 100 | -52.2 ms | 1.95e-140 | secondary |
| patched/O-abstain | 563.56 | 21.12 | **no** | 473 / 1 / 100 | -52.2 ms | 1.95e-140 | secondary |

Its mean passes fall to 563.56 against the floor of 565.15, and its wrong commits
rise to 21.12 against the ceiling of 18.01. 59 of those wrong commits over the
sixteen phases go to A0, from 11 captures: three of C♯5 and three of C♯7 carry 51
of them, and one or two each of A♯7, D♯7, E7 and B7 the other 8. Every one lands
on a `Stable` frame, 8 to 17 hops after the onset, and one C♯5 capture commits to
A0 at every phase. The vote from the onset alone makes 4 such commits (two C♯5,
two D♯6) and the picker change alone 8. No committing window held a hop with
nothing to score, and the abstaining variant is identical, so these are ordinary
votes for A0, not the artefact of an empty scan.

On the extended captures the pair is non-inferior:

| cell | mean pass | mean wrong | non-inferior | latency: lower / higher / tied | median Δ | p | role |
| --- | --- | --- | --- | --- | --- | --- | --- |
| base/S (baseline) | 446.12 | 9.06 | — | — | — | — | the baseline |
| base/O | 449.75 | 7.44 | yes | 333 / 0 / 118 | -47.9 ms | 1.14e-100 | **the candidate** |
| patched/S | 445.81 | 10.00 | yes | 15 / 0 / 435 | 0.0 ms | 6.1e-05 | secondary |
| patched/O | 449.62 | 8.44 | yes | 347 / 0 / 103 | -49.3 ms | 6.98e-105 | secondary |
| patched/O-abstain | 449.62 | 8.44 | yes | 347 / 0 / 103 | -49.3 ms | 6.98e-105 | secondary |

On Iowa, one phase and no margin, the pair passes 212 against the shipped rule's
213. On the Disklavier, at phase 0 only, it passes 152 of 179 and 238 of 257
against the shipped rule's 149 and 237. With both changes in the tree, piano #2
fails this rule; no other instrument read shows the failure.

## A correction to the record

The research replays that preceded this one, and the repository's
`replay_lock_rules.py`, which report 0010 was measured with, apply the rule to
the sequence of `Stable` hops without clearing the window at a bypass in the
middle of a capture, as `Engine::process` does. Where a capture holds a second
strike their window carries votes from before it, and their commit frame is
early. This replay clears the window there, and the integration check sides with
it. On piano #2's two reads 23 captures are affected (12 and 11: A♯0, E6 and D7 on
piano #2; C6, E6 and C7 on the extended captures), and 2 on Iowa (F♯5, G♯5), by up
to 15 hops, 348 ms (`key_063_C6_1787076020`). No outcome, committed key or pass
count changes, so every lock figure on record stands; only those captures' commit
frames, and the latencies built on them, were early.

## Limitations

- **Two reads of one piano decide it.** The extended captures are the same
  instrument as piano #2's set, so neither satisfies the two-instrument rule
  ([`CONTRIBUTING.md`](../CONTRIBUTING.md)). Iowa, read once, agrees; the
  Disklavier, read once at two distances with the rules fixed in advance,
  confirms; the three RWC pianos agree, descriptively. None of these reads
  selects a configuration for shipping: they test one fixed change.
- **The margin is the hop grid's scatter, not a tolerance**: what the grid alone
  moves the baseline by, not what a tuner would notice.
- **The latency test sees only captures both rules pass**, so it says nothing of
  the extended captures' gained ones.
- **`engine lock` prints no commit frame.** The integration check confirms the
  outcomes against the engine, not the latency, which rests on the replay.
- **Every commit measured lands on `Stable`, not every commit.** It was tabulated
  on piano #2 and Iowa. A note whose gate never reaches `Stable` could not lock
  before and can now lock on `Unstable` hops. No scored population held one:
  piano #2's and Iowa's captures all reach `Stable`, and the MAPS and RWC reads
  set apart the notes that do not (one on the close Disklavier read, five soft
  notes from F5 up on the Bösendorfer). That case is unmeasured.
- **The cost is a bench, not a profile**: one synthetic frame through
  `discover()`. The extraction and masking are not in it, and neither is the
  tracker, which now starts a median two hops sooner.
- **Both of the project's pianos are out of tune**, the regime the app targets,
  and a lock score is relative to its instrument
  ([`capture-sets.md`](../docs/internals/capture-sets.md)).

## Reproduce

- `cargo lab engine lock diagnostics_piano2` gives 568 / 595 and
  `cargo lab engine lock diagnostics_piano_1` gives 82 / 87; the extended store's
  472 open and undeclared captures give 449 / 472. Before the change the same
  three give 568, 81 and 447.
- `cargo test -p tuner-core --lib engine::tests::unstable_frames_vote_and_silence_clears`:
  four `Unstable` hops then `Stable` ones commit on the seventh vote, and a
  `Silence` hop after the fourth clears the window. The test fails on the engine
  before the change.
- `cargo bench -p tuner-core --bench discovery_cost`.
- The sixteen-phase replays and the Iowa, Disklavier and RWC reads were made with
  research tooling kept outside the repository; every table above is copied from
  its output files.

## References

- Emiya, V., Badeau, R. & David, B. (2010). Multipitch estimation of piano sounds
  using a new probabilistic spectral smoothness principle. *IEEE Transactions on
  Audio, Speech, and Language Processing* 18(6), 1643–1654.
  doi:10.1109/TASL.2009.2038819 — the citation MAPS asks for.
- Goto, M., Hashiguchi, H., Nishimura, T. & Oka, R. (2003). RWC Music Database:
  Music Genre Database and Musical Instrument Sound Database. *Proc. ISMIR 2003*,
  229–230.
- Goto, M., Balke, S. & Müller, M. (2026). RWC Instruments Database. Zenodo.
  doi:10.5281/zenodo.17170844.
- University of Iowa Electronic Music Studios. Musical Instrument Samples: piano
  (recorded 2001). <https://theremin.music.uiowa.edu/MISpiano.html>
