# DSP Pipeline & Hot Path

The hot path is the set of code that runs on every audio sample or every
DSP hop. We keep it allocation-free and non-blocking to stay within the
realtime budget. The Worker thread (`worker.rs`) and the GUI thread are
explicitly outside the hot path and have no allocation restriction.

## Hot-path inventory

- **Thread 1 — the CPAL input callback** in `audio.rs`. DC blocking plus
  the ringbuf push. The callback is invoked by the OS audio driver at
  the device's buffer rate.
- **Thread 2 — the analysis loop** in `audio.rs::spawn_analysis_thread`:
  drains the ringbuf, accumulates samples into the COLA `CircularFifo`,
  and calls `AudioPipeline::push_audio` →
  `AudioPipeline::process_cola_hop` once per hop.
- Everything reached transitively from `process_cola_hop`: the
  `Gatekeeper`, the `Engine`, and the `algorithms/*` functions invoked
  by them. These are the only DSP entry points; nothing else in the
  audio path should be calling into `algorithms/`.

## `process_cola_hop` is the only DSP entry point

The pipeline's per-hop work happens entirely inside
`AudioPipeline::process_cola_hop`. New DSP behaviour goes inside that
function or in a component it already calls. Calling into `Engine` or
`Gatekeeper` from outside the pipeline (for example, directly from
`audio.rs` or from the GUI) bypasses the shared-state syncing step at
the end of the hop and breaks observability.

## The per-hop sequence (authoritative)

`process_cola_hop`, step by step, with each step's actual consumers.
**Chain** steps carry data a later chain step depends on; the **capture
limb** is the chain's asynchronous measurement branch; **taps** are
parallel observers whose removal leaves gating, detection, and
measurement bit-identical. The change bar for each class is defined in
[`01-architecture.md`](01-architecture.md) — this table is the ground
truth it judges against.

| # | Step | Feeds | Class |
| --- | --- | --- | --- |
| 0 | Drain crossing-#4 rings: profile templates (apply all), strobe references (newest wins) | Engine templates; Strobe | chain input / tap input |
| 1 | COLA `read_window` (8192) → treble FFT (newest 2048) + bass FFT (8192), hop acknowledge | Gatekeeper (treble complex spectrum), Engine + Strobe (audio) | **chain** |
| 1b | History-buffer accumulation (newest hop) | diagnostic pre-roll only | tap |
| 2 | Read config atomics: thresholds, noise floor, `target_note` (crossing #3) | Gatekeeper, Engine, Strobe gate | chain input |
| 3 | **Gatekeeper** → `GateResult` (5-state machine: RMS/EMA on the time signal, NHWRSF + NINOS² on the treble spectrum); observations synced to runtime atomics | pipeline control flow, Engine resets, capture baton | **chain** |
| 4 | Bass magnitude spectrum | **Engine discovery** | **chain** |
| 4 | Treble magnitude spectrum | spectrogram (`FrameOutput`) only | tap |
| 5 | **Engine** → `Option<PitchResult>`: silence/transient resets → discovery (Stage-A discrete scoring over the bass magnitudes, M-of-N acquisition lock, tracker seeding) or tracking (adaptive Goertzel bank, NP gate, EMA) | telemetry, capture latch | **chain** |
| 5b | **Strobe** → `StrobeResult`: fixed-reference beat phase, its sliding-window least-squares rate per reference, the per-reference baseband record and the unison lines it resolves (with the discriminator's verdict), and a bounded CFAR-gated coarse spectral readout at the nominated reference partial (skipped during `Silence`) | `FrameOutput` only | tap |
| 6 | Capture accumulation & dispatch — the `CaptureState` baton: onset pre-roll → `Recording` on Stable → the latched fill target (1.5 s by default), decay, or an operator abort → dispatch gate → `CapturePayload` to the Worker (crossing #5), with backpressure recovery | Worker → MAT → `KeyMeasurement` → profile | **capture limb** (chain branch) |
| 7 | `FrameOutput` assembly: treble magnitudes, gate telemetry, pitch fields when locked, strobe fields (angle, gate, rate, amplitude, unison lines + resolution + verdict) + `coarse_hz` unconditionally → triple buffer (crossing #2) | GUI | out |

Two things this table encodes that a "stream → gate → engine" sketch
hides: the windowing/FFT front-end is itself a chain stage (the
Gatekeeper's transient metrics read the treble spectrum; discovery reads
the bass magnitudes), and the chain has **two outputs** — the continuous
`FrameOutput` telemetry and the asynchronous capture limb through the
Worker, which produces the `KeyMeasurement`s the entire product is built
on. Neither output may be treated as a tap.

## Allocation discipline

On the hot path:

- No `Vec::push` / `Vec::with_capacity` / `Vec::new` / `String::new` /
  `Box::new` / any other heap allocation.
- No `clone()` on heap-owning types (`Vec`, `Box<[T]>`, `String`).
  `Arc::clone` is fine because it is just an atomic increment.
- All scratch buffers used inside the hop are owned by the pipeline
  (`ProcessingFrame`), the components (`Gatekeeper`, `Engine`, `Strobe`),
  or the COLA buffer — allocated once at startup via `Box<[T]>` or a
  fixed-size array. A component's cross-hop history counts: a sliding
  window is a fixed-size ring on the component, never a growing buffer.
- **Transform plans are startup state, not per-hop state.** `rustfft`'s and
  `realfft`'s planners allocate on every `plan_*` call, so a component that
  transforms at a length chosen at runtime (`strobe::unison`, whose record
  grows) plans *every* length it can reach once, at construction, and
  indexes them thereafter. Its execution scratch is sized to the largest
  plan's requirement the same way.
- Algorithms accept `&[T]` / `&mut [T]` slices for input and output;
  they do not allocate their own working space.

The Worker thread is allowed to allocate freely because it runs async
to the audio path. The same goes for the GUI thread.

## Blocking discipline

On the hot path:

- No `std::thread::sleep`, no `std::sync::Mutex::lock` (uncontended or
  not), no `RwLock`, no `Condvar`, no file I/O, no UDP/TCP.
- Channel sends use `.try_send()` and accept that `Err(TrySendError::
Full)` is a valid outcome (typically: drop the frame, increment a
  counter that the GUI can observe).
- Channel receives in the audio path use `.try_recv()` likewise.

## Diagnostics and Telemetry

Extracting state from the hot path without breaking real-time audio constraints requires strict discipline. There are two distinct patterns depending on whether you need lightweight textual traces or heavy mathematical profiling.

### 1. Lightweight Textual Traces (`debug_assertions`)

`println!` / `eprintln!` / `dbg!` on the hot path produce non-deterministic latency. Unconditional prints belong entirely in the Worker thread or GUI thread.

For development-time tracing (e.g., observing lock acquisition), the project gates hot-path prints behind `#[cfg(debug_assertions)]`. These prints survive incremental development work but disappear entirely in `--release` builds, ensuring the shipped binary never incurs I/O blocking. The pattern looks like:

```rust
#[cfg(debug_assertions)]
eprintln!("[ENGINE] *** LOCK ACQUIRED *** -> key_idx={}", winning_key);
```

### 2. Mathematical Profiling (`feature = "telemetry"`)

Because the DSP pipeline must run in `--release` mode to keep up with the audio thread without buffer underruns, `debug_assertions` are physically unusable for acoustic analysis.

Heavy diagnostic data structures (such as arrays of `[f32; 128]` for Goertzel tracking) must instead be gated behind `#[cfg(feature = "telemetry")]`. This ensures:

- The structures are completely compiled out in production, preventing cache-line bloat and preserving the pipeline's blazingly fast memory footprint.
- CLI diagnostic tools (like `diagnose_engine`) can compile the optimized `--release` binary *with* the heavy array data included by passing `--features telemetry`.

## Sample-rate handling (transitional)

The `Engine` now carries a `sample_rate` field (set from the resolved
stream rate in `spawn_analysis_thread`) rather than hard-coding it in
`Engine::new`. But the rate is not yet genuinely dynamic: the live
capture path still *requires* 44 100 Hz from CPAL (`open_input_stream`
selects a mono `f32` config whose range covers it, relying on the OS to
resample, and fails cleanly when no such config exists),
`CapturePayload` still passes a literal `44100` to the Worker, and the
`AudioPool` buffer sizes, the COLA window, and the Gatekeeper timing
constants are all dimensioned for 44.1 kHz. So the pipeline is not yet
safe at other rates; enabling true dynamic-rate operation is tracked in
`TODO.md`. Until it lands, new code must not
introduce more hard-coded references to 44 100 — read the rate from the
single source of truth (the `SAMPLE_RATE` constant or the `Engine`'s
`sample_rate` field) so the eventual migration stays a single-point
change.
