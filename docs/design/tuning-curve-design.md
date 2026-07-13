# Manual-mode tuning curve — design note

**Status:** Agreed; **implemented 2026-07-09** (Prompt F) per §13 #8's flat
layout — engines a–d in `algorithms/tuning.rs`, the Rigaud B-model layer in
`algorithms/inharmonicity.rs` (repurposed in place; `calculate_b_value` and
the `linreg` dependency deleted), the Giordano engine in
`algorithms/dissonance.rs`, Whittaker + banded LS in
`algorithms/smoothing.rs`, `TuningCurve`/`CurveInput` + the blocking
`captured_in_auto` provenance flag in `models.rs`, §11 diagnostics in
`examples/curve_compare.rs`. Validation evidence for the (c) composition is
still pending (n = 1) — the (c) ADR remains unwritten by design (§12). This
note remains the Prompt C deliverable
(`next-chats-handoff.md`), expanded well beyond its original "Sethares vs
Hinrichsen" framing after research, primary-source audits, and empirical
dry-runs on the real captures. Prompt-C item mapping: objective functions → §3
and §6; cost/placement → §9; strobe targets → §7; treble → §8; validation → §11;
recommendation → §12.

**Where this fits:** manual mode measures every key the user names — the Worker's
MAT returns per-key $(f_0, B)$ plus the partial list (frequencies, amplitudes),
persisted as `KeyMeasurement` in `tuning_profile.json`. This note designs the
next stage: profile → **tuning curve** (a target pitch for all 88 keys) →
**strobe targets** (per-partial reference frequencies). Curve computation is
offline/cold-path; the live strobe stays deterministic. **Scope (user decision):
the tuning curve is a manual-mode-only feature until the auto algorithm is
finalized** — this subsumes the ADR 0006 item-3 provenance rule.

---

## 1. Problem statement and definitions

Equal temperament (ET) prescribes $f_{ET}(m) = 440 \cdot 2^{(m-48)/12}$ Hz for
key index $m$ (0 = A0, 48 = A4, 87 = C8). A real piano must not be tuned there:
string stiffness makes every partial sharp of its harmonic position, so
beat-free intervals require a **stretched** tuning. The tuning curve is the
deviation, in cents, of each key's target from ET:

$$d(m) = 1200 \cdot \log_2\!\big(f_1(m) / f_{ET}(m)\big)$$

**Convention (pin this in code):** $d(m)$ is defined on the **audible first
partial** $f_1$, per Rigaud Eq. 4 — what a tuner reads. The stiff-string model
$f_n = n f_0 \sqrt{1+Bn^2}$ (Rigaud Eq. 1; already in `models.rs` / `mat.rs`)
uses the *flexible-string* fundamental $f_0$, with $f_1 = f_0\sqrt{1+B}$. MAT
outputs $f_0$ in this convention. Mixing the two conventions is a silent
half-percent-of-B class of bug; all curve code converts explicitly.

Reference pitch: **A4 = 440 Hz default, user-settable later** — carried as
Rigaud's global deviation $d_g$ (his Eq. 32), a vertical offset of the whole
curve. Temperament: ET only.

Inputs available per measured key: `measured_f0` (Goertzel seed), MAT $(f_0,
B)$, partials (number, frequency, amplitude). Per ADR 0006 item 3 the curve
consumes **manual-mode captures only** (§10).

## 2. Physical foundations and the one hard constraint

With $B > 0$ (stiffness only ever raises partials; $B = \pi^3 E R^4 / 4\tau
L^2$), the lower note's $2\rho$-th partial sits sharp of $2\rho f_0$, so a
beat-minimized octave is always **wider** than 2:1. For the 2:1 octave type the
audible-fundamental ratio is

$$\frac{f_1(m{+}12)}{f_1(m)} = 2\sqrt{\frac{1+4B_L}{1+B_L}} > 2
\quad\Longleftrightarrow\quad B_L > 0,$$

unconditionally (the upper note's $B$ **cancels exactly** — see §8). For a
general octave type $\rho$ the first-order stretch is $(4\rho^2{-}1)B_L -
(\rho^2{-}1)B_U > 0$ whenever $B_U/B_L < (4\rho^2{-}1)/(\rho^2{-}1) \ge 4$; real
pianos' per-octave $B$ growth is $\lesssim 3.1\times$ (the treble asymptote
slope, $e^{12 \cdot 0.0926}$), so beat-minimized octaves are strictly stretched
across the compass.

**Hard constraint (D2, signed off):** $d(m{+}12) \ge d(m)$ for every key. A
computed *negative* octave stretch is definitionally an estimator artifact
(observed: Giordano scan edge-hits in the starved treble, §4). It is implemented
as a **validity detector — never a clamp** (internals/04): the offending
measurement is flagged and excluded, and the UI recommends recapture. Scope
honesty: the theorem holds *given* the program's standing premise that tuning =
beat/roughness minimization of a $B>0$ partial series.

## 3. Frameworks surveyed

### 3.1 Coincident-partial family — the backbone (faithful instance: Rigaud)

Terminology note: **"coincident partials" is the established piano-technician
trade term** (PTG literature; octave *types* 2:1 / 4:2 / 6:3 are *defined* by
which partial pair coincides), so the family name is recognizable to the
domain audience; academic papers say "partial matching" or "beat cancellation
between partials" for the same thing — synonyms on first use.

Principle: pick a partial pair $p{:}q$ per interval and tune it beat-free
(or to a tempered beat rate). Uses only partial *frequencies* — functions of
$(f_0, B)$, structural string properties — hence immune to strike strength,
decay, and amplitude noise. The cost: someone must *choose* the pair (the
"octave type"), which is where all tuning taste lives.

**Rigaud, David & Daudet 2013** (JASA 133(5); PDF at `resources/moba/`) is the
peer-reviewed, prescriptive instance, already trusted in this repo (its Eqs. 7–8
are `get_expected_beta`). Octave relation (his Eq. 6):

$$F_0(m{+}12) = 2\,F_0(m)\sqrt{\frac{1 + 4\rho^2 B(m)}{1 + \rho^2 B(m{+}12)}}$$

with octave type $\rho(m)$ following an erf-shaped published curve (Eq. 9,
$\rho_\phi(m) = \tfrac{\kappa}{2}(1-\mathrm{erf}\tfrac{m-m_0}{\alpha}) + 1$;
"typical" $\kappa{=}3.5, m_0{=}60, \alpha{=}25$, treble asymptote $\rho{\to}1$),
per-instrument $B_\xi$ fit by L1 (Eq. 29, treble pair fixed universal per Young
1952), A-chain from A4 + interpolation, $\rho$ *estimation from a given tuning*
by inversion (Eq. 30), mean model $\bar\rho_\phi \pm 1$ as high/low-stretch
variants (§IV.C.2). Validated on 6 pianos.

**Industry survey (2026-07-08, saturating):** every established product is in
this family. Sparse-sample parametric: TuneLab (≥4–6 notes, user-picked interval
per region), CyberTuner Chameleon (samples the **A's** — literally Rigaud's
A-chain — 10 style presets), Accu-Tuner FAC. Dynamic weighted multi-interval:
**Verituner (US Patent 6,529,843, read)** — per-note inharmonicity matrix,
Interval Prioritization table, iterative damped minimization of weighted
interval-width irregularity, full recalc after each measured note; PianoMeter
(1,200+ intervals incl. octaves/fifths/nineteenths, curve stabilizes as notes
accumulate, curve-lock UX); Pianoscope. Single-interval variant: OnlyPure
(Stopper pure-12ths, 3:1). Dirks measures all 88 keys, one string per key,
others muted (capture-protocol hint, §10). **None ships dissonance or entropy.**

### 3.2 Sensory dissonance — the perceptual layer (faithful recipe: Giordano)

**Giordano 2015** (JASA 138(4):2359–2366; PDF at `resources/worker/`): sum
Plomp–Levelt pure-tone roughness over **all** cross partial pairs of an interval,
weighted by amplitude product (his Eqs. 3–6, Sethares parametrization: $d_2 =
e^{-b_1 s \Delta f} - e^{-b_2 s \Delta f}$, $s = 0.24/(0.021 f_{\min} + 19)$,
$b_1{=}3.5$, $b_2{=}5.75$), notes normalized to equal total power; slide the
upper note (partial $n$ shifts by $n\,df$) and take the dissonance-minimum
offset. Reproduces Railsback quantitatively from one piano's measured spectra.
This is the *soft* generalization of §3.1: coincident-partial = all weight on
one pair; Giordano = perception-derived weights over all pairs. **It is the
principled way to derive the octave type that §3.1 otherwise assumes** — and no
commercial product does it (novel contribution → ADR, §12).

Three first-principles costs: (1) it is an *interval* method, not a curve
method (Giordano measured only A/C/E/G; 88 keys need chaining/fill); (2)
amplitude weights make it capture-condition dependent (offline computation from
the stable window neutralizes the *live* instability objection, but strike
strength/decay timing still enter — unquantified, research item §11); (3)
intrinsic fragility where partials are few: with 3–6 treble partials the
dissonance well is shallow or edge-hits (observed, §4) — an information floor,
not a data-quantity problem.

### 3.3 Entropy — ruled out (structural, user-confirmed)

Hinrichsen 2012 (arXiv:1203.5101, read in full): Shannon entropy of the
A-weighted, 1-cent-log-binned sum of all 88 keys' power spectra, minimized by
zero-temperature Monte Carlo over 88 offsets. Requires ~20 s recordings; our
captures are 1.5 s (0.67 Hz resolution vs the ~0.03 Hz a bass cent spans), we
persist partial lists not spectra (synthetic reconstruction would be a bespoke
assembly), and the author reports non-reproducible local minima. The octave-wise
2020 variant (Szwajcowski & Pilch, Applied Acoustics) does not change the input
mismatch. Closed.

### 3.4 Scrutiny records folded into this design

- **Gemini deep-research report (audited, adopt-nothing):** its coincidence
  residual re-derives to the stiff-string equation (= already ours via Rigaud);
  its weight values are unsourced free knobs (rejected); its smoothness term
  penalizes Hz-curvature — fails scale-invariance (weights C8 over A0 by
  $\sim(4186/27.5)^2 \approx 2.3{\times}10^4$) and even penalizes ET's own
  exponential shape — salvageable only in cents-space (§5); its B-value tables
  contradict our measured instrument and invite the banned clamp anti-pattern
  (rejected). Its one durable service: the strobe rendering sketch (§7).
- **Gràcia & Sanz-Perela 2017** (arXiv:1603.05516, read): rigorous stiff-string
  derivation; *scale-step* method (single semitone ratio, constant-$B$
  mid-keyboard scope) — not a curve method. Its Eq.-24 weight vector
  $w=(1,1,4,4,5,2,6,4,4,2,1,10)$ is a within-octave interval-role weighting,
  chosen ad hoc by the authors (robustness-checked) → citable *precedent* for
  musical-importance weighting, **not** a source of $W$ numbers. Its conclusion
  (tune by 3:2 fifths / their $\mathcal{A}_{3,2}$) and OnlyPure's pure 12ths
  both motivate including non-octave intervals in engine (d).

## 4. Empirical groundwork (what was measured before this note)

**Stale-data correction (decisive).** The 87 `diagnostics/key_*/analysis.json`
files (2026-05-30) predate the Serial-MAT default: partial lists capped at 12
(the old `SIM_MAX_PARTIALS`) and bass $B$ inflated ~2× (A0: stale
$1.44{\times}10^{-3}$ vs current $6.7{\times}10^{-4}$). All keys kept
`audio.raw`, so `examples/regenerate_partials.rs` (new, uncommitted) re-derives
partials with the shipped Worker path: **bass 30–32 partials (median 32 — hits
the `MAX_PARTIALS` ceiling), mid median 18, treble 3–6 (physics, not a cap)**.
Giordano's bass needs ~16/8 (his §VI.C) — met with headroom. No capture-code
change needed; do **not** raise `MAX_PARTIALS` for the curve (high-$n$ partials
are weak and noise-risky; the $n^2$ B-leverage question is separately gated on
instrument #2).

**Head-to-head dry-run on regenerated data** (Python prototypes, session
scratchpad; to be re-implemented as the Rust comparison harness, §11):

| diagnostic | Rigaud typical $\bar\rho$ | Rigaud, Giordano-calibrated $\rho$ | Giordano-pure |
| --- | --- | --- | --- |
| bass stretch (median ¢/oct) | 20.97 | — | 9.75 |
| $d$(A0) / $d$(C8) ¢ | −88.7 / +17.8 | −50.8 / +20.0 | −27.5 / +28.5 |
| roughness (median adjacent-key jump ¢) | 0.29 | 0.34 | **3.25** (max 118) |

Readings: (i) Giordano systematically prescribes *gentler* stretch than Eq. 6
under the grand-derived typical $\bar\rho$ — the perceptual pull that makes his
paper reproduce Railsback; on this high-$B$ upright the typical-$\bar\rho$ bass
lands at −89 ¢ (implausibly wide) vs Giordano-pure's −27.5 ¢ (Railsback-like);
(ii) the calibration mechanism works end-to-end (Eq.-30 inversion + Eq.-9 refit
gave $\kappa{=}4.10, m_0{=}32, \alpha{=}60$ on this piano); (iii) Giordano-pure
is **unusable raw** — per-octave independent optima propagate noise (3.25 ¢
median jag) and the starved treble edge-hits produce unphysical compressions
(§2's detector catches these). Caveats: single captures, auto-mode provenance,
n=1, **no ground truth** — these numbers are diagnostics, not selection
evidence (§11).

## 5. Locked decisions

**D2 (signed off).** Negative-stretch validity detector (§2). Smoothness is a
**prior, not a realism claim**: real aural curves fluctuate (Giordano Fig. 4;
Hinrichsen's conjecture that fluctuations are meaningful), but on single
captures a real anomaly and estimator noise are indistinguishable, and $B(m)$
continuity (incremental string design, Rigaud §II.B.2) justifies shrinking
toward smooth until **reproducibility evidence** (repeat captures — a research
experiment, not a user feature) discharges it. The penalty lives in
**cents-space only**, on the **residual from the prior mean**, so a pure-ET
curve and a Rigaud-shaped bow cost zero; known limitation: a uniform penalty
attenuates *real* discontinuities (bass break) — first-order mitigated because
the Rigaud prior itself carries the break shape additively.

**D3 (resolved, conditionally objective).** Smoothing coordinate = **d-space
residual-from-prior** (option ii). Forced by the roadmap: B-space smoothing
cannot ingest Giordano-derived targets (they don't come from $B$), and in-solve
penalties exist only inside engine (d). B-space survives *only* as the prior
generator ($B_\xi$ fit) and treble fallback. **Strobe/curve B split (new design
rule):** strobe partial targets always use the key's **raw measured** $B$ —
targets must match the physical string or partial-$n$ shows a false beat when
$f_0$ is correct; any smoothed $B$ is curve-input only.

**Smoothness, defined.** Magnitude of the second difference (discrete
curvature) of $d(m)$ in cents. Second-order, not first: a first-difference
penalty fights the Railsback slope itself; a second-difference penalty passes
any straight trend free and charges only bending — bow is cheap, jitter is
expensive. The fit minimizes $\sum_m w_m (r_m - \hat r_m)^2 + \lambda \sum_m
(\Delta^2 \hat r_m)^2$ with $r = d - d_{\text{prior}}$ — the **Whittaker
smoother** (Whittaker 1923; Eilers 2003), a pentadiagonal linear solve with
fast leave-one-out cross-validation for $\lambda$. Effective DOF =
$\mathrm{tr}[(W + \lambda D^\top D)^{-1} W]$ slides continuously from the prior
(λ→∞) to per-key data (λ→0): **the curve earns detail as captures accumulate**
(the user's α resolution; PianoMeter's documented behavior). λ selected by
CV/L-curve — statistical model selection, categorically distinct from the
banned tune-on-the-87-captures.

> **Post-implementation refinement (2026-07-09, ADR 0007).** Two mechanics of
> this layer were corrected after the first implementation round, changing no
> decision above. (i) *Boundary reversion:* the pure second-difference penalty
> extrapolates the residual **linearly** through data-free regions (its null
> space is affine — observed as a −4.3 ¢/octave arithmetic progression above
> the last trusted key), contradicting §8's "treble rides the prior". Keys
> without data now carry pseudo-observations of the prior mean at weight
> $w_0 = 4\lambda/\ell^4$ — exponential reversion with derived length
> ℓ = 12 keys (the $B_\xi$ asymptotes' own e-folding scale). This also makes
> the λ→∞ = prior statement above literally exact (with pure $D^2$ that limit
> is the LS straight line). (ii) *Chain gauge:* the Eq.-6 chains'
> unidentifiable per-chain offsets are fixed by the minimum-norm
> (mean-centered residual) projection instead of pinning anchor keys to the
> prior — no fabricated residual-0 data points enter the smoother.
> Derivations, alternatives rejected, and harness before/after: ADR 0007.

**D1 (user decision).** Ship later; **implement all four engines as
independently runnable** so they can be measured against each other as the
pipeline matures, with **(c) and (d) as the primary study subjects**. The
subset structure (a ⊂ b ⊂ c, d generalizes) makes this cheap: shared
primitives, four thin compositions, one comparison harness.

## 6. The four engines

**Shared primitives** (stateless, `algorithms/`): stiff-string predictor
(exists); Eq.-6 octave stretch; $B_\xi$ L1 fit (Eq. 29; treble pair fixed);
$\rho_\phi$ evaluation (Eq. 9; erf via Abramowitz & Stegun 7.1.26, hand-rolled,
cited, tested — no new deps); Whittaker smoother (pentadiagonal solve + LOO-CV);
Giordano dissonance engine ($d_2$, amplitude-product $B_{ij} = a_i a_j$,
equal-power normalization, 1-D scan); Eq.-30 $\rho$ inversion; sparse banded
weighted-LS solver; negative-stretch detector. **Architecture (per user
direction):** the engines are *progressive compositions*, not four
implementations of one abstraction — (b) calls (a)'s functions plus the
smoother, (c) calls (b)'s plus the dissonance/inversion stage; a common entry
point exists only as far as the comparison harness needs one. All of this
**supersedes `algorithms/inharmonicity.rs`** (its deprecated `calculate_b_value`
is verified unused; the `linreg` dependency has no other user and is dropped
with it). Exact module layout: **to be agreed before building** (§13 note).

**(a) Rigaud-pure** *(faithful port, one paper end-to-end).* $B_\xi$ fit →
$\bar\rho_\phi(m)$ (paper's mean model; ±1 variants as the user's stretch
preset) → Eq.-6 A-chain from A4 → interpolation → $d_g$. ~3 effective DOF.
Bias-heavy/variance-light; known systematic risk: grand-derived $\bar\rho$
over-stretches this upright's bass (§4).

**(b) Per-key coincidence + Whittaker** *(componentwise faithful: Rigaud +
Whittaker/Eilers).* Eq.-6 stretches from **measured per-key** $B$ (ρ still
$\bar\rho$) → raw $d(m)$ → subtract (a)'s prior curve → Whittaker(λ by CV) →
add back. Realizes data-adaptive DOF. Honest note: **inherits (a)'s taste
bias** — measured B improves local fidelity, not the global stretch choice.

**(c) = (b) + Giordano-calibrated octave type** *(pipeline of faithful uses;
the composition is ours → ADR; primary study subject).* Where partials suffice
(bass/mid; sufficiency gate §13), per-octave Giordano scan → optimal widths →
Eq.-30 inversion → implied $\rho$ points → Eq.-9 3-parameter fit
(regularization weight by LOO-CV with the 1-SE rule — ADR 0008 Decision 3,
which is what makes the "strong regularization" intent precise) →
instrument-specific $\rho(m)$; treble rides the $\rho \to 1$
asymptote where Giordano is information-starved. Then as (b). Perceptual taste
enters **once, offline, as the octave-type selection** — never the live loop.

**(d) Weighted multi-interval least squares** *(components faithful; assembly =
industry practice, Verituner patent as the citable document; primary study
subject).* Key insight making this trivial to solve: in cents-space the system
is **linear**. For interval $(m, k)$ with coincident pair $p{:}q$, the beatless
width's deviation from ET is a *constant* computed from $B$:

$$c_{m,k} = 1200\log_2\!\Big[\tfrac{p}{q}\sqrt{\tfrac{1+B_m p^2}{1+B_{m+k}q^2}}\Big] - 100k$$

so each residual is $d(m{+}k) - d(m) - (c_{m,k} + \tau_k)$, with $\tau_k$ the
ET tempering offset ($\tau = 0$ for pure-target octaves 12 / twelfths 19 /
double octaves 24; $\tau_{\text{fifth}} = 700 - 1200\log_2(3/2) = -1.955$ ¢,
fourths analogous). Objective: $J(\mathbf d) = \sum W_{m,k}\,(\text{residual})^2
+ \lambda \sum (\Delta^2 (d - d_{\text{prior}}))^2$ — a sparse banded linear LS
(bandwidth ≤ 24), solved directly. Interval set includes the **3:1 twelfth** so
a pure-12ths (OnlyPure-style) preset is expressible. Weights $W$: **derived**
from Giordano pair contributions (bridge Form 2, implemented — ADR 0008
Decision 4: $W_{m,k} \propto a_p a_q (b_2{-}b_1) s(\bar f)\bar f \ln 2/1200$,
zero new parameters), with style presets surviving only as taste multipliers —
never silent magic numbers. Statistical advantage: interval
redundancy averages per-key $B$ noise before any smoothing.

> **Post-implementation refinement (2026-07-10, ADR 0008 — review Sets
> 2–3).** Giordano layer: the octave scan's fixed window (centred on the
> *current mistuned* interval) is replaced by the mistuning-independent
> **coincidence bracket** (hull of the pair's beatless 2j:j widths, j ≤ 7
> from the ρ-fit's own κ-domain, ±10 ¢ margin); the sufficiency gate is
> re-derived from Giordano §VI.C as **≥ 8 coincident pairs**
> (min(⌊N_low/2⌋, N_up) — verified against the PDF; the old above-median
> amplitude count is a demoted diagnostic); the Eq.-9 regularization
> constant is deleted for LOO-CV **with the one-standard-error rule** (ESL
> §7.10) — the ρ points are noise-dominated and end at key ~44, so the
> bare CV argmin let un-scored treble extrapolation set the (c) treble
> (±13 ¢ at A7); 1-SE breaks flat-CV ties toward the prior, restoring this
> section's ρ→1 claim. Engine (d): Form-2 weights are in, and they
> **genuinely explain** the BALANCED preset's bass gap rather than resolve
> it — deep-bass rows carry ~50× less perceptual weight than upper-mid
> (s(f̄)·f̄ falls toward low f; 30-partial power dilution), so under
> Giordano's own functional the deep-bass stretch is the prior/chain
> layers' job, not the interval data's. GCV is retired for the weighted
> fast-LOO identity + 1-SE (heterogeneous importance weights broke its
> equal-variance premise — observed 26 ¢ deep-bass kink at its λ pick).
> Deltas, probes, and derivations: ADR 0008.

## 7. Strobe targets (Prompt C item 3) and unison assist

From any engine's $d(m)$: target audible fundamental $f_1^\*(m) = f_{ET}(m)
\cdot 2^{d(m)/1200}$, target flexible fundamental $f_0^\*(m) =
f_1^\*(m)/\sqrt{1+B_{\text{raw}}(m)}$, and **per-partial strobe references**

$$f_n^\*(m) = n\,f_0^\*(m)\sqrt{1 + B_{\text{raw}}(m)\,n^2}$$

— one strobe band per tracked partial; the band freezes when the live partial
(the Engine's Goertzel/MQ tracker already provides per-partial frequency/phase)
matches its reference. **$B_{\text{raw}}$ = the key's own measured B, always**
(§5, D3). Every engine yields identical *structure* here; engine choice changes
only $d(m)$. Display semantics (absolute-partial vs interval-beat labeling,
which partials shown — CyberTuner's "Smart Partials" precedent) are **deferred**
until the strobe primer discussion. **Unison assist is near-free:** all 2–3
strings of a note tune to the *same* absolute targets, making them mutually
beatless transitively; a direct inter-string beat display is an optional later
refinement. Import from PianoMeter: a **curve lock** so the curve cannot shift
mid-fine-tuning.

## 8. Treble handling (Prompt C item 4)

Extreme treble is information-limited at the source: 3–6 partials exist below
Nyquist (measured, §4), so measured treble $B$ rests on ≤ 10 pairwise estimates
and MAT's own confidence is low. Policy: treble $B$ for **curve** purposes
falls back to the $B_\xi$ fit (universal treble pair, Young 1952 — the same
asymptote `get_expected_beta` ships).

**Why this is interpolation, not assumption (user question, answered):** the
$B_\xi$ fit is anchored to *this piano's own measured keys* — everything
through the upper-mid, where 5–18 partials still pin $B$ well — and the
universal treble *slope* holds across pianos because treble string design is
standardized (Rigaud's cross-piano finding). Only the top ~octave rides the
extrapolation, and that is exactly where the $\rho{=}1$ cancellation makes the
*curve* insensitive to those keys' own $B$: $f_1^{u}/f_1^{l} =
2\sqrt{(1+4B_L)/(1+B_L)}$ (§2) depends only on the lower-of-pair $B$. So the
curve algorithm itself already "handles" the treble — the fallback covers a
variable the curve barely consumes.

**Why a different estimator would not help:** the limit is information, not
method. PFD (Rauhala), inharmonic comb filtering (Galembo & Askenfelt), NMF
(Rigaud §III), or LS variants all consume the same 3–6 partial frequencies the
signal contains; none can create pairs that don't exist, and CSPE already
supplies super-resolution frequencies for the ones that do. Marginal physical
mitigations exist (96 kHz capture would admit ~2 more C8 partials below
Nyquist, but those partials are physically weak and fast-decaying) — not worth
the pipeline change. A future refinement, if wanted: *blend* measured treble
$B$ with $B_\xi$ weighted by pair count instead of hard-switching — noted, not
default. Giordano is excluded from the treble by the sufficiency gate (§13);
Rigaud's $\rho \to 1$ asymptote (pitch perception rides the fundamental above
~F6, his §II.B.3) covers it. Strobe rendering in the treble still uses raw
measured $B$ (§5) — with 3–6 partials the targets are few but exact.

> **Post-implementation refinement (2026-07-10, ADR 0009).** The "blend,
> noted, not default" above is now the shipped default, in its principled
> form: the instrument-#2 repeat-capture experiment measured
> $\sigma_{\ln B}(n) = \max(19.3\,n^{-3}, 0.0035)$ (capture-to-capture, per
> partial count $n$) and the per-instrument prior scatter $\sigma_p$
> (0.062 vs 0.186 on the two uprights — self-calibrated, robust MAD), so
> curve-side $B$ is the **precision-weighted (inverse-variance) blend**
> $\ln B_{curve} = w\ln B_{meas} + (1{-}w)\ln B_\xi$,
> $w = \sigma_p^2/(\sigma_p^2+\sigma_m^2)$ — the conjugate-normal posterior
> mean, with pair count replaced by measured precision. The
> `CURVE_B_MIN_PARTIALS = 8` hard switch is deleted; the ADR-0007 ±5 ¢
> threshold sensitivity is dissolved (no boundary exists) and the six (b)
> boundary flags vanish with it. Measurements, derivation, and harness
> before/after: ADR 0009.

## 9. Computational cost and placement (Prompt C item 2)

All engines are cold-path, run on profile update / load: (a)–(b) ≪ 1 ms; (c)
adds the per-octave scans (~10–100 ms for 88 keys, vectorizable, still
irrelevant offline); (d) is one banded LS solve, ≪ 1 ms. No new dependencies
(erf hand-rolled §6; pentadiagonal/banded solvers ~50–100 lines each).
Placement per internals/04: stateless math → `algorithms/curve.rs` +
`algorithms/dissonance.rs` (sizing rule will likely split the module); domain
types (`TuningCurve`, engine selection, flags) → `models.rs`; GUI consumes.
**The curve is derived data: recomputed on load, never persisted** — the stale
`analysis.json` incident (§4) is the standing proof of why.

## 10. Integration prerequisites

1. **Provenance flag (blocking, ADR 0006 item 3):** add `captured_in_auto` (or
   equivalent) to `KeyMeasurement`; legacy `tuning_profile.json` entries
   deserialize as **untrusted/auto** so pre-flag data can never feed the curve.
2. **Reference-pitch plumbing:** $d_g$ offset, 440 default, user-settable later.
3. **Capture protocol note (manual-mode docs) — muting is OPTIONAL, not
   required:** our 87 unmuted captures produced solid $B$ (synthetic-truth
   validated), and commercial ETDs sample unmuted. Muting unison neighbors
   (Dirks' protocol) helps only when unisons are badly detuned (split/broadened
   partial peaks → noisier amplitudes for (c)'s weights) — and the strobe
   tuning workflow has mutes in place anyway, so captures taken *during* tuning
   get this for free. Document as best-practice, never a gate.
4. **Sympathetic-resonance / instrument-fingerprint module (user idea):
   deferred to its own investigation** — queued as Prompt E in
   `next-chats-handoff.md`. Sketch: peaks recurring at fixed frequencies across
   many different keys' captures = soundboard/room/undamped-string signature;
   a persisted spectral fingerprint could mask them during MAT association.
   Promising and cheap to prototype offline from existing diagnostics; out of
   scope here.
5. Uncommitted diagnostic already in tree: `examples/regenerate_partials.rs`.

## 11. Validation plan (Prompt C item 5) and testing strategy

**What offline validation CAN show** (all on `partials_current.json`-class
data, regenerated auto-mode captures, validation-only): each port's
faithfulness (unit tests against paper values); Railsback-*shape* sanity per
engine; negative-stretch detector behavior on the known treble edge-hits;
DOF-growth behavior of the Whittaker layer (feed k = 4, 8, …, 88 keys; curve
must interpolate Rigaud→per-key); cross-engine deltas from the comparison
harness (`examples/curve_compare.rs`, the Rust re-implementation of the session
prototypes — reports stretch tables, roughness, flags per engine).

**What it CANNOT show (stated honestly):** which engine *sounds better*. The
instrument is out of tune, no aurally-tuned reference tuning exists, and n = 1
— comparative numbers are diagnostics, not selection evidence. **The
acceptance test is tuning the piano to a curve and listening.** Selection
between (c) and (d) styles is deferred to that, plus instrument #2.

**Measurable error functionals that need no ground truth (user question,
answered — yes):** (i) **implied beat-rate profiles** — every curve determines
the beat rate of every coincident pair (4:2, 6:3, 3:1, …) at every key; plot
per interval type; aural practice expects them slow and *smoothly progressing*
(the Verituner patent's own criterion), so magnitude and jaggedness are
quantitative diagnostics; (ii) **cross-scoring under the perceptual objective**
— evaluate every engine's curve under the Giordano dissonance functional
(caveat: selecting on this biases toward (c) by construction — report,
don't select); (iii) **leave-keys-out prediction error** — fit a curve on a
subset of measured keys, predict the held-out keys' stretch, measure in cents:
the cleanest genuine "error number" available, and it also measures the
DOF-growth claim directly; (iv) constraint-violation counts from the §2
detector. These go into the comparison harness.

**Unit/property tests while building (user directive):** Eq.-6 closed-form
values; $\rho = 1$ $B_U$-cancellation identity; erf vs table (A&S) to ~1e-7;
Whittaker λ-limits (λ→0 reproduces input; λ→∞ reproduces prior) and LOO-CV
against brute-force; banded LS vs dense reference solve; $d(m{+}12) \ge d(m)$
property on valid synthetic profiles; Giordano engine reproduces the
session-prototype optima on `partials_current.json`; detector flags the known
treble edge-hits.

**Research items (separate, not user features):** repeat-capture
reproducibility (does a per-key wiggle reproduce? → how far to trust per-key
DOF / how low λ may go); Giordano capture-condition sensitivity (same key,
different strike strengths → spread of the dissonance optimum).

## 12. Recommendation and staging (Prompt C item 6)

Per the user's decision: build the shared primitives, then all four engines
behind one trait, **(c) and (d) as the primary study subjects**, comparison
harness alongside, tests as we go. (a) and (b) cost little beyond the
primitives and serve as baselines and fallbacks. The perceptual layer —
instrument-measured octave type — is this program's **novel contribution** (no
commercial product derives taste from measured spectra; they all make the user
pick a preset), and therefore carries the rigor: **a dedicated ADR** documents
the (c) composition (Giordano → Eq. 30 → Eq. 9), its gates (sufficiency
criterion, detector, treble exclusion), and its validation evidence before any
default flips to it. Style presets (Rigaud $\bar\rho \pm 1$; pure-12ths via
(d)) remain the user-facing taste control throughout.

## 13. Documented defaults (veto at review)

| # | Default | Basis |
| --- | --- | --- |
| 1 | Dissonance model: amplitude-product $B_{ij}=a_i a_j$, constants $b_1{=}3.5, b_2{=}5.75, x^\*{=}0.24, s_1{=}0.021, s_2{=}19$; equal-total-power notes | Giordano Eqs. 3–6 (recipe paper; his bass-favored variant); Gràcia's min-loudness/5.7 variant noted, not used |
| 2 | Giordano sufficiency gate: interior minimum required (no scan-edge), **≥ 8 coincident 2j:j pairs** (min(⌊N_low/2⌋, N_up) ≥ 8; ADR 0008 — the former above-median-amplitude count is a demoted diagnostic); else key excluded from ρ fit | his §VI.C convergence analysis, verified against the PDF (16/8 partials for A0–A1 ⇔ 8 coincident pairs; A2–A3's 6/3 makes 8 the bass-derived conservative floor); edge-hit = detector artifact (§2) |
| 3 | Curve-B: precision-weighted shrinkage of measured $B$ toward the $B_\xi$ fit, $w = \sigma_p^2/(\sigma_p^2+\sigma_m^2)$ with repeat-measured $\sigma_m(n)$ and self-calibrated $\sigma_p$ (ADR 0009 — replaces the former ≥ 8-partial hard fallback); treble is prior-dominated automatically | §8 cancellation result + ADR 0009 (inverse-variance posterior mean; instrument-#2 repeat experiment) |
| 4 | λ selection: Eilers fast LOO-CV; L-curve as cross-check | Eilers 2003; never tuned on the 87-capture benchmark |
| 5 | erf: A&S 7.1.26 polynomial, documented + tested | no-dep style |
| 6 | Interval set for (d): octaves (2:1, 4:2, 6:3), twelfth 3:1, double octave 4:1, fifths 3:2 tempered ($\tau{=}{-}1.955$¢), fourths 4:3 tempered; weights = Form-2 derived sensitivities × preset taste multipliers (ADR 0008) | §6(d); OnlyPure/Gràcia precedent for 12ths/5ths |
| 7 | Anchoring: A4 fixed by reference pitch; temperament-region anchor for chains as in the prototypes | Rigaud §II.B.4 |
| 8 | Module layout **(DECIDED, Option 1 — flat, 2026-07-08)**: `algorithms/tuning.rs` (four engines, Eq.-6 stretch, $c_{m,k}$/$\tau_k$, detector, anchoring); `algorithms/inharmonicity.rs` **repurposed in place** (Rigaud B-model layer: $B_\xi$ L1 fit, $\rho_\phi$/erf, Eq.-30 inversion — deprecated `calculate_b_value` deleted, `linreg` dep dropped); `algorithms/dissonance.rs` (Giordano engine); `algorithms/smoothing.rs` (Whittaker + LOO-CV + shared banded LS solver); `TuningCurve` in `models.rs`. Split further only if the internals/04 sizing rule triggers. | user decision (avoid many-small-module bloat); matches flat `algorithms/` convention |
| 9 | Curve-lock UX; recompute-on-load | PianoMeter precedent; §9 |

## 14. References

- F. Rigaud, B. David, L. Daudet, "A parametric model and estimation techniques
  for the inharmonicity and tuning of the piano," JASA 133(5):3107–3118 (2013).
  `resources/moba/`.
- N. Giordano, "Explaining the Railsback stretch in terms of the inharmonicity
  of piano tones and sensory dissonance," JASA 138(4):2359–2366 (2015).
  `resources/worker/2359_1_online.pdf`.
- R. Plomp, W. J. M. Levelt, "Tonal consonance and critical bandwidth," JASA
  38:548–560 (1965). — W. A. Sethares, "Local consonance and the relationship
  between timbre and scale," JASA 94(3):1218–1228 (1993) (parametrization).
- H. Hinrichsen, "Entropy-based tuning of musical instruments,"
  arXiv:1203.5101 / Rev. Bras. Ensino Fís. 34, 2301 (2012). Ruled out (§3.3).
  — Szwajcowski & Pilch, Applied Acoustics (2020) (octave-wise variant).
- X. Gràcia, T. Sanz-Perela, "The wave equation for stiff strings and piano
  tuning," arXiv:1603.05516 (2017). Scale-step scope (§3.4).
- E. T. Whittaker, "On a new method of graduation," Proc. Edinburgh Math. Soc.
  41:63–75 (1923). — P. H. C. Eilers, "A perfect smoother," Analytical
  Chemistry 75 (2003).
- R. W. Young, "Inharmonicity of plain wire piano strings," JASA 24:267–273
  (1952) (universal treble pair).
- O. L. Railsback, "Scale temperament as applied to piano tuning," JASA 9:274
  (1938).
- D. J. Carpenter, "Beat rate tuning system and methods of using same," US
  Patent 6,529,843 B1 (Verituner) — industry document for engine (d).
- Abramowitz & Stegun, *Handbook of Mathematical Functions*, 7.1.26 (erf).
- Industry documentation (directional, non-peer-reviewed): TuneLab manual
  (tunelab-world.com), Reyburn CyberTuner, PianoMeter (pianometer.com),
  OnlyPure (Stopper), Dirks (dirksprojects.com).
