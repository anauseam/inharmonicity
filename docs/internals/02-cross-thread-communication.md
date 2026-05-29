# Cross-Thread Communication

The DSP and audio threads stay strictly wait-free. Generic MPMC/MPSC
channels (such as `std::sync::mpsc`) are avoided inside the real-time
pipeline — they can fall back to OS locks, spin-locks, or heap
allocations under contention or capacity growth.

Every ring buffer is initialised with an explicitly named constant.
Magic-number literals are not used. Each constant carries a doc-comment
explaining how the value was derived (for example: buffer capacity
scaled to absorb worst-case OS scheduler latency jitter).

The five sanctioned crossings:

## 1. Raw audio stream (CPAL callback → DSP thread)

- **Primitive:** fixed-capacity SPSC ring buffer (`ringbuf`).
- **Purpose:** move blocks of continuous `f32` audio samples with zero
  allocation and a bounded execution time.

## 2. Continuous DSP output (DSP → UI)

- **Primitive:** wait-free triple buffer (`triple_buffer` crate),
  carrying a `FrameOutput` struct.
- **Purpose:** ship both continuous visualization data (spectrogram
  magnitudes, level meters) and structural per-hop telemetry (F0,
  partial frequencies, cents deviation) in a single packed struct. Visualizations and structural
  state ride together because they are sampled from the same DSP hop
  and the GUI parses the entire frame on each tick.
- The GUI thread reads the freshest frame on each tick. Because the
  GUI typically runs at ~60 FPS while the DSP hop rate is lower
  (bounded by audio stream latency plus the per-hop DSP cost), the
  GUI will often re-read the same frame multiple times before a new
  one is written. This is by design — staleness is bounded and
  visually imperceptible.

## 3. Single / isolated DSP parameters (UI → DSP)

- **Primitive:** hardware atomics (`std::sync::atomic::AtomicUsize`,
  `AtomicU32`, etc.).
- **Purpose:** adjust individual settings (thresholds, multipliers).
  Use `f32::to_bits()` / `f32::from_bits()` for floats.

## 4. Grouped / dependent DSP parameters (UI → DSP)

- **Primitive:** fixed-capacity SPSC ring buffer carrying command
  enums (`ringbuf`).
- **Purpose:** change complex states that must update atomically on a
  DSP frame boundary. Anticipated use case: the UI hands the compiled
  `InharmonicityProfile` (or another grouped DSP configuration) back
  to the DSP thread for live use.

### Heap-allocation invariant

DSP-side data is one-time allocated at startup via `Box<[T]>` (or
equivalent) and lives until program shutdown. Command-enum variants
flowing from the UI to the DSP thread must not carry heap-allocated
fields (`Vec<T>`, `Box<[T]>`, `String`, etc.).

Rust's ownership model guarantees correctness, but `Drop` on a heap
object invokes the global allocator's `free()`, which uses OS-level
mutexes and can produce non-deterministic latency spikes (xruns) even
without a data race.

### Trash queue (design smell — avoid)

Only required if the heap-allocation invariant above is ever violated
(for example, a hot filter-coefficient swap). The pattern is to push
the _old_ object out through a dedicated SPSC ring buffer (DSP → UI)
so the UI thread drops it on its next tick. Treat this scenario as a
design smell — reconsider the design before implementing it.

## 5. Async background-worker dispatch (DSP → Worker → UI)

- **Primitive:** `crossbeam::channel::bounded`, with `.try_send()` on
  the DSP side.
- **Purpose:** offload heavy, non-realtime computation (high-resolution
  FFT, CSPE-based peak refinement, MAT partial extraction, β
  calculation, diagnostics I/O) to a single dedicated background worker
  thread.

### Mechanism

- **DSP → Worker.** The pipeline transfers a `CapturePayload`
  (containing a pre-allocated `Box<[f32; 66150]>` from the `AudioPool`
  plus metadata such as `target_note` and `sample_rate`) to the worker
  via `.try_send()` — wait-free on a bounded crossbeam channel.
- **Worker → UI.** The worker sends the resulting `KeyMeasurement`
  (which may carry heap-allocated fields like `Vec<Partial>` and
  `String`) back to the UI via `.send()`. The UI drains it with
  `.try_recv()` in its tick loop.
- **Worker → AudioPool.** After processing, the worker recycles the
  buffer back to the `AudioPool` and resets `CaptureState` to `Idle`
  via the shared `AtomicU8`.

### CaptureState baton-pass

`CaptureState` is an `AtomicU8` whose transitions are partitioned among
three threads by convention. Each thread writes only its own
transitions:

- **GUI:** `Idle → Armed` (arm), `Armed → Idle` (cancel).
- **DSP pipeline:** `Armed → Recording` (stability detected),
  `Recording → Processing` (buffer full or silence decay).
- **Worker:** `Processing → Idle` (computation complete).

This is currently a convention-only contract (plain `.store(...,
Ordering::Relaxed)` on each side). Hardening it to `compare_exchange`
is tracked in the README's Work-in-Progress section; when that change
lands, update this section to reflect the new contract.

### Heap-allocation exception for the Worker → UI path

Because the Worker → UI path involves only non-realtime threads, data
structures crossing this boundary (such as `KeyMeasurement`) are
allowed to carry heap-allocated fields (`Vec<T>`, `String`, etc.). The
strict zero-allocation invariant applies only to paths that interact
with the real-time DSP thread.
