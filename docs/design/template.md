# <Subject> — design note

**Status:** Draft | Accepted
**Tags:** exploratory (omit when the note is meant to be built)

> Delete this blockquote when you copy the template.
>
> A design note is where a change is *argued* and alternatives are weighed. It
> may be long, it may change, and it may be wrong — those are the properties
> that distinguish it from a *Decisions* entry in `ARCHITECTURE.md`, which is
> a short paragraph and records only what was decided.
>
> Mark a note `exploratory` when it exists to think with and nothing should be
> built from it yet. The tag is load-bearing: it is what stops a later reader
> treating a sketch as a specification.
>
> When a note's measurements were made in a model (Python, a spreadsheet) rather
> than in the app, its numbers are the *model's*. Porting is a measurement in
> its own right and may overturn them, so the port's figures go in a `reports/`
> report.
>
> A note lives only while its proposal is argued. `Accepted` means agreed and not
> yet built. The note is deleted in the change that ships the work, or as soon as
> the proposal is dropped; before it goes, move its open items to `TODO.md` and
> anything a reader still needs into the code docs or a report. Git history keeps
> the argument.

## Context and scope

What prompted this, and what is deliberately out of scope.

## Goals and non-goals

Non-goals are the more useful half. Write them.

## Design

The proposal itself.

## Alternatives considered

Each with the reason it was not chosen. An alternative dismissed without a
reason will be re-proposed within the year.

## Decision rule

**Required if this note will be measured. Write it before the run.**

What result would make us keep this, change it, or remove it — with thresholds
and directions, each threshold anchored to a prior measurement, a published
criterion, or the estimator's own scatter. An unanchored bar is a **product
judgment** and is labelled as one.

Everything measured outside this rule is exploratory and may not move the
decision on its own. The full standard is
[`reports/README.md`](../../reports/README.md).
