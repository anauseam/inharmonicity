# Thread Crossings

The DSP and audio threads stay strictly wait-free. Generic MPMC/MPSC channels
(such as `std::sync::mpsc`) are avoided inside the real-time pipeline: they can
fall back to OS locks, spin-locks, or heap allocations under contention or
capacity growth. Every ring buffer and pool is sized by a named constant whose
doc-comment says how the value was derived (a capacity scaled to worst-case
scheduler jitter, `AUDIO_POOL_CAPACITY` from the most buffers ever outstanding);
magic-number literals are not used.

There are **six** sanctioned crossings. Several carry more than one payload, so
the rows below group by crossing: a second payload of the same class is a new
message type, never a new channel. Each primitive sidesteps a category of
latency or correctness problem (OS mutexes, allocator contention, lossy MPMC
fallback); the crossing-by-crossing contract is the body of this file.

| # | Pathway | Primitive | Direction | Purpose |
| --- | --- | --- | --- | --- |
| **1** | Hardware Capture | `ringbuf` SPSC | Stream (1) → DSP (2) | Lossless elastic buffer for incoming raw audio. |
| **2** | Structural Output | `triple_buffer` | DSP (2) → UI (4) | `FrameOutput` — lossy per-hop viz and telemetry, plus anything the DSP integrates across hops. |
| **3** | DSP Parameters | `Arc<Atomic*>` | UI (4) ↔ DSP (2) | Wait-free configuration and metric reads/writes. |
| **4** | Template Update | `ringbuf` SPSC | UI (4) → DSP (2) | Recompiled `KeyProfile` (measured $B$) into the engine's templates. |
| **4** | Strobe References | `ringbuf` SPSC | UI (4) → DSP (2) | `StrobeRefUpdate` — curve targets + coarse partial — into the `Strobe`. |
| **4** | Capture Commands | `ringbuf` SPSC | UI (4) → DSP (2) | `CaptureCommand`: `Arm(ArmRequest)` — fill target + string declaration — and `Cancel`. |
| **5** | Capture Dispatch | crossbeam SPSC (bounded) | DSP (2) → Worker (3) | `CapturePayload` containing pooled audio buffer + metadata. |
| **5** | Buffer Recycling | Lock-Free Object Pool | DSP (2) ↔ Worker (3) | Recycled `Box<[f32]>` buffers at the `CAPTURE_MAX_SAMPLES` ceiling — zero allocation during capture. |
| **5** | Capture Completion | `AtomicBool` | Worker (3) → DSP (2) | `capture_in_flight`: the pipeline raises it as it dispatches, the Worker clears it once the buffers are home. |
| **5** | Worker Results | crossbeam SPSC (bounded) | Worker (3) → UI (4) | `WorkerOutput`: `Measurement(KeyMeasurement)` per capture, `Curve(CurveBundle)` per recompute. |
| **6** | Worker Jobs | crossbeam SPSC (bounded) | UI (4) → Worker (3) | `WorkerJob`: curve recomputes (`CurveInput` snapshot, latest-wins) and the dump directory. |

## 1. Raw audio stream (CPAL callback → DSP thread)

- **Primitive:** fixed-capacity SPSC ring buffer (`ringbuf`).
- **Purpose:** move blocks of continuous `f32` audio samples with zero
  allocation and a bounded execution time.

## 2. Continuous DSP output (DSP → UI)

- **Primitive:** wait-free triple buffer (`triple_buffer` crate), carrying a
  `FrameOutput` struct.
- **Purpose:** ship continuous visualization data (spectrum magnitudes, level
  meters) and structural per-hop telemetry (F0, partial frequencies, cents
  deviation, strobe-bank angles) in a single packed struct: they are sampled
  from the same hop and the GUI parses the whole frame each tick. The GUI reads
  the freshest frame per tick and, at ~60 FPS against a lower hop rate, often
  re-reads one; staleness is bounded and imperceptible.
- **The buffer is lossy, which sorts the payload into two kinds.** Per-hop
  snapshots — the magnitude spectrum, gate telemetry, the pitch and
  coarse-readout fields, the `CaptureState` — lose one update to a dropped
  frame and nothing else. Quantities a dropped frame would *destroy* are owned
  by the DSP thread across hops, which ships the result: the strobe's
  accumulated beat phase (an integrated count, not an increment), its
  least-squares rate, fit over a window indexed by hop rather than by the GUI's
  irregular tick, and the unison lines, resolved from a per-reference baseband
  record the GUI never sees (the *lines* are a snapshot and survive a drop; the
  record could not). **Anything cumulative, or fitted across hops, belongs on
  the DSP side of this buffer** — the consumer selects and formats, it does not
  integrate.
- **Frequencies ship as absolute Hz, never cents** (`coarse_hz`,
  `strobe_beat_hz`, `unison_lines[..].offset_hz`, `unison_resolution_hz`): the
  reference a number is displayed against is the frontend's policy, and the DSP
  does not hold it (report 0011, report 0012).
- **A detector's verdict crosses as a verdict, not as the numbers behind it**
  (`unison_verdict`). Whether two lines are a unison or one partial splitting
  against itself is a test over signal estimates, so it runs DSP-side with the
  estimator it tests; the GUI renders the outcome and does not re-derive it
  (report 0012 §6).
- Per-field semantics — what each `Option` means, which entries of an array are
  valid — live in `FrameOutput`'s own doc comments. This section is the
  crossing's contract; the struct is its schema.

## 3. Single / isolated DSP parameters (UI → DSP)

- **Primitive:** hardware atomics (`AtomicUsize`, `AtomicU32`, …; floats via
  `f32::to_bits()` / `f32::from_bits()`).
- **Purpose:** adjust individual settings (thresholds, multipliers).

**Where a new atomic goes is decided by *when* it is read**, which is what
keeps each struct scannable:

- `ConfigAtomics` — parameters the DSP consults **every hop** (`silence_threshold`,
  `nhwrsf_threshold`, `sustain_stability_threshold`, `target_note`). Step 2 of
  `process_cola_hop` reads the whole struct; that is what it is for.
- `RuntimeAtomics` — scalar observations several independent consumers may poll.
  A one-way per-hop snapshot with a single consumer is **not** one of these: it
  belongs on `FrameOutput` (crossing #2), which the GUI already parses every
  tick and where a dropped update costs nothing. `capture_progress_samples`
  and the `CaptureState` ride there for exactly that reason.
- **`capture_in_flight`** — the one scalar that travels **Worker → DSP**, a
  third direction beyond `ConfigAtomics` (UI → DSP) and `RuntimeAtomics`
  (DSP → consumers). It sits loose on `PipelineAtomics`; see *The capture
  lifecycle* below.

`target_note` is the boundary case. It stays in `ConfigAtomics` because the
engine consults it every hop for manual-mode targeting; that it is *also* read
at `Armed → Recording` and held in `CaptureLatch` is a second use, not a reason
to move it. What a capture is *of* has to be sampled when the audio starts, not
at dispatch: a record can run for seconds, and by its end the operator may be
setting up the next capture. The rest of what labels a capture — its fill
target and the operator's string declaration — arrives with the `Arm` command
(crossing #4), since they are values the frontend holds and the pipeline needs
only when it acts on a command.

## 4. Grouped / dependent DSP parameters (UI → DSP)

- **Primitive:** fixed-capacity SPSC ring buffer (`ringbuf`) carrying a
  heap-free payload.
- **Purpose:** change complex states that must update atomically on a DSP
  frame boundary.

Three instances share this charter, each on its own ring. The producer is a
single-owner `ringbuf` producer held in `HostHandle` (not the cloneable
`PipelineHandle`), pushed at user rate — a full ring returns `false` and the
GUI retries next tick — and `AudioPipeline` drains each ring at the top of
`process_cola_hop`.

### Live inharmonicity-template updates

- **Payload:** `pipeline::KeyProfileUpdate { key_index, profile }` — one key's
  recompiled discovery template. Heap-free (`KeyProfile` is
  `{f32, f32, [f32; MAX_PARTIALS], usize}`), so swapping it drops no heap data
  on the audio thread. `KeyProfile` and its constructors live in `models`;
  `pipeline` holds only the transport.
- **Producer:** `pipeline::ProfileSender`.
- **Consumer:** every queued update is applied, swapped into the live
  `[KeyProfile; 88]` array (allocated once at startup) that the engine reads.

### Strobe reference updates

- **Payload:** `strobe::StrobeRefUpdate` — one key's per-partial reference
  frequencies, which of them the coarse read centres on, and the key's partial
  spacing; `count: 0` clears the bank. Heap-free and `Copy`; field semantics
  are on the struct. One message carries both readouts' targets because they
  are one component, so they cannot disagree about which key they are looking
  at.
- **Policy direction:** every value is the **frontend's** choice. The DSP
  searches where it is told and never nominates a target of its own.
- **Producer:** `pipeline::StrobeSender`, pushed on key change / re-lock /
  engine switch. Capacity `STROBE_REF_QUEUE_CAPACITY = 2`.
- **Consumer:** drained to the *newest* update (a superseded reference set is
  worthless) and handed to `Strobe::retarget`, which resets the bank's
  accumulated angles and rate fits.

### Capture-lifecycle commands

The UI asks for the lifecycle transitions an operator drives rather than
writing them: arming is a *command*, not a store, and the pipeline makes
`Idle → Armed` on receipt, which is what lets it own the lifecycle outright.

- **Payload:** `pipeline::CaptureCommand` — `Arm(ArmRequest)` or `Cancel`,
  heap-free and `Copy`. `ArmRequest` carries what the record will be: its fill
  target, which the pipeline clamps to `HOP_SIZE..=CAPTURE_MAX_SAMPLES` rather
  than trusting the writer, and the operator's `SoundingStrings` declaration
  (`None` when nothing was declared). One `Cancel` covers disarming from
  `Armed` and dropping the take in progress from `Recording`.
- **Policy direction:** the frontend holds the declaration and the duration and
  states them with every `Arm`; the DSP keeps no standing copy. An `Arm` while
  already `Armed` updates the request and moves nothing, so a declaration made
  *after* an auto-rearm still reaches the capture it describes. Per-capture
  metadata must be ordered against its capture, which is why it rides here and
  not a `WorkerJob` (crossing #6): the worker drains captures before jobs, so a
  declaration sent that way could land on a capture already processed. Here
  the lifecycle orders it — the request in force at `Armed → Recording` is the
  one the record carries.
- **Producer:** `pipeline::CaptureSender`, pushed on operator actions.
  Capacity `CAPTURE_COMMAND_QUEUE_CAPACITY = 2`.
- **Consumer:** drained to the *newest* command and applied at step 6, where
  the lifecycle lives. Newest-wins is correct here, not merely cheap: a command
  supersedes the one before it, so `Arm` then `Cancel` leaves nothing armed and
  the reverse order arms, and a `Cancel` applies in the hop it arrives — it
  cannot linger, as an abort flag could, to kill the next take.

### Heap-allocation invariant

DSP-side data is allocated once at startup (`Box<[T]>` or equivalent) and lives
until shutdown, and no command variant flowing to the DSP thread carries a
heap-allocated field (`Vec<T>`, `Box<[T]>`, `String`, …). Ownership keeps that
correct, but `Drop` on a heap object calls the allocator's `free()`, which takes
OS-level mutexes and can spike latency (xruns) without any data race. The
invariant binds every path that touches the DSP thread — the three rings above
and the capture dispatch (DSP → Worker); the Worker ↔ UI messages
(`WorkerOutput`, `WorkerJob`) join only non-realtime threads and may carry heap.
If a hot swap of a heap object is ever unavoidable, the pattern is a dedicated
DSP → UI ring that hands the *old* object to the UI thread to drop; needing one
is a design smell, so reconsider the design first.

## 5. Async background-worker dispatch (DSP → Worker → UI)

- **Primitive:** `crossbeam::channel::bounded`, with `.try_send()` on the DSP
  side.
- **Purpose:** offload heavy, non-realtime **capture processing**
  (high-resolution FFT, CSPE peak refinement, MAT partial extraction, β
  calculation, diagnostics I/O) to a single background worker thread, spawned
  in `AudioPipeline::new`, which also serves crossing #6's curve recomputes.
  **One thread is sufficient** because captures are serialised at the source:
  a stable note is held for 1.5 s before the next can be captured, and the
  Worker finishes well inside that window; more threads would buy nothing and
  complicate buffer recycling.
- **DSP → Worker** — `CapturePayload`: a `stable_buffer` + optional
  `full_event_buffer` from the `AudioPool`, plus metadata such as `target_note`,
  `sample_rate` and the provenance fields `captured_in_auto` and
  `sounding_strings`. Capacity `CAPTURE_QUEUE_CAPACITY = 2`, subordinate to the
  `AudioPool`'s capacity of 8, the true backpressure ceiling. The Worker
  measures under the identity the payload carries and never re-identifies the
  note.
- **Worker → UI** — `WorkerOutput`, an enum of `Measurement(KeyMeasurement)`
  (one per capture) and `Curve(Box<CurveBundle>)` (one per crossing-#6
  recompute; boxed so the common case stays small). One output stream, a sum
  type of everything the worker produces — the actor pattern, so a new result
  kind is a variant, not a new channel. `.try_send()`, drained with
  `.try_recv()` in the UI tick loop. Capacity `WORKER_RESULT_QUEUE_CAPACITY = 4`.
  The curve *result* rides here rather than on a channel of its own because
  the reuse test (crossing #6) is not met: same producer, same consumer, no
  priority split.
- **Worker → AudioPool.** After a capture the worker recycles the buffers into
  the `AudioPool` and then clears `capture_in_flight`, which is what ends the
  capture lifecycle. Curve jobs touch neither the pool nor the flag.

## 6. Background-job dispatch (UI → Worker)

- **Primitive:** `crossbeam::channel::bounded`, with `.try_send()` on the UI
  side.
- **Purpose:** let the UI hand the worker heavy, non-realtime **background
  jobs** — today a tuning-curve recompute; the Giordano-calibrated engine (c)
  alone takes ~1.3 s, far too slow for the GUI thread, which is why the curve
  is computed off-thread rather than inline on load.
- **UI → Worker** — `WorkerJob`, an enum of the jobs the UI can request:
  `Curve(CurveJob)`, a trust-filtered `CurveInput` snapshot plus a generation
  counter, and `SetDumpDir(Option<PathBuf>)`, where capture dumps are written
  so they follow the open instrument. `.try_send()` via
  `HostHandle::send_curve_job` / `send_dump_dir`. Capacity
  `WORKER_JOB_QUEUE_CAPACITY = 1`. Results return on crossing #5's
  `WorkerOutput`; `SetDumpDir` has none.

**Why a distinct crossing, not a reuse of #5.** Reuse a channel via a message
enum by default; split into a new one only when **(a)** it would cross the
real-time boundary and widen the real-time payload, or **(b)** it needs
priority or ordering separation from existing traffic. The job channel meets
both: its producer is the UI thread, not the DSP thread that feeds captures
over #5, and captures must be serviced ahead of curve jobs.

**Captures first, latest-wins jobs.** The worker drains every pending
`CapturePayload` before looking at a `WorkerJob` — measurement latency is
user-facing mid-session, a curve recompute is not — then blocks on a `select!`
over both. A capture arriving while a bundle computes waits out the ~1.3 s:
`Processing` lasts that much longer, once. The UI sends a curve job on every
trusted-set edit (a capture merged, an undo, a profile load), because the curve
is derived state, recomputed from the profile rather than stored. Jobs are
latest-wins: the UI stamps a monotonic `generation` on each and the returned
`CurveBundle` echoes it, so a bundle superseded by a newer edit is dropped on
arrival; the single-slot channel, a `curve_dirty` retry flag on the UI (re-send
next tick if the slot was full) and worker-side coalescing (drain to the newest
queued job) collapse a burst of edits to one recompute of the final state.

**Coalescing is per kind, and that distinction is load-bearing.** A superseded
curve bundle is worthless, so only the newest survives the drain; a superseded
`SetDumpDir` is not — dropping one files an instrument's captures under
another's name, silently. The worker keeps the newest of *each* kind and
applies the directory first; the UI's `pending_dump_dir` retries every tick
until accepted, exactly as `curve_dirty` does, because a full slot must not
cost the change. The ordering works in the frontend's favour: captures are
drained before jobs, so a capture still in flight when the instrument changes
is written under the old root — the instrument it was taken on. And because
the job carries a read-only `CurveInput` snapshot, a curve recompute can never
overwrite or race a `KeyMeasurement`: the two ride separate channels with
separate types.

## The capture lifecycle

A capture is the one piece of state that spans three crossings, and following
it is the clearest way to see why the six are shaped as they are.
`CaptureState` — Idle → Armed → Recording → Processing → Idle — is **not shared
state**. The pipeline owns it as a plain field and makes every transition:
`Idle → Armed` (on `Arm`); `Armed → Idle` and `Recording → Idle` (on `Cancel`);
`Armed → Recording` (stability detected); `Recording → Processing` (buffer full
or silence decay); `Recording → Armed` (no note identity to dispatch under);
`Processing → Armed` (worker queue backpressure recovery); `Processing → Idle`
once the Worker reports the capture finished.

Because one thread owns the machine, an out-of-sequence transition has no code
path to come from: the states are a plain `enum`, every move is an assignment
through `&mut self`, and the compiler enforces the exclusivity a partitioned
atomic could only police at runtime. **Prefer that over hardening a shared
state machine** — a `compare_exchange` detects a mis-sequenced move, ownership
makes it unrepresentable. Two threads still need to see it, and each gets what
its need actually is: a consumer reads it off `FrameOutput` (crossing #2) and
asks for a transition with a `CaptureCommand` (crossing #4), never writing, so
the display cannot claim a state the DSP disagrees with; the Worker signals one
fact — the dispatched capture is finished — by clearing `capture_in_flight`,
the single `AtomicBool` on `PipelineAtomics`, which is the pipeline's one blind
spot: it knows when it handed a capture over, not when the Worker was done.

The flag needs no `compare_exchange`. The pipeline sets it as it dispatches and
holds `Processing` until it reads the flag clear, so a second capture cannot be
dispatched while one is outstanding and the two writers strictly alternate.
`Relaxed` is sufficient: no data rides the flag, and the payload it refers to
crossed on channels that carry their own ordering.

**Order matters at both ends, and both orders are the same rule — the fact is
published last.** The pipeline raises the flag *before* the `try_send` on
crossing #5, and lowers it again if the send fails; in the other order the
Worker could finish and clear a flag the pipeline had not yet set, stranding
the lifecycle in `Processing`. The Worker recycles the buffers into the
`AudioPool` *before* clearing, so "not in flight" also means the buffers can be
borrowed again, and sends the `Measurement` *after*, so a consumer that arms on
that message finds the lifecycle already finished.

## What a frontend holds

`AudioPipeline::new()` returns `(AudioPipeline, PipelinePorts)` — the Split /
Handle pattern: the audio thread takes the pipeline and the frontend keeps the
ports, which `spawn_analysis_thread` folds into a `HostHandle`. Each port is
the near end of one crossing, numbered as in this file:

- `PipelinePorts.handle.atomics` — the cloneable `PipelineHandle`: wait-free
  reads of `RuntimeAtomics` and writes of `ConfigAtomics`. Crossing #3.
  Nothing capture-related: the lifecycle arrives on `FrameOutput` and leaves
  as a `CaptureCommand`.
- `PipelinePorts.profiles`, `.strobe_refs`, `.capture_commands` — one
  `ringbuf` producer each. Crossing #4, three instances of one shape.
- `PipelinePorts.worker_rx` — the `WorkerOutput` receiver. Crossing #5.
- `PipelinePorts.worker_job_tx` — the `WorkerJob` sender. Crossing #6.
- `HostHandle.frame_rx` — the `triple_buffer` reader carrying `FrameOutput`;
  a host with its own audio thread takes each hop's `FrameOutput` from
  `push_audio`'s return value instead. Crossing #2.

Anything that fits none of these is a sign that the boundary is being violated.

## Cold-path modules and the seventh crossing

`synth` (additive resynthesis of a `TuningCurve` to audio) is **cold-path**: it
runs on no pipeline thread, holds no shared state and owns **no audio stream**.
It returns a `Vec<f32>` (or writes a WAV) and the caller sets the level and
plays or saves it, so it is inert with respect to [`hot-path.md`](hot-path.md).

Speaker playback is not built (`TODO.md`). When it is, the CPAL **output**
stream lives in `audio` — the single CPAL boundary — never in the GUI, because
the core is headless and the GUI speaks only the six channels in this file. It
mirrors crossing #1: the output callback is the real-time *consumer* filling a
`&mut [f32]`, fed by a lock-free ring whose *producer* is the cold `synth`.
That is the sanctioned **seventh crossing**, exposed as an opt-in handle like
`spawn_analysis_thread` and under the same wait-free callback discipline.
Duplex (playback during capture) is out of scope.
