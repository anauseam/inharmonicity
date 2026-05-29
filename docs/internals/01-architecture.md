# Architecture & Module Boundaries

This file documents the structural decisions that are stable: how the two
crates relate, how the pipeline's components are owned, and what each
component is allowed to know about the rest of the system.

## Crate boundary: `tuner-core` vs `tuner-gui`

`tuner-core` is **headless**. No GUI code, no GUI-specific types, no
dependencies on `iced`, `egui`, or any frontend framework. Any future
frontend — a CLI, a WebAssembly viewer, an alternative GUI — must be
able to consume the crate as-is.

`tuner-gui` depends on `tuner-core`. The reverse never holds.

The GUI interacts with the core through exactly four channels:

1. `AudioPipeline::new()` returns `(AudioPipeline, PipelineHandle)` — the
   Split / Handle pattern. The GUI keeps the handle; the audio thread
   takes the pipeline.
2. `PipelineHandle.atomics` — wait-free reads of `RuntimeAtomics` and
   wait-free writes of `ConfigAtomics`, plus the `CaptureState`
   lifecycle atomic.
3. `PipelineHandle.result_rx` — a crossbeam SPSC receiver for
   `KeyMeasurement` results coming back from the Worker thread.
4. A `triple_buffer` carrying the live, continuous `FrameOutput` from
   the DSP thread to the GUI for visualization.

Anything that doesn't fit one of these four shapes is a sign that the
boundary is being violated.

## Pipeline ownership (Split / Handle pattern)

`AudioPipeline` owns the real-time DSP components: `Gatekeeper`,
`Engine`, the COLA `CircularFifo`, the `AudioPool`, and the inline
capture-accumulation state. It is moved to the audio thread and is the
only thing that mutates the pipeline's internal state.

`PipelineHandle` is the GUI's only window into shared state. It carries
`Arc<PipelineAtomics>` and the `result_rx` receiver — nothing more.

A frontend contributor calls `AudioPipeline::new()`, gets a
`PipelineHandle`, and never needs to know about Gatekeeper internals,
EMA calculations, or lock management.

## DSP components are pure

`Gatekeeper` and `Engine` have zero knowledge of `PipelineAtomics`,
shared state, or the GUI. They are stateful (each has internal
buffers and a state machine) but they expose their results by value:

- `Gatekeeper::process_frame` returns a `GateResult`.
- `Engine::process` returns an `Option<PitchResult>`.

The `AudioPipeline` reads these return values and syncs observations
back to the shared atomics. Nothing else does.

This convention means the components are unit-testable in isolation and
the data flow is auditable from a single file (`pipeline.rs`).

## `process_cola_hop` is the single DSP entry point

Every audio hop runs exactly one function:
`AudioPipeline::process_cola_hop`. Its body, in order:

1. Reads config from shared atomics into the DSP components.
2. Runs the `Gatekeeper` for signal validation; receives a `GateResult`.
3. Runs the `Engine` for F0 detection when the Gatekeeper approves the
   signal.
4. Syncs observations back to the shared atomics and produces a
   `FrameOutput` for the GUI's `triple_buffer`.
5. Manages capture accumulation: `Armed → Recording → dispatch to
Worker` via `CaptureState`.

New DSP behaviour goes inside this function (or in a component it
already calls). Bypassing it — for example, having `audio.rs` call into
`Engine` directly — is what this rule exists to prevent.

## `tuner-gui` internal conventions

- **Widgets are stateless renderers.** They receive data and return
  `Element`s. They do not own application state.
- **Views compose widgets.** `main_view.rs` and `settings_view.rs`
  arrange widgets into layouts; they do not implement DSP.
- **`app.rs` is the state hub.** All application state, message
  handling, and thread management live there. DSP logic that has
  historically leaked into `app.rs` is being migrated into
  `tuner-core` — new code should not add to that backlog.
