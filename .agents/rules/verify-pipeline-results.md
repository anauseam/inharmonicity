---
trigger: model_decision
description: When diagnosing issues or analyzing pipeline results from CLI output.
---

# Verifying Pipeline Results

**CRITICAL RULE FOR ALL AGENTS:** Do not treat raw CLI logs (like `eprintln!` traces) as definitive proof that the entire DSP pipeline is working flawlessly.

These print traces (e.g. `[GATEKEEPER] Transition: Unstable -> Stable` or `Scout Locked: Bass Engine`) exist intentionally for human debugging and must **not** be removed. However, they only confirm *structural routing*. They provide absolutely no information on whether the Two-Way Mismatch (TWM) algorithm or XQIFFT actually succeeded in extracting the requested frequency.

## What Agents Must Do Instead:
- If you see a healthy logging progression routing to a specific engine, you must still actively verify that the F0 Pitch Engine isn't silently failing or dropping requests downstream.
- You must rely on explicit numeric unit tests (such as within `tuner-core/tests/`) that inject predetermined waveforms and explicitly assert `(f32, Option<f32>)` target extractions. 
- You are forbidden from concluding "the pipeline is identifying notes perfectly" just because the CLI log trace reached `Stable` without panicking.
