# Inharmonicity - Professional Piano Tuning Application

![Inharmonicity Interface](images/interface-screenshot.png)

An open source professional-grade piano tuning application built in Rust with real-time audio analysis, pitch detection, and cent deviation measurement. Designed for piano tuners with planned support for inharmonicity compensation.

For a detailed overview of the algorithms used, see the [Anauseam documentation](https://docs.anauseam.org/project-docs/inharmonicity-tuner/00_intro).
For the design rationale and open observations, see [ARCHITECTURE.md](ARCHITECTURE.md). For contribution guidelines, see [CONTRIBUTING.md](CONTRIBUTING.md).

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
│   │   │   ├── peaks.rs            # Spectral peak extraction with Jacobsen sub-bin interpolation
│   │   │   ├── twm.rs              # Canonical Two-Way Mismatch (Primary F0 discovery)
│   │   │   ├── pitch.rs            # XQIFFT/Quinn estimation, sub-cent refinement
│   │   │   ├── templates.rs        # Structural matched-filters (2-asymptote β model, Gaussian weighting)
│   │   │   ├── mat.rs              # Median-Adjustive Trajectories algebraic combinatorial solver
│   │   │   ├── metrics.rs          # RMS, EMA, CSD, NHWRSF, NINOS2 signal metrics
│   │   │   ├── tuning.rs           # Cent deviation, inharmonicity-compensated frequencies
│   │   │   └── inharmonicity.rs    # B-coefficient calculation (pending replacement)
│   │   ├── cola.rs                 # CircularFifo — COLA circular FIFO for overlapping frame analysis
│   │   ├── models.rs               # Domain types: Note, Partial, KeyMeasurement, profiles
│   │   ├── pipeline.rs             # AudioPipeline mediator — Dual-FFT unconditional execution
│   │   ├── engine.rs               # F0 Engine — TWM Discovery + Goertzel Phase Tracking
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
> | Component                                                      | Status         |
> | -------------------------------------------------------------- | -------------- |
> | `pipeline.rs` — AudioPipeline mediator + shared state          | 🟡 Testing     |
> | `gatekeeper.rs` — 5-state signal validator (pure DSP)          | ✅ Implemented |
> | `engine.rs` — TWM Discovery + Goertzel Phase Tracking          | 🟡 Testing     |
> | `peaks.rs` — Jacobsen sub-bin peak extraction                  | 🟡 Testing     |
> | `twm.rs` — Canonical M&B (1994) Two-Way Mismatch               | 🟡 Testing     |
> | `worker.rs` — Background worker (single thread)                | 🟡 Testing     |
> | `app.rs` — Wait-free GUI dispatcher integration                | 🟡 Testing     |
>
> **Currently**, the architecture is in a working testing phase. `app.rs` has been successfully migrated to use the overlapping frame pipeline alongside a lock-free, wait-free SPSC telemetry queue. The `AudioPipeline` serves as the sole frontend-facing DSP orchestrator. The pipeline natively computes unconditional Dual-Track FFT mappings, while the Engine identifies the fundamental frequency using the canonical Maher & Beauchamp (1994) Two-Way Mismatch (TWM) algorithm for discovery, followed by per-partial Goertzel phase tracking for sub-cent accuracy. Heavy algorithms like MAT are routed completely off-thread to the Background Worker, returning high-resolution inharmonicity measurements natively back to the GUI's `InharmonicityProfile` memory map without ever blocking the audio hot-path.

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

- **Dynamic Sample Rate Plumbing**: The sample rate is currently hardcoded to 44,100 Hz in the `audio.rs` module and `CapturePayload`. The actual CPAL-negotiated rate needs to be plumbed from `spawn_analysis_thread()` through the `AudioPipeline` constructor, into the `Engine`, and into the Worker to prevent silent frequency miscalculation on 48 kHz hardware. Note: The `03-dsp-pipeline.md` guideline explicitly asks new code to read the rate from a single source of truth so this migration remains a single-point change.
- Move File I/O for inharmonicity profiles into `tuner-core` for true frontend agnosticism.

### Engine TODOs

- **TWM Parameter Optimization (MOBO)**: The default heuristics for the Two-Way Mismatch algorithm ($q=1.4, r=0.5, \rho=0.33$) were originally calibrated by Maher & Beauchamp for general audio. To fully optimize these constants for the unique inharmonicity and missing-fundamental characteristics of a piano, a Multi-Objective Bayesian Optimization (MOBO) framework will be implemented to rigorously tune them against a 10,000-frame generative synthetic dataset. See [`docs/adr/0001-mobo-tuning.md`](docs/adr/0001-mobo-tuning.md) for the full methodology.
- **Viterbi Transient Contamination**: The Viterbi path cost accumulator runs from the first frame after an onset, which includes frames during the Gatekeeper's TRANSIENT state (State 2) before the chaotic strike energy has fully decayed. Bass key profiles are cheap to score on broadband transient noise (due to the $f^{-0.5}$ frequency weighting and fewer predicted partials), giving them an early path cost advantage that the `JUMP_PENALTY` then makes very sticky. The proposed fix is to reset `path_costs` on every frame while the Gatekeeper is in TRANSIENT state, so accumulation only begins once the Gatekeeper reaches its STABILITY window. The `JUMP_PENALTY` itself can also be tuned lower (experimentally, `5.0`–`8.0` reduced perceived stickiness in manual testing). See [`docs/adr/0002-twm-peak-masking-validation.md`](docs/adr/0002-twm-peak-masking-validation.md) for the diagnostic methodology to test changes.
- **TWM Bass-Lock Bias (Sub-Harmonic Locks)**: Real-world testing shows the engine occasionally locks to extreme bass candidates (e.g. D#4 → A#0, F#3 → A0, D2 → E1). This is a TWM scoring problem independent of Viterbi: because A#0's harmonic series is so dense across the mid-range spectrum, many mid-range peaks accidentally align with its predicted partials, and the $f^{-0.5}$ weighting makes those alignments very cheap. The Viterbi amplifies this by making the false lock sticky, but it does not cause it. Resolution requires MOBO-tuned parameters or an additional penalty for octave-relationship false locks.
- **Inharmonic Curve Calculation**: A solver to calculate the optimal tuning stretch (inharmonic curve) for the entire piano so that the harmonics of the bass align with the fundamentals of the treble. This will be implemented after the inharmonicity profile cross-thread transfer is completed.

### Worker TODOs

- **Replace Quinn with CSPE in MAT**: The offline MAT solver (`worker.rs` → `mat.rs`) currently uses Quinn's second estimator for sub-bin peak extraction. Since MAT runs entirely on the non-realtime Worker thread, it should be upgraded to use CSPE (Combined Spectral Peak Estimation) — the same method used in the original Hodgkinson DAFx-09 paper. CSPE provides instantaneous frequency from phase derivatives, giving fundamentally more information than any magnitude-only interpolator. The complex spectrum is already available in the worker's scratch buffers.
- **CaptureState `compare_exchange`**: The current baton-pass relies on convention (each thread writes only its owned transitions). Switching to `compare_exchange` would enforce correct ordering at the atomic level and prevent a category of future bugs.

### Known Issues

- **Noisy Environments**: The engine uses a `-30 dB` relative amplitude threshold to filter sympathetic noise before TWM evaluation. If used in a noisy rehearsal environment where the Signal-to-Noise Ratio (SNR) drops below 30 dB, the noise will bleed past the mask and may cause stability issues.
- **MAT instability**: MAT skews the fundamental frequency in some cases, so the template's $f_0$ is used as the final $f_0$. Upgrading to CSPE-based peak extraction may resolve this.
- **Stereo DC Blocker**: The `DcBlocker` currently uses a single state variable. If a stereo audio stream is ingested, the interleaved channels will corrupt the 1-pole IIR filter. The app either needs to explicitly force CPAL to Mono or support stereo state tracking.

### Follow-up Documentation & Verification

- **Review the `unsafe` byte-slice transmute in `worker.rs::write_diagnostics`.** Used to serialize the captured `f32` audio buffer to `audio.raw`. Functionally correct but worth reviewing whether `bytemuck` or a safe-wrapper crate could replace the raw `from_raw_parts` call without a performance regression.

### Architecture Goals (No ETA)

- **`models/` growth pattern**: The current `models.rs` is a single file. It will eventually become a `models/` directory with submodules (`models/note.rs`, `models/partial.rs`, …) once the single file becomes uncomfortable. There is no specific threshold beyond "when it stops feeling right." Documented in `04-algorithms-and-models.md`.

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

Every capture will produce two files in a key-specific subdirectory within the `diagnostics/` folder (e.g., `diagnostics/key_001_A#0/`):

- `audio.raw`: The raw audio buffer that was captured.
- `analysis.json`: The analysis of the audio buffer.

This is useful for debugging and for testing new algorithms. Each folder is categorized by the note that was detected.

A script has been created to quickly analyze and visualize the data. Running the script alone will just print results to the console.

```bash
python3 scripts/analyze_capture.py diagnostics/
```

to view the plots that visualize the data:

```bash
python3 scripts/analyze_capture.py diagnostics/ --gui
```

#### CLI Developer Tools

To test the core DSP algorithms offline without launching the GUI, a standalone Cargo example is provided:

- **`diagnose_engine`**: A heavy-duty offline telemetry harness. It allows you to feed a captured `.raw` audio file back into the `tuner-core` engine to dump exactly what the STFT, peak extractor, and TWM algorithm observed at every frame into `spectrum.csv` and `peaks.csv`.

  ```bash
  cargo run --example diagnose_engine -- diagnostics/key_001_A#0/audio.raw
  ```

### License & Contact

This project is licensed under the terms specified in the LICENSE file.

For questions, suggestions, or collaboration opportunities, please contact [the team](mailto:contact@anauseam.org).
