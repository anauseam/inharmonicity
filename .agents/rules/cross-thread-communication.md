---
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
   - Purpose: Moving F0 pitch tracks, partials, metrics, and state out of the DSP. The old pattern of sending discrete events via `rtrb` or channels is **deprecated** in favour of continuous lossy outputs to be parsed completely by the UI tick. Avoid keeping state buffers between DSP and UI threads unless unavoidable.

4. **Non-RT Background Calibration (Background Worker -> UI)**
   - **Telemetry Exception Rule**: While generic unbounded channels (`std::sync::mpsc`) are strictly forbidden inside the Real-Time audio pipeline (to prevent CPU overhead and xruns), they are the **approved standard** for offline background UI telemetry (like calibration wizard sensing).
   - Use: Standard MPSC channel (`std::sync::mpsc::channel`)
   - Purpose: Calibration sequences (e.g. noise floor reading, strike capture) happen offline, wait for user input, and silence the main pipeline. They do not share the strict RT boundary of the CPAL-driven audio stream. Thus, an allocation-free (after setup), lossless elastic buffer like an standard channel is technically superior and safer, providing reliability without risking hardware dropouts.

5. **Single/Isolated DSP Parameters (UI -> DSP)**
   - Use: Hardware Atomics (`std::sync::atomic::AtomicUsize`, `AtomicU32`, etc.)
   - Purpose: Adjusting individual settings (thresholds, multipliers). Use `f32::to_bits()` / `f32::from_bits()` for floats.

6. **Grouped/Dependent DSP Parameters (UI → DSP)**
   - Use: Fixed capacity SPSC Ring Buffer sending Command Enums (`ringbuf`).
   - Purpose: Changing complex states that must change atomically on the DSP frame boundary.
   - **Heap-Allocation Invariant:** All DSP-side data must be one-time allocated at startup via `Box<[T]>` or equivalent and must live until program shutdown. Command enum variants flowing from the UI to the DSP thread MUST NOT carry heap-allocated fields (`Vec<T>`, `Box<[T]>`, `String`, etc.). Rust's ownership model guarantees correctness, but `Drop` on a heap object invokes the global OS allocator's `free()`, which uses OS-level mutexes and produces non-deterministic latency spikes (xruns) even without a data race.
   - **Trash Queue:** Only required if the heap-allocation invariant above is ever violated (e.g., a hot filter-coefficient swap). In that case, push the *old* object out through a dedicated SPSC ring buffer (DSP → UI) to be dropped by the UI thread on its next `Tick`. This scenario should be treated as a design smell — reconsider before implementing.
