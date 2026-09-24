# DSP Pipeline & Hot Path

The hot path is the code that runs on every audio sample or every DSP hop. It
is allocation-free and non-blocking to stay within the real-time budget; the
Worker thread (`worker.rs`) and the GUI thread are outside it and may allocate
freely.

## Hot-path inventory

- **Thread 1 — the CPAL input callback** in `audio.rs`: DC blocking plus the
  ringbuf push, invoked by the OS audio driver at the device's buffer rate.
- **Thread 2 — the analysis loop** in `audio.rs::spawn_analysis_thread`: drains
  the ringbuf, accumulates samples into the COLA `CircularFifo`, and calls
  `AudioPipeline::push_audio` → `AudioPipeline::process_cola_hop` once per hop.
- Everything reached transitively from `process_cola_hop`: the `Gatekeeper`,
  the `Engine`, the `Strobe`, and the `algorithms/*` functions they call. These
  are the only DSP entry points; nothing else in the audio path calls into
  `algorithms/`.

## One entry point, pure components

The pipeline's per-hop work happens entirely inside
`AudioPipeline::process_cola_hop`; new DSP behaviour goes inside that function
or in a component it already calls. Calling into `Engine` or `Gatekeeper` from
outside the pipeline (from `audio.rs`, from the GUI) bypasses the shared-state
syncing step at the end of the hop and breaks observability.

The components know nothing of `PipelineAtomics`, shared state, or the GUI.
They are stateful (each has internal buffers and a state machine) but return
their results by value — `Gatekeeper::process_frame` a `GateResult`,
`Engine::process` an `Option<PitchResult>`, `Strobe::process` a
`StrobeResult` — and only the `AudioPipeline` reads those and syncs
observations back to the shared atomics. That keeps each component
unit-testable in isolation and the data flow auditable from one file
(`pipeline.rs`).

## The per-hop sequence (authoritative)

`process_cola_hop`, step by step, with each step's actual consumers. **Chain**
steps carry data a later chain step depends on; the **capture limb** is the
chain's asynchronous measurement branch; **taps** are parallel observers whose
removal leaves gating, detection, and measurement bit-identical. The change bar
for each class is *The processing chain is sacred*, below; this table is the
ground truth it judges against.

| # | Step | Feeds | Class |
| --- | --- | --- | --- |
| 0 | Drain crossing-#4 rings: profile templates (apply all), strobe references and capture commands (newest wins; the command is held for step 6) | Engine templates; Strobe; capture lifecycle | chain input / tap input |
| 1 | COLA `read_window` (8192) → treble FFT (newest 2048) + bass FFT (8192), hop acknowledge | Gatekeeper (treble complex spectrum), Engine + Strobe (audio) | **chain** |
| 1b | History-buffer accumulation (newest hop) | diagnostic pre-roll only | tap |
| 2 | Read config atomics: thresholds, noise floor, `target_note` (crossing #3) | Gatekeeper, Engine, Strobe gate | chain input |
| 3 | **Gatekeeper** → `GateResult` (Silence / Unstable / Stable, plus a per-hop onset flag: RMS/EMA on the time signal, NHWRSF + the inverse participation ratio on the treble spectrum); observations synced to runtime atomics | pipeline control flow, Engine resets, capture lifecycle | **chain** |
| 4 | Bass magnitude spectrum | **Engine discovery** | **chain** |
| 4 | Treble magnitude spectrum | spectrum display (`FrameOutput`) only | tap |
| 5 | **Engine** → `Option<PitchResult>`: silence/transient resets → discovery (Stage-A discrete scoring over the bass magnitudes, M-of-N acquisition lock, tracker seeding) or tracking (adaptive Goertzel bank, NP gate, EMA) | telemetry, capture latch | **chain** |
| 5b | **Strobe** → `StrobeResult`: fixed-reference beat phase, its sliding-window least-squares rate per reference, the per-reference baseband record and the unison lines it resolves (with the discriminator's verdict), and a bounded CFAR-gated coarse spectral readout at the nominated reference partial (skipped during `Silence`) | `FrameOutput` only | tap |
| 6 | Capture accumulation & dispatch — the pipeline's own `CaptureState`: the Worker's completion flag → the hop's capture command (`Arm`/`Cancel`) → onset pre-roll → `Recording` on Stable → the latched fill target (1.5 s by default), decay, or a `Cancel` → dispatch gate → `CapturePayload` to the Worker (crossing #5), with backpressure recovery (`thread-crossings.md`) | Worker → MAT → `KeyMeasurement` → profile | **capture limb** (chain branch) |
| 7 | `FrameOutput` assembly: treble magnitudes, gate telemetry, pitch fields when locked, strobe fields (angle, gate, rate, amplitude, unison lines + resolution + verdict) + `coarse_hz` and the `CaptureState` unconditionally → triple buffer (crossing #2) | GUI | out |

Two things this table encodes that a "stream → gate → engine" sketch hides: the
windowing/FFT front-end is itself a chain stage (the Gatekeeper's transient
metrics read the treble spectrum; discovery reads the bass magnitudes), and the
chain has **two outputs** — the continuous `FrameOutput` telemetry and the
asynchronous capture limb through the Worker, which produces the
`KeyMeasurement`s the entire product is built on. Neither may be treated as a
tap.

## Allocation discipline

On the hot path:

- No `Vec::push` / `Vec::with_capacity` / `Vec::new` / `String::new` /
  `Box::new` / any other heap allocation, and no `clone()` on a heap-owning
  type (`Vec`, `Box<[T]>`, `String`); `Arc::clone` is fine, being an atomic
  increment.
- Every scratch buffer used inside the hop is owned by the pipeline
  (`ProcessingFrame`), a component (`Gatekeeper`, `Engine`, `Strobe`) or the
  COLA buffer, allocated once at startup as `Box<[T]>` or a fixed-size array.
  A component's cross-hop history counts: a sliding window is a fixed-size
  ring on the component, never a growing buffer.
- **Transform plans are startup state, not per-hop state.** `rustfft`'s and
  `realfft`'s planners allocate on every `plan_*` call, so a component that
  transforms at a length chosen at runtime (`strobe::unison`, whose record
  grows) plans *every* length it can reach once, at construction, and indexes
  them thereafter; its execution scratch is sized to the largest plan the same
  way.
- Algorithms accept `&[T]` / `&mut [T]` slices for input and output and do not
  allocate their own working space.

## Blocking discipline

On the hot path:

- No `std::thread::sleep`, no `std::sync::Mutex::lock` (uncontended or not),
  no `RwLock`, no `Condvar`, no file I/O, no UDP/TCP.
- Channel sends use `.try_send()` and accept `Err(TrySendError::Full)` as a
  valid outcome (typically: drop the frame, increment a counter the GUI can
  observe); receives in the audio path use `.try_recv()` likewise.
- No unconditional prints: `println!` / `eprintln!` / `dbg!` block on console
  I/O and belong on the Worker or GUI thread. A development trace on the hot
  path is gated by `debug_assertions`, heavy per-frame telemetry by the
  `telemetry` feature — *Feature Flags vs Debug Assertions* in
  [`style.md`](style.md).

## Sample-rate handling (transitional)

`Engine::new` takes a `sample_rate`, but nothing reads the resolved stream
rate yet: the pipeline constructs the Engine with the `SAMPLE_RATE` constant,
`CapturePayload` carries the same constant to the Worker, `open_input_stream`
asks CPAL for 44 100 Hz (a mono `f32` config whose range covers it, relying on
the OS to resample, and failing cleanly when no such config exists), and the
`AudioPool` buffer sizes, the COLA window and the Gatekeeper timing constants
are all dimensioned for it. So the pipeline is not safe at other rates; true
dynamic-rate operation is tracked in `TODO.md`. Until it lands, new code must
not add hard-coded references to 44 100 — read the `SAMPLE_RATE` constant — so
the eventual migration stays a single-point change.

## The processing chain is sacred

The hot path has exactly one processing chain, with two outputs:

```text
CPAL callback ─► ringbuf ─► COLA/FFT front-end ─► Gatekeeper ─► Engine ─┬─► FrameOutput ─► GUI
  (DC block)                (Gatekeeper + discovery inputs)             │
                                                                        └─► capture limb ─► Worker (MAT) ─► KeyMeasurement
```

`Gatekeeper` decides whether the hop's signal is usable; `Engine` is the
**single center of real-time pitch DSP** — discovery, tracking, manual-mode
targeting; the **Worker** is the single home of asynchronous high-resolution
measurement, kept off-thread precisely so the Engine stays unpolluted; the
front-end and the capture limb are chain stages too, as the per-hop table
records. Inserting a new **stage** anywhere in this chain — anything a chain
component would depend on, anything that transforms the data flowing between
them, anything whose removal would change gating, detection, or measurement —
is a foundational change to the architecture. It needs its own design note,
review against this file and [`thread-crossings.md`](thread-crossings.md), and
a *Decisions* entry in `ARCHITECTURE.md`. The default answer is **no**.

The hop also hosts **taps**: parallel observers that read the hop's audio or
the components' outputs and produce telemetry without sitting in the chain's
data path; the per-hop table's Class column is the only list of them. The test
for a tap is **deletability**: removing it must leave gating, detection, and
measurement bit-identical, because nothing in the chain consumes its output —
the `Strobe` reads a *target* the UI nominated, never the engine's tracker,
and writes only `FrameOutput`. (The capture accumulator is not a tap by this
test; it is the measurement limb.) Adding a tap is still an
architecture-level change — lighter than a stage, far heavier than a function:

- it follows the purity conventions above (own file, results by value, no
  knowledge of atomics or the GUI) and is called only from `process_cola_hop`;
- it must satisfy the deletability test — a "tap" the chain starts depending on
  has become a stage, and gets the stage-level bar;
- any new UI ↔ DSP data flow maps onto an existing crossing charter in
  `thread-crossings.md` (a new payload instance, a wider `FrameOutput`) rather
  than a new channel shape — a genuinely new crossing needs
  `thread-crossings.md` §6's reuse test and its own documented charter;
- `ARCHITECTURE.md`'s diagram and file map, and `thread-crossings.md`'s
  affected crossing sections, are updated in the same change.
