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
  2. `PipelineHandle.atomics` for reading `RuntimeAtomics` / writing `ConfigAtomics` and the `CaptureState` lifecycle atomic
  3. `PipelineHandle.result_rx` for receiving `KeyMeasurement` results from the Background Worker
  4. A `triple_buffer` for receiving the live, continuous `FrameOutput` from the DSP thread

---

## Pipeline Ownership (Split / Handle Pattern)

- **`AudioPipeline` owns all real-time DSP components**: `Gatekeeper`, `Engine`, `AudioPool`, and inline capture accumulation.
- **`PipelineHandle`** is the GUI's only window into shared state. It carries `Arc<PipelineAtomics>` and the `result_rx` crossbeam SPSC receiver.
- **DSP components are "pure."** `Gatekeeper` and `Engine` have zero knowledge of `PipelineAtomics`, shared state, or the GUI. The `Gatekeeper` returns a `GateResult` by value; the `Engine` returns `Option<PitchResult>`. The `AudioPipeline` reads these return values and syncs observations to shared atomic state.
- **`process_cola_hop()` is the single DSP entry point:**
  1. Reads config from shared atomics into DSP components
  2. Runs the Gatekeeper (signal validation) — receives `GateResult`
  3. Runs the Engine (F0 detection) when the Gatekeeper approves the signal
  4. Syncs observations back to shared atomics
  5. Manages capture accumulation (Armed → Recording → dispatch to Worker)

---

## Module Layout: `algorithms/` vs `models/`

### `algorithms/` — Pure, Stateless DSP Math

Every function in `algorithms/` is **stateless**: takes input buffers, returns computed values. No side effects, no lock-free channels, atomics, or global mutable state.

Organized by **primary output**:

| Module | Domain | Returns |
| --- | --- | --- |
| `spectral.rs` | Time ↔ frequency transforms, windowing, magnitudes | Complex spectra, magnitude vectors |
| `templates.rs` | 88-key sparse matched-filter generation (2-asymptote β model) | `SparseTemplate` arrays |
| `mat.rs` | Median-Adjustive Trajectories algebraic combinatorial solver | Refined $f_0$, partial count |
| `phantom.rs` | Predictive Phantom Partial Mask for intermodulation products | In-place magnitude zeroing |
| `pitch.rs` | XQIFFT/Quinn sub-bin estimation, unison coherence check | Frequency (Hz), coherence flags |
| `twm.rs` | Two-Way Mismatch (superseded by Dot-Product Correlation) | Frequency (Hz) |
| `metrics.rs` | Signal property measurement | RMS, EMA, NHWRSF, NINOS2 scalars |
| `tuning.rs` | Tuning math (cent deviations, compensated frequencies) | Cent values, target frequencies |
| `inharmonicity.rs` | B-coefficient calculation (pending replacement) | B coefficient |

**Sizing rule:** If an algorithm exceeds ~200 lines or introduces its own internal types (e.g., `BiquadCoeffs`, `PitchCandidate`), it gets its own file. Otherwise it belongs in the group file.

**Shared primitives** reused across algorithms (e.g., `parabolic_interpolation_offset`, `yin_difference`) live in their group file with `pub(crate)` visibility.

### `models/` — Domain Data Types & Lookup Tables

Non-DSP domain types, lookup tables, and data structures used by the GUI or other consumers. Not algorithms — domain knowledge.

Current contents:

| Item | Description |
| --- | --- |
| `Note` | Note name + frequency |
| `NOTES`, `NOTE_MAP` | 88-key piano lookup tables (lazy statics) |
| `find_nearest_note()` | Frequency → note name lookup |
| `find_nearest_note_by_index()` | Key index → note name + frequency |
| `find_nearest_note_index()` | Frequency → key index (allocation-free) |
| `get_key_index_from_name()` | Note name → key index |
| `Partial` | Measured partial (number + frequency) |
| `KeyMeasurement` | Partials for one key + computed B value |
| `InharmonicityProfile` | Full piano profile (serializable to JSON) |

> `models/` may start as a single `models.rs` file and grow into a `models/` directory
> with submodules (e.g., `models/note.rs`) as needed.

---

## Top-Level `tuner-core` Modules

| Module | Role | Status |
| --- | --- | --- |
| `pipeline.rs` | Mediator, shared state types, memory infrastructure, capture accumulation | ✅ Implemented |
| `engine.rs` | F0 detection chain (3-Stage Matched Filter: Correlation → Phantom Mask → MAT). Owned by `AudioPipeline` | ✅ Implemented |
| `gatekeeper.rs` | 5-state signal validator. Pure DSP, returns `GateResult`. Owned by `AudioPipeline` | ✅ Implemented |
| `worker.rs` | Background worker for heavy offline DSP (high-res FFT, Template Matcher, MAT, β calculation, diagnostics I/O). Communicates via crossbeam SPSC | 🟡 Testing |
| `audio.rs` | CPAL audio capture, DC blocking, stream setup, standalone host extension (`AudioSource`, `HostHandle`, `spawn_analysis_thread`) | ✅ Implemented |
| `cola.rs` | CircularFifo — COLA overlapping frame analysis | ✅ Implemented |

| `algorithms/` | Pure stateless DSP math | ✅ Active |
| `models/` | Domain data types and lookup tables | ✅ Implemented |



## Open Design Questions

> **[OPEN]** items are unresolved. Do not treat them as settled.
> Ask the user before making decisions that depend on them.

- **[SETTLED] Pipeline output type.** `FrameOutput` is the permanent, unified zero-allocation transport mechanism. All continuous metrics (`rms_ema`, `nhwrsf`), discrete conditions (`is_silence`, `note_index`, `detected_frequency`, `cents_deviation`), real-time partial frequencies, and the `suspend_beta_update` flag are packed structurally into this struct via `triple_buffer`.
- **[SETTLED] Worker output.** `KeyMeasurement` results from the background worker are delivered to the GUI via a bounded crossbeam SPSC channel (`PipelineHandle.result_rx`). This path is non-realtime and permits heap-allocated fields.
- **[SETTLED] Capture lifecycle.** `CaptureState` is a baton-pass `AtomicU8` with three writers: GUI (Idle ↔ Armed), Pipeline (Armed → Recording, Recording → Processing), Worker (Processing → Idle).
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