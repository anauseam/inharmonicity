# Strobe display & manual-tuning UI — design note

**Status:** Reviewed and approved 2026-07-17 — **build-ready**; nothing is
built yet beyond the compute/threading foundation (§2). This is the Prompt-I
specification: it describes the full user-facing manual-tuning surface in
detail, reviewed thoroughly *before* any widget is written. Cross-references:
`tuning-curve-design.md` (the curve engines this consumes), ADRs 0007/0008/0009,
project memory `tuning-curve-prompt-i.md`, and `next-chats-handoff.md` Prompt I.
Decisions carried here as **D1–D7** were agreed in the strobe-primer discussion
(2026-07-15/16); the partial-selection findings (§6) come from a 2026-07-16
survey of the commercial field (§12); the review resolutions **R1–R13** (§4.1)
were verified against the code and the field and approved 2026-07-17.

Standing project constraints apply: real captures are validation-only (n = 1);
`tuner-core` stays headless (the GUI speaks only the six sanctioned crossings —
`docs/internals/01`, `02`); no new heuristic constants without a derivation or a
cited precedent; commit only when asked.

---

## 1. Scope

Manual mode is the path where the user names each key, the Worker measures its
(f₀, B) and partials, a **tuning curve** is computed from the trusted set, and a
**strobe** lets the user tune each string to its own per-partial targets. The
curve library and its off-thread compute are done (§2). This note specifies
everything still unbuilt:

- **The strobe display widget**: the **absolute-partial** strobe (v1, §5). (An
  interval-beat second mode was originally planned; it is now build-if-requested
  — §7.0 — since intervals are correct by construction once notes sit on the
  curve.)
- **Partial selection** (§6): which partial(s) each key's strobe shows.
- **Curve lock** (§8): freezing the target set during a tuning pass.
- **Curve selection + comparison gallery and detail view** (§9).
- **The live curve-plot widget** (§10).
- **Reference-pitch control and flagged-key surfacing** (§11).
- The **state model** (§13), **sequencing** (§14), and **open questions** (§15).

Out of scope here (own efforts, cross-referenced where they touch): the
`audio.rs` split (handoff Prompt K), baton-pass hardening (Prompt L), the
in-app curve auralization playback (the deferred **seventh** crossing —
`ARCHITECTURE.md`), and quantitative curve-comparison metrics (README No-ETA,
"Advanced mode").

---

## 2. What already exists (the foundation — do not rebuild)

Shipped 2026-07-16 (Prompt I foundation; uncommitted, 68 lib tests, clippy-clean):

- **Curve compute is off-threaded to the Worker.** Measured single-engine cost
  on 87-key data: (a) 9 ms, (b) 12 ms, **(c) 1333 ms**, (d) 45 ms. (c)'s
  Giordano scans alone rule out the GUI thread, so all recompute runs on the
  Worker via **crossing #6** (`WorkerJob::Curve`), returning a `CurveBundle`
  (all engines) on **crossing #5** (`WorkerOutput::Curve`).
- **Latest-wins protocol:** the UI stamps a monotonic `generation`; a superseded
  bundle is dropped on arrival; captures are always serviced before curve jobs.
  Undo-race safe. The job carries a read-only `CurveInput` snapshot, so a curve
  recompute can never touch the profile a measurement writes.
- **State:** `TunerApp` holds `curve_bundle: Option<CurveBundle>`,
  `curve_dirty`, `curve_generation`. Recompute triggers on every trusted-set
  edit (capture merge / undo / load). The curve is **never persisted**
  (recompute-on-load — curve design note §9).
- **Math already in `models`:** `TuningCurve::target_f1(key)` (the audible f₁
  target) and `TuningCurve::strobe_partials(key, b_raw, out)` — per-partial
  reference frequencies `f_n* = n·f₀*·√(1 + B_raw·n²)`, **raw measured B always**
  (the string must match physically or a correctly-tuned partial shows a false
  beat — curve design note §5). Currently `strobe_partials` emits **all** partials
  up to Nyquist; it does *not* select (§6 adds selection).

Everything below consumes these; none of it changes the compute path.

---

## 3. How a strobe works (first principles, so this note stands alone)

A strobe tuner is a **phase comparator**. Given a reference oscillation at the
target frequency `f_ref` and the live signal component at `f_live`, the display
shows the *phase difference* between them as the angular position of a pattern:

- Phase difference grows at rate `2π·(f_live − f_ref)`. So the pattern **rotates
  at exactly `f_live − f_ref` revolutions per second** (per band segment).
- **In tune ⇒ stationary.** Off by Δf ⇒ rotates at Δf; direction = sharp/flat.

Its resolution advantage over a needle is *integration*: a needle shows
frequency error (noisy at small offsets), while a strobe shows *accumulated
phase*, so even a 0.05-cent error drifts visibly over a few seconds. The
sensitivity is free — it comes from integrating phase.

The classic optical device spins a disc of concentric segment-bands (each band
2× the segments of the one below, covering several octaves — the origin of the
band-per-partial layout) under a lamp flickering with the input. Electronic and
software strobes emulate the phase comparison directly.

Two consequences shape the whole design:

- **Aliasing.** A pattern rotating faster than ~half the display frame rate is
  an unreadable blur — exactly like a real strobe far from pitch. So the usable
  capture range is only a few Hz, which is why a strobe is always **paired with
  a coarse cents indicator** to get within range (D4).
- **Amplitude.** A partial that has decayed below the noise floor has *random*
  phase, so an ungated band spins on noise. Every strobe implicitly gates on
  amplitude — the lamp doesn't flash without signal (D3).

---

## 4. Decision register (D1–D7 + research answers)

| # | Decision | Basis |
| --- | --- | --- |
| **D1** | Phase readout via Goertzel at the reference frequency (I/Q / lock-in). | Audit-08 proved our Goertzel hop-to-hop phase differences are exact at non-integer bins — the strobe-readiness property. |
| **D2** | Rotation rate = **beat rate in Hz** (not cents-normalized). | The display equals a physical observable — the beat a tuner counts. Zero extra processing from D1. Peterson precedent — but the field splits (R9): Verituner's spinner speed is cents-proportional, so both conventions ship; D2 stands on the physics, not on unanimity. See §5.5. |
| **D3** | **Amplitude gate**: freeze/dim a band when its partial falls below the noise floor. | Phase of noise is uniform-random; mechanism forced, threshold tunable. |
| **D4** | Pair the strobe with a **coarse wide-range cents indicator**. | Forced by aliasing; also supplies the uniform "how many cents" magnitude read across registers (subsumes any need for cents-normalized rotation — §5.5). Computes against the **locked** `target_f1`, not `FrameOutput.cents_deviation`, which is vs ET (R13). |
| **D5** | Display-partial **register constraints**: treble leans **low** partial, bass uses a strong **higher** partial. | ADR 0009: treble raw-B carries ×8 σ ⇒ high-partial targets jump between captures; deep-bass fundamentals are physically weak. Refined by §6; the treble rule is **exact**, not a lean — the n = 1 target is B-immune (R4, §6.3). |
| **D6** | Strobe targets **never shift mid-pass** — the curve **locks**. Auto-engage on entering the tuning view + a visible "locked" indicator + a "re-lock to latest" button guarded by a confirm modal. After a recapture while locked, prompt once to re-lock (R6, §8). | §8. PianoMeter precedent. |
| **D7** | The Worker computes **all** engines per job (the bundle); the manual-mode default view is **(d) multi-interval BALANCED**. | Built (§2). Instant engine switching with no worker round-trip. |

Research answers folded in (2026-07-16):

- **Both display modes** (absolute-partial + interval-beat) are what the field
  ships. We ship only the absolute mode: interval-beat is build-if-requested
  (§7.0 — intervals are correct by construction under a curve-based approach),
  not the planned second effort this line originally implied.
- **Partial selection standard:** there is *no* universal fixed partial number;
  the professional standard is amplitude+inharmonicity-informed per-piano
  selection ("Smart Partials"), or show-all. Our plan adopts the former,
  computed at curve time (§6).
- **Cents-normalized rotation:** not in v1 — D4 covers the need. **Not fully
  ruled out**: if v1 testing shows Hz sensitivity is unreadable across
  registers, re-investigate (§5.5).

### 4.1 Review resolutions (2026-07-17 — code- and field-verified, all approved)

| # | Resolution |
| --- | --- |
| **R1** | **A-mode struck.** Path A is the target; Path B is a *visual prototype only*. The adaptive tracker must stay live alongside the strobe bank — D4's coarse read during a pitch raise (> ±21.5 Hz from target) and recapture-while-tuning both need it — so the fixed-reference bank is a **second** bank, never a retargeting. (§5.2) |
| **R2** | The DSP ships the **accumulated strobe angle mod 2π** (plus per-reference magnitude), never raw per-hop phase. `FrameOutput` travels a lossy triple buffer; GUI-side unwrap/integration corrupts permanently on any dropped frame. Angle-as-state is drop-proof. This also demotes Path B: its angle is untrustworthy *by construction*, not merely noisy. (§5.2) |
| **R3** | **Deep-bass strobe Goertzels need a longer window.** The 1024-sample Hann main lobe is ±86 Hz; bass partial spacing ≈ f₀ (27.5 Hz at A0) puts the neighbor at −1.4 dB *inside* the lobe below ~E2 — visible shimmer even in tune. A ≥ 4096-sample variant (main lobe ±21.5 Hz) resolves it; window ≠ hop, update rate unchanged. Partial *selection* cannot fix this — spacing doesn't grow with n. (§5.2) |
| **R4** | §6.3's B-uncertainty penalty is exactly **(n²−1)**, not n²: the n = 1 target is *identically immune* to B error (f₁ is the anchor). Closed form δcents ≈ 866·B·(n²−1)·σ_lnB(measured partial count) — two distinct n's, previously conflated. (§6.3) |
| **R5** | **v1 partial mode = single-partial**, coarse table = TuneLab's shipped defaults (6th → 4th → 2nd → fundamental from A4 up). Show-all is **deferred, not eliminated** — it stays the planned user toggle. (§6.4) |
| **R6** | **Recapture while locked → prompt once**: "Key X was recaptured — re-lock now?", routed through the §8 confirm modal. No silent target shift, no silent no-op. (§8) |
| **R7** | Gallery: (a)/(b) render as **plain cards**, not single-item thumbnail rows. (§9) |
| **R8** | `display_partials` lives **inside `CurveBundle`** — locking the curve locks the partials; a parallel GUI array is a desync bug waiting to happen. (§13) |
| **R9** | D2 basis amended: the field ships **both** rotation conventions (Peterson Hz, Verituner cents-proportional). Decision unchanged; the §5.5 re-investigation trigger stands. |
| **R10** | §7 rewritten: coincident partials (Hz apart or less) are **not spectrally separable in real time**. Phase 2 displays the **computed beat** (from each note's separable non-coincident partials) and/or the **unsigned envelope beat** — never literal two-partial phase. (§7) |
| **R11** | Peterson's octave shift is, in software, a pure display transform θ → 2^k·θ — zero DSP cost; a cheap fallback, not a contingency with real cost. (§5.3) |
| **R12** | Display calibration convention: **one segment-width of travel per beat period** (the classic disc read — keeps D2's beat correspondence literal). An S-fold pattern read at the 43 Hz hop rate aliases at S·Δf > 21.5 Hz ⇒ keep S small (4–6). (§5.5) |
| **R13** | D4's coarse indicator computes against the **locked** `target_f1`, not `FrameOutput.cents_deviation` (which is deviation from ET). (§5.2, §11) |

---

## 5. Widget 1 — the absolute-partial strobe (v1)

The primary tuner: for the currently-selected key, it shows how far each of that
key's displayed partials is from its locked curve target.

### 5.1 Inputs

- **Reference frequencies** `f_ref[·]` — from the **locked** curve's
  `strobe_partials(key, b_raw)` (raw measured B), restricted to the selected
  partial(s) (§6). Static while a key is being tuned (they change only on key
  change or an explicit re-lock — D6).
- **Live signal** — the struck string, on the audio thread.

### 5.2 The phase readout, and the one real plumbing question

D1 says: evaluate a Goertzel **at `f_ref`** each hop and read its phase; the
band's rotation is the accumulated phase drift of the live signal at `f_ref`
(exactly `f_live − f_ref`). The subtlety: the reference is the **desired** target
(from the curve, GUI/Worker side), while the engine's existing per-partial
Goertzels track the **physical** string (their seed adapts toward it via the
`α = 0.05` EMA — `engine.rs`, ARCHITECTURE Thread 2). A Goertzel locked to the
string is ~stationary; it does **not** give the drift-against-target the strobe
needs. And **`FrameOutput` currently ships per-partial frequency but no phase.**

So a true strobe needs the phase of the live signal *evaluated at the fixed
reference frequencies*, which requires the audio samples — i.e. it must be
computed on the DSP thread. Resolved at review (R1/R2/R3):

- **Path A — true phase strobe (approved target).** Push the selected key's
  reference frequencies to the DSP thread (infrequent — only on key change /
  re-lock; a small heap-free array over an atomics group or a crossing-#4-style
  ringbuf). The DSP runs a small **Goertzel bank at those references** each hop
  — a **second** bank; the adaptive tracker stays live for the coarse
  indicator and recapture (R1) — and accumulates each reference's unwrapped
  drift into a **strobe angle**, shipped mod 2π with its magnitude in a widened
  `FrameOutput` (`partial_strobe_angle: [f32; 12]` + magnitudes — crossing #2,
  no new crossing). Angle-as-state makes the lossy triple buffer harmless: a
  dropped frame skips one visual update instead of corrupting the display (R2).
  Deep-bass references (below ~E2) use a **longer-window Goertzel variant**
  (≥ 4096-sample Hann) so the neighboring partial — spaced ≈ f₀, inside the
  1024-window main lobe down there — leaves the lobe (R3); window ≠ hop, so
  the update rate is unchanged. Reference angles reset on key change / re-lock
  (one warm-up hop, as the tracker already does).
- **Path B — frequency-integration (visual prototype ONLY).** The GUI
  integrates `(f_live − f_ref)` from the existing `partial_freqs` into a strobe
  angle. **Zero DSP change** — good enough to validate the widget's rendering
  and interaction. But its angle is untrustworthy *by construction*, not merely
  noisy: the triple buffer is lossy and integration has memory, so any dropped
  frame (compositor hiccup, resize, load) corrupts the accumulated angle
  permanently (R2). Never the shipping fidelity.
- **Path A-mode — struck at review (R1).** Retargeting the existing tracker
  Goertzels to fixed references breaks what must run *concurrently* with the
  strobe: a fixed-reference Goertzel reads validly only within the ±21.5 Hz
  unwrap range, so D4's coarse indicator during a pitch raise (a 30-cent-flat
  C7 is ~36 Hz off) and the §5.6 recapture flow both need the adaptive tracker
  alive. A second bank costs ~12 extra 1024-sample Goertzels per hop — trivial.

**Plan:** prototype the widget on **Path B** to nail the visuals and
interaction, then ship **Path A**. Concrete Path-A deliverables: the
reference-push, the accumulated-angle + magnitude `FrameOutput` fields, and the
long-window bass Goertzel variant (`goertzel()` is currently hardcoded to 1024
samples). Note this is *still* only the six crossings — a wider crossing #2
payload and a small crossing-#4-style parameter push, no new crossing. The
coarse cents indicator computes against the **locked** `target_f1` — not
`FrameOutput.cents_deviation`, which is deviation from ET (R13).

### 5.3 Layout — band per partial

Each displayed partial gets one rotating band (concentric or stacked); the band
carries a small **partial-number label** (`n`), per Peterson/CyberTuner
convention. In **single-partial** mode (§6, the v1 default — R5) there is one
band; in **show-all** mode (§6, deferred toggle) there is a band per partial,
the Peterson multi-band look (Verituner, by contrast, blends all partials into
one *combined* spinner — §12). Bands the amplitude gate has frozen are dimmed
(§5.4).

**Bass/treble visibility (Peterson +2/−1 precedent).** A deep-bass fundamental's
band rotates very slowly and a high-treble partial's very fast (rotation ∝ Hz
error). Peterson's AutoStrobe shifts the *displayed* octave (bass shown +2
octaves, treble −1) to keep patterns in a readable middle band. We get most of
this **for free** via §6: choosing a higher bass partial and a lower treble
partial already lands the displayed frequency in a readable range (A0 on
partial 8 rotates at 0.13 Hz/cent — readable as drift). In software the shift
is a pure display transform θ → 2^k·θ, zero DSP cost (R11) — retained as a
cheap fallback if §6 selection isn't enough.

### 5.4 Amplitude gate (D3)

Each band tracks its partial's live amplitude (the Goertzel magnitude is already
computed). Below a noise-floor-relative threshold, **freeze and dim** the band
(hold last angle, grey it) rather than spinning on noise. Threshold: reuse the
Neyman–Pearson SNR gate the tracker already applies to partials (ARCHITECTURE
Thread 2, "Amplitude SNR Gate") — no new constant. A frozen band is also the
honest signal that this partial has decayed and the tuner should re-strike.

### 5.5 Rotation scaling (D2) and the cents-normalized question

Ship **Hz** (beat-rate) rotation, direction = sharp/flat. It equals the
audible beat, needs no extra processing, and is the piano-relevant quantity
(tuners set beats). **Calibration (R12):** the pattern travels **one
segment-width per beat period** — the classic disc read, keeping the beat
correspondence literal. An S-fold symmetric pattern sampled at the 43 Hz hop
rate becomes ambiguous at S·Δf > 21.5 Hz, so segment count trades sensitivity
against capture range — keep S small (4–6; final value by eye during the
Path-B prototype).

The field splits on this convention (R9): Peterson rotates at the physical
beat rate, Verituner's spinner speed is cents-proportional. D2 stands on the
physics, not on unanimity.

Cents-normalized rotation (rescale so 1 cent = the same speed everywhere) is
**deliberately not in v1**. Rationale: within one note you tune one partial at
one frequency, so sensitivity is already *constant* while nulling it; the null
(stationary = in tune) is scale-free; and the cross-register "how many cents"
read is exactly what D4's coarse indicator provides, uniformly, by construction.
Normalizing would duplicate D4 while breaking the beat-correspondence.
**Re-investigation trigger (per user):** if v1 testing shows the Hz display is
genuinely unreadable across registers despite §6 partial selection and D4, add
cents-normalized as an optional per-band angle rescale (cheap; no new crossing).

### 5.6 Flagged keys

If the selected key is flagged by the curve (`CurveKeyFlags`:
`negative_stretch`, `excluded`, `giordano_excluded`, or `curve_b_fallback`),
overlay a **red ✗** on the strobe with a one-line reason and a "recapture
recommended" hint (curve design note §2 — the detector never clamps; it advises).
This reuses flags already on every `TuningCurve`.

---

## 6. Partial selection — which partial(s) to show (absolute mode)

### 6.1 The finding (survey, §12)

There is **no universal fixed partial number** for piano strobe tuning. The
field splits two ways:

- **Smart, fixed-per-session** (Reyburn CyberTuner "Smart Partials"): choose
  **one** partial per note, per piano, at analysis time from *partial loudness
  (amplitude), inharmonicity, octave type, and piano size* — then hold it fixed
  while tuning. Instrument-adaptive without any mid-note change.
- **Show-all** (Verituner: all partials in one combined spinner + a numeric
  cents readout; Peterson: a multi-band disc): display every partial and let the
  tuner's eye choose.

Aural practice has rough register conventions (bass on higher partials because
the fundamental is weak; treble on the 1st–2nd) — which is exactly what **D5**
encodes — but the electronic tuners do **not** hardcode a number; they adapt.

### 6.2 What this means for us — "dynamic" = decided once, at curve time

The clarified intent (user, 2026-07-16): partial choice is **not** a continuous
mid-tuning change; it is the *fixed* partial(s) we will watch, **decided during
curve generation from amplitude**. That is precisely CyberTuner Smart Partials.
It fits our pipeline cleanly and is supported by the capture design:

- The curve engines **already consume amplitude** — Giordano (c) is
  amplitude-product roughness `a_i·a_j` with equal-total-power normalization;
  (d)'s Form-2 weights are `a_p·a_q·…` (`giordano.rs`). The per-key
  `(n, freq, amplitude)` list is already in `CurveKeyData`.
- Our capture is the **post-attack "Golden Window"** of stable decay
  (`gatekeeper.rs` State 3), so the amplitudes we hold are the **sustained**
  amplitudes — the correct basis for "which partial is strong while you tune",
  not attack-transient loudness.

So the display partial per key can be chosen **once, when the curve computes**,
baked into the bundle, and held fixed for the session (it **locks with the
curve**, D6). Never real-time; no mid-note jump.

### 6.3 The selection rule (proposed)

For each key, pick the displayed partial `n*` to **maximize a stability-weighted
strength score** subject to the D5 register window:

- Prefer partials with high **sustained amplitude** (from the measured list).
- Penalize partials whose **raw-B target uncertainty** is large. Exact form
  (R4): with `f₀* = f₁*/√(1+B)` anchored on f₁, the target sensitivity is
  `d ln f_n*/d ln B = (B/2)(n²−1)/((1+Bn²)(1+B))`, i.e.
  **δcents(n) ≈ 866·B·(n²−1)·σ_lnB** — where `n` is the *displayed* partial
  number and σ_lnB is the ADR-0009 model evaluated at the key's *measured
  partial count* (two distinct n's). The n = 1 target is **identically
  B-immune** (it *is* the anchor), so D5's treble rule is exact, not a lean: a
  treble key with B ≈ 6·10⁻³ and 4 measured partials carries ~5 ¢ of target
  uncertainty on partial 2 and zero on the fundamental; A0 on partial 8
  carries ~0.08 ¢ — the bass is free to use high partials.
- Keep the displayed frequency in a **readable rotation band** (bass: high
  enough to rotate visibly; treble: low enough not to alias) — the §5.3 Peterson
  concern, satisfied by the choice itself.

All inputs (amplitude, B, σ_B(n) from ADR 0009, key index) are already on hand
at curve time. The output is a per-key `n*` (and, for show-all mode, an ordered
list) added to the `CurveBundle`. **No new measured data, no new constant** —
σ_B(n) is the ADR-0009 model, amplitudes are measured, the register window is
D5.

### 6.4 v1 vs upgrade

- **v1 (decided — R5): single-partial**, with the D5 coarse table instantiated
  as **TuneLab's shipped defaults**: 6th partial in the low bass → 4th → 2nd →
  **fundamental from A4 up** (a user-editable per-note table in TuneLab; our
  treble entry is now *derived*, not just precedented — R4's B-immunity). One
  band is the smallest rendering surface, and single-partial is the primary
  mode of the field's piano-specific tools (CyberTuner, TuneLab).
- **Show-all: deferred, not eliminated** — it remains the planned user toggle
  (`strobe_partials` already emits all partials; the toggle is band-per-partial
  rendering). Two costs keep it out of v1: high treble partials would display
  targets carrying the R4 uncertainty (Verituner can afford show-all because it
  re-measures B continuously — a philosophy incompatible with our D6 lock),
  and deep-bass low-partial bands would sit mostly frozen behind the D3 gate
  (physically weak fundamentals). When it lands, adopt TuneLab's on-the-fly
  per-note partial-override gesture (not persisted) as the escape hatch for a
  dead auto-chosen partial.
- **Upgrade (target):** the **amplitude-informed per-key selection** of §6.3,
  computed at curve time — our "Smart Partials", the professional standard; it
  slots into the existing curve-compute with no new crossing or constant.

**Build order:** the widget consumes a per-key `n*` field in the bundle from
the start; populate it with the TuneLab coarse table first, then swap in the
§6.3 amplitude selection — the widget doesn't change, only how `n*` is filled.

### 6.5 Interval mode would select differently (build-if-requested)

Everything above is single-note. An interval-beat display would select by the
interval's *coincidence ratio*, not by amplitude: an octave beats at 2:1 (partial
2 of the lower vs partial 1 of the upper), a twelfth at 3:1, etc. So §6 governs
the absolute mode; the coincidence structure would govern interval mode. Note
interval-beat is **build-if-requested, not planned** (§7.0): intervals are
correct by construction once both notes sit on the curve, so there is no
required use for it at this program's goal.

---

## 7. Interval-beat strobe & unison assist — status (2026-07-27)

**Superseded status (2026-07-27 review).** §7 originally specified an
interval-beat strobe as a planned "phase 2, two modes from the start." That
review concluded interval-beat between *different* notes has **no required use
case for this program's goal**, and downgraded it to **build-if-requested**.
Unison assist — the one genuine tuning need in this neighbourhood — was found
to be a *separate, lighter* concept that does not need the interval-beat
machinery (§7.4). The original spec is preserved below (§7.1–7.3) for the case
where interval-beat is ever requested.

### 7.0 Why interval-beat is not required — intervals are correct by construction

Our goal is to tune the instrument to the computed inharmonicity-aware curve.
The absolute-partial strobe (§5) does this completely: null every string to its
own `d(m)` target and the whole instrument is on the curve. Crucially, engines
(c)/(d) *compute* `d(m)` so the interval relationships come out right — so when
every note sits on the curve, the **octaves, fifths, and twelfths are already at
their intended beats**. You tune notes; the intervals follow. There is nothing
left to tune by interval.

Every candidate interval-beat use therefore reduces to something optional:

- **Verify/tune an interval by its beat** — redundant: the interval is right by
  construction once both notes are on the curve.
- **Cross-verify the curve live** (does the 3:1 twelfth beat as predicted?) —
  trust-but-verify, and we verify the curve far more rigorously offline
  (`curve_compare` prints interval beat-rate smoothness — median/max/jag — up
  the keyboard; the Verituner criterion).
- **Aural-workflow parity / temperament-by-ear** — a familiarity idiom our
  curve-based approach supersedes, not a capability gap.

None is required at the current scope. Interval-beat would become a *requirement*
only if the goal expanded to "a general aural-hybrid tool competitive with
Verituner" — a scope change, tracked as **build-if-requested**, not planned work.
(The anticipatory two-mode switch in §13 is harmless if retained but no longer
implies planned work.)

### 7.4 Unison assist — separate, lightweight, a real tuning need

A multi-string note (2–3 strings in the tenor/treble) must have its strings
tuned to zero-beat against each other. This is the one genuine tuning need near
interval-beat, and it does **not** need the interval mechanism:

- The strings are the **same note the user already selected** — no two-note
  *discovery*, no separate (f₀, B) tracking.
- It is the **envelope beat** on that one note's partial: the magnitude of a
  single Goertzel oscillates at the strings' beat rate. The absolute strobe's
  band already *stutters* visibly when a unison beats, so a clean unison readout
  is an enhancement of what is partly there — not new two-note capability.

So unison assist is a small, standalone future note (an envelope-beat readout on
the selected note), not a consequence of interval-beat. It is a quality feature,
not strictly required (unisons can be set by ear), so it too is unscheduled — but
when built it is cheap and needs none of §7.1–7.3.

### 7.1 Purpose (original spec — if interval-beat is ever requested)

Show the beat between two notes' **coincident partials** — what a tuner actually
listens for when setting an interval (octave, fifth, twelfth …). Unlike absolute
mode, the partial pair is fixed by the interval ratio (§6.5). The curve supplies
the *target* beat (0 for a pure octave, nonzero for the stretched target).

### 7.2 What is actually measurable (R10)

Near coincidence the two partials sit a few **Hz or less** apart — no real-time
window separates them (resolving 1 Hz needs > 1 s of signal), so "show the
phase between the two coincident partials" is not an implementable display.
What such a feature could ship:

- **Computed beat (the field's approach):** track each note's **non-coincident**
  partials — separable, since manual mode names both keys and their partial
  grids sit ≥ the note spacing apart away from the coincidence — derive each
  note's (f₀, B), and *compute* the coincident-pair beat against the curve's
  target beat.
- **Envelope beat (unsigned):** the magnitude of a single Goertzel at the
  coincidence frequency oscillates at exactly the beat rate a tuner hears; the
  sign is disambiguated by which note the user is moving.

Either way, the engine has only ever tracked **one** locked note (discovery +
the Goertzel tracker are single-note); two-note tracking is **new engine
capability**, not a display toggle — its own build phase, its own validation.
**Gate it behind manual mode** (the user names the interval's two keys), which
sidesteps the genuinely hard part — two-note *discovery* (an unexplored
auto-mode problem).

### 7.3 Deferral

Not built. The mode switch and layout (§13) were designed to anticipate a
second mode; that anticipatory scaffolding is retained but, per §7.0, interval-
beat is build-if-requested rather than planned.

---

## 8. Curve lock (D6)

The curve recomputes on every trusted-set edit; the strobe must not chase a
moving target mid-pass.

- **Auto-engage.** On entering the tuning/strobe view (or first strobing a key),
  **freeze** the current `CurveBundle` as the *locked* curve. All strobe
  references derive from the locked copy, not the live one. Recaptures continue
  to update the *live* bundle (and the gallery previews) but not the locked
  targets.
- **Indicator.** An always-visible "🔒 Curve locked (gen N)" state so the user
  knows targets are frozen and at which generation.
- **Re-lock to latest.** A button that copies the live bundle into the locked
  slot. Guarded by a **confirm modal** — feasible in Iced 0.14 via a
  `bool`-gated overlay (`stack!` a confirmation card over a scrim; no built-in
  dialog needed). Warranted because re-lock, while not data-destructive, **shifts
  every target** — keys already tuned to the old lock are now off relative to
  the new one, i.e. it can cost part of a pass. Copy: "Re-lock to the latest
  curve? This shifts all strobe targets. [Re-lock] [Cancel]".
- **Recapture while locked (R6).** When a capture merges while the lock is
  engaged — the §5.6 flagged-key flow ends exactly here — prompt once:
  "Key X was recaptured — re-lock now?", routed through the same confirm
  modal. The recapture never silently shifts targets, and never silently does
  nothing either.
- **Engine/preset changes while locked.** Switching the displayed engine/preset
  (a–d, ρ, pure-12ths) re-derives targets from the **locked** bundle's copy of
  that engine — instant, no recompute, still frozen in time. (The bundle holds
  all engines, D7.)

State: a `locked_curve: Option<CurveBundle>` distinct from the live
`curve_bundle`, plus the selected engine and the per-key `n*` (which locks with
it).

---

## 9. Curve selection & comparison gallery (+ detail view)

Agreed layout (user, 2026-07-16): a **master–detail gallery**.

- **Four vertical sections, one per class (a)–(d).** Each section: a header
  naming the class, then a **row of small clickable curve thumbnails** for that
  class's sub-classes, names beneath.
  - (a) Rigaud-pure — 1 thumbnail. (b) per-key+Whittaker — 1. (c)
    Giordano — the **ρ Low / Mean / High** trio. (d) multi-interval — **Balanced
    / Pure-12ths**. Decided (R7): the sub-class-less (a)/(b) render as **plain
    cards**, not single-item thumbnail rows.
- **Thumbnails are small d(m) plots** (sparkline-scale curve renders) for now;
  restyle later.
- **Click → detail view:** the full curve plot, a slot for **curve metrics
  (deferred** — README No-ETA / Advanced mode), and a **"listen" button
  (deferred** — the seventh-crossing playback). The detail view *shell* is built
  now with those two as empty/greyed slots.
- **Deferred (c) presets:** ρ Low/High are **not yet computed** (computing three
  (c) presets naively re-runs the ~1.3 s Giordano scan 3×). Until the calibration
  is factored out of the per-preset path, render those thumbnails as a **greyed
  "deferred" placeholder** (the idiomatic missing-feature slot, matching the
  settings sidebar's `ButtonType::Disabled`). See §14 — this gallery is the
  trigger to factor (c)'s calibration out.
- **Entry point:** the greyed **"Curve Select"** button already added to
  `settings_view.rs` `TONAL_CONFIG`.
- **Selection = display only.** Choosing an engine sets which curve the strobe
  and the live plot show; it never triggers a recompute (all engines are in the
  bundle, D7).

---

## 10. The live curve-plot widget

A plot of the selected engine's `d(m)` across the 88 keys, shown while capturing
so the user watches the curve form. Default engine **(d) BALANCED** (D7). Updates
whenever a new bundle lands (live bundle for the plot; the strobe uses the
*locked* copy — §8). At launch, with zero trusted measurements, it shows the
prior-only curve and morphs from there (the `bundle_from_empty_input` test
guarantees this renders without panic). This is the full-scale sibling of the
gallery thumbnails (§9) — same rendering, one engine, 88 keys, axis labels.

(Rendering is a dataviz task — follow the `dataviz` skill when it is built:
theme-aware, one visual system across thumbnails, detail plot, and this widget.)

---

## 11. Reference pitch & flagged keys

- **Reference pitch (A440).** A settings view (its own entry — the existing
  greyed **"Tuning Standard"** button is the natural home) to set the reference,
  which maps to the curve's global offset `d_g` (`TuningCurve.d_g`,
  `d_g = 1200·log₂(ref/440)`). **Locked to A440 (`d_g = 0`) for v1**; the
  non-440 UI is deferred UX. The plumbing (`d_g` on the curve) already exists.
- **Flagged keys (red ✗).** In the strobe (§5.6) and optionally as small marks
  on the keyboard/plot, surface `CurveKeyFlags` with a recapture hint. Deferred
  styling; the flags are already computed per key.

---

## 12. What the commercial field does (survey, 2026-07-16)

Recorded so the decisions above have their precedents on the record:

- **Peterson AutoStrobe 490/590** — true stroboscopic, multi-band disc showing
  fundamental + overtones, each band labeled with its harmonic number; a **+2/−1
  octave display shift** keeps bass/treble patterns in a readable middle range.
  (§5.3.)
- **Reyburn CyberTuner — "Smart Partials"** — automatically chooses the optimal
  partial **per note, per piano** from *partial loudness, inharmonicity, octave
  type, and piano size*; gathers up to 12–16 partials in the bass. This is the
  model for §6.
- **Verituner** — listens to *all* partials on every note, measures
  inharmonicity continuously, and shows all partials at once in a **combined
  spinner with a numeric cents readout in the hub** (its D4-equivalent; needle
  for large deviations, spinner blades for small). The show-all precedent —
  note the spinner *blends* the partials rather than showing a band each, and
  its speed is **cents-proportional** (the R9 counter-precedent to D2).
- **TuneLab** — per-note **table of partials**, user-editable and stored with
  the tuning file, defaulting to 6th partial in the low bass → 4th → 2nd →
  fundamental from A4 up, plus an on-the-fly per-note partial-override gesture
  (not persisted). The v1 coarse table (R5) adopts these defaults.
- **PianoMeter** — curve-lock workflow precedent (D6): a manual Lock toggle
  that "protects the tuning from any future changes"; ours auto-engages, a
  strengthening.
- **Katsura "Piano Tuner" (macOS)** — 12-note chromatic strobe + partial
  analyzer + customizable stretch table; confirms per-partial analysis + a
  user-editable stretch table is standard.

Takeaway: no universal fixed partial number; the two shipped philosophies are
**smart-single** (CyberTuner — our target) and **show-all** (Verituner/Peterson
— our simplest v1 / optional toggle). Both pair the strobe with a coarse/numeric
cents read (D4).

---

## 13. State model (GUI)

Additions to `TunerApp` (names indicative):

- `curve_bundle: Option<CurveBundle>` — the **live** bundle (exists). Drives the
  gallery + live plot.
- `locked_curve: Option<CurveBundle>` — the **frozen** bundle the strobe reads
  (§8). `None` until the tuning view is first entered.
- `selected_engine: EngineChoice` — which curve the plot/strobe show (a, b,
  c-{Low,Mean,High}, d-{Balanced,Pure12}). Default d-Balanced.
- `display_partials` — per-key `n*` (or ordered list for show-all), §6; carried
  **in the bundle** so it locks with the curve (decided — R8).
- Reference-mode state ships as `ReferenceMode { Curve, Et }` (the target the
  readouts measure against). The originally-planned `strobe_mode: Absolute |
  IntervalBeat` was **not** built — interval-beat is build-if-requested (§7.0).
- `tuning_key: Option<u8>` — the key currently being strobed (drives the
  reference-frequency push, §5.2 Path A).
- Iced view flags for the gallery / detail / reference-pitch sub-views and the
  re-lock confirm modal.

No new persisted fields — the curve and all of the above are derived/session
state (recompute-on-load).

---

## 14. Sequencing (phases)

1. **Live curve plot + gallery shell** (§9, §10) on the built foundation —
   renders the bundle already in state; detail view with deferred metric/listen
   slots. Populate `display_partials` with the **TuneLab coarse table** (R5)
   first.
2. **Absolute-partial strobe, Path B** (§5) — GUI-side frequency-integration
   prototype to validate the widget/interaction with no DSP change (visual
   prototype only — R2).
3. **Absolute-partial strobe, Path A** (§5.2) — reference-push to DSP + second
   Goertzel bank at references + accumulated-angle/magnitude `FrameOutput`
   fields + the long-window bass Goertzel variant (R1/R2/R3). Shipping
   fidelity.
4. **Smart-Partials selection** (§6.3) — swap the coarse-table `n*` for the
   amplitude-informed per-key choice at curve time. (Widget unchanged.)
5. **Curve lock** (§8) — can land with step 2/3; the confirm modal and the R6
   recapture prompt with it.
6. **(c) ρ-preset compute** — factor Giordano calibration out of the per-preset
   path so the gallery's (c) trio renders (retires the greyed placeholders).
7. **Reference-pitch view** (A≠440), **flag styling** — deferred UX polish.
8. ~~Interval-beat strobe~~ — **removed from the plan** (§7.0: build-if-requested,
   not planned). Unison assist, if ever wanted, is a separate lightweight
   envelope-beat readout (§7.4), not this.

Commit boundary (user): **do not commit until the strobe display is built** —
the Prompt-I foundation ships with the feature, not before.

---

## 15. Review outcome — questions closed, what stays open

All five original review questions were closed 2026-07-17 (R-numbers in §4.1):
**Q1** Path A approved as target with Path B as visual prototype; A-mode struck
(R1; deliverables R2/R3). **Q2** single-partial v1 with the TuneLab coarse
table; show-all deferred, not eliminated (R5). **Q3** rely on §6 selection; the
octave shift is a zero-cost display transform kept as fallback (R11). **Q4**
plain cards for (a)/(b) (R7). **Q5** `display_partials` in the bundle (R8).
The recapture-while-locked gap found at review is closed by the R6 prompt.

Still open — implementation-time observations, none design-blocking:

1. **Segment count / band visual design** — S = 4–6 per R12; final value by
   eye during the Path-B prototype.
2. **Coarse-table break points** — adopt TuneLab's defaults verbatim for v1;
   revisit only when §6.3 Smart-Partials replaces the table anyway.
3. **D3 gate feel** — the NP threshold is derived; whether the freeze/dim
   needs hysteresis to avoid flicker at the boundary is a prototype-time
   observation.

### Final acceptance — Path A architecture (re-review 2026-07-27)

**Verdict: option A (keep) — accepted, no longer provisional.** The pre-commit
architecture re-review resolved the fork below in favour of the shipped
Path-A surface — the `Strobe` hop tap (Step 5b, with the coarse read folded
in), the three `FrameOutput` strobe fields, and the second crossing-#4
instance and its `HostHandle` endpoint. Findings:

- **Kept (A) over revert (D).** The offline evidence (below) plus ADR 0011's
  coarse read (validated on all three capture sets) plus live confirmation
  (ET, string-matching, fine+coarse readout, steady bass all working) settle
  the fidelity questions decisively; reverting would delete the unlocked-
  regime coverage and the R3 bass fix for no gain.
- **Rejected the ring merge (B).** The strobe-ref ring and the profile ring
  differ in payload size (`StrobeRefUpdate` ≈ 60 B vs `KeyProfileUpdate`
  ≈ 0.5 KB), capacity (2 vs 88), drain semantics (newest-wins vs apply-all),
  and consumer (the `Strobe` vs the `live_profiles` array) — and the profile
  path is gated off (`APPLY_MEASURED_B_TO_DISCOVERY`) while the strobe path is
  live. They are two instances of the crossing-#4 *class* (which `02`
  explicitly accommodates), not one channel; the "bloat" is a single `pub`
  field plus a one-method sender, correctly scoped. Keep separate.
- **Tap doctrine holds** post-fold: `Strobe::process` (Step 5b) runs after the
  `Engine` (Step 5), the chain never reads `StrobeResult`, and it writes only
  `FrameOutput` — deletable, gating/detection/measurement bit-identical.
- **NP amplitude gate** (band, ambient-σ during sustain) remains a documented
  suspected issue (`docs/internals/suspected-issues.md`), not a blocker: the
  band validates, the coarse read already uses the CFAR fix. Reopens only if
  late-gating proves a problem in use.
- **Module layout** (the strobe feature spans `strobe.rs` / `peaks` coarse /
  `spectral` primitives / `curves` policy / `models` target-math / `pipeline`
  orchestration / GUI; `models.rs` and `peaks.rs` are the largest files) is
  **by charter, not a mess** — deferred to a dedicated refactor (with the
  standing `models/` split, `04`), not a commit blocker.

Docs reconciled in the re-review: `02` (pipeline no longer retains the
reference set — the `Strobe` does) and `03` (`coarse_scratch` is owned by
`Strobe`, not the pipeline).

**Offline evidence (2026-07-19, `examples/strobe_replay.rs`).** The shipped
`Strobe` was replayed over real detuned-piano captures (595 piano-2 + a
piano-1 set), which are exactly the far-from-ET regime the strobe claims to
serve. Findings that inform the re-review:

- **Detuning coherence (unlocked-regime, decisive test):** with an ET
  fundamental reference across 553 in-range piano-2 captures spanning the full
  ±21 Hz readable band, the band's rotation rate matched the *true* pitch
  detuning to **0.24 Hz median** (0.70 Hz on the more-detuned piano-1). The
  fixed-reference strobe coherently shows a string's offset where the engine
  would not lock — confirmed on real audio, not eyeballed.
- **Bass window (R3):** the 4096-sample window cut bass rotation residual from
  0.071 → **0.017 cycles (≈ 4× steadier) on 97/101** piano-2 bass captures
  (0.050 → 0.025, 18/20 on piano-1). Direct quantified fix for the erratic
  bass the user observed; the strongest single result for keeping Path A.
- **Register caveat:** treble fundamental bands are ≈ 2.3× noisier (0.25 vs
  0.11 residual) and gate ≈ 11% — short decays, fast-dying fundamentals.
  Consistent with the treble-lean-low rule and D4's existence; a known
  characteristic, now measured.

This offline harness answers the *fidelity* questions with numbers; the live
protocol below answers *readability/feel*, which it cannot.

The re-review is informed by the hands-on validation protocol:

1. **Unlocked-regime test (decisive for Step 5b):** detune a string far off
   (≥ a semitone), select its key, strike. If the band spins usefully while
   the engine holds no lock (readout stuck at "listening…"), the
   lock-independence argument is confirmed on hardware; if the band offers
   nothing there, Step 5b's strongest justification is refuted and (D)
   becomes the recommendation by this section's own terms.
2. **In-tune stillness:** band stationary ⇔ an ordinary tuner reads ≈ 0 ¢
   near A4 (the anchor, where curve target = ET). Judge stillness quality.
3. **Beat correspondence (D2/R12):** slightly detuned, the band travels one
   pattern period per audible beat — count them against each other.
4. **Bass steadiness (R3):** compare band stability on a string below the
   ≈ 86 Hz window boundary (guitar E2) vs above (A2) — the long-window path
   vs the short one — and against the known-erratic cent-meter behavior.
5. **Decay gating (D3, with the standing σ caveat —
   `docs/internals/suspected-issues.md`):** on decay, note when the band
   freezes/dims vs keeps spinning on noise; judge the *band*, not the gate —
   the gate is a known revision target shared with the engine tracker.
6. **Re-strike continuity:** after gating, a fresh strike must resume
   without a phase jump. **Key switching:** targets and band reset cleanly.
7. **Cross-register readability (§5.5 trigger):** is Hz-rotation readable in
   both extremes, or does the cents-normalized option need re-investigation?

### D4 resolved — the coarse readout is a spectral read, not the tracker (2026-07-25, ADR 0011)

§5.5/D4 paired the band with a "coarse cents indicator" and left its source as
the engine's adaptive tracker. That source is now **replaced**, and the readout
regime is settled:

- **Band-slope** while ungated, filled, and in range — unchanged, still the
  accurate read (±0.2 ¢).
- **Coarse spectral read** otherwise: a bounded CFAR-gated search of the
  already-computed magnitude spectrum at a nominated reference partial
  (`FrameOutput::coarse_hz`, pipeline step 5b). Lock-independent, target-
  relative, and free of the ±21.5 Hz unwrap limit — so it is what remains
  readable during a pitch raise, which is the regime D4 was created for.
- **"listening…"** only when neither is available.

Three things this changes in the note's earlier text:

1. **The in-range test is now derived, not asserted.** The coarse read supplies
   the string's offset in *cents*, which the equal-cents identity makes exact at
   any partial, so the offset in Hz at the displayed reference r is
   r·(2^(¢/1200) − 1) — compared against the existing `BAND_READABLE_HZ`. No
   second constant, and the engine tracker is out of the display path entirely
   (`StrobeState::live_hz` removed).
2. **The coarse read uses its own partial, not §6.4's display table.** Fixed
   n\* = 4 below key 16 (C#2), fundamental above — cross-instrument, since the
   guitar's E2 fails at n = 5 where the piano prefers it. The band keeps the
   6/4/2/1 table. Two readouts, two questions; the displayed *number* is
   identical either way by the equal-cents identity.
3. **The §5.5 "cross-register readability" trigger (protocol item 7) is
   partly answered.** The alias limit is fixed in Hz and therefore shrinks as
   ≈ 37200/f in cents — ±1000 ¢ at A0 but ±9 ¢ at C8 — so the treble was never
   going to be served by rotation alone. The coarse read, not a
   cents-normalized rotation, is the answer there.

**D3 silence gate (2026-07-26).** The bank now also gates on the Gatekeeper's
`Silence` state, not only on its per-reference amplitude test. The two are not
redundant: the amplitude test admits a *single bin* above `noise_floor · K(n)`,
and `K ∝ 1/√n` puts that at **10 % of the silence threshold** at the 4096-sample
window — so low-frequency room rumble cleared it while the broadband RMS still
read silence, and the band rotated with no note playing. R3's long window made
this worse, since it halved K. Angles are held, not reset (R2), so
lock-independence is untouched. No new constant: the fix reuses the calibrated
silence state.

D3's standing σ caveat is also now **measured** rather than suspected, and a
correctly-specified local-noise gate exists to port from — but has deliberately
*not* been ported to the band or the engine tracker, which need the 87-capture
revalidation first (`docs/internals/suspected-issues.md`).

**ET reference mode (2026-07-19).** The strobe targets the piano tuning curve
by default; a `Ref: Curve / ET` toggle switches it to pure equal temperament,
**fundamental only** — the n = 1 target is B-immune (R4), so no per-string
inharmonicity is needed and a correctly-pitched string shows no false beat.
This makes the strobe usable on a **non-piano** (tune a guitar string to its
ET pitch: select the matching key, e.g. E2, and null the band) and is the
honest instrument-agnostic baseline (a step toward §11's reference-pitch
control). Part of the provisional surface — covered by the same re-review;
if Path A is reverted, ET mode goes with it.

---

## 16. References

- Design: `tuning-curve-design.md` (engines, strobe-target math §5/§7);
  ADRs 0007/0008/0009, and **0011** (the coarse readout — D4's source, its
  gate, and the FFT-size rule); internals `01`, `02` (crossings), `05` (style).
- Audit: `docs/audits/faithfulness-audit-08-goertzel.md` (exact hop-to-hop
  phase — the D1 basis).
- Field survey (2026-07-16, verified at review 2026-07-17): Peterson
  AutoStrobe 490/590 (petersontuners.com/products/autoStrobe490); Reyburn
  CyberTuner "Smart Partials" (cybertuner.com/irct); Verituner features + iOS
  User Guide (veritune.com/features.html); PianoMeter Lock
  (pianometer.com/support); TuneLab 4.4 manual, "Table of Partials"
  (tunelab-world.com); Katsura Piano Tuner (katsurashareware.com); octave
  types / coincident partials (onpitch.com/why-do-the-octave-tests-work).
- Strobe/DSP background: strobe-tuner phase-comparator principle; lock-in / I/Q
  demodulation (the D1 math); Goertzel 1958 / Sysel–Rajmic 2012 (already cited
  in `engine.rs`).
