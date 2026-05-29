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

## Diagnostic printing

`println!` / `eprintln!` / `dbg!` on the hot path can produce
non-deterministic latency, so unconditional prints belong in the Worker
(writing `analysis.json` to disk) or in the GUI thread.

For development-time tracing, the project gates hot-path prints behind
`#[cfg(debug_assertions)]` — the same compile-time switch used in
`engine.rs`. Debug-only prints survive incremental development work,
disappear entirely in release builds, and never appear in shipped
binaries. The pattern looks like:

```rust
#[cfg(debug_assertions)]
println!("[engine] f0={f0:.2} cents={cents:.2}");
```

If a hot-path probe needs to ship in release builds, it should be moved
out of the hop and into the Worker or the GUI tick.

## Sample-rate handling (transitional)

The audio sample rate is currently hard-coded to 44 100 Hz in
`Engine::new`, in `CapturePayload`, and in the constants of `audio.rs`.
Plumbing the CPAL-negotiated rate end-to-end is tracked in the README's
Work-in-Progress section. Until that lands, new code must not introduce
_more_ hard-coded references to 44 100 — read the rate from the
`AudioPipeline` (or the constant being used as the single source of
truth) so the migration to dynamic rates remains a single-point
change.
