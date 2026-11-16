# report 0024 — the tuning curve's B, from the fitted curve for every key

Which B engine (d)'s interval widths should read, a key's own measurement blended with the instrument's fitted curve (report 0009) or the fitted curve alone (report 0022), and what the app does with a capture once its B no longer sets its own key's width.

## Where this stands

**Accepted (2026-11-09), as the owner's product judgment, not a measured result.** Five pre-registered measurements advanced no form of the blend. On 2026-10-28 the owner made it unavailable until it is shown analytically sound, or shown never to be; on 2026-11-05 engine (d)'s key-level row gate went too, because building B and balancing intervals are kept as separate stages. The change:

- **Engine (d)'s widths read the fitted curve** (`CurveBSource::Model`, now the default) for every key, as engine (a)'s chain already did. The blend stays in the code for engines (b) and (c) and the lab.
- **Engine (d)'s row gate goes.** A row exists wherever both coincident partials were measured and Giordano's sensitivity is finite and positive.
- **Engines (b) and (c) leave the app.** The worker no longer computes them; a profile saved with either chosen opens on the default engine.
- **The red ✗ reads the physical range of piano strings.** It marks a capture whose B lies above it, alongside the final curve's negative stretch; the blend's negative-stretch exclusion, which no longer touches the default curve, is no longer shown.
- **Keys from A4 up never enter the bass fit**, whatever their B.

Not landed here: the decade exclusion band, the between-sitting step, per-key variances, pooling, and anything about engine (d)'s weights or its λ.

## The question, and the state before

Report 0009 replaced a hard partial-count switch with a precision-weighted blend: a key's curve B is w·ln B_meas + (1 − w)·ln B_ξ, with w = σ_p²/(σ_p² + σ_m²), σ_m(n) = max(19.3·n⁻³, 0.0035) from the capture's partial count and σ_p the instrument's scatter about the fit. Report 0022 made the fit B_ξ̂ Rigaud's two-asymptote model with its bass pair fitted on the bass-bridge keys. Engine (d) read the blend in every interval width and admitted a row only where both keys were "measurement-dominated" (w ≥ ½). The question was whether the blend's weights hold up as measurements, read on eight data sets: piano #2 (two sittings), the Iowa Steinway, the MAPS Disklavier (ambient and close microphones), and the RWC Yamaha, Bösendorfer and Steinway.

## Pre-registered decision rule

Each of the five runs fixed its own rule before any real number was read, and each is summarized with its verdict below. This change had its own fixed checks, set before the code was edited:

- engines (a), (b) and (c) bit-identical to the tree before the change on six complete profiles;
- engine (d), both presets, equal to an independent patched copy that made the same change, on the same six profiles;
- `engine lock` byte-identical with and without `--regen` (discovery is not touched);
- no `negative_stretch` flag on any profile.

## Population

The six complete profiles: piano #2's two sittings, piano #1 (one capture per key), and RWC's Yamaha, Bösendorfer and Steinway (`mat regen` dumps). The five runs read the eight data sets above, piano #1 aside. The red ✗'s range was scored on 2,276 captures of nine recording sets, those eight and piano #1, each replayed as its key's newest. No instrument was aurally tuned. Every rule was learned on these sets, so nothing here is confirmatory; the ✗'s edges and the A4 cut were set from published scales and the model, not tuned on them.

## Method

`cargo lab curve compare` on the six complete profiles, from a lab binary built at `58b6b80` and one built from the changed tree; `cargo lab engine lock` on the same sets with and without `--regen`; engine (d)'s curves compared with the stored output of an independent patched copy of `58b6b80` that made the same change. Before any other edit, two tests were added and seen to fail on the unchanged tree: the default B source is the fitted curve, and a key off the fit with its B divided by three moves neither engine (a) nor engine (d) by a bit. The ✗ was scored on harm, the strobe target error a capture's B causes at its displayed partial.

## Results

### The five runs

**AQ — does a measurement steer its key? NO FINDING.** The rule read the intraclass correlation R of a key's B across repeats, below the partial-count floor (about seven partials) and from it up: below, R's upper bound should sit under ½; from it up, the lower bound over ½. Every line failed, and the across-sitting check for the below-floor class failed by three decimals (r = +0.376 against a null 95th percentile of +0.377). One to five gross captures per read, B 33–525 times the model on bass keys, swallowed the key variance: the RWC Steinway's D1 at 524 times the model with 20 partials entered at a weight near one whenever drawn. Read robustly (exploratory), the from-floor-up class showed reproducible per-key structure on every read, and the below-floor class repeated within a session but not across sittings.

**TC-01 — a heavy-tailed weighting. NO FINDING.** Two models, a Student-t likelihood and an inlier/outlier mixture, each passed calibration on 5 of 16 read-classes. Both rejected all 32 captures a decade from the model with no threshold and added no flag; the D1 draw SD fell from 69.6 ¢ to 0.7 ¢. But one scale law cannot serve every instrument: the fitted t has ν = 0.835, heavier than Cauchy, absorbing a 0.46–2.6× spread of the law's scale between instruments, so it over-covers on the quiet RWC pianos and both sittings of piano #2 and under-covers on the Disklavier and Iowa. And within a sitting the below-floor captures agree with each other, so a likelihood fitted there trusts them more than the shipped law does.

**TC-02 — a decade band and a between-sitting term. NO FINDING as written.** Line 1 contradicted itself on one capture: piano #2's A♯6, 8 partials, a decade below the model, which one clause required excluded and another forbade. Read consistently (not pre-registered) the band alone would have advanced. What it established: the decade band removes exactly the 32 captures a decade from the model and nothing else, and the between-sitting term is a step at the floor, not a law: on piano #2's two sittings a key's deviation moves by τ = 0.78 at four partials or fewer and 0.69 at five to seven, against 0.0115 at eight to fifteen and 0.0065 at sixteen or more.

**TC-03 — per-key variances (a mixed model). STOP as written.** Neither candidate passed calibration on piano #2 from the floor up under the predictive the prompt fixed, which plugged the moderated variance in as if known. That error was the prompt's: the data give about one prior degree of freedom per key (d₀ = 0.70–1.18 on every read), so the model's own predictive is a Student-t, under which the same candidate covers 88.9 % and 88.3 % on the two failing cells. Read that way (not pre-registered), the outcome is NO FINDING. It established that each key's scatter is its own, not a scaled law.

**TC-04 — the complete model. NO FINDING.** Per-key t-variances calibrate on seven of the eight from-floor-up cells, missing MAPS ambient by one capture (38 of 51 against a region from 39). The step across sittings is located by the data, its size uncertain by about a factor of two. The model added six negative-stretch flags on piano #1 under engine (b), and its draw SD exceeded the shipped curve's on 3 of 15 dump-registers. The measurement phase closed.

**Where the rules themselves were at fault.** AQ's statistic was not robust to the gross captures that defeated it. TC-02's line 1 could not be satisfied on these data by any band. TC-03's predictive was the wrong form, and the run's own synthetic control had shown it covering 83–88 % where the right form covers 89–91 %. TC-04's line 3 had 43 % power under the true model, as its own audit found before the run. An independent cold read of all five then found:

- on the default engine the complete model equals AQ's floor candidate to about 0.1 ¢ register by register, so four runs of machinery arrived back at the floor plus pooling;
- the per-key variance machinery moves the curve by at most 0.04 ¢;
- the "third independent measurement" of the step was three analyses of the same two sittings of one piano.

### The guard's facts

The partial-count law's scale runs 0.039–1.43 across the eight recordings, so it does not calibrate across instruments. Below eight partials piano #2's second sitting reads B about a fifth lower than its first (mean ln B −0.235), where from eight up the sittings agree to 2 %. With engine (d)'s interval rows held fixed, swapping the blend for the fitted curve moves (d) balanced by 0.09–0.33 ¢ on the five complete profiles with more than one capture per key, and by 5.71 ¢ at A0 on piano #1, which has one.

### The checks

All 84 fixed checks hold. Engines (a), (b) and (c) are bit-identical on all six profiles; engine (d), both presets, equals the independent patched copy exactly; `engine lock` is byte-identical with and without `--regen` (lock totals as report 0022: 568, 82 and 449 without regen; 567, 450, 85, 251, 240 and 245 with); no profile has a `negative_stretch` flag. The fit's bass pair is unchanged on all six.

### Before and after

The largest |after − before| per register, in cents, at its key:

| profile | (d) balanced: bass / mid / treble | pure twelfths: bass / mid / treble |
| --- | --- | --- |
| piano #2 | 1.54 A0 / 0.25 D♯3 / 2.82 C8 | 1.67 A0 / 0.29 D♯3 / 1.48 C8 |
| piano #2, second sitting | 0.33 B2 / 0.33 C3 / 2.52 C8 | 0.47 A0 / 0.24 D3 / 1.19 C8 |
| piano #1 | 5.83 A0 / 0.23 C3 / 2.66 C8 | 4.86 A0 / 0.66 G♯5 / 0.69 C8 |
| RWC Yamaha | 0.07 A0 / 0.04 F♯3 / 1.44 D7 | 0.71 A0 / 0.06 D♯4 / 1.38 C8 |
| RWC Bösendorfer | 0.33 B2 / 0.37 E3 / 2.12 C8 | 0.22 B2 / 0.24 E3 / 0.17 D♯7 |
| RWC Steinway | 0.56 A0 / 0.09 E5 / 2.42 C8 | 0.06 C2 / 0.07 G♯5 / 0.82 C8 |

Most of the treble move is the gate's removal: the top octave is now balanced by the rows ending on the upper note's fundamental instead of following engine (a). Piano #1's A0 sits where its single captures read A0–C1 20–25 % below the fit and G1 2.5 times above it; one capture per key cannot separate strings from misreads. For scale, piano #2's two sittings differ by at most 0.81 / 0.22 / 0.46 ¢ (bass / mid / treble) under the new (d) balanced, against 1.04 / 0.31 / 0.46 ¢ before; under pure twelfths 0.17 / 0.16 / 0.16 against 1.31 / 0.29 / 0.54. On piano #2, A0 now lands at −50.4 ¢ under (a), −19.6 ¢ under (d) balanced and −14.1 ¢ under pure twelfths.

## Exploratory

Labelled, and deciding nothing on its own.

**The gate under the fitted curve.** A cold review compared the gate as shipped, a gate at eight partials, and no gate. The gate tested a B-stage grade (w ≥ ½), so its partial-count threshold moved with other keys' B (five to seven partials on these profiles, flipping between five and six across draws), and on the real path it inherited the blend's negative-stretch pre-exclusion. Without it, 20–25 top keys with two to six partials gain rows, and the top octave is set by the preset's rows instead of engine (a): C8 moves −1.0 to −2.7 ¢ (balanced) and up to +1.4 ¢ (pure twelfths). The rows touching keys with six or fewer partials are 26–34 % of all rows and carry 21–45 % of the weight, 0.74–1.33 times the average row: a count effect, not inflation.

**λ.** Under the fitted curve the interval targets carry no noise, so the leave-one-row-out score that picks λ scores conflicts between interval families: it changes by 6–18 % across λ = 10² to 10⁴ on the six profiles (balanced), and the pick steps whenever the set of rows changes. Each arm of `curve compare` picks its own λ, so the default and the lab's blend arm can differ by a λ step rather than by B: 4.99 ¢ at A0 on piano #2's second sitting, where the blend arm picks 31,623 and the default 3,162. λ also slides the bass toward engine (a): A0 under (a) / (d) at λ = 10² / (d) at the cross-validated λ is −50.4 / −16.8 / −19.6 ¢ on piano #2. Every interval kept regardless of audibility, at stated weights, puts C8 17.5–18.0 ¢ higher on every profile: the balance then honours coincidences of partials the top notes do not produce.

**The physical range behind the red ✗.** The top edge is ln(B/B₀) = 2.272 on the bass bridge (A0–B2) and 1.082 from C3 up, B₀ the medium-piano model, read from Conklin's measured and calculated scales (1996, Part III, Fig. 48: a 0.91-m upright and a 2.74-m grand, A0–C4 measured, the rest calculated, read at its ±2 ¢; Eq. A18 over a design envelope keeps the treble inside), widened by three of Rigaud's per-note scatter of ln B (0.157 below C5, 0.116 from C5; DAFx-11). That is ×15.5 the model on the bass bridge, ×4.73 from C3, ×4.18 from C5. Scored on harm, the strobe target error a capture's B causes at its displayed partial, over 2,276 captures of nine recording sets: twelve captures misplace the target by 226–761 ¢, all of them external bass readings where MAT's partial 1 caught a stray line. The top edge flags all twelve and one of the 2,264 others; adding the bottom edge flags 36 more that move no target, and in the treble a low B is missing information a recapture cannot mend. The negative-stretch exclusion the ✗ read before caught 3 of the twelve and flagged 5 others, and in 8 of 48 capture draws it marked piano #2's A♯6, E6 or B6, keys whose B moves no target. On the six complete profiles the new ✗ marks one key, piano #1's G1, a capture that misread its partials.

**Keys from A4 up and the bass fit.** The fit admits any key whose B reaches twice the treble term with at least eight partials, and on both measured pianos' fits the bass term is 3.6 % of the treble term at A4, so a key from A4 up passes only by reading high: 12 of 1,805 such readings across the nine sets do, at 2.1–5.4 times the treble term. On a profile with two bass captures, adding piano #2's E6 at 2.3 times the treble term (nine partials) spans the points past twelve keys, switches the fit from level-only to level and slope, and moves the curve by up to 2.2 ¢; with four bass captures across A0–B2 the least-absolute-deviation fit outvotes it. On the six complete profiles no key from A4 up is a point today. An independent review found the cut sound, with one caveat: the weakness is the span rule, which the cut closes only from A4 up (C3–G♯4 stays open; no such reading among 1,058 there).

**The lab's blend arm is not the old curve.** The gate went for both B sources, so `curve compare`'s blend arm differs from the curve at `58b6b80` by up to 5.16 ¢ (piano #2's second sitting, A0: the blend arm at −24.22 ¢ against −19.06 ¢). Engine (d)'s rows in reports 0008, 0009 and 0022 reproduce at `58b6b80`, not on this tree.

**The worker's cost.** Without engines (b) and (c) a curve bundle computes in 96–106 ms (median of seven, three profiles, release build), against 1.6–1.9 s before, nearly all of it engine (d)'s two presets.

## Limitations

- No aurally tuned instrument, so no curve here is shown better than another.
- The eight data sets are spent: every rule was learned on them.
- Piano #1 has one capture per key.
- The external recordings' tuning history is unknown.
- The physical range rests on one designer's scales and one estimate of the model's scatter; a piano outside them would read as a misread.

## What stays open

Engine (d)'s weights, which coincidences count in each register, and its λ (TODO.md, *What engine (d) balances, and how smoothly*); which engine to offer, since on piano #2 A0 lands at −50.4, −19.6 and −14.1 ¢ under (a), (d) balanced and pure twelfths; and a bass fit that weighs each reading by what it can say instead of trusting it whole (TODO.md, *The bass-bridge fit takes each reading at face value*).

## Reproduce

At the commit that lands this report:

```bash
cargo lab curve compare <dump.json> --json <out.json>    # every engine, flags, the blend arm
cargo lab engine lock <capture set> [--regen <dump.json>]
cargo test -p tuner-core the_default_curve_reads_the_fitted_b
cargo test -p tuner-core a_measured_b_off_the_fit_does_not_move_the_default_curve
cargo test -p tuner-core a_treble_reading_never_enters_the_bass_bridge_fit
cargo test -p tuner-core the_physical_range_flags_only_above_its_top_edge
```

The curve before this change is at `58b6b80`.

## References

- Conklin, H. A. (1996). "Design and tone in the mechanoacoustic piano. Part III. Piano strings and scale design." JASA 100(3), 1286–1298.
- Rigaud, F., David, B., & Daudet, L. (2011). "A parametric model of piano tuning." Proc. DAFx-11.
- Rigaud, F., David, B., & Daudet, L. (2013). "A parametric model and estimation techniques for the inharmonicity and tuning of the piano." JASA 133(5), 3107–3118.
