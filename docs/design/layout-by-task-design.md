# Layout by task — GUI design note

**Status: BUILT 2026-08-20.** The two-column shape was agreed with the user
2026-08-07; this note argues it, settles what has accreted since, and specifies
the build, which then happened in the same session. §6 and §10 record what the
build measured and where it departed from the specification. A frontend decision, so it lives here and not in
[`docs/adr/`](../adr/), which is for technical/DSP decisions.

It supersedes no other note. It *consumes* three: the strobe panel's own layout
rules ([`strobe-and-manual-tuning-ui-design.md`](strobe-and-manual-tuning-ui-design.md)
§5.3, §8), the unison panel's ([ADR 0012](../adr/0012-unison-line-estimator.md)
§9, [`unison-assist-design.md`](unison-assist-design.md)), and the session
surfaces in
[`session-persistence-and-profile-library.md`](session-persistence-and-profile-library.md).
Nothing those decided is re-opened here; this note decides only where things go
and what they say when they are empty.

---

## 1. What is wrong

The layout grew one panel at a time, and each panel landed where there was
room. The result groups by **data type and arrival order** rather than by what
the tuner is doing:

- The unison panel landed as a third card in the curve row — beside the strobe
  it belongs *under* ([`main_view.rs:256`](../../tuner-gui/src/views/main_view.rs#L256)),
  because the curve row was where a 360 px card would still fit.
- A dead **Partials** panel held prime space until 2026-08-07: an empty canvas
  behind a comment claiming the pipeline no longer produced partials, which had
  not been true for some time. Nobody noticed, because nothing about the layout
  said what that space was *for*.
- The sidebar has taken three new occupants in nine days — the **Strings**
  declaration, the **`Extended · N.N / M.M s`** progress line, and
  **"Recomputing curve"** — each placed where the capture controls already were
  rather than deliberately. They are already gathered as one parameter
  ([`SessionStatus`](../../tuner-gui/src/views/main_view.rs#L1006)), which is
  the grouping this note formalises.
- Settings grew an **Advanced** heading over Instrument Select, String Isolation
  and Capture Duration — the surfaces an ordinary tuning session never touches.
  That one was deliberate, and it is the precedent: a heading named for *who
  needs it and when*, not for what it contains.

The failure mode is not ugliness. It is that a panel's position carries no
information, so a panel can be wrong (empty, stale, deleted-but-rendered) with
nothing on screen contradicting it.

## 2. The organising principle

Three tasks happen at a tuning session, and each wants a different amount of the
eye:

1. **Context** — *what am I tuning, and what is the plan?* Consulted between
   notes, glanced at while striking. The spectrogram, the note-select surface,
   the tuning curve, the engine's own state.
2. **The live loop** — *what is the eye on while the hand is on the lever?*
   Read continuously, at arm's length, while not looking at the screen for long.
   The strobe and the unison readouts.
3. **The measurement session** — *is the capture I asked for happening?* Only on
   screen during a measurement session, and never during tuning proper. The
   capture button, the string declaration, the extended-take progress, the
   recompute notice.

Tasks 1 and 2 get a column each. Task 3 stays in the sidebar, which is already
where its condition (`measurement_mode_active`) is evaluated, and is now stated
as a grouping rather than left as an accident.

## 3. The arrangement

```
┌─ sidebar ─┐ ┌──────── left: context ────────┐ ┌── right: live loop ──┐
│ Settings  │ │ Inharmonicity — <instrument>  │ │ ┌── Strobe ────────┐ │ pinned
│           │ │ ┌───────────────────────────┐ │ │ │  band + readout  │ │
│ Tools     │ │ │ Spectrogram               │ │ │ │  target · lock   │ │
│  …toggles │ │ └───────────────────────────┘ │ │ └──────────────────┘ │
│           │ │ ┌───────────────────────────┐ │ │ ┌── Unison n* ─────┐ │ ⇅
│ Reference │ │ │ Keyboard Key Select       │ │ │ │  one row, large  │ │ scrolls
│           │ │ └───────────────────────────┘ │ │ └──────────────────┘ │
│ Measure…  │ │ ┌───────────────────────────┐ │ │ ┌── Unison, all ───┐ │
│  Strings  │ │ │ Tuning Curve              │ │ │ │  12 fixed rows   │ │
│  Capture  │ │ └───────────────────────────┘ │ │ │                  │ │
│  Extended │ │ Note A4 · 440.12 Hz · Tracking│ │ └──────────────────┘ │
│  Undo     │ │ [needle, until sunset]        │ │                      │
└───────────┘ └───────────────────────────────┘ └──────────────────────┘
```

**Left — context**, top to bottom: spectrogram, note-select (keyboard or the
guitar picker), tuning-curve plot. The order is how a session reads: what the
instrument is doing acoustically, which key I am on, where that key sits in the
plan. The curve plot is also a select surface, so it sits nearest the keyboard
it duplicates.

**Right — the live loop**, top to bottom: strobe, one-partial unison,
all-partials unison. Descending magnification of the same question, *is this
string where I want it* — the strobe reads one partial to a fraction of a cent,
the one-partial unison reads that same partial's strings, the all-partials panel
shows every partial's strings at once. The eye moves down as the answer needs
more context, and moves down through **the same axis at the same scale**.

## 4. Decisions carried in (2026-08-07, restated so this note stands alone)

- **The strobe takes the prime slot permanently.** A mode-swap — cent meter in
  Auto, strobe in Manual — was considered and rejected: it encodes today's
  limitation, that the strobe needs a user-nominated key, as layout structure,
  and would have to be unpicked exactly when auto detection starts working. The
  panel already degrades gracefully with no target
  ([`main_view.rs:344`](../../tuner-gui/src/views/main_view.rs#L344)).
- **The cent meter is demoted and its needle has a sunset condition.** The
  strobe's coarse read is already a lock-independent, unbounded deviation
  readout ([ADR 0011](../adr/0011-coarse-spectral-readout.md)), so the needle is
  a third rendering of a number shown twice. What the cent meter uniquely
  carries is **engine state** — detected note, frequency, tracking/dropped —
  which is context. Retire the needle when Auto mode can strobe.
- **The all-partials panel absorbs the partials list** (§5, D5).
- **The one-partial panel is a magnified row of the all-partials panel** — same
  axis, same scale, same marker style, so the eye re-learns nothing between
  them.
- **The Partials panel is deleted** (done 2026-08-07).

Two earlier fixes hold in any arrangement and are not layout choices: **fixed
row slots** (one per reference the bank targets, empty rows drawn rather than
omitted) and the **stepped cents ladder**
([`UNISON_SPAN_LADDER`](../../tuner-gui/src/app.rs#L392)). Both exist because a
readout whose scale or row set tracks its own content cannot be read while it
changes.

## 5. Decisions this note takes

### D1 — Both unison panels are on screen at once, and the choose-one question is dissolved

[`TODO.md`](../../TODO.md) carries "which panel layout to keep — drop one once
it has been used on a real instrument", from when the two were rival layouts
behind a toggle. Under this arrangement they are not rivals: the one-partial
panel is the all-partials panel at magnification, and a tuner uses both in the
same minute. Decided with the user 2026-08-20 — the TODO item is out of date and
this note takes priority; the item is rewritten, not deleted, to record that the
question was dissolved rather than answered.

`UnisonMode` is **deleted outright**, not demoted as this note first proposed.
Once the row label draws in both panels — it must, since a magnified row that
dropped its label would not be the same row — the enum decided nothing: the
panels differ in the rows they are handed, which is the caller's business. What
the widget gained instead is a `row_height`, because magnification is the one
thing the renderer does have to know. `Message::ToggleUnisonMode` and
`AppDisplayData::unison_mode` go with it.

**One measured consequence, and it is a gain.** The two layouts disagree about
whether a unison is asserted on 3 of 192 solo captures, because the verdict is
bank-wide while `Displayed` shows one row
([ADR 0014](../adr/0014-unison-panel-against-isolation-truth.md) §4). Behind a
toggle that disagreement was invisible. Side by side it is on screen: **each
panel prints its own resolution figure**, computed from the rows it draws, and
when they differ that difference is the evidence.

### D2 — The right column scrolls; the strobe is pinned above the scroll

The height budget (§6) does not fit a 900 px window, and shrinking the
all-partials rows below 18 px is the wrong lever — 18 px is where a marker and
its blind zone are separable. So the column scrolls.

Scrolling a live instrument out of view mid-tune is the obvious hazard, and the
strobe is the instrument that must never move: it is read without looking away
from the string. So the strobe sits **outside** the scrollable, pinned to the
top of the column, and the unison panels scroll beneath it. The left column
scrolls as one, with nothing pinned — extending the user's answer, because at a
900 px window the left column is also over budget by a little, and nothing in it
is read continuously.

`scrollable` is already used in two views
([`inspector_view.rs:209`](../../tuner-gui/src/views/inspector_view.rs#L209),
[`library_view.rs:263`](../../tuner-gui/src/views/library_view.rs#L263)), so
this adds no dependency and no new idiom. One thing it does need: the row that
holds the two columns must fill its height rather than shrink to content, or the
scrollable has no bound to scroll within and the column simply overflows.
Verified at 1100 × 760, where both columns scroll and the strobe stays put.

### D3 — Every per-panel toggle survives, the live loop gains two, and hiding a panel collapses its space

The five Tools toggles (Spectrogram, Centmeter, Key select, Curve Plot, Strobe)
stay. Two are added, because the live loop is now three panels and `Strobe`
governed all of them: **Unison (partial)** and **Unison (all)**. They answer one
question at three magnifications, and how much of it a tuner wants on screen
changes with the note and with the stage of the job (user, 2026-08-20).

What changes is what the column does with the hole:
today [`wrap_panel`](../../tuner-gui/src/views/main_view.rs#L234) substitutes a
`Space` for a hidden panel, which still consumes the column's `spacing(10)`, so
hiding a panel leaves a gap where it was. Columns are instead **built by pushing
the panels that exist**, so hiding collapses.

That rule is already in the file for exactly one element, with exactly this
reason: the auto-mode notice "sits last and is pushed rather than wrapped"
because "appearing and disappearing must not move the panels above it"
([`main_view.rs:266`](../../tuner-gui/src/views/main_view.rs#L266)). Generalise
it; `wrap_panel` goes.

Two edge states follow and are specified rather than discovered:

- **An empty right column** (all three live-loop panels toggled off) — the
  column is omitted and the left column takes the full width. It does not sit
  there as a 360 px stripe of background.
- **A live-loop panel in Auto mode** — shown **with its instrument drawn
  inert**, not replaced by its own refusal. All three read against a nominated
  key's targets, which Auto has not got, so each says so in the same words: *off
  in Auto mode, it needs a nominated key,* and names the surface that supplies
  one. The strobe draws its band frozen, which is the state it already uses for
  a partial it cannot read; the unison panels draw their fixed row slots empty,
  labelled by partial number and carrying no frequency, because no key has been
  nominated to have one. A panel switched on that draws nothing reads as broken
  rather than as waiting, and a paragraph where an instrument belongs does not
  say what will be there (user, 2026-08-20).
- **An empty left column** — same rule, mirrored. The title row belongs to the
  page, not to the column, so it survives either way.

### D4 — The cent meter keeps its place, and the sunset condition applies to the whole panel

*Revised 2026-08-20, in use.* This note first specified splitting the panel
along the seam
[`CentMeterDisplay`](../../tuner-gui/src/widgets/cent_meter.rs#L115) already has
— a text header (note / frequency / status) over a canvas needle — putting the
header at the foot of the context column as an always-on strip and leaving the
needle a toggleable panel beneath it. Built that way, it read as two orphans:
a bare needle with no label, under a strip with no instrument.

The panel stays whole and stays **beside the spectrogram**, where it was. What
demotion was arguing about was screen position, and the position was not the
problem; the sunset condition was. That stands, and now applies to the panel
rather than to the needle alone: **when Auto mode can strobe, the cent meter
goes.** Until then it is the only deviation readout Auto has, and the only
surface that names the note the engine landed on without a human nominating one
(user, 2026-08-20).

Two records bear on this and are worth keeping together with it:

- The needle is the weaker instrument wherever the strobe works — *"a needle
  shows frequency error (noisy at small offsets), while a strobe shows
  accumulated phase, so even a 0.05-cent error drifts visibly"*
  ([strobe design §3](strobe-and-manual-tuning-ui-design.md)) — which is the
  demotion argument, and it is about Manual mode only.
- In the **deep bass** the strobe band is measurably wrong: it *"rotates
  1.5–6 Hz while the string is within 1.5 Hz of target"* because the bank
  integrates the neighbouring strings, observed as *"a band that spins forever
  in the low bass while the cent meter disagrees"* (Prompt O). That is an
  argument for fixing the gate, not for keeping a needle — but until it is
  fixed, the disagreement is on screen, which is better than not being.

The engine also has a documented policy pointing here: `detected_frequency` is
**partial-1 only**, deliberately, to stay immune to a `B_profile` / `B_true`
divergence that would put an n² cents error into the readout
([ARCHITECTURE.md](../../ARCHITECTURE.md), audit 08 item 7). Whatever replaces
the cent meter inherits that.

### D5 — Unison rows carry their reference frequency and dim by amplitude

This is what makes the all-partials panel absorb the deleted Partials panel.
Each row gains:

- **Its reference frequency**, beside the `n` label — `f_ref` is already in hand
  where the row is built ([`app.rs:1538`](../../tuner-gui/src/app.rs#L1538)) and
  is currently used only to convert Hz to cents and then dropped. With the
  row's markers being signed offsets from that reference, label + marker *is*
  the measured partial frequency, which is strictly more than the old panel
  showed.
- **Dimming by `strobe_amplitude`**, normalised to the strongest reference in
  the bank this hop. The field ships from the pipeline
  ([`lib.rs:112`](../../tuner-core/src/lib.rs#L112)) and is read nowhere in the
  GUI. A floor under the dimming, so a live-but-weak row fades rather than
  vanishing; rows the D3 gate has frozen are drawn dim and frozen, the same
  convention the strobe band uses (strobe design §5.4).

One thing is lost against the deleted panel: it listed the *engine's* tracked
partials and so had something to show in Auto mode, while these rows are the
strobe bank's references and exist only in Manual. Accepted — the panel it
replaces rendered an empty canvas, so the Auto-mode capability was notional.

### D6 — What the unison panel says when the verdict is `Undetermined`

Today, with two or more lines resolved and no verdict, the panel prints
"verdict undetermined — too few partials resolved"
([`main_view.rs:602`](../../tuner-gui/src/views/main_view.rs#L602)). True, and
weak: it names the estimator's difficulty and says nothing about the thing on
screen. On single-strung bass keys the panel typically resolves two markers
25–50 ¢ apart under that caption, and a reader has no way to tell whether that
is a discovery or a bug.

[ADR 0013](../adr/0013-bass-extra-lines-attribution.md) gives it more to say:
those markers are **real spectral content, not a detector artefact**, and their
separation is fixed in Hz across the note's partials — which excludes a second
string. The caption therefore states the class-level fact:

> Undetermined — a second line is not proof of a second string; a single string
> can split this way (ADR 0013).

**A register-conditioned caption is declined.** ADR 0013's finding is about keys
0–27, and saying so on screen would put a key threshold in the frontend that the
DSP deliberately refuses ([ADR 0011](../adr/0011-coarse-spectral-readout.md) §7
declined the same threshold). The sentence above is true at every key, needs no
constant, and is firmer than what it replaces.

### D7 — The amber resolution flag stays, and its two-thirds coverage is not a display bug

With the default display table the resolution figure goes amber on **59 of 88
keys** (ADR 0014 §3): the panel declares itself insufficiently precise across
two-thirds of the compass. That is an honest report of the panel's floor, so the
flag stays as it is, on the comparator it already uses (`UNISON_SPAN_LADDER[0]`
= 3.0 ¢, [`main_view.rs:583`](../../tuner-gui/src/views/main_view.rs#L583)) with
no new constant.

The *reason* the floor sawtooths — `curves::default_display_partials` is chosen
for the strobe band, not for this consumer, so precision jumps coarser at every
`n*` break (B2 3.59 ¢ → C3 5.08 ¢, and twice more) — is a table question, not a
layout one. Recorded in §11 as a follow-up, not fixed here.

### D8 — Session status is a named sidebar section, not a run of pushes under the button

The three occupants of `SessionStatus` (string declaration, extended-take
progress, recompute notice) go under one **Measurement session** heading with
the capture button, on the Advanced precedent: a heading named for who needs it
and when. This is a grouping, not a behaviour change — each element keeps its
current condition, wording and colour.

### D10 — The live-loop panels align on the target, not on an edge

Reported in use, in two rounds. The strobe's band began at the panel's content
edge while the unison plots began after their label gutter, so the column had
two left margins and neither meant anything. Indenting the band by the gutter
fixed the edges and **was still wrong**: the gutter shifts the plot area right,
so the plots' own centre — their zero line — is not the panel's centre, and a
band aligned to their left edge misses it. Centring the band in the panel misses
it the other way (user, 2026-08-20).

**The line the panels share is the target**, which the unison plots draw at the
midpoint of their plot area (`x_of(0)` is exactly that midpoint) and the strobe
represents as the whole band. So the band is centred **in the plot area**, not
in the panel and not against the gutter: gutter, then the plot span, with the
band centred in it. Its centre and the zero line are then the same x at any
panel width, because both are composed from `unison_display::GUTTER` and
`PLOT_RIGHT_MARGIN` rather than from an offset computed at one width. Both
constants are public for that reason — two panels agreeing on a number by
coincidence would drift.

The gutter also carries the same kind of label the unison rows carry, the
partial number, and the strobe's readout moved beneath the band, which the
indent leaves no room for and which reads better anyway: the number belongs
under the instrument.

**Third round: the text follows the plot too.** With the plots inset by the
gutter and the panels' text still running out to the panel's own edge, every
panel had a notch — a band of empty space on the left that nothing accounted
for. A live-loop panel's text is read *against* its plot, so it now starts where
the plot starts and ends where the plot ends (`on_plot_span`). The gutter itself
shrank from 74 px to 52 by shortening what it holds: a row's target drops its
decimal above 1 kHz, which is also the more honest figure — 0.1 Hz at 4.9 kHz is
0.035 ¢, far finer than the display can resolve.

### D11 — The axis is never erased

A live-loop panel that cannot show a reading keeps its axis and its row slots
and says why in its readout line. It does not replace the plot with the
explanation.

The panels already refuse to reflow while a reading is live — fixed row slots, a
held cents span — for a reason that does not stop applying when the reading
stops: the axis is what a returning marker is placed against, and a panel that
becomes a paragraph reads as a panel that has gone missing. Four states, one
shape:

| state | drawn | said |
| --- | --- | --- |
| out of range | slots, markers withheld — past ±21.5 Hz the lines alias | bring the string inside ±21.5 Hz |
| no references yet | empty slots | listening, strike the note |
| targeted, nothing resolved (a decayed note) | the rows, keeping their labels | listening, strike the note |
| Auto mode | empty slots | off in Auto mode, needs a nominated key |

While blocked the panel also withholds its **verdict** and its handoff line:
both are claims about markers that are not on screen.

The tuning-curve plot follows the same rule for the ~2 s its first bundle takes
(§D9's sibling problem, and the one launch always shows): the grid draws with no
series on it and the title carries "computing…". Non-finite cents draw nothing,
so no key can read as measured at 0 ¢ while it waits.

### D14 — Unison assist is an Advanced mode, off by default

*Revised 2026-08-21.* D1 put both unison panels on screen permanently. They are
now behind a **Unison Assist** switch in Settings ▸ Advanced, off by default,
alongside String Isolation and Capture Duration — the surfaces an ordinary
tuning session never touches (user, 2026-08-21).

What D1 decided still holds *within* the mode: the two panels are one
measurement at two magnifications, both on screen, each with its own Tools
entry. What changed is the audience. The panels answer one narrow question — is
this unison set — and cannot answer it below their own `2/T` floor, which on the
instrument ADR 0014 measured covered most of the compass. A surface that is
silent or amber across two-thirds of the keyboard is not one to hand a tuner by
default.

The mode owns the panels' visibility outright: enabling it shows both, disabling
it hides both. That is not a preference but a necessity — the Tools entries that
toggle them appear and vanish with the mode, so a panel left on screen after the
mode went off could not be dismissed. The setting persists with the other
Advanced switches in `AppSettings`.

### D13 — The text slot is reserved, not grown

Reported in use: the panels jump — a verdict or a handoff line appears, the
panel grows, and everything below it moves. The panels had fixed heights before
the task layout and lost them in the rewrite; sizing to content was the
regression.

Restored, and with the rule stated where it belongs: **a unison panel's height
is fixed, and so is the text slot inside it.** `UNISON_FOOTER_HEIGHT` reserves
room for the longest footer either panel produces — a readout wrapping to two
lines plus a verdict or a two-line handoff — and lines appear inside that slot
rather than pushing it open. It is the same rule the row slots follow, for the
same reason, applied to the half of the panel that is prose.

The readout also takes a *share* of its row (`width(Fill)`) rather than its
natural width, so a long message wraps within its share instead of running over
the resolution figure beside it. While blocked there is no figure at all: the
resolution of a reading the panel is not showing is not a fact about anything on
screen.

### D12 — A row label is one line, and the gutter is sized from the label

Reported in use: a row label sometimes wrapped onto a second line and the
overflow landed on top of the row beneath it. Word wrapping is iced's default,
and a fixed-height row slot cannot hold two lines.

Both halves are fixed, and they are different kinds of fix:

- **`Wrapping::None` on every label** — the row labels and the axis ticks. This
  is the invariant: a one-line slot renders one line, so a label that outgrows
  its slot is clipped rather than folded into its neighbour. Overlap stops being
  possible rather than becoming unlikely.
- **`GUTTER` derived from the widest label the format can produce** — nine
  characters at the label size plus the label padding, rather than a number
  chosen by eye. It is an estimate of a font metric and is written as one, which
  is why the first fix carries the guarantee and this one only carries the fit.

### D9 — Labels are widgets, not canvas text (measured 2026-08-20)

Reported in use: the frame rate drops visibly once the unison panels are on
screen. Measured in a debug build by counting spectrogram canvas draws over
30 s, which is the widget the drop is seen in:

| | frames / 30 s | fps |
| --- | --- | --- |
| unison panels hidden | 840 | 28 |
| unison panels drawn | 360 | **12** |
| drawn, canvas text suppressed | 840 | 28 |
| drawn, axis labels only (6 runs) | 540 | 18 |
| drawn, 26 row labels all the **same** string | 540 | 18 |
| drawn, 26 row labels each **distinct** | 360 | 12 |

The geometry is not the cost — the draw closure is 10–12 µs, against the
strobe's 39 µs. **Canvas text is shaped on every frame it is drawn on**, and
identical strings collapse to one shaping while distinct ones do not, which is
why 26 copies of `"n"` cost nothing and 26 real labels cost half the frame rate.

So the labels moved out of the canvas into the widget tree: a gutter column of
`text` widgets and an axis strip above the plot, with the canvas left drawing
what actually moves — grid lines, blind zones, markers. iced re-shapes a text
widget only when its content changes, and these change only when the key does.
**Restored in full: 840 frames with the panels on, identical to hidden.**

The rule this leaves: *canvas text is for what moves.* A label that changes when
the key changes belongs in the widget tree.

## 6. Height budget, measured

Panel heights are fixed in the source, so the budget is arithmetic rather than a
guess. As built:

| | | px |
| --- | --- | --- |
| **Left** | spectrogram \| cent meter, side by side | 250 |
| | note select | 200 |
| | curve plot | 240 |
| | spacing, 2 × 10 | 20 |
| | **column** | **710** |
| **Right** | strobe | 270 |
| | unison, magnified (`48 + 22` canvas + chrome) | ~190 |
| | unison, stacked (`12 × 18 + 22` canvas + chrome) | ~345 |
| | spacing, 2 × 10 | 20 |
| | **column** | **~825** |
| **Page** | title row + gap | 48 |
| | outer padding | 40 |

So the right column wants **~913 px** of window and the left **798 px**. A maximised 1080p window has roughly 1000–1040 px
usable after decorations and a panel, so both fit; a 1600 × 900 window has about
820, and both scroll. Confirmed by building at both — at 1100 × 760 the left
column scrolls as one and the right scrolls beneath a pinned strobe.

The unison panels are sized by their content rather than pinned to a fixed
height, so those two rows are approximate; what is fixed is the part that must
be, the **canvas**, at one row slot per reference the bank can target.

Two levers are deliberately **not** used. Shrinking the stacked rows below 18 px,
because that is where a marker and its `2/T` blind zone stop being separable; and
dropping the magnified panel at small heights, because a panel that disappears
when the window is resized is the panel-by-panel drift this note exists to undo.

## 7. What changes in the code

Confined to the frontend, and mostly to one function:

- [`main_view.rs`](../../tuner-gui/src/views/main_view.rs) —
  `create_widget_area` rebuilt as two columns (§3); `wrap_panel` deleted in
  favour of push-if-present (D3); the cent meter kept whole beside the
  spectrogram (D4); the unison panel built twice with different row sets and
  each printing its own resolution (D1); `Undetermined` caption (D6); sidebar
  gains the **Measurement session** heading (D8).
- [`unison_display.rs`](../../tuner-gui/src/widgets/unison_display.rs) —
  `UnisonRow` gains `ref_hz` and a per-row amplitude; row label renders the
  frequency; row opacity follows the amplitude (D5).
- [`app.rs`](../../tuner-gui/src/app.rs) — `update_unison` fills the two new row
  fields from data it already has; `unison_mode` and `ToggleUnisonMode` removed
  (D1); `unison_displayed_visible` / `unison_all_visible` and their two messages
  added (D3). No other state changes: every panel above reads fields that
  already exist.
- No `tuner-core` change of any kind. No new crossing, no new module.

## 8. What this must not disturb

- **`app.rs` holds no DSP and widgets stay stateless renderers**
  ([`01-architecture.md`](../internals/01-architecture.md) §"Widgets are
  stateless renderers"). Everything here is composition and text.
- **The six crossings are untouched.** The strobe reference push, the worker
  jobs, and the capture commands all keep their current shape
  ([`02-cross-thread-communication.md`](../internals/02-cross-thread-communication.md)).
- **Nothing the readouts say gets weaker.** The unison panel's three
  non-negotiables — the current resolution, the beat rate in Hz, the visible
  verdict — survive the relayout, as does the handoff line ("listen, or mute two
  strings and tune each one on the strobe") and the filled `2/T` blind zone.
  They are what stop the panel over-claiming (ADR 0012 §9).
- **The strobe's curve-lock footer and its ✗ advisory** keep their place in the
  panel; both are about trusting the *target*, which is the strobe's own
  question.

## 9. Open, and not settled here

Whether the strobe should follow the **engine's** identified key once auto
detection is trusted. Architecturally it is fine — the GUI would read
`FrameOutput.note_index` and push it over crossing #4, so the frontend still
nominates and the strobe stays a tap. The unsolved part is **retarget thrash**:
every key change resets the accumulated angle and the unison rings, so an
auto-following strobe would reset constantly while the tuner moves around. It is
also the trigger for D4's sunset, so it wants its own decision, not a paragraph
here.

## 10. Sequencing — all done 2026-08-20

1. Two-column composition with the panels exactly as they render today (§3, D3)
   — the shape, verified against a real window before anything else moves.
2. Scroll and pin (D2), checked at 1080p maximised and at 900 px.
3. Cent-meter split (D4) and the sidebar heading (D8) — both pure moves.
4. Both unison panels, each with its own resolution figure (D1).
5. Row frequency and amplitude dimming (D5); caption (D6).
6. `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
   and a run of the app at both window sizes.

## 11. Follow-ups

- ~~`00-overview.md`'s file map lists `partials_display.rs`~~ — removed with the
  build; the widget was deleted 2026-08-07.
- ~~`TODO.md`'s "which panel layout to keep"~~ — rewritten per D1: dissolved,
  not answered.
- **The same pattern is still in `curve_plot`** (D9): ~12 distinct axis labels
  drawn into the canvas every frame, in a panel that is always on screen.
  `envelope` and `seismograph` have it too, but they are calibration views.
  Not touched here — it was not what was reported — and worth the same
  treatment if frame rate matters again.
- **The display table serves the strobe, not the unison panel** (D7): a
  consumer-specific `n*` would lift the panel's floor across the compass, and is
  an estimator/table question for whoever next opens ADR 0014's queue.
