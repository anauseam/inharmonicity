# Inharmonicity - Professional Piano Tuning Application

![Inharmonicity Interface](images/interface-screenshot.png)

An open source professional-grade piano tuning application built in Rust with real-time audio analysis, pitch detection, and cent deviation measurement. Designed for piano tuners with planned support for inharmonicity compensation.

For a detailed overview of the algorithms used, see the [Anauseam documentation](https://docs.anauseam.org/project-docs/inharmonicity-tuner/00_intro).

> [!IMPORTANT]
> **Project Status**
>
> The project is currently under active development. The core DSP pipeline is functional, but some features are still under construction. See the [Project Work in Progress](#project-work-in-progress) section for more information.

## Architecture

### Project Structure

```text
inharmonicity/
├── tuner-core/                     # Headless audio processing & analysis (no GUI code)
│   ├── src/
│   │   ├── algorithms/             # Stateless DSP building blocks
│   │   │   ├── spectral.rs         # FFT, Hann windowing, spectrum magnitude extraction
│   │   │   ├── pitch.rs            # XQIFFT/Quinn estimation, sub-cent refinement, unison coherence
│   │   │   ├── templates.rs        # Structural matched-filters (2-asymptote β model, Gaussian weighting)
│   │   │   ├── phantom.rs          # Predictive Phantom Partial Mask for intermodulation products
│   │   │   ├── mat.rs              # Median-Adjustive Trajectories algebraic combinatorial solver
│   │   │   ├── twm.rs              # Two-Way Mismatch (superseded by Dot-Product Correlation)
│   │   │   ├── metrics.rs          # RMS, EMA, CSD, NHWRSF, NINOS2 signal metrics
│   │   │   ├── tuning.rs           # Cent deviation, inharmonicity-compensated frequencies
│   │   │   └── inharmonicity.rs    # B-coefficient calculation (pending replacement)
│   │   ├── cola.rs                 # CircularFifo — COLA circular FIFO for overlapping frame analysis
│   │   ├── models.rs               # Domain types: Note, Partial, KeyMeasurement, profiles
│   │   ├── pipeline.rs             # AudioPipeline mediator — Dual-FFT unconditional execution
│   │   ├── engine.rs               # F0 Engine — 3-Stage Matched Filter Architecture
│   │   ├── gatekeeper.rs           # 5-state signal validator (DSP only, no shared state)
│   │   ├── worker.rs               # Background worker for heavy offline DSP
│   │   ├── audio.rs                # CPAL audio capture, stream management, DC blocking
│   │   └── lib.rs                  # Crate root
│   └── Cargo.toml
├── tuner-gui/                      # Iced-based GUI frontend
└── Cargo.toml                      # Workspace configuration
```

See [tuner-gui/README.md](tuner-gui/README.md) for more information about the GUI.

### The AudioPipeline (Mediator Pattern)

The `tuner-core` crate is designed to be **frontend-agnostic**. Any GUI (Iced, egui, WASM, etc.) can consume it through the `AudioPipeline` — the single entry point that orchestrates all DSP components and manages cross-thread shared state.

```text
AudioPipeline::new()  →  (AudioPipeline, PipelineHandle)
        │                         │
        ▼                         ▼
    Audio Thread              Frontend Thread
    ┌─────────────────┐       ┌──────────────────────────────┐
    │ Gatekeeper      │       │ PipelineHandle               │
    │   (5-state SM)  │       │   .atomics.config  ← rw      │ (silence threshold, target key)
    │       ↓         │       │   .atomics.runtime ← ro      │ (current RMS EMA, NHWRSF)
    │ Engine (F0 DSP) │       │   .result_rx       ← recv    │ (KeyMeasurement from Worker)
    │       ↓         │       └──────────────────────────────┘
    │ Capture Accum.  │              ↑ polls via Arc<Atomic*>
    │   (AudioPool)   │              │
    │       ↓         │──────────────┘ triple_buffer (FrameOutput)
    └───────┬─────────┘
            │ crossbeam SPSC (CapturePayload)
            ▼
      Worker Thread
      ┌──────────────────┐
      │ Template Matcher  │ ← high-res FFT
      │ MAT (β solver)   │ ← offline partial extraction
      │ Diagnostics I/O  │ ← analysis.json + audio.raw
      └────────┬─────────┘
               │ crossbeam SPSC (KeyMeasurement) → Frontend
               │ returns buffer → AudioPool (recycled)
               ▼
```

The pipleine GUI relationship follows the **Split / Handle pattern** (the same convention used by `crossbeam_channel`, `ringbuf::split()`, `std::thread::spawn`):

- **`AudioPipeline`** is moved to the audio thread. It owns the pure DSP components (`Gatekeeper`, `Engine`) and is the **only** thing that mutates the pipeline's internal state. After calling each DSP component's `process_frame()`, the pipeline reads their returned `GateResult` and syncs observations to the shared atomic state. It also manages inline capture accumulation — when `CaptureState` is `Recording`, it copies each hop's newest samples into a pooled buffer.
- **`PipelineHandle`** is kept by the frontend. It provides:
  - `atomics.config` — wait-free reading and writing of configuration values (e.g., silence threshold, target key)
  - `atomics.runtime` — wait-free polling of runtime observations (e.g., smoothed RMS for the Envelope Viewer)
  - `atomics.capture_state` — baton-pass `AtomicU8` governing the capture lifecycle across three threads
  - `result_rx` — crossbeam SPSC receiver for `KeyMeasurement` results from the Worker thread

A frontend contributor just calls `AudioPipeline::new()`, gets a `PipelineHandle`, and never needs to know about Gatekeeper internals, EMA calculations, or lock management.

The pipeline also manages the **`WorkerManager`** (`worker.rs`), which owns a single dedicated background thread for computationally expensive offline DSP. When the pipeline's capture accumulator fills a 1.5-second buffer (or silence is detected), it dispatches a `CapturePayload` to the worker via a bounded crossbeam channel. The worker runs a high-resolution FFT, identifies the note via the 88-key Template Matcher (Auto mode) or bounded peak search (Manual mode), runs MAT for partial extraction, and computes the inharmonicity coefficient ($B$). The result is sent to the frontend via `result_rx`, and the audio buffer is recycled back into the `AudioPool`. A single thread is sufficient because captures are infrequent (one stable note at a time) and the algorithms are fast enough to complete before the next capture could arrive.

> [!IMPORTANT]
> **Migration Status**
>
> The unified `AudioPipeline` system is partially implemented:
>
> | Component | Status |
> | --- | --- |
> | `pipeline.rs` — AudioPipeline mediator + shared state | 🟡 Testing |
> | `gatekeeper.rs` — 5-state signal validator (pure DSP) | ✅ Implemented |
> | `engine.rs` — 3-Stage Matched Filter Architecture | 🟡 Testing |
> | `worker.rs` — Background worker (single thread) | 🟡 Testing |
> | `app.rs` — Wait-free GUI dispatcher integration | 🟡 Testing |
> | Dot-Product Correlation — Replaced Scout/TWM for deterministic lock | 🟡 Testing |
> | Phantom Partial Masking — Intermodulation filter | 🟡 Testing |
> | MAT / Guided Trajectory — Algebraic $f_0$ combinatorial solver | 🟡 Testing |
> | Unison Coherence — `suspend_beta_update` beating detection | 🟡 Testing |
> | Probabilistic Pitch Tracking — Gatekeeper-governed Kalman | ⬜ Planned |
> **Currently**, the architecture is in a working testing phase. `app.rs` has been successfully migrated to use the overlapping frame pipeline alongside a lock-free, wait-free SPSC telemetry queue. The `AudioPipeline` serves as the sole frontend-facing DSP orchestrator. The pipeline natively computes unconditional Dual-Track FFT mappings, while the Engine strictly delegates deterministic matching using 88-key sparse templates. Heavy algorithms like MAT are routed completely off-thread to the Background Worker, returning high-resolution inharmonicity measurements natively back to the GUI's `InharmonicityProfile` memory map without ever blocking the audio hot-path.

### Global Data Structures & Memory Management

To maintain real-time performance without relying on OS priority elevation, the core system completely avoids dynamic heap allocation during the audio hot-path by using pre-allocated, lock-free structures:

- **The Elastic Ring Buffer:** A lock-free circular buffer connecting Thread 1 and Thread 2. Acts as an elastic shock absorber — if the OS briefly suspends the processing thread, audio samples continue to accumulate safely without drops.
- **Lock-Free Object Pool (`AudioPool`):** Pre-allocated pool of `Box<[f32; 66150]>` arrays (1.5 seconds at 44.1 kHz). Thread 2 borrows an array to record a stable note and passes it to the background worker, which recycles it back to the pool when finished.
- **`ProcessingFrame`:** Thread-local scratch buffers for zero-allocation per-frame DSP. All fields are `Box<[T]>` — allocated once in `AudioPipeline::new()` via `vec![..].into_boxed_slice()`, never resized. Includes dedicated `treble_magnitude_buffer` (1024 bins) and `bass_magnitude_buffer` (4096 bins) for the Dual-Track FFT paths. The Engine reads from these directly — no per-frame heap allocation in the correlation + MAT chain.
- **`CircularFifo` (COLA):** Owned by `AudioPipeline`. A `Box<[f32]>` ring buffer that accumulates samples and triggers a new FFT + pipeline frame on every 50% hop. Invisible to `tuner-gui` — the GUI only calls `pipeline.push_audio(&[f32])`.

### Threading Model

#### Thread 1: The Audio Stream

This thread is the high-speed hardware ingestor and signal conditioner.

- **Action:** Continuously captures raw audio from the microphone at 44,100 Hz. Each sample passes through a `DcBlocker` (single-pole high-pass IIR, α = 0.995, ~3.5 Hz cutoff) to remove hardware-dependent DC offset, then is pushed into the Elastic Ring Buffer. This guarantees every downstream consumer sees a zero-mean signal regardless of microphone, audio interface, or OS driver.
- **Rule:** This thread performs zero allocations and no analysis. The DC blocker is the only computation — one multiply and two additions per sample — and is classified as signal *conditioning*, not signal analysis. Its job is to guarantee pristine, zero-mean data throughput.

#### Thread 2: The Audio Processing Pipeline

This thread constantly consumes data from the Elastic Ring Buffer and executes a deterministic DSP pipeline via `AudioPipeline.process_frame()` to calculate the fundamental frequency ($f_0$).

```text
    Shared ProcessingFrame (Dual FFT Spectra + Sample Buffer)
                  │
                  ▼
    ┌─────────────────────────┐
    │ AudioPipeline (Mediator)│
    └──────────┬──┬───────────┘
               │  │
    (Synchronous Frame Tick)
               │  │
    ┌──────────▼──┴───────────┐  (Logic Relay)  ┌────────────▼────────────┐
    │  Gatekeeper (Stabilizer)│ ──────────────▶│   f0 Engine (Detector)  │
    ├─────────────────────────┤ is_silence /    ├─────────────────────────┤
    │ [0] IDLE (Silence Gate) │ is_new_onset    │ [A] Dual-Track Routing  │
    │ [1] ATTACK (NHWRSF Flux)│ ──────────────▶│     (Energy Density)    │
    │ [2] TRANSIENT (Wait)    │                 │ [B] Phantom Partial Mask│
    │ [3] STABILITY (NINOS2)  │                 │     (Bass Isolation)    │
    │ [4] RELEASE             │                 │ [C] Guided Trajectory   │
    └────────────┬────────────┘                 │     (MAT Solver)        │
                 │                              └────────────┬────────────┘
                 │                                           │
                 ▼                                           ▼
          RuntimeAtomics                                FrameOutput
                                                     (→ triple_buffer)
    ┌──────────────────────────────┐
    │ Capture Accumulation         │
    │  CaptureState: Armed →       │
    │    Recording → Processing    │
    │  AudioPool buffer fill       │
    └─────────────┬────────────────┘
                  │ crossbeam SPSC (CapturePayload)
                  ▼
    ┌──────────────────────────────┐
    │ Background Worker (Thread 3) │
    │  High-Res FFT → Template     │
    │  Matcher → MAT → β calc      │
    └─────────────┬────────────────┘
                  │ crossbeam SPSC (KeyMeasurement)
                  ▼
              GUI (Thread 4)
```

- **The Gatekeeper (Signal Validator & 5-State Logic):** An always-running traffic cop monitoring the signal envelope. It evaluates stability via a `GateResult` return value (replacing the old direct-field-read pattern), executing a 5-stage state machine. The pipeline reads the Gatekeeper's `SignalState` and uses it to drive capture accumulation:
  - *State 0 (IDLE / Silence Gating):* Uses a dynamic RMS baseline with Exponential Moving Average (EMA) to bypass heavy DSP during periods of noise or silence.
  - *State 1 (ATTACK):* Uses Normalized Half-Wave Rectified Spectral Flux (NHWRSF) to detect hammer strikes. Sends onset pulse to the Engine to begin pitch detection.
  - *State 2 (TRANSIENT):* Institutes a hard delay waiting for the chaotic broadband noise of the strike to physically decay.
  - *State 3 (HARMONIC DECAY):* Uses NINOS2 (Normalized Identification of Note Onset based on Spectral Sparsity) to monitor the signal. It ignores volume swells and identifies the "Golden Window" of pure, stable harmonic decay for capture.
  - *State 4 (RELEASE):* Caps the capture at 1.5 seconds. The pipeline detects completion (buffer full or silence decay), dispatches the `CapturePayload` to Thread 3 via a bounded crossbeam channel, and transitions `CaptureState` to `Processing` — all without blocking the real-time pipeline.
- **The Engine (3-Stage Algorithm Router):** A pitch detection chain that operates as an independent state machine, **synchronously reset** by the Gatekeeper's onset pulse but otherwise decoupled from the Gatekeeper's internal transient delays.
  - **Stage 1: Dual-Track Correlation:** Both 2048-pt (Treble) and 8192-pt (Bass) magnitudes are evaluated on every frame using dot-product correlation against 88 `SparseTemplate` arrays (generated via the two-asymptote β model). An **Energy Density Equivalence** crossover multiplies the Treble score by 4.0 to combat array volume differentials—deterministically selecting the optimal resolution path (`is_bass`).
  - **Stage 1.5: Phantom Partial Mask:** If the Bass track triggers, a predictive filter identifies longitudinal intermodulation sums (the combinations m+n). We calculate a dynamic frequency smearing radius based on β, completely zeroing out the phantom energy bins to prevent the solver from locking onto physical noise.
  - **Stage 2: Guided Trajectory Extraction:** Sub-bin frequency peaks are aggressively mapped utilizing the base matching template's layout. We utilize exponential QIFFT refinement on the Treble arrays, and phase-independent `quinn_second_estimator` logic on the dense Bass limits. Concurrent with extraction, spectral peak symmetries and lobe widths are evaluated; if beating unisons are detected, the subsystem flags `suspend_beta_update` to quarantine subsequent offline configurations.
  - **Stage 3: Median-Adjustive Trajectories (MAT):** The algebraic core evaluating constraints against up to 66 combinatorial pairs of extracted components. Returns the pure fundamental $f_0$.
  - **Probabilistic Pitch Tracking** *(experimental)*: During the Gatekeeper's NINOS2-confirmed Stable phase, a smoothing algorithm (such as a Kalman Filter, HMM, or Viterbi algorithm) smooths the mathematical output. The filter is bypassed during Attack/Transient states and hard-reset on each new onset pulse. Whether this stage ships in the final release is undecided.
- **Output:** Pushes a `FrameOutput` structure every hop, containing the treble magnitude spectrum, sub-cent accurate $f_0$, real-time partial frequencies, and the `suspend_beta_update` flag, to the UI thread via a wait-free `triple_buffer`.

Once the Gatekeeper detects silence, it closes the gate by sending the `is_silence` flag to the Engine to force an immediate state reset and prevent pitch detection from running on background noise.

#### Thread 3: The Background Worker

This is a single detached worker thread spawned at pipeline construction inside `AudioPipeline::new()`. It blocks on a crossbeam receiver, waking only when a `CapturePayload` arrives.

- **Action:** When the pipeline dispatches a filled capture buffer, the worker:
  1. Performs a high-resolution power-of-two FFT on the captured audio (up to 65,536 points).
  2. **Auto Mode** (`target_note == 255`): Runs the full 88-key Template Matcher at the worker's high-resolution FFT to identify the note, then refines *f₀* via parabolic interpolation.
  3. **Manual Mode**: Performs a bounded ±1 semitone peak search around the user-selected target, refining with parabolic interpolation.
  4. Runs MAT (Median-Adjustive Trajectories) to extract partials and compute the inharmonicity coefficient ($B$) via pairwise partial combinations.
  5. Writes diagnostic files (`audio.raw` + `analysis.json`) to the `diagnostics/` directory.
- **Output:** Sends a `KeyMeasurement` (containing `key_index`, `measured_f0`, extracted `partials`, and `calculated_b`) to the GUI via the `result_tx` crossbeam SPSC channel. Resets `CaptureState` to `Idle` and recycles the audio buffer back into the `AudioPool`.

#### Thread 4: The UI Thread (The Visual Renderer)

This is the graphical interface thread operating at 60 FPS.

- **Action:** Consumes the high-speed stream of `FrameOutput` structures from Thread 2 via the `triple_buffer` to drive the instantaneous tuning visualizers (spectrogram, cents-deviation, keyboard). Drains `KeyMeasurement` results from the Worker via `pipeline_handle.result_rx` and inserts them into the `InharmonicityProfile`. Reads/writes configuration (e.g., silence threshold, target key) and polls runtime observations (e.g., smoothed RMS for the Envelope Viewer) via `Arc<PipelineAtomics>`.

#### Cross-Thread Communication Topology

Because `tuner-core` enforces strict zero-allocation, wait-free real-time audio constraints, it relies on a rigidly defined topology for inter-thread message passing:

| Pathway | Primitive | Direction | Purpose |
| --- | --- | --- | --- |
| **Hardware Capture** | `ringbuf` SPSC | Stream (1) → DSP (2) | Lossless elastic buffer for incoming raw audio. |
| **Structural Output** | `triple_buffer` | DSP (2) → UI (4) | Lossy continuous viz telemetry (`FrameOutput`). |
| **DSP Parameters** | `Arc<Atomic*>` | UI (4) ↔ DSP (2) | Wait-free configuration and metric reads/writes. |
| **Capture Dispatch** | crossbeam SPSC (bounded) | DSP (2) → Worker (3) | `CapturePayload` containing pooled audio buffer + metadata. |
| **Buffer Recycling** | Lock-Free Object Pool | DSP (2) ↔ Worker (3) | Recycled `Box<[f32; 66150]>` arrays — zero allocation during capture. |
| **Capture Lifecycle** | `AtomicU8` (baton-pass) | UI (4) → DSP (2) → Worker (3) | `CaptureState`: Idle → Armed → Recording → Processing → Idle. |
| **Worker Results** | crossbeam SPSC (bounded) | Worker (3) → UI (4) | `KeyMeasurement` with partials, $f_0$, and $B$ coefficient. |

## Iced 0.14.0 UI

### Features

- **Spectrogram Visualization**: Real-time frequency spectrum display
- **Cent Meter**: Visual tuning accuracy indicator with color-coded feedback
- **Piano Keyboard Select**: 88-key piano interface with click-to-select frequency functionality
- **Inharmonicity Measurement**: Capture and analyze piano-specific inharmonicity characteristics
- **Profile Management**: Save and load piano tuning profiles with JSON persistence
- **Transient Detection Calibration**: Manual and automatic transient detection calibration
- **Noise Floor Calibration**: Manual and automatic noise floor calibration

### Planned Features

- **Inharmonicity Compensation**: Professional piano-specific tuning curve adjustment
- **Temperament Selection**: Support for various tuning temperaments
- **Tuning Standard Options**: A440 and other reference frequencies
- **Pitch Raise**: Adjustable over-pull calculation parameters to offset the structural drop of tension during gross pitch adjustments (e.g. blindly tuning up an out-of-tune piano).
- **Diagnostics Viewer**: A dedicated panel in the settings view to securely read, parse, and graph the offline `analysis.json` telemetry mathematically for the tuner.

> [!TIP]
> **Graphics Issues? Check Your Vulkan Drivers**
>
> This application uses `iced` with the `wgpu` backend (Vulkan on Linux). If you experience
> invisible widgets, flickering, or blank panels, the most common cause is stale or
> incompatible Vulkan drivers. Ensure your GPU drivers are fully up-to-date before
> reporting rendering bugs.

See [tuner-gui](tuner-gui/README.md) for more information.

## Project Work in Progress

### Pipeline Hardening

- **Dynamic Sample Rate Plumbing**: The sample rate is currently hardcoded to 44,100 Hz in `Engine::new()` and `CapturePayload`. The actual CPAL-negotiated rate needs to be plumbed from `spawn_analysis_thread()` through the `AudioPipeline` constructor and into the Worker to prevent silent frequency miscalculation on 48 kHz hardware.
- Move File I/O for inharmonicity profiles into `tuner-core` for true frontend agnosticism.

### Engine TODOs

- **Engine Refinement / Tuning Rules**: Modify or adjust the current template matching to better avoid note misidentification. If after further testing, template matching deems to be insufficient, we will move to a different algorithmic approach. 
- **Probabilistic Pitch Tracking** *(experimental — may not ship)*: If the engine's deterministic tuning is deemed visually jittery, probabilistic pitch tracking will be refined (e.g. Hidden Markov Model (HMM), Viterbi sequence, or a Gatekeeper-governed Linear Kalman filter).

### Worker TODOs

- **CaptureState `compare_exchange`**: The current baton-pass relies on convention (each thread writes only its owned transitions). Switching to `compare_exchange` would enforce correct ordering at the atomic level and prevent a category of future bugs.

### Known issues 

- **MAT instability**: MAT scews the fundamental frequency, so we use the template's f0 as the final f0. This will be fixed in the future.
- **Bass register identification instability**: Lower bass notes are often misidentified due to a mix of tighter partial density, and the linear limits of the bin width regarding the FFT. Further invesitagtion will be done. Manual selection of the key is a workaround for now.


### Architecture Goals (No ETA)

- **Tuner-core standalone package**: Completely separate the DSP pipeline from the GUI into its own crate. This will allow us to use the tuner-core library in other applications ourside of the tuner GUI.

## Getting Started

### Building and Running

```bash
# Clone the repository
git clone <repository-url>
cd inharmonicity

# Build the project
cargo build

# Run the GUI application
cargo run -p tuner-gui
```

### Diagnostic Files

Every capture will produce two files in the `diagnostics/` directory:

- `audio.raw`: The raw audio buffer that was captured.
- `analysis.json`: The analysis of the audio buffer.

This is useful for debugging and for testing new algorithms. Each file is categorized by the note that was detected.

A script has been created to quickly analyze and visualize the data. Running the script alone will just print results to the console.

```bash
python3 scripts/analyze_capture.py diagnostics/
```

to view the plots that visualize the data:

```bash
python3 scripts/analyze_capture.py diagnostics/ --gui
```

### License & Contact

This project is licensed under the terms specified in the LICENSE file.

For questions, suggestions, or collaboration opportunities, please contact [the team](mailto:contact@anauseam.org).
