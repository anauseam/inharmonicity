# `retiring/` — staged for the documentation site

These reports and audits belong to what is **stable**: the audio front end,
the Gatekeeper, the MAT `(f₀, B)` estimator, and Rigaud's piano model — a
published model that engine (a) ports end to end, stable even though the
tuning-curve engines composed on it are still being validated. Their findings are
being written up as pages on the documentation site. Each report leaves the
repository once its page is published; the audits wait on one more thing,
below.

Nothing here is deprecated, and nothing here has been edited for the move. This
directory is a **queue**, not a graveyard: a file sits here while it is being
turned into a lesson, and is deleted when the lesson exists.

## What each file feeds

| File | Subsystem | Where it is going |
| --- | --- | --- |
| [`0019-dc-blocker-corner.md`](0019-dc-blocker-corner.md) | audio front end | the DC-blocking page — the reopen condition and the exact-inversion trick are the two facts the guard does not carry |
| [`0003-gatekeeper-rejection-of-sfm.md`](0003-gatekeeper-rejection-of-sfm.md) | Gatekeeper | the transient-detection page — why spectral flatness was tried and removed |
| [`0015-ambient-sigma-gates-measured.md`](0015-ambient-sigma-gates-measured.md) | Gatekeeper | the amplitude-and-smoothing page — the one finding that survives is that the defect is *spectral*, not temporal |
| [`0004-instrument-scope.md`](0004-instrument-scope.md) | whole system | the site's overview — the three assumption layers, which is argument rather than measurement |
| [`faithfulness-audit-05-metrics.md`](faithfulness-audit-05-metrics.md) | Gatekeeper | the transient and sparsity pages. **Read this one first:** the site currently mis-attributes both NHWRSF and NINOS², and this audit is the corrective source. The metric it exonerates is `inverse_participation_ratio`, which the code called `ninos2` at audit time; every report here uses the shipped name |
| [`faithfulness-audit-02-cspe.md`](faithfulness-audit-02-cspe.md) | MAT | the MAT pages — sub-bin refinement |
| [`faithfulness-audit-06-b-prior.md`](faithfulness-audit-06-b-prior.md) | MAT | the MAT pages — the Rigaud inharmonicity prior |
| [`faithfulness-audit-07-mat.md`](faithfulness-audit-07-mat.md) | MAT | the MAT pages — the estimator itself |
| [`faithfulness-audit-08-goertzel.md`](faithfulness-audit-08-goertzel.md) | MAT | the MAT pages — the Goertzel recurrence and the phase property the strobe rests on |
| [`faithfulness-audit-09-rigaud.md`](faithfulness-audit-09-rigaud.md) | the piano model | the Rigaud page, written thoroughly: the whole-compass inharmonicity model B_ξ(m), the octave-type curve ρ_φ(m), and how a tuning curve is generated from them. Audit 06 checks the same paper's Eqs 7–8 as the discovery prior uses them |

## When a file may be deleted

A **report** leaves when both hold, not either:

1. Its page is published on the site, carrying the findings a reader needs.
2. **No `//` guard in `tuner-core` or `tuner-gui` still rests on it.**

One fails the second condition today: `audio.rs` guards `DC_BLOCK_ALPHA` with
`report 0019`. The guard keeps its facts either way — the numbers are already in
the comment — but where the pointer should go once the report is gone is an open
question: the site page's URL is the obvious answer and has not been ratified.
0003, 0004 and 0015 rest on nothing and may go as soon as they are written up.

An **audit** needs a third condition, and it is the binding one:

3. **Its port has an executable reference implementation with a differential
   test in the repository, and the test carries the audit's verdict itself** —
   which equations it holds the port to, citing the paper.

Every audit has two halves, and each needs a home before the file can go. The
*paper specification* — what the source says — is explanation, and the page
takes it. The *verdict* — that the code matches the paper, equation by
equation — is a claim about code that keeps changing, and only a test keeps it
true; the planned verification work gives each ported algorithm an independent
reference implementation to diff against. So a page may be written now, which
is what staging here is for, but the file stays until its test exists.

The halves weigh differently. Audit 02's verdict — faithful, no findings — is
the part worth keeping, and a test is its natural home. Audit 09 inverts that:
Rigaud's model is itself a subject the site has to teach thoroughly, so its
page reaches well past what the audit records, while the verdict still has to
stay true of `rigaud.rs`.
