# Cross-Thread Communication

The DSP and audio threads stay strictly wait-free. Generic MPMC/MPSC
channels (such as `std::sync::mpsc`) are avoided inside the real-time
pipeline — they can fall back to OS locks, spin-locks, or heap
allocations under contention or capacity growth.

Every ring buffer and pool is initialised with an explicitly named constant.
Magic-number literals are not used. Each constant carries a doc-comment
explaining how the value was derived (for example: buffer capacity
scaled to absorb worst-case OS scheduler latency jitter, or `AUDIO_POOL_CAPACITY`
from the worst-case number of buffers outstanding).

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
**Where a new atomic goes is decided by *when* it is read**, which is the
property that keeps each struct scannable:

- `ConfigAtomics` — parameters the DSP consults **every hop** (`silence_threshold`,
  `nhwrsf_threshold`, `ninos2_stability_threshold`, `target_note`). Step 2 of
  `process_cola_hop` reads the whole struct; that is what it is for.
- `RuntimeAtomics` — scalar observations several independent consumers may poll.
  A one-way per-hop snapshot with a single consumer is **not** one of these: it
  belongs on `FrameOutput` (crossing #2), which the GUI already parses every
  tick and where a dropped update costs nothing. `capture_progress_samples`
  rides there for exactly that reason.
- **The capture-lifecycle atomics** (`capture_state`, `capture_samples`,
  `capture_strings`, `capture_abort`) — read at lifecycle **transitions**, not
  per hop. They sit loose on `PipelineAtomics`; see crossing #6.

`capture_samples` is the illustration: it sets how long a record fills for, and
above the shipped default it also suppresses the decay stop — so the DSP
genuinely acts on it — but it is read **once, at `Armed → Recording`**, which is
what puts it with the lifecycle atomics rather than in `ConfigAtomics`. The
pipeline clamps what it reads (`HOP_SIZE..=CAPTURE_MAX_SAMPLES`) rather than
trusting the writer.
- **Everything that labels a capture is latched at that same instant** — the
  length, `target_note` and `capture_strings`, gathered in `CaptureLatch`. They
  describe the audio, so they must be sampled when the audio starts, not when it
  is dispatched: a record can run for seconds, and by the end the operator may
  be setting up the next capture. Reading them at dispatch would file a record
  under whatever was selected last.
- `capture_abort` sits with the capture-lifecycle atomics rather than here: it
  is a **request**, not a transition. The GUI raises it and the pipeline
  consumes it and makes the `Recording → Idle` move itself, so the baton keeps
  one writer per transition (see crossing #6). The pipeline clears it when a
  recording starts, so a request that arrived earlier cannot kill the next take.

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
  `target_note`, `sample_rate`, and the capture-provenance fields
  `captured_in_auto` and `sounding_strings`), `.try_send()` from the DSP thread.
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

- **UI → Worker** — `WorkerJob`, an enum of the jobs the UI can request:
  `Curve(CurveJob)` (a trust-filtered `CurveInput` snapshot + a generation
  counter) and `SetDumpDir(Option<PathBuf>)` (where capture dumps are
  written, so they follow the open instrument). `.try_send()` from the UI
  via `HostHandle::send_curve_job` / `send_dump_dir`. Capacity
  `WORKER_JOB_QUEUE_CAPACITY = 1`. Results return on crossing #5's
  `WorkerOutput`; `SetDumpDir` has no result.

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
a burst of edits collapses to one recompute of the final state.

**Coalescing is per kind, and that distinction is load-bearing.** A
superseded curve bundle is worthless, so only the newest survives the
drain; a superseded `SetDumpDir` is not — dropping one files an
instrument's captures under another's name, silently. The worker keeps the
newest of *each* kind and applies the directory first. The UI mirrors that
asymmetry: `pending_dump_dir` retries every tick until accepted, exactly as
`curve_dirty` does, because a full slot must not cost the change.

The ordering works in the frontend's favour: captures are drained before
jobs, so a capture still in flight when the instrument changes is written
under the old root — which is the instrument it was taken on. Because
the job carries a read-only `CurveInput` snapshot, a curve recompute can
never overwrite or race a `KeyMeasurement` — the two ride separate
channels with separate types.

### Capture-lifecycle atomics

Four atomics sit on `PipelineAtomics` outside `ConfigAtomics`, because none of
them is read per hop: the `CaptureState` baton below, `capture_samples` (how
long the record fills for), `capture_abort` — a one-shot request to drop the
recording in progress, which the pipeline consumes and acts on so the baton
keeps one writer per transition — and `capture_strings` —
the operator's declaration of how the tuned key is strung and which of its
strings are sounding (`06-capture-sets.md`). The declaration is written by the
GUI at user rate and read by the pipeline **only** as it assembles a
`CapturePayload`, which is the point: per-capture metadata has to arrive
*with* its capture, and the payload is the only thing ordered against it. A
`WorkerJob` (crossing #6) could not do this — the worker drains captures
before jobs, so a declaration could be applied to a capture that was already
processed.

It stays a single byte so the string count and the sounding set are updated
together, which is the atomicity requirement crossing #4 exists to serve; one
word satisfies it without a ring. It is **not** packed into the baton: the two
have different writers and different lifecycles, and sharing a word would
break the baton's single-writer-per-transition partition below.

#### CaptureState baton-pass

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
