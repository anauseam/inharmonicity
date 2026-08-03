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

The GUI interacts with the core through exactly six channels. **"Crossing #N"
always refers to the canonical numbering in
[`02-cross-thread-communication.md`](02-cross-thread-communication.md)** —
this list is *not* numbered the same way (it enumerates the GUI's
interaction points, which include construction and exclude the CPAL→DSP
crossing the GUI never touches):

1. `AudioPipeline::new()` returns `(AudioPipeline, PipelinePorts)` — the
   Split / Handle pattern. The GUI keeps the ports; the audio thread
   takes the pipeline. (Construction, not a runtime crossing.)
2. `PipelinePorts.handle.atomics` — wait-free reads of `RuntimeAtomics`
   and wait-free writes of `ConfigAtomics`, plus the `CaptureState`
   lifecycle atomic. (`handle` is the cloneable `PipelineHandle`.)
   Crossing #3.
3. `PipelinePorts.worker_rx` — a crossbeam SPSC receiver for `WorkerOutput`
   coming back from the Worker: `Measurement` results per capture and
   `Curve` bundles per recompute (one enum stream). The Worker → UI leg
   of crossing #5.
4. `PipelinePorts.worker_job_tx` — a crossbeam SPSC sender for `WorkerJob`
   background requests to the Worker (UI → Worker; today curve recomputes).
   Crossing #6.
5. `PipelinePorts.profiles` — a `ringbuf` SPSC producer for pushing
   recompiled `KeyProfile` templates back into the live engine (UI → DSP).
   Crossing #4.
6. A `triple_buffer` carrying the live, continuous `FrameOutput` from
   the DSP thread to the GUI for visualization. Crossing #2.

Anything that doesn't fit one of these six shapes is a sign that the
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
headless and the GUI speaks only the six channels above. It is the mirror of
crossing #1: the output callback is the real-time *consumer* filling a
`&mut [f32]`, fed by a lock-free ring buffer whose *producer* is the cold
`synth`. That is a sanctioned **seventh crossing**, exposed as an opt-in handle
like `spawn_analysis_thread` and subject to the same wait-free callback
discipline. It is deferred; duplex (playback during capture) is out of scope.

## Pipeline ownership (Split / Handle pattern)

`AudioPipeline` owns the real-time DSP components: `Gatekeeper`,
`Engine`, the `Strobe`, the COLA `CircularFifo`, the `AudioPool`,
and the inline capture-accumulation state. It is moved to the audio
thread and is the only thing that mutates the pipeline's internal state.

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
- `Strobe::process` returns a `StrobeResult`.

The `AudioPipeline` reads these return values and syncs observations
back to the shared atomics. Nothing else does.

This convention means the components are unit-testable in isolation and
the data flow is auditable from a single file (`pipeline.rs`).

### The processing chain is sacred

The hot path has exactly one processing chain, with two outputs (the
authoritative step-by-step, with every step's consumers, is the per-hop
sequence table in [`03-dsp-pipeline.md`](03-dsp-pipeline.md)):

```text
CPAL callback ─► ringbuf ─► COLA/FFT front-end ─► Gatekeeper ─► Engine ─┬─► FrameOutput ─► GUI
  (DC block)                (Gatekeeper + discovery inputs)             │
                                                                        └─► capture limb ─► Worker (MAT) ─► KeyMeasurement
```

`Gatekeeper` decides whether the hop's signal is usable; `Engine` is
the **single center of real-time pitch DSP** — discovery, tracking,
manual-mode targeting; the **Worker** is the single home of
asynchronous high-resolution measurement, kept off-thread precisely so
the Engine stays unpolluted. The FFT front-end and the capture limb
are chain stages too — the Gatekeeper's transient metrics read the
treble spectrum, discovery reads the bass magnitudes, and the capture
limb produces the measurements the product is built on. Inserting a
new **stage** anywhere in this chain — anything a chain component
would depend on, anything that transforms the data flowing between
them, anything whose removal would change gating, detection, or
measurement — is a foundational change to the architecture. It needs
its own design note, review against this file and
[`02`](02-cross-thread-communication.md), and ADR-grade justification.
The default answer is **no**.

Not everything inside `process_cola_hop` is a chain stage, though. The
hop also hosts **taps**: parallel observers that read the hop's audio
or the components' outputs and produce telemetry without sitting in
the chain's data path. Which steps are taps, and what each produces, is
the Class column of [`03`](03-dsp-pipeline.md)'s per-hop table — the
ground truth this file judges against, and the only place the list is
kept. The test for a tap is **deletability**: removing it
must leave gating, detection, and measurement bit-identical, because
nothing in the chain consumes its output — the `Strobe` reads a *target*
the UI nominated, never the engine's tracker, and writes only
`FrameOutput`. (The capture accumulator is *not* a tap by this test —
it is the measurement limb.)

Adding a tap is still an architecture-level change — lighter than a
stage, far heavier than a function:

- it follows the purity conventions above (own file, results by value,
  no knowledge of atomics or the GUI) and is called only from
  `process_cola_hop`;
- it must satisfy the deletability test — a "tap" the chain starts
  depending on has become a stage, and gets the stage-level bar;
- any new UI ↔ DSP data flow maps onto an existing crossing charter in
  `02` (a new payload instance, a wider `FrameOutput`) rather than a
  new channel shape — a genuinely new crossing needs `02` §6's reuse
  test and its own documented charter;
- `00`'s diagram and file map, this file's ownership list, and `02`'s
  affected crossing sections are updated in the same change.

## `process_cola_hop` is the single DSP entry point

Every audio hop runs exactly one function:
`AudioPipeline::process_cola_hop`. Its body, in order:

1. Reads config from shared atomics into the DSP components.
2. Runs the `Gatekeeper` for signal validation; receives a `GateResult`.
3. Runs the `Engine` for F0 detection when the Gatekeeper approves the
   signal.
4. Runs the `Strobe` for fixed-reference beat phase and coarse spectral
   readout; receives a `StrobeResult`.
5. Syncs observations back to the shared atomics and produces a
   `FrameOutput` for the GUI's `triple_buffer`.
6. Manages capture accumulation: `Armed → Recording → dispatch to
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
- **The GUI owns the profile, so the GUI owns where it goes.** The
  *schema* is a domain type (`models::InharmonicityProfile`, so the
  Worker and the offline harnesses share it), but every file-location
  policy — the per-user directories, the app-settings document, the
  listing the browser renders, the one-time import of a pre-move profile
  — lives in `library.rs`. `tuner-core` stays headless and knows nothing
  about `directories` or XDG. Persistence *timing* (autosave on capture
  and undo, the session `.bak`) is likewise `app.rs` policy, not core's.
  → [`session-persistence-and-profile-library.md`](../design/session-persistence-and-profile-library.md)
- **File locations are injected, never assumed.** The frontend hands the
  Worker its dump root (`Option<PathBuf>`; `None` writes none, so an
  embedded host can opt out) rather than `tuner-core` resolving one.
  The directory *name* for a capture is `worker::dump_dir_name`, next to
  the code that writes it and public because the GUI deletes the dump of
  an undone capture — when both sides hardcoded the path independently, a
  change on one silently turned the other into a no-op.
