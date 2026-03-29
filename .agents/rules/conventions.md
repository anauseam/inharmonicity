---
trigger: model_decision
description: When working on the tuner-core structure.
---

# Inharmonicity — Project Conventions

> These conventions govern how code is structured in `tuner-core` and `tuner-gui`.
> Items marked **[OPEN]** are active design questions — do not treat them as settled.

---

## Crate Boundary: `tuner-core` vs `tuner-gui`

- **`tuner-core` is headless.** Zero GUI code, zero GUI-specific types, zero references to `iced`, `egui`, or any frontend framework. Any frontend must be able to consume it.
- **`tuner-gui` depends on `tuner-core`, never the reverse.**
- **Coupling is minimized.** The GUI interacts with `tuner-core` exclusively through:
  1. `AudioPipeline::new()` → `(AudioPipeline, PipelineHandle)` (Split/Handle pattern)
  2. `PipelineHandle` for reading `RuntimeState` / writing `ConfigState` via `Arc<Mutex<..>>`
  3. A crossbeam channel for receiving pipeline output
  4. Standalone async utilities like `calibration::calibrate_noise_floor()`

---

## Pipeline Ownership (Split / Handle Pattern)

- **`AudioPipeline` owns all real-time DSP components**: `Gatekeeper`, `Engine`, `AudioPool`.
- **`PipelineHandle`** is the GUI's only window into shared state.
- **DSP components are "pure."** `Gatekeeper` and `Engine` have zero knowledge of `Arc`, `Mutex`, shared state, or the GUI. They expose results via `pub` fields. The `AudioPipeline` reads those fields and syncs them to shared state.
- **`process_frame()` is the single DSP entry point:**
  1. Reads config from shared state into DSP components
  2. Runs the Gatekeeper (signal validation)
  3. Runs the Engine (F0 detection) when the Gatekeeper approves the signal
  4. Syncs observations back to shared state

---

## Module Layout: `algorithms/` vs `models/`

### `algorithms/` — Pure, Stateless DSP Math

Every function in `algorithms/` is **stateless**: takes input buffers, returns computed values. No side effects, no `Arc`, `Mutex`, channels, or global mutable state.

Organized by **primary output**:

| Module | Domain | Returns |
|---|---|---|
| `pitch.rs` | Pitch detection (YIN/pYIN) | Frequency (Hz), confidence |
| `dpyin.rs` | Bass pitch detection (standalone, >200 lines) | Frequency (Hz), confidence |
| `scout.rs` | Rough frequency neighborhood | Frequency (Hz) |
| `spectral.rs` | Time ↔ frequency transforms, windowing, magnitudes | Complex spectra, magnitude vectors |
| `metrics.rs` | Signal property measurement | RMS, EMA, CSD, NINOS2 scalars |
| `tuning.rs` | Tuning math (cent deviations, compensated frequencies) | Cent values, target frequencies |
| `inharmonicity.rs` | B-coefficient calculation (deprecated, pending replacement) | B coefficient |

**Sizing rule:** If an algorithm exceeds ~200 lines or introduces its own internal types (e.g., `BiquadCoeffs`, `PitchCandidate`), it gets its own file. Otherwise it belongs in the group file.

**Shared primitives** reused across algorithms (e.g., `parabolic_interpolation_offset`, `yin_difference`) live in their group file with `pub(crate)` visibility.

### `models/` — Domain Data Types & Lookup Tables

Non-DSP domain types, lookup tables, and data structures used by the GUI or other consumers. Not algorithms — domain knowledge.

Current contents:

| Item | Description |
|---|---|
| `Note` | Note name + frequency |
| `NOTES`, `NOTE_MAP` | 88-key piano lookup tables (lazy statics) |
| `find_nearest_note()` | Frequency → note name lookup |
| `find_nearest_note_by_index()` | Key index → note name + frequency |
| `get_key_index_from_name()` | Note name → key index |
| `Partial` | Measured partial (number + frequency) |
| `KeyMeasurement` | Partials for one key + computed B value |
| `InharmonicityProfile` | Full piano profile (serializable to JSON) |

> `models/` may start as a single `models.rs` file and grow into a `models/` directory
> with submodules (e.g., `models/note.rs`) as needed.

---

## Top-Level `tuner-core` Modules

| Module | Role | Status |
|---|---|---|
| `pipeline.rs` | Mediator, shared state types, memory infrastructure | ✅ Implemented |
| `engine.rs` | F0 detection chain (Scout → Router → Bass/Treble). Owned by `AudioPipeline` | ✅ Active |
| `gatekeeper.rs` | 5-state signal validator. Pure DSP. Owned by `AudioPipeline` | ✅ Implemented |
| `worker.rs` | Background worker for offline DSP (B coefficient). Communicates via channels | ⬜ Wireframe |
| `audio.rs` | CPAL audio capture, DC blocking, stream setup. Bootstrap only | ✅ Implemented |
| `calibration.rs` | Async calibration utilities (noise floor, future calibrations) | ✅ Implemented |
| `algorithms/` | Pure stateless DSP math | ✅ Active |
| `models/` | Domain data types and lookup tables | ✅ Implemented |

### Files Slated for Removal

- **`capture_processing.rs`** — Legacy frame processing. Will be replaced by Gatekeeper (States 3-4) + WorkerManager. **Do not add new functionality here.**

---

## Open Design Questions

> **[OPEN]** items are unresolved. Do not treat them as settled.
> Ask the user before making decisions that depend on them.

- **[OPEN] Pipeline output type.** `AnalysisResult` in `lib.rs` is legacy with GUI-specific fields. The pipeline's output type will be determined as Engine integration completes. Do not assume it is the final design.
- **[OPEN] Overlapped frames.** Currently non-overlapping 2048-sample frames. A 50% COLA design is planned post-migration. Whether this becomes a runtime setting or compile-time architecture choice is undecided.
- **[OPEN] `models/` growth pattern.** Whether `models` stays as a single file or becomes a directory with submodules will be decided as more types are added.

---

## Rust Conventions

- Follow **standard Rust conventions**: `snake_case` functions/variables, `CamelCase` types, `SCREAMING_SNAKE_CASE` constants.
- Use `///` doc comments on all public items. Include `# Arguments`, `# Returns`, and `# Panics` sections where applicable.
- Prefer `pub(crate)` over `pub` for items not part of the external API.
- Use `#[inline]` only on small, frequently-called functions in tight DSP loops.
- Prefer `assert!` for programmer errors (wrong buffer sizes) and `Option`/`Result` for runtime conditions (silence, no pitch detected).

## Memory Allocation

This project runs on `std` (Linux, iced, cpal). Follow std-idiomatic allocation patterns —
not embedded/no-std patterns — for owned DSP state:

| Use case | Idiom | Example |
|---|---|---|
| Owned DSP state buffer, fixed size | `Box<[T]>` via `into_boxed_slice()` | `ProcessingFrame`, `CircularFifo` |
| Compile-time lookup table / const data | `[T; N]` | `beta_thresholds()` in `dpyin.rs` |
| Object pool items (crossbeam) | `Box<[T; N]>` | `AudioPool` |
| Algorithms — input/output | `&[T]` / `&mut [T]` slices | All `algorithms/` functions |
| **Never** on the audio hot-path | `Vec::push`, `Vec::new`, any heap alloc | — |

**The `Box<[T]>` idiom:**

```rust
// Allocate once at startup — never resize, never reallocate.
// Box<[T]> is smaller than Vec<T> (16 bytes vs 24) and communicates
// "fixed capacity" at the type level.
buffer: vec![0.0_f32; WINDOW_SIZE].into_boxed_slice()
```

**What this is NOT:** The embedded no-std `[T; N]` const-generic pattern (from `heapless` etc.)
is not used here. Inline fixed-size arrays are reserved for small stack temporaries and
lookup tables returned from pure functions, not for large owned state buffers.


---

## `tuner-gui` Conventions

- **Widgets are stateless renderers.** They receive data and return `Element`s. No owned application state.
- **Views compose widgets.** `main_view.rs` and `settings_view.rs` arrange widgets into layouts.
- **`app.rs` is the state hub.** All application state, message handling, and thread management lives here. DSP logic is being migrated out.