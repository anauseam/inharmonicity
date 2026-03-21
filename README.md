# Inharmonicity - Professional Piano Tuning Application

![Inharmonicity Interface](images/interface-screenshot.png)

An open source professional-grade piano tuning application built in Rust with real-time audio analysis, spectrogram visualization, and interactive piano keyboard interface. Designed for professional piano tuners with planned support for inharmonicity compensation via advanced tuning algorithms.

> [!TIP]
> **Graphics Issues? Check Your Vulkan Drivers**
>
> This application uses `iced` with the `wgpu` backend (Vulkan on Linux). If you experience
> invisible widgets, flickering, or blank panels, the most common cause is stale or
> incompatible Vulkan drivers. Ensure your GPU drivers are fully up-to-date before
> reporting rendering bugs.

## Features

### Core Functionality

- **Real-time Audio Analysis**: Live audio capture and processing using CPAL
- **Spectrogram Visualization**: Real-time frequency spectrum display
- **Cent Meter**: Visual tuning accuracy indicator with color-coded feedback
- **Interactive Piano Keyboard**: 88-key piano interface with click-to-select frequency functionality
- **Cent Meter Confidence**: Probabilistic confidence value for auto-detected notes
- **Partials Analysis**: Harmonic partial frequency display
- **Inharmonicity Measurement**: Capture and analyze piano-specific inharmonicity characteristics
- **Profile Management**: Save and load piano tuning profiles with JSON persistence

### Planned Features

- **Inharmonicity Compensation**: Professional piano-specific tuning curves
- **Buffer Size Selection**: Choice between 2048 and 4096 sample buffers
- **Temperament Selection**: Support for various tuning temperaments
- **Tuning Standard Options**: A440 and other reference frequencies

### Technical Features

- **High-Performance Audio Processing**: FFT-based frequency analysis with YIN pitch detection and spectrum refinement
- **DC Offset Removal**: Always-on single-pole high-pass IIR filter in the audio stream callback removes hardware-dependent DC bias, ensuring device- and platform-agnostic operation
- **Thread-Safe Architecture**: Dedicated audio processing thread with crossbeam channels
- **Real-time Updates**: Continuous GUI updates with audio analysis

## 🏗️ Architecture

### Project Structure

```text
inharmonicity/
├── tuner-core/                     # Headless audio processing & analysis (no GUI code)
│   ├── src/
│   │   ├── algorithms/             # Stateless DSP building blocks (spectral, pitch, metrics, tuning)
│   │   │   ├── spectral.rs         # FFT, windowing, and spectrum magnitude extraction
│   │   │   ├── pitch.rs            # YIN / pYIN pitch detection and spectrum refinement
│   │   │   ├── dpyin.rs            # Decimated pYIN — bass register pitch detection
│   │   │   ├── scout.rs            # Rough frequency neighborhood detection
│   │   │   ├── metrics.rs          # RMS, EMA, CSD, NINOS2 signal metrics
│   │   │   ├── tuning.rs           # Cent deviation, inharmonicity-compensated frequencies
│   │   │   └── inharmonicity.rs    # B-coefficient calculation (deprecated, pending replacement)
│   │   ├── models.rs               # Domain data types: Note, Partial, KeyMeasurement, profiles
│   │   ├── pipeline.rs             # AudioPipeline mediator, shared state types, memory pools
│   │   ├── engine.rs               # F0 Engine — Scout / Bass / Treble DSP (wireframe)
│   │   ├── gatekeeper.rs           # 5-state signal validator (DSP, no shared state)
│   │   ├── worker.rs               # Background worker manager for heavy offline DSP (wireframe)
│   │   ├── audio.rs                # CPAL audio capture, stream management, DC blocking
│   │   ├── calibration.rs          # Standalone noise-floor calibration
│   │   ├── capture_processing.rs   # Legacy frame processing (deprecated — see migration note)
│   │   └── lib.rs                  # Crate root and AnalysisResult definition
│   └── Cargo.toml
├── tuner-gui/                      # Iced-based GUI frontend
│   ├── examples/                   # Standalone visual sandboxes
│   │   ├── shared/mod.rs           # Shared audio testing utility
│   │   ├── dashboard_test.rs       # Composite widget area integration test
│   │   ├── spectrogram_test.rs     # Isolated spectrogram widget test
│   │   └── cent_meter_test.rs      # Isolated cent meter widget test
│   ├── src/
│   │   ├── main.rs                 # Binary entry point
│   │   ├── lib.rs                  # Crate configuration
│   │   ├── app.rs                  # Application state, audio thread, message loop
│   │   ├── views/                  # Layouts orchestrating multiple components
│   │   │   ├── main_view.rs        # Main composed layout (widgets + sidebar)
│   │   │   └── settings_view.rs    # Settings layout (noise floor, envelope viewer)
│   │   ├── widgets/                # Independent UI drawing components
│   │   │   ├── cent_meter.rs       # Cent deviation meter
│   │   │   ├── envelope.rs         # RMS envelope viewer (time-domain)
│   │   │   ├── partials_display.rs # Harmonic partials display
│   │   │   ├── piano_keyboard.rs   # Interactive 88-key piano
│   │   │   └── spectrogram.rs      # Frequency spectrum visualization
│   │   └── utils/                  # Shared UI helpers (sidebar, timers)
│   └── Cargo.toml
└── Cargo.toml                      # Workspace configuration
```

### The AudioPipeline (Mediator Pattern)

The `tuner-core` crate is designed to be **frontend-agnostic**. Any GUI (Iced, egui, WASM, etc.) can consume it through the `AudioPipeline` — the single entry point that orchestrates all DSP components and manages cross-thread shared state.

```text
AudioPipeline::new()  →  (AudioPipeline, PipelineHandle)
        │                         │
        ▼                         ▼
    Audio Thread              Frontend Thread
    ┌─────────────────┐       ┌──────────────┐
    │ Engine (F0 DSP) │       │ config       │ ← read/write (silence threshold, etc.)
    │       ↓         │       │ runtime      │ ← read-only  (current RMS EMA, etc.)
    │ Gatekeeper      │       └──────────────┘
    │   (5-state SM)  │              ↑
    │       ↓         │              │ polls via Arc<Mutex<...>>
    │ SharedState ────────────────────
    │       ↓ capture trigger
    │ AudioPool ──────────┐
    └─────────────────┘   │
                          ▼
                    Worker Thread(s)
                    ┌──────────────────┐
                    │ MAT / ICF        │ ← heavy offline DSP
                    │ (B coefficient)  │
                    └────────┬─────────┘
                             │ returns buffer
                             ▼
                         AudioPool (recycled)
```

This follows the **Split / Handle pattern** (the same convention used by `crossbeam_channel`, `ringbuf::split()`, `std::thread::spawn`):

- **`AudioPipeline`** is moved to the audio thread. It owns the pure DSP components (`Engine`, `Gatekeeper`) and is the **only** thing that touches `Arc<Mutex<...>>` shared state. After calling each DSP component's `process_frame()`, the pipeline reads their public fields and syncs observations to shared state.
- **`PipelineHandle`** is kept by the frontend. It provides `Arc<Mutex<...>>` handles for:
  - `config` — reading and writing configuration values (e.g., silence threshold)
  - `runtime` — polling runtime observations (e.g., smoothed RMS for the Envelope Viewer)

A frontend contributor just calls `AudioPipeline::new()`, gets a `PipelineHandle`, and never needs to know about Gatekeeper internals, EMA calculations, or lock management.

The pipeline also manages the **`WorkerManager`** (`worker.rs`), which owns a single dedicated background thread for computationally expensive offline DSP. When the Gatekeeper's 5-state machine triggers a capture (State 4: RELEASE), the pipeline dispatches a 1.5-second audio buffer from the `AudioPool` to the worker thread. The worker runs heavy algorithms (MAT or ICF) to calculate the inharmonicity coefficient ($B$), sends the result to the frontend, and recycles the buffer back to the pool. A single thread is sufficient because captures are infrequent (one stable note at a time) and the algorithms are fast enough to complete before the next capture could arrive.

> [!IMPORTANT]
> **Migration Status**
>
> The unified `AudioPipeline` system is partially implemented:
>
> | Component | Status |
> | --- | --- |
> | `pipeline.rs` — AudioPipeline mediator + shared state | ✅ Implemented |
> | `gatekeeper.rs` — 5-state signal validator (pure DSP) | ✅ Implemented |
> | `engine.rs` — F0 Engine (Scout / Bass / Treble) | ⬜ Wireframe |
> | `worker.rs` — Background worker (single thread) | ⬜ Wireframe |
> | Pipeline fully encapsulates all output | ⬜ In progress |
> | STFT overlap — Hamming window → 50% COLA w/ Hann | ⬜ Planned (post-migration) |
>
> **Currently**, `app.rs` still calls algorithm functions directly (FFT, YIN, tuning)
> and uses the legacy `capture_processing` module. The `AudioPipeline` is live for
> Gatekeeper orchestration and shared state (RMS / threshold), while the `Engine`
> and `Worker` are scaffolded for future integration. As the migration completes,
> `app.rs` will reduce to a thin `pipeline.process_frame()` call with zero
> knowledge of DSP internals.

> [!NOTE]
> **Planned: COLA STFT Architecture**
>
> The pipeline currently processes **non-overlapping** 2048-sample frames. Because
> frame boundaries are asynchronous to real piano strikes, the original Hann window
> (which tapers to exactly 0.0 at both edges) created transient detection dead zones:
> a hammer strike landing on a frame boundary would be attenuated to silence in both
> adjacent frames, preventing the CSD algorithm from tripping.
>
> **Immediate mitigation (current):** The Hann window is replaced with a **Hamming**
> window (`0.54 − 0.46·cos(…)`), which maintains an 8% amplitude pedestal at the
> edges. This ensures boundary transients retain enough broadband energy to breach
> the CSD threshold, while its −43 dB sidelobe floor provides sufficient spectral
> clarity for partial interpolation at 0% overlap.
>
> **Target architecture (post-migration):** Once `app.rs` is fully migrated to
> `AudioPipeline.process_frame()`, the frame geometry will be upgraded to a
> **50% overlap COLA** design (1024-sample hop, circular FIFO ring buffer). This
> satisfies the Constant Overlap-Add property with a Hann window, mathematically
> guaranteeing that every sample is analyzed at full window amplitude in at least
> one frame — eliminating temporal blind spots entirely. The Hann window's
> −18 dB/octave sidelobe roll-off also provides superior noise floor suppression
> over Hamming's −6 dB/octave for resolving high-order inharmonic partials. The
> FFT rate doubles (~21.5 → ~43 frames/sec), adding roughly 1 ms of CPU time per
> second of audio — a negligible cost on modern hardware.

### Global Data Structures & Memory Management

To maintain real-time performance without relying on OS priority elevation, the system completely avoids dynamic heap allocation (`std::thread::spawn`, `Vec::push`, etc.) during runtime by using pre-allocated, lock-free structures:

- **The Elastic Ring Buffer:** A massive lock-free circular buffer (e.g., 16,384 samples) connecting Thread 1 and Thread 2. It acts as an elastic shock absorber; if the OS briefly suspends the processing thread, the audio stream can still safely dump samples into the buffer without dropping data.
- **Lock-Free Object Pool (`AudioPool`):** A pre-allocated pool of fixed-size arrays (each large enough to hold 1.5 seconds of audio at 44,100 Hz). If capture mode is enabled, Thread 2 borrows an array to record a stable note and passes the reference to the background worker, which returns it to the pool when finished.
- **`ProcessingFrame`:** Thread-local scratch buffers (audio, time-domain, frequency-domain) for zero-allocation per-frame DSP.

### Threading Model

#### Thread 1: The Audio Stream (The Harvester)

- **Role:** High-speed hardware ingestor and signal conditioner.
- **Action:** Continuously captures raw audio from the microphone at 44,100 Hz. Each sample passes through a `DcBlocker` (single-pole high-pass IIR, α = 0.995, ~3.5 Hz cutoff) to remove hardware-dependent DC offset, then is pushed into the Elastic Ring Buffer. This guarantees every downstream consumer sees a zero-mean signal regardless of microphone, audio interface, or OS driver.
- **Rule:** This thread performs zero allocations and no analysis. The DC blocker is the only computation — one multiply and two additions per sample — and is classified as signal *conditioning*, not signal analysis. Its job is to guarantee pristine, zero-mean data throughput.

#### Thread 2: The Audio Processing Pipeline (The Brains)

This thread constantly consumes data from the Elastic Ring Buffer and executes a deterministic DSP pipeline via `AudioPipeline.process_frame()` to calculate the fundamental frequency ($f_0$).

- **The Gatekeeper (Signal Validator & 5-State Logic):** An always-running traffic cop monitoring the $f_0$ stream. It outputs a discrete `SignalState` to the UI, holding the last reliable note for 1.5 seconds to prevent visual flickering. When "Capture Mode" is enabled, it utilizes a perfect 5-stage state machine to control the 1.5-second capture window:
  - *State 0 (IDLE / Silence Gating):* Uses a dynamic RMS baseline with Exponential Moving Average (EMA) to completely ignore background room noise and momentary unison beating volume dips. Bypasses heavy DSP.
  - *State 1 (ATTACK):* Uses Complex Spectral Difference (CSD) to detect the massive positive derivative spike of the hammer strike.
  - *State 2 (TRANSIENT):* Institutes a hard delay waiting for the chaotic broadband noise of the strike to physically decay.
  - *State 3 (HARMONIC DECAY):* Uses the NINOS2 (Normalized Identification of Note Onset based on Spectral Sparsity) metric to monitor the signal. It ignores volume swells and identifies the "Golden Window" of pure, stable harmonic decay for capture.
  - *State 4 (RELEASE):* Caps the capture at 1.5 seconds to prevent flat-lining pitch drift, closing the gate, recycling the buffer, dispatching the payload to Thread 3, and immediately resets to State 0.
- **The Scout:** Applies a **Hamming** window to a 2048-sample buffer to eliminate spectral leakage, then runs a Real FFT to find an accurate rough frequency neighborhood. The Hamming window is the correct choice for the current non-overlapping frame geometry (see COLA migration note above). Once 50% overlap is implemented, this will switch to a Hann window to satisfy the COLA constraint.
- **The Router & Dual Engines:**
  - *Bass Engine (< 150 Hz):* Instructs the ring buffer to pull an 8192-sample window (necessary for long bass wavelengths). It applies an anti-aliasing filter, decimates the audio, and runs the pYIN algorithm. The Hidden Markov Model within pYIN effectively prevents the octave errors that plague stiff, copper-wound bass strings.
  - *Treble Engine (> 150 Hz):* Uses the standard 2048-sample window. It features two toggleable modes. For professional use, it seeds a Digital Phase-Locked Loop (DPLL) with the Scout's frequency, using a Proportional-Integral filter to lock tightly onto the string's phase. For educational use, it utilizes a Quadratic Interpolated FFT (QIFFT) to demonstrate highly accurate frequency-domain sub-bin peak detection.
- **Output:** Pushes the sub-cent accurate $f_0$ float to the UI thread via a lock-free channel.

#### Thread 3: The Background Worker (The Heavy Lifter)

- **Role:** A single detached worker thread pre-allocated at application launch.
- **Action:** When the Gatekeeper conditionally triggers a capture, the sleeping worker receives the 1.5-second audio array from the Object Pool. It offers two toggleable algorithms for calculating the inharmonicity coefficient ($B$):
  - *Professional Mode:* Runs the Median-Adjustive Trajectories (MAT) algorithm. MAT uses extremely narrow frequency bands to iteratively adjust its trajectory, finding the true inharmonic partials with extreme computational efficiency and precision.
  - *Educational Mode:* Runs an Inharmonic Comb Filter (ICF), which sweeps a grid of values to align the "teeth" of a comb filter with the stretched partials, providing a highly intuitive visual representation of piano stiffness.
- **Output:** Sends the $B$ coefficient to the UI thread and recycles the audio array back into the Object Pool.

#### Thread 4: The UI Thread (The Visual Renderer)

- **Role:** The graphical interface operating at 60 FPS.
- **Action:** Consumes the high-speed stream of $f_0$ floats from Thread 2 to drive the instantaneous tuning visualizer (strobe, cents-deviation, or dial). Polls the `PipelineHandle` for runtime observations (e.g., RMS envelope) and reads/writes configuration (e.g., silence threshold) via `Arc<Mutex<...>>`.
- **Background Duty:** When it receives a new $B$ coefficient from Thread 3, it applies a Bayesian moving average to prevent measurement jitter. It then recalculates the 88-key Railsback tuning curve in the background, smoothly adjusting the target pitches for the user.

## 🚀 Getting Started

### Prerequisites

- Rust 1.70+
- Linux with ALSA/PulseAudio support
- X11 or Wayland display server

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

### Dependencies

- **Iced 0.14.0**: Modern Rust GUI framework with canvas support
- **CPAL 0.16.0**: Cross-platform audio library
- **RustFFT 6.4.1**: High-performance FFT implementation
- **Crossbeam-channel 0.5.15**: Lock-free concurrent data structures
- **Anyhow 1.0.100**: Error handling utilities
- **Once-cell 1.18**: Lazy static initialization

## 🔬 Planned  Features

- **Stretch Tuning**: Compensation for the natural inharmonicity of piano strings
- **Partial Frequency Analysis**: Advanced analysis of harmonic partials
- **Young's Inharmonicity Model**: Implementation of the standard inharmonicity calculation
- **Custom Tuning Profiles**: User-defined tuning curves for specific piano models
- **Temperament Settings**: Various tuning temperaments (Equal, Just, etc.)
- **Tuning Profiles**: Save/load custom tuning configurations
- **Sample Buffer Adjustment**: Configurable audio buffer sizes (2048/4096)
- **Audio Device Selection**: Choose input device from GUI
- **Export Functionality**: Save tuning data and reports

At a much later date, complete piano voicing analysis may be implemented after core tuning functionality is complete.

## 🎛️ Usage

### Interface Overview

The application features a professional layout with:

1. **Spectrogram Panel**: Real-time frequency spectrum visualization
2. **Cent Meter**: Tuning accuracy indicator (-50 to +50 cents)
3. **Piano Keyboard**: Interactive 88-key piano for manual note selection
4. **Partials Panel**: Harmonic partial frequency display
5. **Control Sidebar**: Tool visibility toggles and settings
6. **Measurement Mode**: Automatic capturing of stable note sustain

## 📝 License

This project is licensed under the terms specified in the LICENSE file.

## 🤝 Contributing

Contributions are welcome! Please ensure that:

- Code follows Rust best practices
- Threading requirements are maintained
- Audio processing performance is preserved
- GUI responsiveness is maintained

## 📞 Support

For technical support or bug reports, please include:

- Operating system and version
- Graphics driver information
- Audio system details (ALSA/PulseAudio)
- Complete error logs if applicable
