---
trigger: model_decision
description: When developing and architecting cross-thread communication between the DSP thread and UI/system threads.
---

# Cross-Thread Communication Architecture

When connecting the real-time audio thread (DSP) to the UI or other OS threads, the DSP thread must be strictly wait-free. DO NOT use generic MPMC/MPSC channels (like `std::sync::mpsc`) inside the real-time audio pipeline, as they can fall back to OS locks, Spin-Locks, or heap allocations under contention or capacity growth.

**Capacity Rule:** Every hardware/network ring buffer must be initialised with an explicitly named constant. Magic number literals are forbidden. Each constant must carry a doc-comment explaining how the value was derived (e.g., buffer capacities scaled for worst-case OS scheduler latency jitter).

Always use the following SPSC/Wait-Free architectures:

1. **Raw Audio Stream (CPAL Callback → DSP Thread)**
   - Use: Fixed capacity SPSC Ring Buffer (`ringbuf`)
   - Purpose: Moving blocks of continuous `f32` audio samples with zero allocations and guaranteed execution time.

2. **Continuous Visualizations (DSP -> UI)**
   - Use: Wait-Free Triple Buffer (e.g. `triple_buffer` crate)
   - Purpose: Sending lossy, high-frequency continuous screen data (e.g., Spectrogram slices, Meters).
   - Rule: The UI thread only reads the freshest frame. DO NOT use queues to process intermediate drawing frames.

3. **Structural DSP Output (DSP -> UI)**
   - Use: Wait-Free Triple Buffer (e.g. `triple_buffer` crate)
   - Purpose: Moving F0 pitch tracks, partials, metrics, and state out of the DSP. The old pattern of sending discrete events via `rtrb` or channels is **removed** in favour of continuous lossy outputs to be parsed completely by the UI tick. Avoid keeping state buffers between DSP and UI threads unless unavoidable.

4. **Single/Isolated DSP Parameters (UI -> DSP)**
   - Use: Hardware Atomics (`std::sync::atomic::AtomicUsize`, `AtomicU32`, etc.)
   - Purpose: Adjusting individual settings (thresholds, multipliers). Use `f32::to_bits()` / `f32::from_bits()` for floats.

5. **Grouped/Dependent DSP Parameters (UI → DSP)**
   - Use: Fixed capacity SPSC Ring Buffer sending Command Enums (`ringbuf`).
   - Purpose: Changing complex states that must change atomically on the DSP frame boundary.
   - **Heap-Allocation Invariant:** All DSP-side data must be one-time allocated at startup via `Box<[T]>` or equivalent and must live until program shutdown. Command enum variants flowing from the UI to the DSP thread MUST NOT carry heap-allocated fields (`Vec<T>`, `Box<[T]>`, `String`, etc.). Rust's ownership model guarantees correctness, but `Drop` on a heap object invokes the global OS allocator's `free()`, which uses OS-level mutexes and produces non-deterministic latency spikes (xruns) even without a data race.
   - **Trash Queue:** Only required if the heap-allocation invariant above is ever violated (e.g., a hot filter-coefficient swap). In that case, push the *old* object out through a dedicated SPSC ring buffer (DSP → UI) to be dropped by the UI thread on its next `Tick`. This scenario should be treated as a design smell — reconsider before implementing.

6. **Offline Background Worker Dispatches (DSP → Worker → UI)**
   - Use: `crossbeam::channel::bounded` (with `.try_send()` on the DSP side).
   - Purpose: Offloading heavy, non-realtime computations (high-res FFT, Template Matching, MAT partial extraction, β calculation, diagnostics I/O) to a single dedicated background worker thread.
   - Mechanism: 
     - DSP → Worker: The pipeline transfers a `CapturePayload` (containing a pre-allocated `Box<[f32; 66150]>` from the `AudioPool` + metadata like `target_note` and `sample_rate`) to the worker via `.try_send()` (wait-free on a bounded crossbeam channel).
     - Worker → UI: The worker sends the resulting `KeyMeasurement` (which may contain heap-allocated fields like `Vec<Partial>` and `String`) back to the UI via `.send()`. The UI drains it via `.try_recv()` in its tick loop.
     - Worker → AudioPool: After processing, the worker recycles the buffer back to the `AudioPool` and resets `CaptureState` to `Idle` via the shared `AtomicU8`.
   - **CaptureState Baton-Pass:** The `CaptureState` `AtomicU8` uses a strict baton-pass pattern. Three threads each own distinct transitions:
     - GUI: `Idle → Armed` (arm), `Armed → Idle` (cancel)  
     - DSP Pipeline: `Armed → Recording` (stability detected), `Recording → Processing` (buffer full or silence decay)
     - Worker: `Processing → Idle` (computation complete)
   - **Heap-Allocation Exception:** Because the Worker → GUI path involves only non-realtime threads, data structures crossing this boundary (like `KeyMeasurement`) *are allowed* to carry heap-allocated fields (`Vec<T>`, `String`, etc.). The strict zero-allocation invariant only applies to paths interacting with the real-time DSP thread.