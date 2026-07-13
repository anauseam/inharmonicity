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

The GUI interacts with the core through exactly five channels:

1. `AudioPipeline::new()` returns `(AudioPipeline, PipelinePorts)` — the
   Split / Handle pattern. The GUI keeps the ports; the audio thread
   takes the pipeline.
2. `PipelinePorts.handle.atomics` — wait-free reads of `RuntimeAtomics`
   and wait-free writes of `ConfigAtomics`, plus the `CaptureState`
   lifecycle atomic. (`handle` is the cloneable `PipelineHandle`.)
3. `PipelinePorts.worker_rx` — a crossbeam SPSC receiver for
   `KeyMeasurement` results coming back from the Worker thread.
4. `PipelinePorts.profiles` — a `ringbuf` SPSC producer for pushing
   recompiled `KeyProfile` templates back into the live engine (UI → DSP).
5. A `triple_buffer` carrying the live, continuous `FrameOutput` from
   the DSP thread to the GUI for visualization.

Anything that doesn't fit one of these five shapes is a sign that the
boundary is being violated.

## Cold-path modules and the future audio-out crossing

Not every `tuner-core` module is on a real-time thread. `synth` (offline
additive resynthesis of a `TuningCurve` to audio — the auralization tool) is
**cold-path**: it runs on no pipeline thread, holds no shared state, and owns
**no audio stream**. It returns a `Vec<f32>` (or writes a WAV); the caller
sets the level and plays or saves it. It is inert with respect to the
real-time rules above.

Speaker **playback** is a separate, currently-unbuilt concern. When it is
added (the GUI "hear the curve" feature), the CPAL **output** stream will live
in `audio` — the single CPAL boundary — **not** in the GUI, because the core is
headless and the GUI speaks only the five channels above. It is the mirror of
crossing #1: the output callback is the real-time _consumer_ filling a
`&mut [f32]`, fed by a lock-free ring buffer whose _producer_ is the cold
`synth`. That is a sanctioned **sixth crossing**, exposed as an opt-in handle
like `spawn_analysis_thread` and subject to the same wait-free callback
discipline. It is deferred; duplex (playback during capture) is out of scope.

## Pipeline ownership (Split / Handle pattern)

`AudioPipeline` owns the real-time DSP components: `Gatekeeper`,
`Engine`, the COLA `CircularFifo`, the `AudioPool`, and the inline
capture-accumulation state. It is moved to the audio thread and is the
only thing that mutates the pipeline's internal state.

`PipelinePorts` is the GUI's window into the running pipeline. It carries
the cloneable `PipelineHandle` (`Arc<PipelineAtomics>`), the Worker → UI
`worker_rx` receiver, and the UI → DSP `profiles` producer — nothing more.
The single-owner endpoints (`worker_rx`, `profiles`) are kept out of the
cloneable handle; `spawn_analysis_thread` folds the ports into a `HostHandle`.

A frontend contributor calls `AudioPipeline::new()`, gets a
`PipelinePorts`, and never needs to know about Gatekeeper internals,
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
  handling, and thread management live there. It holds **no DSP** —
  signal processing lives entirely in `tuner-core`; `app.rs` only reads
  per-hop telemetry (`FrameOutput`) and worker results and writes config
  atomics. Keep it that way: DSP logic does not belong in `app.rs`.
