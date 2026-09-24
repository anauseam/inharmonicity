# report NNNN — <the quantity measured>

A numbered report. If it decided something, the decision is one line in
`ARCHITECTURE.md` § Decisions or the guard comment at a definition, and the
measurement that supports it is here.

> Delete this blockquote when you copy the template.
>
> This is a **publication**, not documentation. It answers to
> [`README.md`](README.md)'s methods standard: pre-registration before the run,
> a named null, exploratory findings labelled as such, and enough written down
> that a reader who was not there could disagree with the conclusion.
>
> Sections moved here from a decision record stayed byte-identical through the
> 2026-09 refactor; edits are ordinary from here. A superseded claim is struck through and pointed at its replacement —
> never edited into agreement, never deleted.

## Pre-registered decision rule

Written **before** the measurement ran. What would make us keep the thing,
change it, or remove it — thresholds, directions, and the anchor each threshold
came from.

State the **null** here too. A piano's spectrum is dense enough to "explain"
almost anything by coincidence; the null is chosen in advance, not once the
scores are in.

## Population

Which instruments, which keys, how many captures, and what state the
instruments were in. Note what the population cannot support: with one or two
instruments, a difference of a few keys is the McNemar-p ≈ 0.2 class of
evidence. The sets themselves are described in
[`capture-sets.md`](../docs/internals/capture-sets.md).

## Method

What was run, at what settings, against what reference.

## Results

The numbers. Report per-register counts and which keys moved, not just a total.

## Exploratory

**Labelled, and separate.** Everything outside the pre-registered rule goes
here. Report it in full — it is often the most interesting part — but it may
not move the decision on its own. It queues work.

Distinguish *suggestive* from *found*, in words. A reader six months from now
cannot recover your confidence from the number alone.

## Limitations

What would change the conclusion. What the population could not test.

## Reproduce

`cargo lab <subsystem> <mode> [args]`, or the commit the figures were made at.
