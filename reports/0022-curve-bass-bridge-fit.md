# report 0022 — the tuning curve's bass pair, from the bass-bridge keys

Every tuning-curve engine starts from Rigaud's whole-compass model B_ξ̂(m) with the
bass pair fitted to the instrument: engine (a) is built from it alone, and the other
engines give it to every unmeasured key and shrink each measured key's B toward it.
Until this change the engines fitted that pair to every measured key
(`curves::instrument_b_fit`), while discovery's templates fitted it to the keys on
the bass bridge alone ([report 0021](0021-curve-seeded-templates.md)). This report
records the change that gives the curve the templates' fit, and measures what it
moved. It selects nothing: there is no ground truth for a tuning curve yet, so the
comparison says what moved and by how much.

**The defect.** Report 0021 found Rigaud's Eq. 29 not identifiable from keys off the
bass bridge: fitted on the Disklavier's mid keys alone it put A0 at 25 times the
default, on piano #2's treble keys alone at 224 times, and on its extended captures'
at 0.08. The curve took that fit at every unmeasured key. Through the curve's own
input, piano #2's mid keys alone fitted the pair (−0.0059, −9.709), its treble keys
alone a bass asymptote rising toward the treble (s_B = +0.0038), and two keys between
A♯4 and B5 put A0 at 112 to 160 times the default and every engine's A0 target 716 to
954 ¢ flat.

**Why a tuning pass makes it a hazard.** The app's strobe freezes the live curve
bundle the first time it targets a curve, and the pass in progress is never moved
under the tuner: a later capture moves the live curve, and only a re-lock moves the
targets. A tuner who had measured only the temperament octave, or any two keys above
the bass, and started a pass tuned the bass to a pair those keys could not fix.

**The change.** The curve engines take the templates' fit, now
`curves::bass_bridge_fit`: Eq. 29 over the measured keys on the bass bridge — a key
whose newest trusted capture holds eight partials or more and whose B is at least
twice the treble term — with the slope held at the default when the points cover
fewer than twelve keys, and the default pair below two points. `instrument_b_fit`
goes, and with it the one call of `rigaud::fit_b_xi` on unscreened points. The fit,
`template_b_fit` until now, and its three `TEMPLATE_*` constants take names for what
they now serve, `BASS_BRIDGE_*`. What it leaves alone: discovery, whose templates were
already this fit, so every lock is byte-identical with and without a regen dump; the
strobe, which still builds each key's references from its own measured B; and the
engines' algorithms — the blend, the smoother, the Giordano calibration and the
multi-interval solve are unchanged, and only the pair they start from moves, with the
prior scatter σ_p the blend calibrates against it.

**What moved.** On the six complete profiles every curve moved by at most 5.3 ¢, at
piano #1's A0 in engine (a); the shipped engine, (d) balanced, by at most 4.0 ¢ at the
same key and 0.75 ¢ on the other five profiles. On a profile measured only above the
bass bridge, engine (a)'s bass is now exactly the curve with nothing measured, where
before the change it sat up to 2.0 ¢ from it (the mid alone), 3.2 ¢ (the temperament
octave), 8.2 ¢ (the treble alone) and 941 ¢ (two keys).

## Pre-registered decision rule

None. The change is the owner's decision, made on a correctness ground during report
0021's review and recorded there as planned: a pair the data cannot identify must not
set the targets a tuner locks. What was fixed in advance — after the before record
and before any after run — is the comparison below and three checks, any of which
failing would have stopped the change:

1. **Mid only and treble only give the default pair, and the bass of the curve with
   nothing measured.** On both profiles the pair after is (−0.066, −7.891) bit for bit,
   and engine (a)'s curve, the one built from the pair alone, equals the empty
   profile's at every bass key (0–26) bit for bit. The other engines also read the
   measured keys, through the octave chain, the smoother and the A4 anchor, so their
   bass differences from the empty profile are reported beside the check, not checked.
2. **One pair.** On every profile the curve's pair after equals the pair discovery's
   templates take from the same dump.
3. **Discovery unchanged.** `cargo lab engine lock` gives 568, 82 and 449 without a
   regen dump and 567, 450, 251, 240 and 245 with one (piano #2, its extended captures,
   the three RWC pianos), each with output byte-identical to the unchanged tree's.

The size of the change decides nothing.

## Population

| read | instrument | regen entries | keys in the curve's input | source |
| --- | --- | --- | --- | --- |
| piano #2 | upright piano #2, out of tune | 595 | 88 | the project's own captures, `cargo lab mat regen diagnostics_piano2` |
| piano #2, extended | the same upright in its as-found state, before tuning | 589, of which 119 are string-isolated solos the curve refuses | 88 | the project's own captures, the `Piano2_extended` store |
| piano #1 | upright piano #1, out of tune | 87 | 87 (F♯2 was never captured) | the project's own captures, regenerated for this report |
| RWC Yamaha, Bösendorfer, Steinway | RWC's PF1, PF2 and PF3 | 249, 233, 249 | 87, 87, 86 | RWC Instruments Database, Zenodo record 17170844 (CC BY-NC 4.0), measured for report 0021 |

**The partial profiles** are piano #2's regen dump cut to a set of keys, each kept key
keeping every one of its entries in the dump's order, so its newest capture is the one
the complete profile reads: the mid alone (C3–G♯5, keys 27–59), the treble alone
(A5–C8, 60–87), the bass alone (A0–B2, 0–26), the temperament octave (F♯3–F♯4,
33–45); report 0021's thirty random subsets, five draws each of 1, 2, 4, 8, 16 and 32
keys from the 60 keys its research had measured (its bass and temperament sets repeat
two of the four above and are not listed twice); and the empty profile, the curve with
nothing measured.

Piano #2's two reads are one instrument, and the partial profiles are that instrument
alone; the capture sets are validation data and may not select a configuration
([`CONTRIBUTING.md`](../CONTRIBUTING.md)). Nothing is selected here.

## Method

Every profile went through `cargo lab curve compare --json` twice, on the tree before
the change (`cb45a5d`) and after it. `curve compare` records each entry of a regen
dump as an auto-mode capture and admits it through
`CurveInput::from_profile_including_auto`, the validation path; the app's curve admits
only trusted captures, and otherwise the same one per key: its newest whole-note
capture with a finite positive B, two partials or more and a solvable Eq.-20 F₀.

The harness prints neither σ_p nor the fit's points, so both were read on the same
inputs by calling `curves::sigma_prior` and the fits directly, with `curve compare`'s
own loader. The pair discovery's templates take from each profile is the `[TEMPLATES]`
line of `cargo lab engine dump --regen`, which reads the dump through
`engine lock`'s loader; that loader also applies the lab's ±200 ¢ plausibility rule,
which changes a key's newest capture on one profile only, the RWC Steinway's B7 and
C8, two- and three-partial captures that are never points of the fit.

A curve's change is |after − before| in cents per key, summarised per engine and
register by its median and its maximum, with the key of the maximum. The six engine
rows are the harness's: (a), (b), (c), (d) balanced and (d) pure twelfths, the five the
app computes, and (d) with model-B widths, a variant only the harness computes.

**The scale** is the harness's leave-one-key-out error from the before run: each
measurement-dominated key is dropped in turn, the engine recomputed without it and
scored against the Eq.-6 octave-chain value at that key, and the median absolute error
reported per register. The harness splits its registers at 0–27, 28–62 and 63–87, not
at the change tables' 0–26, 27–59 and 60–87, and a key's change is set against the
error of the harness register that holds it. The error's reference is the octave
chain, so it favours the chain engines.

## Results

### The checks

| check | result |
| --- | --- |
| 1. mid only and treble only: the pair after | (−0.066, −7.891) bit for bit on both |
| 1. engine (a)'s bass against the curve with nothing measured | equal bit for bit at all 27 bass keys on both; before the change it differed by up to 1.98 ¢ (mid only, at D2) and 8.21 ¢ (treble only, at F1) |
| 1. the other engines' bass against it, reported | mid only: (b) 2.11 ¢ and (c) 2.16 ¢ at F♯2, (d) balanced 1.34 ¢, pure twelfths 0.50 ¢ and model-B 1.53 ¢ at B2; treble only: under 0.0001 ¢ in (b), (c), (d) balanced and model-B, 0.19 ¢ in pure twelfths |
| 2. the curve's pair against the templates' | equal on all 41 profiles, to the digit both print, and bit for bit on the curve's own input through the fit itself |
| 3. `engine lock`, no regen dump | 568 / 595, 82 / 87, 449 / 472, output byte-identical |
| 3. `engine lock --regen` | 567 / 595, 450 / 472, 251 / 264, 240 / 264, 245 / 264, output byte-identical; piano #1, read with a regen dump for the first time, 85 / 87 before and after |
| 3. the templates' pair on every profile | identical before and after on all 41 |

**Every check holds.**

The tests pin the same facts. `test_curve_b_shrinkage` switched to the new fit and
kept every expected value: its deviating profile's σ_p moves from 0.272 to 0.282, and
the starved key it marks prior-dominated keeps a blend weight under one half (0.436,
now 0.450). Three tests are new, and each fails on the unchanged tree: the curve's path
reproduces report 0021's recorded pairs bit for bit, two points closer than twelve keys
at the default slope; a curve bundle's templates are the B curve of the pair every
engine starts from, on six profiles; and a profile measured only in the mid or only in
the treble leaves the curve on the default pair, engine (a) and the curve-side B at
every bass key equal to the curve with nothing measured.

### The complete profiles

The fit before (every measured key) and after (the bass-bridge keys), with A0's B over
the default pair's, σ_p, and the curve's measurement-dominated, prior-dominated and
excluded keys:

| read | before: pair, regime, points | A0, × default | after: pair, regime, points over keys covered | A0, × default | σ_p | dominated / prior-dominated / excluded |
| --- | --- | --- | --- | --- | --- | --- |
| piano #2 | (-0.0667, -6.216), full, 88 | 4.92× | (-0.0614, -6.359), full, 25 over 25 | 4.77× | 0.074 → 0.062 | 66 / 22 / 0 → 66 / 22 / 0 |
| piano #2, extended | (-0.0678, -6.193), full, 88 | 4.92× | (-0.0612, -6.365), full, 25 over 25 | 4.76× | 0.073 → 0.068 | 63 / 25 / 0 → 63 / 25 / 0 |
| piano #1 | (-0.0812, -5.233), full, 87 | 9.60× | (-0.0736, -5.491), full, 28 over 34 | 8.71× | 0.187 → 0.214 | 67 / 20 / 0 → 67 / 20 / 0 |
| RWC Yamaha | (-0.0811, -6.676), full, 87 | 2.34× | (-0.0813, -6.672), full, 21 over 21 | 2.34× | 0.071 → 0.071 | 65 / 22 / 0 → 65 / 22 / 0 |
| RWC Bösendorfer | (-0.0575, -8.756), full, 87 | 0.54× | (-0.0089, -9.922), full, 15 over 20 | 0.48× | 0.129 → 0.148 | 67 / 20 / 0 → 67 / 20 / 0 |
| RWC Steinway | (-0.1275, -6.308), full, 86 | 1.31× | (-0.1378, -6.050), full, 12 over 12 | 1.36× | 0.053 → 0.060 | 64 / 22 / 0 → 65 / 21 / 0 |

The fit is full on every read: 25 points on piano #2's two reads (A0–A2), 28 over 34
keys on piano #1, 21 on the Yamaha, 15 over 20 on the Bösendorfer and 12 on the
Steinway, covering the twelve keys a fitted slope needs. The keys each read counts as
measurement-dominated do not move, except one Steinway key that σ_p's rise tips from
prior-dominated to dominated.

The curve's change, median / maximum (key of the maximum), in cents:

| read | engine | bass (0–26) | mid (27–59) | treble (60–87) | A0, ¢ before → after |
| --- | --- | --- | --- | --- | --- |
| piano #2 | (a) | 0.71 / 0.88 (A#1) | 0.06 / 0.48 (C3) | 0.08 / 0.09 (C8) | -50.67 → -50.42 |
|  | (b) | 0.03 / 0.10 (A0) | 0.03 / 0.05 (E3) | 0.13 / 0.65 (C8) | -50.96 → -50.87 |
|  | (c) | 0.03 / 0.09 (A0) | 0.03 / 0.05 (E3) | 0.13 / 0.64 (C8) | -49.75 → -49.66 |
|  | (d) balanced | 0.05 / 0.58 (A0) | 0.00 / 0.00 (G#5) | 0.03 / 0.09 (C8) | -18.67 → -18.09 |
|  | (d) pure 12ths | 0.02 / 0.36 (A0) | 0.00 / 0.01 (D4) | 0.01 / 0.08 (C8) | -12.79 → -12.43 |
|  | (d) model-B | 0.19 / 0.56 (A0) | 0.02 / 0.10 (C3) | 0.06 / 0.10 (C8) | -18.80 → -18.24 |
| piano #2, extended | (a) | 0.98 / 1.20 (A1) | 0.07 / 0.61 (C3) | 0.10 / 0.12 (C8) | -50.39 → -50.38 |
|  | (b) | 0.05 / 0.14 (A0) | 0.05 / 0.09 (D#5) | 0.20 / 0.51 (C8) | -50.64 → -50.50 |
|  | (c) | 0.05 / 0.14 (A0) | 0.05 / 0.08 (D#5) | 0.19 / 0.50 (C8) | -49.47 → -49.33 |
|  | (d) balanced | 0.05 / 0.75 (A0) | 0.00 / 0.01 (G#5) | 0.05 / 0.12 (C8) | -19.81 → -19.06 |
|  | (d) pure 12ths | 0.05 / 0.57 (A0) | 0.00 / 0.00 (D#4) | 0.02 / 0.11 (C8) | -14.31 → -13.74 |
|  | (d) model-B | 0.24 / 0.61 (A0) | 0.03 / 0.11 (C3) | 0.07 / 0.12 (C8) | -19.68 → -19.07 |
| piano #1 | (a) | 0.79 / 5.31 (A0) | 0.07 / 0.61 (C3) | 0.09 / 0.10 (A7) | -89.11 → -83.80 |
|  | (b) | 0.11 / 0.77 (A0) | 0.08 / 0.17 (F5) | 0.10 / 0.27 (F#6) | -72.85 → -72.08 |
|  | (c) | 0.09 / 0.66 (A0) | 0.06 / 0.14 (F5) | 0.10 / 0.23 (F#6) | -59.38 → -58.73 |
|  | (d) balanced | 0.25 / 4.01 (A0) | 0.00 / 0.03 (C3) | 0.06 / 0.10 (C8) | -49.70 → -45.69 |
|  | (d) pure 12ths | 0.04 / 1.93 (A0) | 0.00 / 0.00 (D#4) | 0.03 / 0.08 (C8) | -24.60 → -22.67 |
|  | (d) model-B | 0.25 / 4.11 (A0) | 0.02 / 0.14 (C3) | 0.07 / 0.10 (C8) | -44.09 → -39.98 |
| RWC Yamaha | (a) | 0.02 / 0.03 (D#1) | 0.00 / 0.01 (C3) | 0.00 / 0.00 (F#7) | -24.48 → -24.47 |
|  | (b) | 0.00 / 0.00 (C#1) | 0.00 / 0.00 (C#3) | 0.00 / 0.01 (C8) | -23.63 → -23.63 |
|  | (c) | 0.00 / 0.00 (C#1) | 0.00 / 0.00 (C#3) | 0.00 / 0.01 (C8) | -17.03 → -17.03 |
|  | (d) balanced | 0.00 / 0.01 (A0) | 0.00 / 0.00 (C3) | 0.00 / 0.00 (A#7) | -7.83 → -7.83 |
|  | (d) pure 12ths | 0.00 / 0.01 (A0) | 0.00 / 0.00 (C#4) | 0.00 / 0.00 (A#7) | -9.11 → -9.12 |
|  | (d) model-B | 0.00 / 0.01 (E1) | 0.00 / 0.00 (C3) | 0.00 / 0.00 (G#7) | -7.75 → -7.75 |
| RWC Bösendorfer | (a) | 1.62 / 1.80 (G#1) | 0.23 / 1.11 (C3) | 0.39 / 0.45 (C8) | -8.54 → -9.69 |
|  | (b) | 0.02 / 0.05 (F1) | 0.02 / 0.05 (F5) | 0.54 / 1.39 (C8) | -9.16 → -9.17 |
|  | (c) | 0.01 / 0.04 (F1) | 0.02 / 0.04 (F5) | 0.40 / 1.09 (C8) | -7.93 → -7.93 |
|  | (d) balanced | 0.09 / 0.36 (A0) | 0.01 / 0.02 (G#5) | 0.25 / 0.50 (C8) | -3.66 → -3.30 |
|  | (d) pure 12ths | 0.09 / 0.36 (A0) | 0.01 / 0.02 (G#5) | 0.25 / 0.50 (C8) | -6.10 → -5.74 |
|  | (d) model-B | 0.35 / 0.44 (A1) | 0.07 / 0.24 (C3) | 0.31 / 0.48 (C8) | -3.59 → -3.51 |
| RWC Steinway | (a) | 0.12 / 0.21 (A0) | 0.00 / 0.04 (C3) | 0.00 / 0.03 (C8) | -13.68 → -13.90 |
|  | (b) | 0.01 / 0.04 (A0) | 0.01 / 0.03 (F4) | 0.17 / 0.27 (G#6) | -14.38 → -14.42 |
|  | (c) | 0.01 / 0.03 (A0) | 0.01 / 0.03 (F4) | 0.17 / 0.26 (G#6) | -13.87 → -13.91 |
|  | (d) balanced | 0.01 / 0.19 (A0) | 0.01 / 0.07 (G#5) | 0.14 / 0.19 (F6) | -5.74 → -5.93 |
|  | (d) pure 12ths | 0.02 / 0.20 (A0) | 0.00 / 0.03 (G#5) | 0.05 / 0.07 (F6) | -7.92 → -8.12 |
|  | (d) model-B | 0.03 / 0.20 (A0) | 0.01 / 0.07 (G#5) | 0.14 / 0.19 (F6) | -5.66 → -5.86 |

Engine (a) moves most, since it is built from the pair alone. On piano #1 the fit on
every key had pulled A0's B to 9.6 times the default, the bass-bridge fit puts it at
8.7, and A0's target moves 5.3 ¢; on the Bösendorfer, whose bass is nearly flat, the
slope moves from −0.0575 to −0.0089 and engine (a)'s bass by up to 1.8 ¢. Engines (b)
and (c) hold their bass within 0.8 ¢, where measured keys dominate the blend, and move
up to 1.4 ¢ at the top of the Bösendorfer. Engine (d) moves up to 4.1 ¢ at piano #1's
A0, 4.0 ¢ in the balanced preset, and 0.75 ¢ or less anywhere else.

### The scale

Each engine's leave-one-key-out error before the change, with the number of keys it
was scored on per harness register, and the keys whose change exceeds it:

| read | engine | leave-one-key-out error before, ¢: bass (0–27) / mid (28–62) / treble (63–87), keys | keys whose change exceeds it |
| --- | --- | --- | --- |
| piano #2 | (a) | 0.65 / 0.44 / 0.90 (28 / 33 / 5) | 17: E1–G2, C#3; largest A#1 0.88 |
|  | (b) | 0.48 / 0.36 / 0.85 (28 / 33 / 5) | none |
|  | (c) | 0.61 / 0.37 / 0.77 (28 / 33 / 5) | none |
|  | (d) balanced | 15.49 / 1.28 / 1.05 (28 / 33 / 5) | none |
|  | (d) pure 12ths | 15.10 / 0.87 / 0.62 (28 / 33 / 5) | none |
|  | (d) model-B | 15.70 / 1.32 / 1.10 (28 / 33 / 5) | none |
| piano #2, extended | (a) | 0.67 / 0.44 / 0.03 (28 / 34 / 1) | 50: C#1–A#2, C#3–D#3, C6–C8; largest A1 1.20 |
|  | (b) | 0.48 / 0.36 / 1.24 (28 / 34 / 1) | none |
|  | (c) | 0.60 / 0.38 / 1.11 (28 / 34 / 1) | none |
|  | (d) balanced | 14.65 / 1.16 / 0.67 (28 / 34 / 1) | none |
|  | (d) pure 12ths | 15.25 / 0.79 / 0.48 (28 / 34 / 1) | none |
|  | (d) model-B | 14.99 / 1.15 / 0.72 (28 / 34 / 1) | none |
| piano #1 | (a) | 3.07 / 1.93 / 1.40 (27 / 35 / 5) | 3: A0–B0; largest A0 5.31 |
|  | (b) | 2.49 / 1.52 / 1.77 (27 / 35 / 5) | none |
|  | (c) | 4.38 / 1.66 / 2.21 (27 / 35 / 5) | none |
|  | (d) balanced | 19.95 / 1.76 / 0.90 (27 / 35 / 5) | none |
|  | (d) pure 12ths | 24.53 / 1.51 / 1.52 (27 / 35 / 5) | none |
|  | (d) model-B | 22.91 / 1.77 / 1.03 (27 / 35 / 5) | none |
| RWC Yamaha | (a) | 0.37 / 0.35 / 0.30 (28 / 35 / 2) | none |
|  | (b) | 0.53 / 0.46 / 0.66 (28 / 35 / 2) | none |
|  | (c) | 3.45 / 0.74 / 0.62 (28 / 35 / 2) | none |
|  | (d) balanced | 6.32 / 0.95 / 1.17 (28 / 35 / 2) | none |
|  | (d) pure 12ths | 4.68 / 0.54 / 0.57 (28 / 35 / 2) | none |
|  | (d) model-B | 6.31 / 0.98 / 1.27 (28 / 35 / 2) | none |
| RWC Bösendorfer | (a) | 0.48 / 0.33 / 0.78 (28 / 35 / 4) | 40: A0–C4; largest G#1 1.80 |
|  | (b) | 0.34 / 0.23 / 0.81 (28 / 35 / 4) | 10: D#7–C8; largest C8 1.39 |
|  | (c) | 0.68 / 0.36 / 0.88 (28 / 35 / 4) | 4: A7–C8; largest C8 1.09 |
|  | (d) balanced | 3.27 / 0.55 / 1.51 (28 / 35 / 4) | none |
|  | (d) pure 12ths | 1.57 / 0.33 / 0.98 (28 / 35 / 4) | none |
|  | (d) model-B | 3.34 / 0.49 / 1.42 (28 / 35 / 4) | none |
| RWC Steinway | (a) | 0.23 / 0.21 / 0.15 (28 / 35 / 1) | none |
|  | (b) | 0.23 / 0.20 / 0.11 (28 / 35 / 1) | 20: C#6–G#7; largest G#6 0.27 |
|  | (c) | 0.23 / 0.18 / 0.14 (28 / 35 / 1) | 17: D6–F#7; largest G#6 0.26 |
|  | (d) balanced | 2.97 / 0.98 / 1.04 (28 / 35 / 1) | none |
|  | (d) pure 12ths | 1.19 / 0.40 / 0.57 (28 / 35 / 1) | none |
|  | (d) model-B | 2.92 / 1.03 / 1.11 (28 / 35 / 1) | none |

No engine (d) row exceeds its own error on any read. Engine (a) exceeds its error
across the bass of piano #2, its extended captures and the Bösendorfer, and at
piano #1's A0–B0; engines (b) and (c) exceed theirs at the top of the Bösendorfer and
the Steinway. Two treble errors rest on a single key, the extended captures' and the
Steinway's, so the 25 top keys of the extended captures' row exceed an error of
0.03 ¢ by 0.05 to 0.09 ¢.

### The partial profiles

The fit, as above; the pair's regime after is followed by its points and, where it
fits, the keys they cover:

| profile | before: pair, regime, points | A0, × default | after: pair, regime, points over keys covered | A0, × default | σ_p | dominated / prior-dominated / excluded |
| --- | --- | --- | --- | --- | --- | --- |
| mid only (C3–G#5) | (-0.0059, -9.709), full, 33 | 0.61× | (-0.0660, -7.891), default, 0 | 1.00× | 0.023 → 0.055 | 28 / 5 / 0 → 31 / 2 / 0 |
| treble only (A5–C8) | (0.0038, -9.300), full, 28 | 1.05× | (-0.0660, -7.891), default, 0 | 1.00× | 0.120 → 0.120 | 10 / 18 / 0 → 10 / 18 / 0 |
| bass only (A0–B2) | (-0.0627, -6.323), full, 27 | 4.80× | (-0.0614, -6.359), full, 25 over 25 | 4.77× | 0.078 → 0.064 | 27 / 0 / 0 → 27 / 0 / 0 |
| temperament octave (F#3–F#4) | (0.0044, -10.347), full, 13 | 0.43× | (-0.0660, -7.891), default, 0 | 1.00× | 0.029 → 0.014 | 13 / 0 / 0 → 13 / 0 / 0 |
| draw 0, 2 keys: D5, B5 | (-0.0979, -2.063), full, 2 | 160× | (-0.0660, -7.891), default, 0 | 1.00× | 0.120 → 0.120 | 2 / 0 / 0 → 2 / 0 / 0 |
| draw 1, 2 keys: E5, B5 | (-0.1039, -2.296), full, 2 | 112× | (-0.0660, -7.891), default, 0 | 1.00× | 0.120 → 0.120 | 2 / 0 / 0 → 2 / 0 / 0 |
| draw 2, 2 keys: C1–C#1 | (-0.0141, -7.557), full, 2 | 3.89× | (-0.0660, -6.312), intercept, 2 over 2 | 4.54× | 0.120 → 0.120 | 2 / 0 / 0 → 2 / 0 / 0 |
| draw 3, 2 keys: F#1, A3 | (-0.0809, -5.687), full, 2 | 6.17× | (-0.0660, -7.891), default, 1 | 1.00× | 0.120 → 0.120 | 2 / 0 / 0 → 2 / 0 / 0 |
| draw 4, 2 keys: A#4, F#5 | (-0.1014, -2.278), full, 2 | 120× | (-0.0660, -7.891), default, 0 | 1.00× | 0.120 → 0.120 | 2 / 0 / 0 → 2 / 0 / 0 |
| draw 0, 4 keys: E2, G3, C4, B4 | (-0.1012, -4.739), full, 4 | 10× | (-0.0660, -7.891), default, 1 | 1.00× | 0.013 → 0.148 | 4 / 0 / 0 → 4 / 0 / 0 |
| draw 1, 4 keys: A1, A4–A#4, E5 | (-0.0419, -6.800), full, 4 | 4.62× | (-0.0660, -7.891), default, 1 | 1.00× | 0.010 → 0.109 | 3 / 1 / 0 → 4 / 0 / 0 |
| draw 2, 4 keys: D#3–E3, A#3, C4 | (0.0024, -10.891), full, 4 | 0.27× | (-0.0660, -7.891), default, 0 | 1.00× | 0.095 → 0.084 | 4 / 0 / 0 → 4 / 0 / 0 |
| draw 3, 4 keys: F2, A#2, C3–C#3 | (-0.1114, -4.402), full, 4 | 12× | (-0.0660, -7.891), default, 1 | 1.00× | 0.062 → 0.390 | 4 / 0 / 0 → 4 / 0 / 0 |
| draw 4, 4 keys: C#1, G#3, G#4, A#4 | (-0.0765, -5.979), full, 4 | 5.06× | (-0.0660, -7.891), default, 1 | 1.00× | 0.072 → 0.066 | 4 / 0 / 0 → 4 / 0 / 0 |
| draw 0, 8 keys: A1, C#2, B2, G3, C#4, F#4, B4, E5 | (-0.0967, -4.993), full, 8 | 8.83× | (-0.0660, -6.005), intercept, 2 over 5 | 6.14× | 0.023 → 0.123 | 8 / 0 / 0 → 8 / 0 / 0 |
| draw 1, 8 keys: B0, E1, C2–C#2, A#4, C5–C#5, C#6 | (-0.0553, -6.533), full, 8 | 4.55× | (-0.0556, -6.525), full, 4 over 15 | 4.56× | 0.040 → 0.038 | 8 / 0 / 0 → 8 / 0 / 0 |
| draw 2, 8 keys: B0, A#1–B1, D2, C3, F#3, G#3, C#4 | (-0.0663, -6.201), full, 8 | 5.03× | (-0.0550, -6.541), full, 4 over 16 | 4.55× | 0.109 → 0.126 | 8 / 0 / 0 → 8 / 0 / 0 |
| draw 3, 8 keys: C#1, F#1–G1, D2, B2, C#5, E5, B5 | (-0.0647, -6.181), full, 8 | 5.30× | (-0.0631, -6.232), full, 4 over 14 | 5.21× | 0.010 → 0.013 | 6 / 2 / 0 → 6 / 2 / 0 |
| draw 4, 8 keys: C#1, E1, C2, E2, B2, F3, C#5, F#5 | (-0.0660, -6.205), full, 8 | 5.04× | (-0.0597, -6.400), full, 4 over 16 | 4.74× | 0.024 → 0.028 | 8 / 0 / 0 → 8 / 0 / 0 |
| draw 0, 16 keys: A#0–C1, G1, C2, D#2–E2, F#2, D3, E3, A3, D#4–E4, D#5–E5, C#6 | (-0.0642, -6.288), full, 16 | 4.82× | (-0.0601, -6.415), full, 8 over 21 | 4.63× | 0.077 → 0.054 | 16 / 0 / 0 → 16 / 0 / 0 |
| draw 1, 16 keys: B0–C1, E1, A#1, D#2, F2, C3, E3, G3, B3–C4, D4–D#4, D5–D#5, F#5 | (-0.0635, -6.345), full, 16 | 4.63× | (-0.0617, -6.385), full, 6 over 19 | 4.61× | 0.033 → 0.042 | 16 / 0 / 0 → 16 / 0 / 0 |
| draw 2, 16 keys: A0, C1, E1, F#1, C2–C#2, E2, G#2, B2, E3, G#3, D4, E4, G4, B5, C#6 | (-0.0625, -6.331), full, 16 | 4.79× | (-0.0596, -6.403), full, 8 over 24 | 4.73× | 0.069 → 0.068 | 15 / 1 / 0 → 15 / 1 / 0 |
| draw 3, 16 keys: C1, E1, F#1, C2, E2, F#2, E3, G3–G#3, C#4, F4–F#4, G#4, B4, E5, B5 | (-0.0681, -6.145), full, 16 | 5.12× | (-0.0653, -6.224), full, 6 over 19 | 5.02× | 0.086 → 0.095 | 15 / 1 / 0 → 16 / 0 / 0 |
| draw 4, 16 keys: D1, F#1–G1, D#2, F2, A#2–B2, E3, G3, A#3–C4, F4, B4, D#5, C#6 | (-0.0780, -5.776), full, 16 | 6.01× | (-0.0754, -5.853), full, 5 over 16 | 5.87× | 0.045 → 0.056 | 16 / 0 / 0 → 16 / 0 / 0 |
| draw 0, 32 keys: A#0, D1, E1, G#1–A1, C#2–E2, F#2, G#2–A#2, C3, D3, E3, F#3, G#3–A#3, E4–F#4, G#4–A4, B4–C5, D5–F5, C#6 | (-0.0693, -6.111), full, 32 | 5.16× | (-0.0613, -6.335), full, 12 over 24 | 4.88× | 0.087 → 0.088 | 32 / 0 / 0 → 32 / 0 / 0 |
| draw 1, 32 keys: A#0, E1–F1, A1–D2, G2–G#2, B2, E3–A#3, C4, D4, E4–F4, G4–D5, C#6 | (-0.0683, -6.138), full, 32 | 5.13× | (-0.0628, -6.318), full, 11 over 23 | 4.81× | 0.074 → 0.068 | 32 / 0 / 0 → 32 / 0 / 0 |
| draw 2, 32 keys: A0, C1, D#1–E1, F#1–G1, A1, B1–C2, D2–D#2, F2, A#2, C3–D#3, F3, G#3, B3, C#4–D4, F#4–G4, A4–C5, F5–F#5, B5, C#6 | (-0.0677, -6.157), full, 32 | 5.10× | (-0.0626, -6.326), full, 12 over 21 | 4.80× | 0.084 → 0.081 | 31 / 1 / 0 → 31 / 1 / 0 |
| draw 3, 32 keys: A0, C1, D#1–E1, G1–B1, C#2–D2, E2, F#2, A2, C3, D3–D#3, F#3, G#3, B3–D#4, F#4, A4–A#4, C5, D#5–F#5 | (-0.0624, -6.305), full, 32 | 4.92× | (-0.0578, -6.442), full, 14 over 25 | 4.73× | 0.054 → 0.063 | 32 / 0 / 0 → 32 / 0 / 0 |
| draw 4, 32 keys: A#0, C1–D1, E1–F1, A1, B1–C#2, F2, A2, B2, D3, F#3–G3, A3–C4, D4, F4, G4–G#4, A#4–D5, E5–F5, C#6 | (-0.0633, -6.308), full, 32 | 4.81× | (-0.0616, -6.354), full, 12 over 24 | 4.77× | 0.032 → 0.033 | 31 / 1 / 0 → 31 / 1 / 0 |

Every profile with fewer than two bass-bridge keys now takes the default pair: the mid
alone, the treble alone, the temperament octave, four of the five two-key draws and all
five four-key draws. The intercept regime binds on two draws, the adjacent C1–C♯1 and
the eight-key draw whose two bass-bridge points cover five keys. The bass alone holds
the complete profile's 25 points and fits its pair.

The curve's change, median / maximum (key of the maximum), in cents; engines whose
rows are identical to the digits shown share a row:

| profile | engine | bass (0–26) | mid (27–59) | treble (60–87) | A0, ¢ before → after |
| --- | --- | --- | --- | --- | --- |
| mid only | (a) | 1.70 / 1.98 (D2) | 0.36 / 1.51 (C3) | 0.60 / 0.70 (C8) | -11.80 → -12.82 |
|  | (b) | 0.23 / 1.07 (A0) | 0.26 / 0.66 (D#3) | 0.50 / 1.21 (C8) | -11.80 → -12.87 |
|  | (c) | 0.23 / 1.07 (A0) | 0.27 / 0.66 (D#3) | 0.51 / 1.21 (C8) | -11.87 → -12.94 |
|  | (d) balanced | 0.90 / 1.35 (A1) | 0.02 / 0.17 (C3) | 0.44 / 0.79 (C8) | -12.47 → -13.17 |
|  | (d) pure 12ths | 1.07 / 1.51 (A#1) | 0.06 / 0.38 (C3) | 0.51 / 0.76 (C8) | -12.21 → -12.95 |
|  | (d) model-B | 1.01 / 1.42 (A#1) | 0.10 / 0.42 (C3) | 0.46 / 0.78 (C8) | -12.45 → -13.21 |
| treble only | (a), (b), (c), (d) balanced | 7.55 / 8.21 (F1) | 1.09 / 4.68 (C3) | 1.91 / 2.29 (C8) | -20.15 → -12.82 |
|  | (d) pure 12ths | 7.55 / 8.21 (F1) | 1.28 / 4.71 (C3) | 1.71 / 1.74 (D#6) | -20.20 → -12.88 |
|  | (d) model-B | 7.55 / 8.21 (F1) | 1.09 / 4.68 (C3) | 1.91 / 2.29 (C8) | -20.15 → -12.82 |
| bass only | (a) | 0.18 / 0.23 (A#1) | 0.02 / 0.13 (C3) | 0.02 / 0.03 (C8) | -50.48 → -50.42 |
|  | (b) | 0.17 / 0.19 (E1) | 0.01 / 0.12 (C3) | 0.02 / 0.02 (C8) | -50.49 → -50.59 |
|  | (c) | 0.17 / 0.19 (E1) | 0.01 / 0.12 (C3) | 0.02 / 0.02 (C8) | -49.92 → -50.02 |
|  | (d) balanced | 0.15 / 0.17 (G1) | 0.02 / 0.11 (C3) | 0.02 / 0.03 (C8) | -22.93 → -22.92 |
|  | (d) pure 12ths | 0.09 / 0.09 (D1) | 0.02 / 0.08 (C3) | 0.02 / 0.03 (C8) | -12.70 → -12.75 |
|  | (d) model-B | 0.16 / 0.19 (A1) | 0.02 / 0.11 (C3) | 0.02 / 0.03 (C8) | -22.24 → -22.22 |
| temperament octave | (a), (b), (c) | 1.24 / 3.18 (A0) | 0.37 / 1.20 (C3) | 0.66 / 0.79 (B7) | -9.64 → -12.82 |
|  | (d) balanced | 0.68 / 2.96 (A0) | 0.08 / 0.32 (C3) | 0.63 / 0.82 (C8) | -10.07 → -13.03 |
|  | (d) pure 12ths | 0.69 / 2.96 (A0) | 0.06 / 0.34 (C3) | 0.63 / 0.82 (C8) | -10.09 → -13.05 |
|  | (d) model-B | 0.93 / 3.06 (A0) | 0.22 / 0.73 (C3) | 0.64 / 0.80 (C8) | -10.07 → -13.13 |
| draw 0, 2 keys | every engine | 289.61 / 940.72 (A0) | 2.89 / 52.89 (C3) | 1.96 / 3.30 (F7) | -953.55 → -12.82 |
| draw 1, 2 keys | every engine | 191.65 / 703.17 (A0) | 1.58 / 31.26 (C3) | 1.04 / 1.80 (F7) | -715.99 → -12.82 |
| draw 2, 2 keys | every engine | 14.23 / 15.95 (G#1) | 1.87 / 9.41 (C3) | 2.89 / 3.36 (C8) | -56.79 → -47.16 |
| draw 3, 2 keys | every engine | 13.20 / 46.17 (A0) | 0.15 / 2.59 (C3) | 0.13 / 0.19 (C8) | -58.99 → -12.82 |
| draw 4, 2 keys | every engine | 212.17 / 749.08 (A0) | 1.88 / 36.03 (C3) | 1.28 / 2.13 (F7) | -761.91 → -12.82 |
| draw 0, 4 keys | every engine | 17.01 / 76.52 (A0) | 0.10 / 2.47 (C3) | 0.06 / 0.26 (C8) | -89.34 → -12.82 |
| draw 1, 4 keys | (a) | 19.68 / 41.56 (A0) | 0.68 / 6.56 (C3) | 0.89 / 1.10 (C8) | -54.38 → -12.82 |
|  | (b), (c) | 18.68 / 40.72 (A0) | 0.82 / 5.98 (C3) | 1.36 / 1.64 (C8) | -54.38 → -13.67 |
|  | (d) balanced, (d) pure 12ths, (d) model-B | 19.68 / 41.56 (A0) | 0.68 / 6.56 (C3) | 0.89 / 1.10 (C8) | -54.38 → -12.82 |
| draw 2, 4 keys | every engine | 1.01 / 5.84 (A0) | 0.15 / 0.31 (E3) | 0.31 / 0.37 (A#7) | -6.98 → -12.82 |
| draw 3, 4 keys | (a), (b), (c) | 16.45 / 84.60 (A0) | 0.05 / 2.01 (C3) | 0.04 / 0.37 (C8) | -97.42 → -12.82 |
|  | (d) balanced | 12.78 / 83.09 (A0) | 1.15 / 4.01 (D#3) | 0.22 / 0.66 (A5) | -95.91 → -12.82 |
|  | (d) pure 12ths | 16.45 / 84.60 (A0) | 0.05 / 2.01 (C3) | 0.04 / 0.37 (C8) | -97.42 → -12.82 |
|  | (d) model-B | 13.61 / 83.42 (A0) | 0.90 / 3.44 (C#3) | 0.20 / 0.51 (A5) | -95.95 → -12.53 |
| draw 4, 4 keys | (a), (b), (c) | 11.19 / 37.11 (A0) | 0.15 / 2.34 (C3) | 0.13 / 0.18 (C8) | -49.93 → -12.82 |
|  | (d) balanced | 11.04 / 37.30 (A0) | 0.11 / 1.58 (C3) | 0.08 / 0.22 (C8) | -50.39 → -13.08 |
|  | (d) pure 12ths | 11.05 / 37.29 (A0) | 0.09 / 1.62 (C3) | 0.08 / 0.22 (C8) | -50.41 → -13.12 |
|  | (d) model-B | 11.10 / 37.23 (A0) | 0.04 / 1.85 (C3) | 0.10 / 0.21 (C8) | -50.38 → -13.14 |
| draw 0, 8 keys | (a) | 2.56 / 15.78 (A0) | 0.22 / 1.91 (C3) | 0.26 / 0.29 (F#7) | -78.22 → -62.44 |
|  | (b), (c) | 1.94 / 16.00 (A0) | 0.32 / 1.28 (C3) | 0.58 / 0.63 (G7) | -78.22 → -62.22 |
|  | (d) balanced | 2.56 / 15.78 (A0) | 0.22 / 1.91 (C3) | 0.26 / 0.29 (F#7) | -78.22 → -62.44 |
|  | (d) pure 12ths | 1.52 / 15.90 (A0) | 0.38 / 1.35 (C3) | 0.33 / 0.41 (A#5) | -78.22 → -62.32 |
|  | (d) model-B | 1.91 / 15.85 (A0) | 0.28 / 1.37 (C3) | 0.27 / 0.29 (F7) | -78.22 → -62.37 |
| draw 1, 8 keys | (a) | 0.07 / 0.09 (F#1) | 0.01 / 0.04 (C3) | 0.01 / 0.01 (C8) | -49.78 → -49.73 |
|  | (b), (c) | 0.05 / 0.06 (E1) | 0.00 / 0.02 (C3) | 0.01 / 0.02 (C8) | -49.75 → -49.72 |
|  | (d) balanced, (d) pure 12ths, (d) model-B | 0.07 / 0.09 (F#1) | 0.01 / 0.04 (C3) | 0.01 / 0.01 (C8) | -49.78 → -49.73 |
| draw 2, 8 keys | (a), (b), (c) | 1.27 / 1.96 (A0) | 0.13 / 0.96 (C3) | 0.18 / 0.22 (C8) | -51.80 → -49.84 |
|  | (d) balanced | 2.59 / 3.22 (E1) | 0.27 / 1.05 (C3) | 0.15 / 0.31 (C8) | -27.41 → -29.71 |
|  | (d) pure 12ths | 1.27 / 1.96 (A0) | 0.13 / 0.96 (C3) | 0.18 / 0.22 (C8) | -51.80 → -49.84 |
|  | (d) model-B | 2.47 / 2.98 (G1) | 0.25 / 1.26 (C3) | 0.14 / 0.31 (C8) | -28.62 → -29.78 |
| draw 3, 8 keys | (a), (b), (c) | 0.17 / 0.41 (A0) | 0.02 / 0.13 (C3) | 0.02 / 0.03 (C8) | -54.78 → -54.37 |
|  | (d) balanced | 0.12 / 0.13 (A0) | 0.02 / 0.08 (C3) | 0.02 / 0.03 (C8) | -33.98 → -33.84 |
|  | (d) pure 12ths | 0.17 / 0.41 (A0) | 0.02 / 0.13 (C3) | 0.02 / 0.03 (C8) | -54.78 → -54.37 |
|  | (d) model-B | 0.12 / 0.17 (A0) | 0.02 / 0.09 (C3) | 0.02 / 0.03 (C8) | -34.47 → -34.30 |
| draw 4, 8 keys | (a) | 0.60 / 1.49 (A0) | 0.07 / 0.48 (C3) | 0.09 / 0.11 (C8) | -52.01 → -50.52 |
|  | (b), (c) | 0.68 / 1.21 (A0) | 0.08 / 0.54 (C3) | 0.02 / 0.05 (A5) | -51.73 → -50.52 |
|  | (d) balanced | 0.41 / 0.95 (A0) | 0.06 / 0.31 (C3) | 0.09 / 0.11 (C8) | -34.50 → -33.54 |
|  | (d) pure 12ths | 0.38 / 0.95 (A0) | 0.07 / 0.33 (C3) | 0.09 / 0.11 (C8) | -33.24 → -32.29 |
|  | (d) model-B | 0.46 / 1.03 (A0) | 0.06 / 0.33 (C3) | 0.09 / 0.11 (C8) | -34.50 → -33.47 |
| draw 0, 16 keys | (a), (b), (c) | 0.39 / 0.92 (A0) | 0.04 / 0.31 (C3) | 0.06 / 0.07 (C8) | -50.25 → -49.33 |
|  | (d) balanced | 0.04 / 0.28 (A0) | 0.01 / 0.02 (C#3) | 0.04 / 0.08 (C8) | -17.28 → -17.00 |
|  | (d) pure 12ths | 0.01 / 0.16 (A0) | 0.01 / 0.02 (C#3) | 0.04 / 0.08 (C8) | -10.76 → -10.60 |
|  | (d) model-B | 0.10 / 0.49 (A0) | 0.01 / 0.06 (C3) | 0.05 / 0.08 (C8) | -17.86 → -17.37 |
| draw 1, 16 keys | (a), (b), (c) | 0.35 / 0.42 (F#1) | 0.02 / 0.18 (C3) | 0.03 / 0.03 (C8) | -48.56 → -48.80 |
|  | (d) balanced | 0.03 / 0.07 (A0) | 0.02 / 0.02 (E3) | 0.01 / 0.04 (C8) | -14.14 → -14.07 |
|  | (d) pure 12ths | 0.04 / 0.08 (F1) | 0.02 / 0.02 (C4) | 0.02 / 0.04 (C8) | -16.59 → -16.53 |
|  | (d) model-B | 0.08 / 0.12 (F#1) | 0.02 / 0.06 (C3) | 0.02 / 0.04 (C8) | -14.67 → -14.64 |
| draw 2, 16 keys | (a), (b), (c) | 0.48 / 0.59 (G#1) | 0.04 / 0.30 (C3) | 0.05 / 0.06 (C8) | -50.37 → -50.50 |
|  | (d) balanced | 0.13 / 0.21 (F1) | 0.08 / 0.11 (B3) | 0.06 / 0.08 (A#5) | -21.91 → -21.88 |
|  | (d) pure 12ths | 0.10 / 0.11 (C1) | 0.09 / 0.11 (C4) | 0.06 / 0.08 (A#5) | -11.91 → -11.98 |
|  | (d) model-B | 0.19 / 0.26 (F#1) | 0.07 / 0.13 (C3) | 0.06 / 0.07 (B5) | -22.51 → -22.48 |
| draw 3, 16 keys | (a) | 0.33 / 0.42 (B1) | 0.03 / 0.23 (C3) | 0.04 / 0.04 (C8) | -52.24 → -51.95 |
|  | (b), (c) | 0.13 / 0.74 (A#2) | 0.15 / 1.23 (G#5) | 1.47 / 2.04 (D#6) | -49.40 → -49.36 |
|  | (d) balanced | 0.33 / 0.53 (C#1) | 0.09 / 0.57 (G#5) | 0.56 / 0.83 (D6) | -22.80 → -23.21 |
|  | (d) pure 12ths | 0.01 / 0.25 (A0) | 0.00 / 0.21 (G#5) | 0.30 / 0.41 (E6) | -16.74 → -16.49 |
|  | (d) model-B | 0.41 / 0.53 (D#1) | 0.12 / 0.70 (G#5) | 0.69 / 1.03 (D6) | -23.83 → -24.14 |
| draw 4, 16 keys | (a) | 0.26 / 0.65 (A0) | 0.02 / 0.17 (C3) | 0.02 / 0.02 (G#7) | -58.21 → -57.56 |
|  | (b), (c) | 0.12 / 0.72 (A0) | 0.07 / 0.16 (G#5) | 0.13 / 0.16 (A5) | -58.21 → -57.49 |
|  | (d) balanced | 0.00 / 0.33 (A0) | 0.00 / 0.00 (G3) | 0.02 / 0.03 (C8) | -18.63 → -18.30 |
|  | (d) pure 12ths | 0.05 / 0.67 (A0) | 0.01 / 0.01 (A#3) | 0.02 / 0.03 (C8) | -55.10 → -54.43 |
|  | (d) model-B | 0.05 / 0.41 (A0) | 0.01 / 0.04 (C3) | 0.02 / 0.03 (C8) | -20.49 → -20.08 |
| draw 0, 32 keys | (a) | 0.97 / 1.21 (B1) | 0.08 / 0.69 (C3) | 0.11 / 0.13 (C8) | -52.37 → -51.59 |
|  | (b) | 0.16 / 0.35 (D1) | 0.11 / 0.25 (D4) | 0.35 / 0.64 (C8) | -50.48 → -50.31 |
|  | (c) | 0.16 / 0.35 (D1) | 0.11 / 0.25 (D4) | 0.35 / 0.63 (C8) | -50.27 → -50.10 |
|  | (d) balanced | 0.03 / 0.90 (A0) | 0.01 / 0.01 (C3) | 0.07 / 0.15 (C8) | -17.49 → -16.59 |
|  | (d) pure 12ths | 0.04 / 0.92 (A0) | 0.01 / 0.01 (G5) | 0.05 / 0.15 (C8) | -18.19 → -17.27 |
|  | (d) model-B | 0.26 / 0.90 (A0) | 0.04 / 0.18 (C3) | 0.08 / 0.15 (C8) | -18.22 → -17.32 |
| draw 1, 32 keys | (a) | 0.45 / 1.73 (A0) | 0.05 / 0.36 (C3) | 0.07 / 0.08 (C8) | -52.27 → -50.54 |
|  | (b) | 0.27 / 0.39 (C#2) | 0.18 / 0.33 (G#4) | 0.14 / 0.25 (A5) | -52.54 → -52.29 |
|  | (c) | 0.27 / 0.39 (C#2) | 0.18 / 0.33 (G#4) | 0.15 / 0.25 (A5) | -52.97 → -52.72 |
|  | (d) balanced | 0.01 / 0.70 (A0) | 0.00 / 0.00 (C#3) | 0.03 / 0.09 (C8) | -24.04 → -23.34 |
|  | (d) pure 12ths | 0.01 / 0.98 (A0) | 0.00 / 0.00 (G5) | 0.03 / 0.09 (C8) | -21.93 → -20.95 |
|  | (d) model-B | 0.10 / 1.12 (A0) | 0.01 / 0.08 (C3) | 0.04 / 0.09 (C8) | -24.66 → -23.54 |
| draw 2, 32 keys | (a) | 0.40 / 1.72 (A0) | 0.05 / 0.33 (C3) | 0.06 / 0.07 (C8) | -52.20 → -50.47 |
|  | (b) | 0.14 / 0.56 (B2) | 0.11 / 0.58 (C#3) | 0.14 / 0.20 (E6) | -50.84 → -50.47 |
|  | (c) | 0.14 / 0.56 (B2) | 0.11 / 0.58 (C#3) | 0.14 / 0.20 (E6) | -50.83 → -50.46 |
|  | (d) balanced | 0.03 / 0.63 (A0) | 0.00 / 0.00 (D#3) | 0.04 / 0.08 (C8) | -20.52 → -19.89 |
|  | (d) pure 12ths | 0.02 / 0.87 (A0) | 0.00 / 0.00 (E3) | 0.03 / 0.08 (C8) | -20.33 → -19.46 |
|  | (d) model-B | 0.10 / 0.99 (A0) | 0.02 / 0.07 (C3) | 0.06 / 0.08 (C8) | -21.40 → -20.42 |
| draw 3, 32 keys | (a) | 0.53 / 0.71 (A0) | 0.06 / 0.41 (C3) | 0.08 / 0.09 (C8) | -51.68 → -50.97 |
|  | (b) | 0.39 / 2.01 (A1) | 0.81 / 2.45 (C#5) | 1.72 / 2.09 (D#6) | -52.31 → -50.97 |
|  | (c) | 0.39 / 2.02 (A1) | 0.82 / 2.46 (C#5) | 1.73 / 2.11 (D#6) | -52.59 → -51.24 |
|  | (d) balanced | 0.04 / 0.49 (A0) | 0.01 / 0.02 (C3) | 0.06 / 0.11 (C8) | -22.19 → -21.70 |
|  | (d) pure 12ths | 0.01 / 0.40 (A0) | 0.00 / 0.01 (D3) | 0.06 / 0.11 (C8) | -17.28 → -16.88 |
|  | (d) model-B | 0.14 / 0.55 (A0) | 0.02 / 0.10 (C3) | 0.06 / 0.10 (C8) | -22.42 → -21.87 |
| draw 4, 32 keys | (a) | 0.25 / 0.30 (A#1) | 0.02 / 0.16 (C3) | 0.03 / 0.03 (C8) | -50.42 → -50.37 |
|  | (b) | 0.12 / 0.16 (F1) | 0.02 / 0.07 (C3) | 0.05 / 0.09 (C8) | -49.42 → -49.45 |
|  | (c) | 0.12 / 0.16 (F1) | 0.02 / 0.07 (C3) | 0.05 / 0.09 (C8) | -49.26 → -49.29 |
|  | (d) balanced | 0.03 / 0.12 (A0) | 0.00 / 0.01 (C3) | 0.01 / 0.04 (C8) | -21.43 → -21.31 |
|  | (d) pure 12ths | 0.01 / 0.07 (A0) | 0.00 / 0.01 (C#3) | 0.01 / 0.04 (C8) | -11.39 → -11.32 |
|  | (d) model-B | 0.08 / 0.12 (A0) | 0.01 / 0.05 (C3) | 0.02 / 0.04 (C8) | -21.64 → -21.53 |

Nothing moves on the empty profile or on the five one-key draws (C1, G2, D♯4, F♯4 and
B4): with one point both fits keep the default pair. Each two-key draw has one row
because its six curves are the same: with two measured keys the smoother is skipped,
and no interval row of engine (d) formed between the two.

## Exploratory

**What moved the curve: the pair or σ_p.** The four combinations of each profile's
pair and σ_p, before and after, run through a copy of the changed tree whose curve
start reads both from outside; the corner cells reproduce the harness's before and
after curves exactly. The maximum |change| from the unchanged curve, in cents: with
both, with the pair alone (σ_p held at its before value), and with σ_p alone:

| read | engine | bass: total / pair alone / σ_p alone | mid | treble |
| --- | --- | --- | --- | --- |
| piano #2 | (b) | 0.10 / 0.09 / 0.01 | 0.05 / 0.05 / 0.01 | 0.65 / 0.26 / 0.39 |
|  | (c) | 0.09 / 0.09 / 0.01 | 0.05 / 0.05 / 0.01 | 0.64 / 0.25 / 0.39 |
|  | (d) balanced | 0.58 / 0.58 / 0.00 | 0.00 / 0.00 / 0.00 | 0.09 / 0.09 / 0.00 |
|  | (d) pure 12ths | 0.36 / 0.36 / 0.00 | 0.01 / 0.01 / 0.00 | 0.08 / 0.07 / 0.00 |
|  | (d) model-B | 0.56 / 0.56 / 0.00 | 0.10 / 0.10 / 0.00 | 0.10 / 0.10 / 0.00 |
| piano #2, extended | (b) | 0.14 / 0.14 / 0.00 | 0.09 / 0.09 / 0.00 | 0.51 / 0.49 / 0.03 |
|  | (c) | 0.14 / 0.14 / 0.00 | 0.08 / 0.08 / 0.00 | 0.50 / 0.47 / 0.03 |
|  | (d) balanced | 0.75 / 0.75 / 0.00 | 0.01 / 0.01 / 0.00 | 0.12 / 0.12 / 0.00 |
|  | (d) pure 12ths | 0.57 / 0.57 / 0.00 | 0.00 / 0.00 / 0.00 | 0.11 / 0.11 / 0.00 |
|  | (d) model-B | 0.61 / 0.61 / 0.00 | 0.11 / 0.11 / 0.00 | 0.12 / 0.12 / 0.00 |
| piano #1 | (b) | 0.77 / 0.77 / 0.00 | 0.17 / 0.15 / 0.04 | 0.27 / 0.26 / 0.16 |
|  | (c) | 0.66 / 0.65 / 0.00 | 0.14 / 0.13 / 0.03 | 0.23 / 0.25 / 0.15 |
|  | (d) balanced | 4.01 / 4.01 / 0.00 | 0.03 / 0.03 / 0.00 | 0.10 / 0.10 / 0.00 |
|  | (d) pure 12ths | 1.93 / 1.93 / 0.00 | 0.00 / 0.00 / 0.00 | 0.08 / 0.04 / 0.04 |
|  | (d) model-B | 4.11 / 4.11 / 0.00 | 0.14 / 0.14 / 0.00 | 0.10 / 0.10 / 0.00 |
| RWC Yamaha | (b) | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 | 0.01 / 0.01 / 0.00 |
|  | (c) | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 | 0.01 / 0.01 / 0.00 |
|  | (d) balanced | 0.01 / 0.01 / 0.00 | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 |
|  | (d) pure 12ths | 0.01 / 0.01 / 0.00 | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 |
|  | (d) model-B | 0.01 / 0.01 / 0.00 | 0.00 / 0.00 / 0.00 | 0.00 / 0.00 / 0.00 |
| RWC Bösendorfer | (b) | 0.05 / 0.05 / 0.00 | 0.05 / 0.05 / 0.00 | 1.39 / 1.37 / 0.05 |
|  | (c) | 0.04 / 0.04 / 0.00 | 0.04 / 0.04 / 0.00 | 1.09 / 1.08 / 0.03 |
|  | (d) balanced | 0.36 / 0.36 / 0.00 | 0.02 / 0.02 / 0.00 | 0.50 / 0.50 / 0.00 |
|  | (d) pure 12ths | 0.36 / 0.36 / 0.00 | 0.02 / 0.02 / 0.00 | 0.50 / 0.50 / 0.00 |
|  | (d) model-B | 0.44 / 0.44 / 0.00 | 0.24 / 0.24 / 0.00 | 0.48 / 0.48 / 0.00 |
| RWC Steinway | (b) | 0.04 / 0.04 / 0.02 | 0.03 / 0.03 / 0.02 | 0.27 / 0.03 / 0.26 |
|  | (c) | 0.03 / 0.04 / 0.02 | 0.03 / 0.03 / 0.02 | 0.26 / 0.03 / 0.25 |
|  | (d) balanced | 0.19 / 0.18 / 0.52 | 0.07 / 0.00 / 0.10 | 0.19 / 0.03 / 0.08 |
|  | (d) pure 12ths | 0.20 / 0.19 / 0.00 | 0.03 / 0.00 / 0.03 | 0.07 / 0.03 / 0.07 |
|  | (d) model-B | 0.20 / 0.19 / 0.01 | 0.07 / 0.01 / 0.07 | 0.19 / 0.03 / 0.19 |

Engine (a) reads no σ_p, and its change is the pair's alone on every read, so its
rows are left out. In the others the pair drives the bass everywhere. In the treble,
where the blend hands most keys back to the model with a weight that depends on σ_p,
σ_p drives the Steinway's (b) and (c) and more than half of piano #2's, and the pair
the Bösendorfer's, the extended captures' and most of piano #1's. The two effects do
not add: the Steinway's (d) balanced bass moves 0.52 ¢ under σ_p alone and 0.19 ¢
under both.

**Partial profiles against piano #2's complete profile.** How far each partial
profile's curve sits from the complete profile's curve of the same tree, in the bass
(A0–B2): the largest |difference| and A0's, in engine (a) and in (d) balanced:

| profile | (a): max \|Δ\| bass, before → after | (a): A0 − complete, before → after | (d) balanced: max \|Δ\| bass, before → after | (d) balanced: A0 − complete, before → after |
| --- | --- | --- | --- | --- |
| mid only | 38.86 → 37.60 | +38.86 → +37.60 | 6.20 → 4.92 | +6.20 → +4.92 |
| treble only | 30.52 → 37.60 | +30.52 → +37.60 | 9.42 → 5.26 | -1.48 → +5.26 |
| bass only | 0.66 → 0.00 | +0.19 → +0.00 | 4.36 → 4.84 | -4.26 → -4.84 |
| temperament octave | 41.02 → 37.60 | +41.02 → +37.60 | 8.59 → 5.06 | +8.59 → +5.06 |
| draw 0, 2 keys | 902.88 → 37.60 | -902.88 → +37.60 | 934.88 → 5.26 | -934.88 → +5.26 |
| draw 1, 2 keys | 665.32 → 37.60 | -665.32 → +37.60 | 697.32 → 5.26 | -697.32 → +5.26 |
| draw 2, 2 keys | 14.67 → 3.26 | -6.13 → +3.26 | 38.21 → 29.07 | -38.13 → -29.07 |
| draw 3, 2 keys | 8.33 → 37.60 | -8.33 → +37.60 | 40.32 → 5.26 | -40.32 → +5.26 |
| draw 4, 2 keys | 711.24 → 37.60 | -711.24 → +37.60 | 743.24 → 5.26 | -743.24 → +5.26 |
| draw 0, 4 keys | 38.68 → 37.60 | -38.68 → +37.60 | 70.67 → 5.26 | -70.67 → +5.26 |
| draw 1, 4 keys | 6.76 → 37.60 | -3.72 → +37.60 | 35.72 → 5.26 | -35.72 → +5.26 |
| draw 2, 4 keys | 43.69 → 37.60 | +43.69 → +37.60 | 11.69 → 5.26 | +11.69 → +5.26 |
| draw 3, 4 keys | 46.76 → 37.60 | -46.76 → +37.60 | 77.24 → 5.26 | -77.24 → +5.26 |
| draw 4, 4 keys | 2.03 → 37.60 | +0.73 → +37.60 | 31.72 → 5.00 | -31.72 → +5.00 |
| draw 0, 8 keys | 27.55 → 12.02 | -27.55 → -12.02 | 59.55 → 44.35 | -59.55 → -44.35 |
| draw 1, 8 keys | 1.79 → 0.83 | +0.89 → +0.69 | 31.11 → 31.64 | -31.11 → -31.64 |
| draw 2, 8 keys | 1.13 → 1.00 | -1.13 → +0.59 | 8.89 → 12.10 | -8.75 → -11.63 |
| draw 3, 8 keys | 4.11 → 3.95 | -4.11 → -3.95 | 16.85 → 16.94 | -15.31 → -15.76 |
| draw 4, 8 keys | 1.34 → 0.35 | -1.34 → -0.10 | 15.83 → 15.46 | -15.83 → -15.46 |
| draw 0, 16 keys | 0.42 → 1.09 | +0.42 → +1.09 | 1.39 → 1.21 | +1.39 → +1.08 |
| draw 1, 16 keys | 2.10 → 1.62 | +2.10 → +1.62 | 4.53 → 4.02 | +4.53 → +4.02 |
| draw 2, 16 keys | 0.66 → 0.36 | +0.30 → -0.08 | 4.09 → 4.20 | -3.24 → -3.80 |
| draw 3, 16 keys | 1.57 → 1.53 | -1.57 → -1.53 | 4.13 → 5.13 | -4.13 → -5.13 |
| draw 4, 16 keys | 7.54 → 7.13 | -7.54 → -7.13 | 1.85 → 1.81 | +0.04 → -0.22 |
| draw 0, 32 keys | 1.70 → 1.17 | -1.70 → -1.17 | 1.97 → 1.96 | +1.18 → +1.49 |
| draw 1, 32 keys | 1.60 → 0.23 | -1.60 → -0.12 | 5.38 → 5.26 | -5.38 → -5.26 |
| draw 2, 32 keys | 1.53 → 0.21 | -1.53 → -0.05 | 1.85 → 1.80 | -1.85 → -1.80 |
| draw 3, 32 keys | 1.30 → 0.93 | -1.01 → -0.54 | 3.52 → 3.61 | -3.52 → -3.61 |
| draw 4, 32 keys | 0.52 → 0.06 | +0.25 → +0.05 | 2.85 → 3.22 | -2.76 → -3.22 |

The guard makes a partial profile's bass predictable, not right. With fewer than two
bass-bridge keys the curve is the curve with nothing measured, and piano #2's bass
sits near 4.8 times the default pair (report 0021), so engine (a)'s A0 on such a
profile is 37.6 ¢ sharp of the complete profile's, and (d) balanced's about 5 ¢; the
fit on every measured key had put engine (a)'s anywhere from 903 ¢ flat to 44 ¢
sharp. Three draws sat nearer the complete profile under the old fit in engine (a):
F♯1 and A3 (8.3 ¢ flat before, 37.6 ¢ sharp after); A1, A4, A♯4 and E5 (3.7 ¢ flat
before); and C♯1, G♯3, G♯4 and A♯4 (0.7 ¢ sharp before). Each holds one bass-bridge
key, and a line through it and some mid keys happened to land near this instrument's
bass. In (d) balanced the same three move from 40.3, 35.7 and 31.7 ¢ flat to 5.3, 5.3
and 5.0 ¢ sharp. That is one instrument, and what a partial profile's bass should be
before two bass keys are measured is the question report 0021's limitations already
name.

## Limitations

- **No ground truth.** No instrument has been tuned aurally and measured, so no curve
  here is right or wrong; the tables say what moved.
- **The lab admits auto-mode captures for validation.** `curve compare` reads every
  regen entry as an auto-mode capture through `CurveInput::from_profile_including_auto`;
  the app's curve reads only trusted captures. The entries each key resolves to are the
  same.
- **The RWC pianos are recordings, not the app's own captures**: published single
  notes, converted and measured with the app's estimator 116 ms after the measured
  onset (report 0021), never through the app's microphone.
- **The partial profiles are one instrument's**, piano #2's, cut from its complete
  profile; a session's partial profile is taken in whatever order the tuner strikes.
- **The scale is thin in the treble.** The leave-one-key-out error rests on one key in
  the treble of the extended captures and of the Steinway, and on two to five
  elsewhere, and its reference is the octave chain.
- **The defect's numbers are the old fit's**, measured through `curve compare`'s input;
  the app's curve would have read the same keys from a trusted profile.

## Reproduce

- `cargo lab mat regen diagnostics_piano_1 > p1_regen.json`; piano #2's, the extended
  store's and the RWC pianos' dumps are the ones report 0021 read.
- `cargo lab curve compare <dump> --json <out>` on each; a partial profile is piano #2's
  dump with only its keys' entries kept, in order. The before side is the same command
  on the tree at `cb45a5d`.
- `cargo lab engine lock diagnostics_piano2`, `diagnostics_piano_1` and the extended
  store give 568 / 595, 82 / 87 and 449 / 472; with `--regen` and each read's dump,
  567 / 595, 450 / 472, and on the RWC pianos 251, 240 and 245 of 264.
- `cargo test -p tuner-core --lib -- the_curve a_profile_off test_curve_b_shrinkage bass_bridge`.
- σ_p, the fit's points and the decomposition were read with research tooling kept
  outside the repository, which calls `curves::sigma_prior` and
  `curves::bass_bridge_fit` on `curve compare`'s input; every table above is copied
  from its output files.

## References

- Rigaud, F., David, B. & Daudet, L. (2013). A parametric model and estimation
  techniques for the inharmonicity and tuning of the piano. *JASA* 133(5), 3107–3118.
  doi:10.1121/1.4799806 — Eqs. 7–8 (the whole-compass model), Eq. 29 (the bass pair's
  fit), §III.B.1 (the universal treble pair).
- Goto, M., Balke, S. & Müller, M. (2026). RWC Instruments Database. Zenodo.
  doi:10.5281/zenodo.17170844.
