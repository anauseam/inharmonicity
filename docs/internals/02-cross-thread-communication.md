# Cross-Thread Communication

The DSP and audio threads stay strictly wait-free. Generic MPMC/MPSC
channels (such as `std::sync::mpsc`) are avoided inside the real-time
pipeline — they can fall back to OS locks, spin-locks, or heap
allocations under contention or capacity growth.

Every ring buffer is initialised with an explicitly named constant.
Magic-number literals are not used. Each constant carries a doc-comment
explaining how the value was derived (for example: buffer capacity
scaled to absorb worst-case OS scheduler latency jitter).

The six sanctioned crossings:

## 1. Raw audio stream (CPAL callback → DSP thread)

- **Primitive:** fixed-capacity SPSC ring buffer (`ringbuf`).
- **Purpose:** move blocks of continuous `f32` audio samples with zero
  allocation and a bounded execution time.

## 2. Continuous DSP output (DSP → UI)

- **Primitive:** wait-free triple buffer (`triple_buffer` crate),
  carrying a `FrameOutput` struct.
- **Purpose:** ship both continuous visualization data (spectrogram
  magnitudes, level meters) and structural per-hop telemetry (F0,
  partial frequencies, cents deviation, strobe-bank angles) in a single
  packed struct. Visualizations and structural
  state ride together because they are sampled from the same DSP hop
  and the GUI parses the entire frame on each tick.
- **The buffer is lossy, which sorts the payload into two kinds.** Most
  fields are per-hop snapshots — the magnitude spectrum, gate telemetry,
  the pitch and coarse-readout fields — where a dropped frame costs one
  update and nothing else. The rest are quantities a dropped frame would
  *destroy*, so the DSP thread owns them across hops and ships the result:
  the strobe's accumulated beat phase (an integrated count, not an
  increment — strobe design R2), its least-squares rate, fit over a
  window indexed by hop rather than by the GUI's irregular tick, and the
  unison lines, resolved from a per-reference baseband record the DSP side
  accumulates and the GUI never sees. **Anything cumulative, or fitted
  across hops, belongs on the DSP side of this buffer** — the consumer
  selects and formats, it does not integrate. Note the *lines* are a
  snapshot of that record, so a dropped frame costs one update; it is the
  record itself that could not survive the crossing.
- **Frequencies ship as absolute Hz, never cents** (`coarse_hz`,
  `strobe_beat_hz`, `unison_lines[..].offset_hz`,
  `unison_resolution_hz`): the reference a number is displayed against is
  the frontend's policy, and the DSP does not hold it (ADR 0011, ADR 0012).
- **A detector's verdict crosses as a verdict, not as the numbers behind
  it** (`unison_verdict`). Deciding whether a pair of lines is a unison or
  one partial splitting against itself is a test over signal estimates, so
  it runs DSP-side with the estimator it tests; the GUI renders the
  outcome and does not re-derive it (ADR 0012 §6).
- Per-field semantics — what each `Option` means, which entries of an
  array are valid — live in `FrameOutput`'s own doc comments, not here.
  This section is the crossing's contract; the struct is its schema.
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

- **Primitive:** fixed-capacity SPSC ring buffer (`ringbuf`) carrying a
  heap-free payload.
- **Purpose:** change complex states that must update atomically on a
  DSP frame boundary. The UI hands a recompiled DSP template back to the
  DSP thread for live use.

Two concrete instances share this charter (each on its own SPSC ring):
the live inharmonicity-template updates and the strobe reference updates.

### Live inharmonicity-template updates

The UI hands a recompiled per-key inharmonicity template
to the live engine.

- **Payload:** `pipeline::KeyProfileUpdate { key_index, profile }` — one key's
  recompiled discovery template. Heap-free (`KeyProfile` is
  `{f32, f32, [f32; MAX_PARTIALS], usize}`), so swapping it drops no heap data on
  the audio thread.
- **Producer:** `pipeline::ProfileSender`, a single-owner `ringbuf` producer held
  in `HostHandle` (not the cloneable `PipelineHandle`).
- **Consumer:** `AudioPipeline` drains the queue at the top of `process_cola_hop`
  via `try_pop`, swapping each update into its live `[KeyProfile; 88]` array
  (allocated once at startup). The engine only references that array.
- **Template location:** `KeyProfile` and its constructors
  (`KeyProfile::from_measurement`, `build_default_profiles`) live in `models`;
  `pipeline` holds only the transport.

### Strobe reference updates

The second instance: the UI pushes the tuned key's per-partial strobe
reference frequencies to the DSP-side `Strobe` (strobe design §5.2,
Path A), which computes both the beat phase and the coarse readout at step 5b.

- **Payload:** `strobe::StrobeRefUpdate` — one key's per-partial reference
  frequencies, plus which of them the coarse read centres on and the key's
  partial spacing; `count: 0` clears the bank. Heap-free and `Copy`. Field
  semantics are on the struct. One message carries both readouts' targets
  because they are one component, so they cannot disagree about which key
  they are looking at.
- **Policy direction:** every value in the payload is the **frontend's**
  choice. The DSP searches where it is told and never nominates a target
  of its own.
- **Producer:** `pipeline::StrobeSender`, a single-owner `ringbuf` producer
  held in `HostHandle`. Pushed only on key change / re-lock / engine switch
  (user-rate); a full ring returns `false` and the GUI retries next tick.
  Capacity `STROBE_REF_QUEUE_CAPACITY = 2`.
- **Consumer:** `AudioPipeline` drains to the *newest* update at the top of
  `process_cola_hop` (a superseded reference set is worthless) and hands it to
  `Strobe::retarget`, which resets the bank's accumulated angles and rate fits.

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
the *old* object out through a dedicated SPSC ring buffer (DSP → UI)
so the UI thread drops it on its next tick. Treat this scenario as a
design smell — reconsider the design before implementing it.

## 5. Async background-worker dispatch (DSP → Worker → UI)

- **Primitive:** `crossbeam::channel::bounded`, with `.try_send()` on
  the DSP side.
- **Purpose:** offload heavy, non-realtime **capture processing**
  (high-resolution FFT, CSPE peak refinement, MAT partial extraction, β
  calculation, diagnostics I/O) to a single dedicated background worker
  thread. The same thread also serves crossing #6's curve recomputes.

### Mechanism

- **DSP → Worker** — `CapturePayload` (a `stable_buffer` + optional
  `full_event_buffer` from the `AudioPool` plus metadata such as
  `target_note` and `sample_rate`), `.try_send()` from the DSP thread.
  Capacity `CAPTURE_QUEUE_CAPACITY = 2` (subordinate to the `AudioPool`'s
  capacity of 8, the true backpressure ceiling).
- **Worker → UI** — `WorkerOutput`, an enum of `Measurement(KeyMeasurement)`
  (one per capture) and `Curve(Box<CurveBundle>)` (one per crossing-#6
  recompute; boxed so the common measurement case stays small). One
  output stream, a sum type of everything the worker produces — the
  actor pattern, so a new result kind is a variant, not a new channel.
  `.try_send()`, drained with `.try_recv()` in the UI tick loop. Capacity
  `WORKER_RESULT_QUEUE_CAPACITY = 4`.
- **Worker → AudioPool.** After processing a capture, the worker recycles
  the buffer back to the `AudioPool` and resets `CaptureState` to `Idle`
  via the shared `AtomicU8`. Curve jobs touch neither the pool nor the
  baton.

The curve *result* rides this crossing's `WorkerOutput` rather than a
channel of its own because the reuse test (see crossing #6) is not met —
same producer (Worker), same consumer (UI), no priority split.

## 6. Background-job dispatch (UI → Worker)

- **Primitive:** `crossbeam::channel::bounded`, with `.try_send()` on
  the UI side.
- **Purpose:** let the UI hand the worker thread heavy, non-realtime
  **background jobs** — today a tuning-curve recompute (the
  Giordano-calibrated engine (c) alone is ~1.3 s, far too slow for the GUI
  thread; this is the whole reason the curve compute is offloaded rather
  than run inline on load).

- **UI → Worker** — `WorkerJob`, an enum of the jobs the UI can request
  (today only `Curve(CurveJob)`, a trust-filtered `CurveInput` snapshot +
  a generation counter). `.try_send()` from the UI via
  `HostHandle::send_curve_job`. Capacity `WORKER_JOB_QUEUE_CAPACITY = 1`
  (latest-wins). Results return on crossing #5's `WorkerOutput`.

### Why a distinct crossing, not a reuse of #5

Reuse a channel via a message enum by default; split into a new one only
when **(a)** it would cross the real-time boundary and widen the
real-time payload, or **(b)** it needs priority or ordering separation
from existing traffic. The job channel meets **both**: its producer is
the UI thread (not the DSP thread that feeds captures over #5), and
captures must be serviced ahead of curve jobs. That is why it is its own
crossing. (The curve *result*, by contrast, meets neither test and so
merges into #5's `WorkerOutput`.)

### Worker loop: captures first, latest-wins jobs

The worker drains every pending `CapturePayload` (crossing #5) before
looking at a `WorkerJob` — measurement latency is user-facing mid-session,
a curve recompute is not — then blocks on a `select!` over both. A capture
arriving while a bundle computes simply waits out the ~1.3 s: the
`Processing` baton state lasts that much longer, once.

Curve jobs are **latest-wins**: the UI stamps a monotonic `generation` on
every job and the returned `CurveBundle` echoes it, so a bundle superseded
by a newer edit is dropped on arrival. The single-slot channel plus a
`curve_dirty` retry flag on the UI (re-send next tick if the slot was
full) plus worker-side coalescing (drain to the newest queued job) means
a burst of edits collapses to one recompute of the final state. Because
the job carries a read-only `CurveInput` snapshot, a curve recompute can
never overwrite or race a `KeyMeasurement` — the two ride separate
channels with separate types.

### CaptureState baton-pass

`CaptureState` is an `AtomicU8` whose transitions are partitioned among
three threads by convention. Each thread writes only its own
transitions:

- **GUI:** `Idle → Armed` (arm), `Armed → Idle` (cancel).
- **DSP pipeline:** `Armed → Recording` (stability detected),
  `Recording → Processing` (buffer full or silence decay),
  `Recording → Armed` (worker queue backpressure failure recovery).
- **Worker:** `Processing → Idle` (computation complete).

This is a convention-only contract (plain `.store(...,
Ordering::Relaxed)` on each side). Hardening it to `compare_exchange`
is tracked in the README's Work-in-Progress section.

### Heap-allocation exception for the Worker ↔ UI paths

Because both the Worker → UI path (`WorkerOutput` — `KeyMeasurement`,
`CurveBundle`) and the UI → Worker job path (`WorkerJob` — a `CurveInput`
snapshot) involve only non-realtime threads, the messages may carry
heap-allocated fields (`Vec<T>`, `String`, boxed curves, etc.). The
strict zero-allocation invariant applies only to paths that interact
with the real-time DSP thread — the capture dispatch (DSP → Worker) and
the profile updates (crossing #4).
