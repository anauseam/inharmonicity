---
description: How to add a new DSP algorithm to tuner-core/src/algorithms/
---

# Adding a New Algorithm to `tuner-core`

## 1. Determine placement

Decide whether the algorithm belongs in an **existing submodule** or needs a **new file**:

| Situation | Action |
|---|---|
| Small function (<50 lines), fits an existing domain | Add to the existing group file |
| Large algorithm (>200 lines) or has its own internal types | Create a new file: `tuner-core/src/algorithms/<name>.rs` |
| Shared primitive reused by multiple algorithms | Add to the group file with `pub(crate)` visibility |

**Algorithm domains** (organized by what the function *returns*):

| File | Domain | Returns |
|---|---|---|
| `pitch.rs` | Pitch detection | Frequency (Hz), confidence |
| `dpyin.rs` | Bass pitch detection (standalone) | Frequency (Hz), confidence |
| `scout.rs` | Rough frequency neighborhood | Frequency (Hz) |
| `spectral.rs` | Time ↔ frequency transforms | Complex spectra, magnitude vectors |
| `metrics.rs` | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
| `tuning.rs` | Tuning math | Cent deviations, compensated frequencies |
| `inharmonicity.rs` | Inharmonicity math | B coefficient |

> **Note:** After the restructure, `fft.rs` becomes `spectral.rs` and `power.rs` becomes `metrics.rs`.

## 2. Write the algorithm

All algorithms must follow these rules:

- **Stateless.** No `&mut self` on module-level state. Functions take input slices and return values.
- **No shared state.** No `Arc`, `Mutex`, channels, or global mutable state.
- **No heap allocation if used on the audio hot-path.** Use pre-allocated scratch buffers passed via `&mut [f32]`.
- **Document everything.** `///` doc comments with `# Arguments`, `# Returns`, `# Panics` sections.

Template for a new file:

```rust
//! # <Algorithm Name> — <One-line description>
//!
//! <Longer description of what this algorithm does, when it's used,
//! and any relevant references or papers.>

/// <Function description>
///
/// # Arguments
/// * `input` — <description>
/// * `scratch` — <description of scratch buffer requirements>
///
/// # Returns
/// * `Some((frequency, confidence))` if pitch detected, `None` otherwise.
pub fn detect_pitch_<name>(
    input: &[f32],
    sample_rate: u32,
    threshold: f32,
    scratch: &mut [f32],
) -> Option<(f32, Option<f32>)> {
    // Implementation
    todo!()
}
```

## 3. Register the module

If you created a new file, add it to `tuner-core/src/algorithms.rs`:

```rust
pub mod new_algorithm;
```

Also update the module-level doc comment table in `algorithms.rs` to include the new module.

## 4. Wire into the Engine (if it's a pitch detection algorithm)

If this is a pitch detection algorithm used by the Engine:

1. Add a variant to `TrebleAlgorithm` or `BassAlgorithm` in `engine.rs`
2. Add the match arm in `Engine::process()` that calls the new function
3. The function signature should match: `fn(...) -> Option<(f32, Option<f32>)>` — tuple of `(frequency_hz, optional_confidence)`

## 5. Build and verify

// turbo
```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo build 2>&1 | head -50
```

// turbo
```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo test -p tuner-core 2>&1
```

// turbo
```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo clippy -p tuner-core -- -W clippy::all 2>&1 | head -30
```
