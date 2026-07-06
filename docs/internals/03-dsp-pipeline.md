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

## Allocation discipline

On the hot path:

- No `Vec::push` / `Vec::with_capacity` / `Vec::new` / `String::new` /
  `Box::new` / any other heap allocation.
- No `clone()` on heap-owning types (`Vec`, `Box<[T]>`, `String`).
  `Arc::clone` is fine because it is just an atomic increment.
- All scratch buffers used inside the hop are owned by the pipeline
  (`ProcessingFrame`), the components (`Gatekeeper`, `Engine`), or the
  COLA buffer — allocated once at startup via `Box<[T]>`.
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
capture path still requests 44 100 Hz from CPAL (`open_input_stream`
calls `with_sample_rate(SAMPLE_RATE)`, relying on the OS to resample),
`CapturePayload` still passes a literal `44100` to the Worker, and the
`AudioPool` buffer sizes, the COLA window, and the Gatekeeper timing
constants are all dimensioned for 44.1 kHz. So the pipeline is not yet
safe at other rates; enabling true dynamic-rate operation is tracked in
the README's Work-in-Progress section. Until it lands, new code must not
introduce more hard-coded references to 44 100 — read the rate from the
single source of truth (the `SAMPLE_RATE` constant or the `Engine`'s
`sample_rate` field) so the eventual migration stays a single-point
change.
