---
description: When developing and architecting cross-thread communication between the DSP thread and UI/system threads.
---
# Cross-Thread Communication Architecture

When connecting the real-time audio thread (DSP) to the UI or other OS threads, the DSP thread must be strictly wait-free. DO NOT use generic MPMC channels (like `crossbeam-channel` or `std::sync::mpsc`) as they can fall back to OS locks, Spin-Locks, or heap allocations under contention or capacity growth.

**Capacity Rule:** Every ring buffer must be initialised with an explicitly named constant (e.g. `const EVENT_RING_CAPACITY: usize = 64;`). Magic number literals are forbidden. Each constant must carry a doc-comment explaining how the value was derived (e.g. `// 2× the maximum DSP hop rate at 60 FPS + headroom`).

Always use the following SPSC/Wait-Free architectures:

1. **Raw Audio Stream (CPAL Callback → DSP Thread)**
   - Use: Fixed capacity SPSC Ring Buffer (`ringbuf` or `rtrb`)
   - Purpose: Moving blocks of continuous `f32` audio samples with zero allocations and guaranteed execution time.

2. **Continuous Visualizations (DSP -> UI)**
   - Use: Wait-Free Triple Buffer (e.g. `triple_buffer` crate)
   - Purpose: Sending lossy, high-frequency continuous screen data (e.g., Spectrogram slices, Meters).
   - Rule: The UI thread only reads the freshest frame. DO NOT use queues to process intermediate drawing frames.

3. **Event & Trigger Data (DSP -> UI)**
   - Use: Fixed capacity SPSC Ring Buffer (`ringbuf` or `rtrb`) storing an `Event` enum/struct.
   - Purpose: Sending lossless, discrete events (e.g., transients, locks).
   - Rule: The UI thread pops until empty every frame. Do NOT use `crossbeam-channel` even if it is bounded.

4. **Single/Isolated DSP Parameters (UI -> DSP)**
   - Use: Hardware Atomics (`std::sync::atomic::AtomicUsize`, `AtomicU32`, etc.)
   - Purpose: Adjusting individual settings (thresholds, multipliers). Use `f32::to_bits()` / `f32::from_bits()` for floats.

5. **Grouped/Dependent DSP Parameters (UI → DSP)**
   - Use: Fixed capacity SPSC Ring Buffer sending Command Enums.
   - Purpose: Changing complex states that must change atomically on the DSP frame boundary.
   - **Heap-Allocation Invariant:** All DSP-side data must be one-time allocated at startup via `Box<[T]>` or equivalent and must live until program shutdown. Command enum variants flowing from the UI to the DSP thread MUST NOT carry heap-allocated fields (`Vec<T>`, `Box<[T]>`, `String`, etc.). Rust's ownership model guarantees correctness, but `Drop` on a heap object invokes the global OS allocator's `free()`, which uses OS-level mutexes and produces non-deterministic latency spikes (xruns) even without a data race.
   - **Trash Queue:** Only required if the heap-allocation invariant above is ever violated (e.g., a hot filter-coefficient swap). In that case, push the *old* object out through a dedicated SPSC ring buffer (DSP → UI) to be dropped by the UI thread on its next `Tick`. This scenario should be treated as a design smell — reconsider before implementing.
