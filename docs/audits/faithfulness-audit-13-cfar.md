# Faithfulness audit 13 — `peaks.rs` `coarse_read` (OS-CFAR) vs Rohling 1983

**Series:** wave 3, item 1 (Prompt P step 1). The first audit of code written
*after* waves 1–2 — the coarse readout shipped 2026-07-25 (ADR 0011).
**Date:** 2026-07-26.
**Source of truth:** H. Rohling (1983), "Radar CFAR Thresholding in Clutter and
Multiple Target Situations," IEEE Trans. Aerospace and Electronic Systems
AES-19(4), pp. 608–621, DOI 10.1109/TAES.1983.309350 — primary source read
(`resources/tracker/Rohling.pdf`: §III p. 611, §IV pp. 615–616 incl. Table II,
§V pp. 618–620, Appendix p. 620). Finn, H. M. & Johnson, R. S. (1968), RCA
Review 29(3) (`resources/tracker/RCA-Review-1968-09.pdf`) is cited by the module
as the cell-averaging predecessor only — a lineage claim, checked and correct.

**Symbol note (2026-08-07):** `COARSE_P_FA` is now `CFAR_P_FA`. A second
OS-CFAR gate in the same file — the unison line estimator's, ADR 0012 — makes the
same 0.001 commitment, and one false-alarm budget per file is the honest shape;
the value and its justification below are unchanged. Everything else audited here
keeps its name.

**Scope:** `coarse_read`, `cfar_multiplier`, and the gate constants
(`COARSE_CFAR_QUANTILE`, `COARSE_CFAR_GUARD_BINS`, `COARSE_CFAR_FLANK_SPACINGS`,
`COARSE_CFAR_FLANK_MIN_HZ`, `COARSE_P_FA`, `COARSE_CFAR_MIN_REFS`), the
search-loss correction, and the three Hann-independence factors. The
band-geometry constants (`COARSE_SPAN_CENTS`, `COARSE_SPAN_MIN_BINS`) make no
claim on this paper and are handled separately (Prompt P step 5).

## Paper specification (as read)

**§III, p. 611 — what an OS detector selects.** "In practice the proposed method
is performed by rank-ordering the values encountered in the neighborhood area
according to their magnitude and by selecting a certain predefined value from the
ordered sequence. **This can be the median, the minimum, the maximum, or any
other value.**" Design idea (3): "The desired insensitivity to violation of the
above statistical assumptions can be obtained by using **quantiles** instead of
statistical moments as clutter power estimators."

**§IV, p. 616 — Eq. 14** (ranks ascending; the Appendix fixes `X_(1)` = minimum,
`X_(N)` = maximum):

```text
  P_fa = k·C(N,k) · (k−1)! (T+N−k)! / (T+N)!
```

**Table II, p. 616.** `T` for `P_fa = 10⁻⁶`, **square-law** detector,
`N ∈ {8, 16, 24, 32}`, `k = 1…N`.

**§V, p. 619 — the rank design rules.** "For CA and CAGO CFAR reference window
sizes of about N = 16 … 24 are commonly used. For OS CFAR window sizes of about
N = 24 … 32 and more are applicable… The rank-order parameter k of the OS CFAR
procedure should be **greater than N/2** which results in selecting a value
greater than the median from the ordered sequence and has the effect of expanding
the clutter areas. On the other hand, **the difference N − k should not be less
than double the target length** in order to avoid two targets from being mutually
blanked." And: "In this context 'minor' means anything that **affects less than
(N − k) resolution cells**." Fig. 10 caption: "For k > N/2 the clutter area
appears extended (dilatation), for k < N/2 it appears shrunk (**erosion**)."
Continuing on p. 620: "For N = 32, e.g., k should be chosen between N/2 and
3N/4."

**§V, p. 620 — guard cells.** "In practical CA CFAR application, guard cells are
used for separating the cell under test from the reference area in order to
prevent target returns from falsifying the clutter level estimation; see Fig.
3(a). In OS CFAR processing **these guard cells become unnecessary** since a
small number of target amplitudes occurring within the reference area have almost
no influence on the clutter level estimation by quantiles… Therefore a reference
window without guard cells can be used with OS CFAR processing." Figs. 3(b) and
9(b) show the two window definitions side by side — with guard cells for CA/CAGO,
without for OS.

**§V, p. 620 — Eq. 17.** `P[Y_q ≥ T_q Z_q] = P[√Y_q ≥ √(T_q Z_q)] = P[Y_lin ≥
T_lin Z_lin]` ⇒ **`T_lin = √T_q`**, for the case where "in practice the absolute
value (linear detector) is most frequently used" and the cells "obey a Rayleigh
distribution". Scope note: "This simple conversion, however, **does not apply for
CA or CAGO CFAR**."

## Verdict summary

| # | Item | Classification |
| --- | --- | --- |
| 1 | `cfar_multiplier` product form vs Eq. 14 | (a) faithful — reduction verified algebraically **and** against Table II (new test) |
| 2 | Eq. 17 `√` conversion for Rayleigh magnitudes | (a) faithful — the paper's scope note puts OS-CFAR on our side; now quoted in the code |
| 3 | `T_sq` in the doc-comment | (c) symbol drift → **FIXED** to the paper's `T_q` |
| 4 | "Rohling's own choice is the median" | (c) **FALSE ATTRIBUTION → FIXED** |
| 5 | `COARSE_CFAR_QUANTILE = 0.25` | (b→a) **upgraded**: derived from §V's `(N − k)` criterion, not ours |
| 6 | Shipping `k < N/2` against the paper's `k > N/2` | (b) deliberate, now *justified by the same criterion* — H₀ inverts |
| 7 | `COARSE_CFAR_GUARD_BINS = 2` | (c) CA-CFAR rationale on an OS detector → measured inert → **REMOVED** per §V |
| 8 | `COARSE_CFAR_FLANK_MIN_HZ` mechanism ("finds the valleys") | (c) **mechanism wrong exactly where it is load-bearing → FIXED** |
| 9 | Search-loss `P_fa / M` correction | (b) ours, documented (ADR 0011 §5) — the paper's P_fa governs one CUT |
| 10 | Three Hann-independence factors of 2 | (b) ours, calibrated as a **composite**; not independently tunable |
| 11 | `COARSE_CFAR_MIN_REFS = 4` | (b) ours — a refusal floor (`T_lin ≈ 45`), not an operating point |
| 12 | `COARSE_P_FA = 0.001` | (b) ours by project convention; Table II's 10⁻⁶ is not our operating point |
| 13 | Bisection interval `[0, 1e6]` | (b) ours — bounds `T_q`; unreachable in use, now stated |
| 14 | Rank convention (0-indexed `rank` → his 1-indexed `k`) | (b) ours, conservative by half a cell |

**No math defects.** One false attribution, one wrong mechanism, one symbol
drift — all corrected; the load-bearing constant promoted from bespoke to
derived; and one constant **deleted** as inert. No value changed.

## Findings

### 1–3. The core port is faithful, and Table II now pins it

Eq. 14's prefactor telescopes for integer `k`:

```text
  k·C(N,k)·(k−1)!·(T+N−k)!/(T+N)!
    = N!/(N−k)! · (T+N−k)!/(T+N)!
    = ∏_{j=0}^{k−1} (N−j)/(T+N−j)
```

which is exactly the product the code evaluates. Better than re-deriving it: the
paper prints the answers. `coarse_cfar_multiplier_table_ii` squares our output
(inverting Eq. 17) and reproduces **all 32 `N = 32` entries** of Table II, plus
spot entries at `N = 8, 16, 24` including the paper's own worked example
(`N = 24, k = 17 → T = 18.6`), to the table's printed precision. `k = 1` is
excluded because `T_q = N/P_fa − N` reaches 3.2 × 10⁷ there, above the bisection
interval — unreachable in use (the widest shipped band puts `T_q` under
2.5 × 10⁴), and now stated rather than implicit.

The Eq. 17 caveat cuts our way: the `√` is valid *for OS-CFAR specifically* and
explicitly not for the cell-averaging variants. Quoted in the code so nobody
reuses it in the wrong place. The doc's `T_sq` is renamed to the paper's `T_q`,
with a note that `q` in that symbol means square-law, not the quantile the module
calls `q` everywhere else.

### 4. The median was never Rohling's choice — false attribution, fixed

`peaks.rs` documented the shipped quantile as "Ours, measured (ADR 0011) —
*Rohling's own choice is the median*." He makes no such choice. §III lists "the
median, the minimum, the maximum, or any other value" as the menu; §V recommends
`k ≈ 3N/4` and states `k > N/2`; `k = N/2` appears once, descriptively, as "the
so-called median filtering". This is the audit-04 / audit-06 defect class — a
constant justified against a claim the source does not make.

### 5–6. The quantile is derived, and the departure follows the paper's own rule

§V's criterion is that an inhomogeneity is tolerable only while it affects fewer
than `N − k` reference cells. Our interferer is the harmonic comb: with partial
spacing `s` bins and a Hann main lobe `W_lobe = 4` bins null-to-null, lobes
occupy `W_lobe/s` of the window, so

```text
  (W_lobe / s)·N  ≤  N − k        ⇒        k/N  ≤  1 − W_lobe / s
```

Measured (`--refset`, lobe occupancy from the ET frequency and the prior `B`; the
selection column from the audio):

| set | key | s (bins) | lobe % of cells | bound on k/N | selected cell is a lobe | level vs band peak |
| --- | --- | --- | --- | --- | --- | --- |
| piano 1 | A0 | 5.11 | 75.0 | **0.250** | 56 % | −18.8 dB |
| piano 1 | A#0 | 5.41 | 71.4 | 0.286 | 51 % | −23.6 dB |
| piano 1 | B0 | 5.73 | 68.6 | 0.314 | 40 % | −32.5 dB |
| piano 1 | C#1 | 6.44 | 60.4 | 0.396 | 37 % | −36.0 dB |
| piano 1 | D#1 | 7.22 | 53.6 | 0.464 | 23 % | −38.5 dB |
| piano 1 | F1 | 8.11 | 47.5 | 0.525 | 4 % | −38.4 dB |
| piano 1 | A1 | 10.22 | 37.5 | 0.625 | 5 % | −57.7 dB |
| piano 1 | C#2 | 12.87 | 20.5 | 0.795 | 0 % | −15.8 dB |
| guitar | E2 | 15.31 | 19.0 | 0.810 | 2 % | −37.7 dB |
| guitar | E4 | 61.23 | 2.7 | 0.973 | 0 % | −54.8 dB |
| piano 2 | A0 ×4 | 5.11 | 75.0 | **0.250** | 56–75 % | −28.8…−29.3 dB |
| piano 2 | A#0 ×5 | 5.41 | 71.4 | 0.286 | 49–60 % | ≈ −22.4 dB |
| piano 2 | C1 ×5 | 6.07 | 63.5 | 0.365 | 28–46 % | ≈ −35 dB |

**A0 is the binding case and the shipped 0.25 meets its bound with zero margin.**
The bound relaxes monotonically upward, so the deep bass sets the quantile for the
whole keyboard — and the median's measured failure (±400 ¢ junk, ADR 0011 §4) is
the criterion being violated for every key up to F1, not an empirical surprise.

The direction of the departure inverts the paper's `k > N/2` because **H₀
inverts**: a radar reference window is mostly clutter with a few interfering
targets; a harmonic reference window is mostly partials with few background
cells. The same inequality therefore binds from the other side. The cost is the
one Rohling names for `k < N/2` — erosion, i.e. under-estimation at an edge
(Fig. 10) — and ADR 0011 §5's realized-P_fa figures are the measurement of it
(0.00097 pooled against nominal 0.001; one 17–32-bin bucket at 0.0027).

### 7. Guard cells: the paper says unnecessary, it is right, and they are gone

§V states guards "become unnecessary" for an OS detector. We ship ±2 on the
CA-CFAR rationale the paper gives for CA/CAGO (signal spilling into adjacent
cells). Sweeping `guard_bins` 0…4 at the coarse partial:

| set (captures) | availability | \|e\| ¢ | jitter ¢ | median margin |
| --- | --- | --- | --- | --- |
| piano 1 (8, keys 0–16) | 85.1 % at every guard | 1.13 | 4.39 | 17.87 (17.93 at guard 4) |
| piano 2 (15, keys 0/1/3) | 99.8 % at every guard | 0.32 | 2.45 | 4.04 (4.11 at guard 4) |
| guitar (6) | 100 % at every guard | 0.34 | 1.05 | 59.98 |

Identical to the printed precision on three instruments — Rohling's claim,
confirmed. And the inertness is **structural**, not just empirical: references
come from *outside* the search band, so the only lobe cells that can enter are
those just past a band edge (always in the deep bass where the band is ≈ 5 bins,
never in the treble), and those are high magnitudes that sort above a low
quantile. Adding them cannot move `k = N/4`. That is Rohling's own argument.

**Removed** (user call, 2026-07-26). An inert item earns its place in this
codebase in three ways, and this had none of them: it is not a degenerate-input
backstop (`twm.rs`'s `a_max.max(1e-6)`, inert by design so a zero frame cannot
divide by zero); not a gated experiment with a named revisit condition
(`TwmConfig`'s four experimental fields, `APPLY_MEASURED_B_TO_DISCOVERY`); and
not a structural minimum (`COARSE_CFAR_MIN_REFS`, without which the order
statistic is undefined). It was a modelling constant imported from the detector
family whose author says it is unnecessary here.

Re-verified after removal: harness↔shipped parity **100.0000 %, Δf = 0** on all
three sets (36,523 hops per FFT size — both sides guard-free); realized AWGN
P_fa 0.00097 → **0.00102** against nominal 0.001, with the per-band-width
structure unchanged (5–8 bins 0.0000, 9–16 0.0010, 17–32 **0.0027**, > 32
0.0013 — the 17–32 bucket identical to ADR 0011 §5's recorded 2.7×); 2048 stays
over-conservative at 0.00000. 89 + 2 tests. The harness keeps its `guard_bins`
field to reproduce the sweep, and its **in-band control keeps ±2** — that variant
is the one case where the CUT's lobe does land inside the reference set.

### 8. The flank floor's recorded mechanism was wrong where it matters

ADR 0011 §4, `peaks.rs`, and the harness all said the wide flank "lets a low
order statistic find the valleys between partials". The table above refutes that
in the deep bass: at 5.11-bin spacing **75 % of reference cells are inside some
partial's main lobe**, leaving ~1.1 bins of valley per period that also sit on
both neighbours' −31 dB sidelobes — and the selected cell is a lobe cell in
56–75 % of A0 hops, 19–36 dB below the band peak. What 172 Hz buys is reach over
partials ≈ 1–11, i.e. the **weak upper** ones, and the low quantile lands on
their skirts.

From F1 upward the original claim is correct (selected cell is a valley cell in
≥ 95 % of hops). So the sentence was true in the register where the floor is
inert and false in the register where it is load-bearing. Corrected in all three
places.

This also settles the constant's status: the floor is an **amplitude-envelope**
property, not window geometry, so it is not reducible to a scale-free formula —
a (b) adaptation, bounded by being active only where `1.5 × spacing < 172 Hz`
(spacing under 115 Hz, ≈ key ≤ 25) and inert above. Prompt P's original
hypothesis — deriving it from the reference *count* needed for a stable order
statistic — is refuted on arithmetic: a count criterion at `q = 0.25` needs
≈ 1/q = 4 independent cells ⇒ 8 bins, which is precisely the setting measured to
collapse A0 availability.

**The unit is right, and that is a positive result rather than a concession**
(step 4). Three forms are available and only one does the job, which is to widen
the flank where the comb is dense and vanish where it is not:

| form | at A0 | at A4 | verdict |
| --- | --- | --- | --- |
| **Hz (shipped)** | 172 Hz | inert (1.5 × spacing wins) | does both |
| bins | 32 bins @ 8192 | — | a different physical width at each FFT size — already rejected in ADR 0011 §4 |
| partial spacings | 6.3 × 27.5 = 172 Hz | 6.3 × 440 = **2.8 kHz** | never turns off; a reference window from DC to 3 kHz around a 440 Hz partial |

The remaining scale-free candidate, "reach partial *m*", is refuted by
measurement: the partial the order statistic lands on is whichever is weakest at
that hop, and its index scatters over n1–n11 with no stable median across keys
(A0 med 2, A#0 9, B0 5, C#1 3, D#1 8, F1 7, A1 1). There is no *m*.

### 9–14. The remaining items are ours, and stay ours

- **Search loss.** Rohling's P_fa governs one cell under test; this detector
  takes an argmax over the band, so the budget is `P_fa / M`. No counterpart in
  the paper; measured closure in ADR 0011 §5.
- **Hann independence.** `m_eff = (hi−lo)/2` and `(n_ref/2, rank/2)` all assert
  that correlation halves the count. The textbook figure is Hann
  **ENBW = 1.5 bins** (Harris 1978), so 2 is conservative — but the *composite*
  lands on nominal (0.00097 vs 0.001), so the three factors are calibrated
  together and are not independently tunable. Recorded so a future reader does
  not "fix" one into a regression.
- **`COARSE_CFAR_MIN_REFS = 4`.** At four references the multiplier is
  `cfar_multiplier(2, 1, ·)` ⇒ `T_q = 2/p − 2` ⇒ `T_lin ≈ 45`: the gate is
  effectively closed. It is a refusal floor, not an operating point — Rohling's
  own OS-CFAR window sizes are `N = 24 … 32 and more`, and the shipped read
  typically has 57.
- **Rank convention.** Our `rank` is 0-indexed; his `k` counts from 1. The
  decimated call passes `(rank/2).max(1)`, i.e. half a cell below the exact
  mapping of `(rank+1)/2`. Lower `k` ⇒ higher `T`, so the rounding is
  conservative.

## What changed

**No value changed**, and one constant was removed. ADR 0011's measurements
therefore stand except where noted below, and parity was re-verified from both
sides after the removal.

- `peaks.rs`: **`COARSE_CFAR_GUARD_BINS` deleted** along with its filter, with
  §V quoted at the site so the absence is documented rather than merely true;
  quantile doc rewritten (false attribution removed, §V derivation and the
  measured A0 bound added, inversion + erosion cost stated); flank-floor mechanism
  corrected
  and its active register stated; `T_sq` → `T_q` with the collision note and the
  "not for CA or CAGO" scope quote; the module doc no longer calls the quantile
  ours.
- `peaks.rs` tests: `coarse_cfar_multiplier_table_ii` added.
- ADR 0011 §4: quantile and guard entries annotated with this audit; flank-floor
  mechanism corrected.
- `examples/pitch_ground_truth.rs`: `--refset` mode added (T5 — reference-set
  anatomy + guard sweep); `ref_window` extracted so the anatomy report cannot
  drift from the gate it describes; `shipping_gate_hz` and the two flanking sweeps
  set to `guard_bins: 0` to track the shipped read (the in-band control keeps 2);
  two mechanism doc-comments corrected; four pre-existing clippy lints fixed.

## Reproduction

```bash
cargo test -p tuner-core --release coarse_cfar          # Table II + the pinned ranks
cargo run --release --example pitch_ground_truth -- diagnostics_piano_1 --refset --keys 0,1,2,4,6,8,12,16
cargo run --release --example pitch_ground_truth -- diagnostics --refset
cargo run --release --example pitch_ground_truth -- diagnostics_piano2  --refset --keys 0,1,3
cargo run --release --example pitch_ground_truth -- diagnostics_piano_1 --verify-shipped
cargo run --release --example pitch_ground_truth -- diagnostics_piano_1 --pfa --fft 8192
```

The guard sweep is reproducible after the removal because `CfarCfg::guard_bins`
survives in the harness; `guard = 0` is now the shipped configuration and the row
that must match `--verify-shipped`.

Part A's lobe occupancy is predicted geometry (ET frequency + prior `B`), so it
is identical for the same key across instruments; the selection column and the
guard sweep are measured from audio.
