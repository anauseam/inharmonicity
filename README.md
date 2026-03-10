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
- **Thread-Safe Architecture**: Dedicated audio processing thread with crossbeam channels
- **Real-time Updates**: Continuous GUI updates with audio analysis

## 🏗️ Architecture

### Project Structure

```text
inharmonicity/
├── tuner-core/          # Audio processing and analysis engine
│   ├── src/
│   │   ├── audio.rs     # CPAL audio capture and stream management
│   │   ├── fft.rs       # FFT processing and spectrum analysis
│   │   ├── pitch.rs     # stateless pYIN pitch detection algorithm
│   │   ├── tuning.rs    # Musical note calculations and cent deviation, inharmonicity curve calculation
│   │   ├── inharmonicity.rs        # Inharmonicity constant calculation and profile management
│   │   ├── capture_processing.rs   # Audio frame processing strategies for inharmonicity measurement
│   │   └── lib.rs       # Core library exports and public API
│   └── Cargo.toml
├── tuner-gui/           # Iced-based GUI application
│   ├── examples/        # Standalone GUI tests and visual sandboxes
│   │   ├── shared/mod.rs         # Shared audio testing utility
│   │   ├── dashboard_test.rs     # Composite widget area integration test
│   │   ├── spectrogram_test.rs   # Isolated spectrogram widget test
│   │   └── cent_meter_test.rs    # Isolated cent meter widget test
│   ├── src/
│   │   ├── main.rs      # Binary entry point
│   │   ├── lib.rs       # Crate configuration
│   │   ├── app.rs       # Main application state and audio integration
│   │   ├── views.rs     # Views module declaration
│   │   ├── views/       # Layouts orchestrating multiple components
│   │   │   └── main_view.rs      # Main composed layout (widgets + sidebar)
│   │   ├── widgets.rs   # Widgets module declaration
│   │   └── widgets/     # Independent UI drawing components
│   │       ├── cent_meter.rs     # Cent deviation meter widget
│   │       ├── piano_keyboard.rs # Interactive piano keyboard
│   │       ├── spectrogram.rs    # Frequency spectrum visualization
│   │       └── partials_display.rs # Harmonic partials display
│   └── Cargo.toml
└── Cargo.toml           # Workspace configuration
```

### Global Data Structures & Memory Management

To maintain real-time performance without relying on OS priority elevation, the system completely avoids dynamic heap allocation (`std::thread::spawn`, `Vec::push`, etc.) during runtime by using pre-allocated, lock-free structures:

- **The Elastic Ring Buffer:** A massive lock-free circular buffer (e.g., 16,384 samples) connecting Thread 1 and Thread 2. It acts as an elastic shock absorber; if the OS briefly suspends the processing thread, the audio stream can still safely dump samples into the buffer without dropping data.
- **Lock-Free Object Pool:** A pre-allocated pool of fixed-size arrays (each large enough to hold 1.5 seconds of audio at 44,100 Hz). If capture mode is enabled, Thread 2 borrows an array to record a stable note and passes the reference to the background worker, which returns it to the pool when finished.

### Threading Model

#### Thread 1: The Audio Stream (The Harvester)

- **Role:** High-speed hardware ingestor.
- **Action:** Continuously captures raw audio from the microphone at 44,100 Hz. It immediately pushes all incoming samples directly into the Elastic Ring Buffer.
- **Rule:** This thread performs zero math, zero allocations, and does no analysis. Its only job is to guarantee pristine data throughput.

#### Thread 2: The Audio Processing Pipeline (The Brains)

This thread constantly consumes data from the Elastic Ring Buffer and executes a deterministic DSP pipeline to calculate the fundamental frequency ($f_0$).

- **The Gatekeeper (Signal Validator & 4-State Logic):** An always-running traffic cop monitoring the $f_0$ stream. It outputs a discrete `SignalState` to the UI, holding the last reliable note for 1.5 seconds to prevent visual flickering. When "Capture Mode" is enabled, it utilizes 4-state logic to control the 1.5-second capture window:
  - *State 1 & 2 (Attack/Transient):* Uses Complex Spectral Difference (CSD) to detect the broadband noise of the hammer strike, instituting a hard delay to ignore the chaotic transient.
  - *State 3 (Stability Gating):* Uses the NINOS2 (Normalized Identification of Note Onset based on Spectral Sparsity) metric to monitor the signal. Because NINOS2 measures structural spectral sparsity, it completely ignores the volume swells caused by unison beating, successfully identifying the "Golden Window" of pure harmonic decay.
  - *State 4 (Timeout):* Caps the capture at 1.5 seconds to prevent flat-lining pitch drift, closing the gate, recycling the buffer, and dispatching the payload to Thread 3.
- **The Scout:** Applies a Hann or Hamming window to a 2048-sample buffer to eliminate spectral leakage, then runs a Real FFT to find an accurate rough frequency neighborhood.
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
- **Action:** Consumes the high-speed stream of $f_0$ floats from Thread 2 to drive the instantaneous tuning visualizer (strobe, cents-deviation, or dial).
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
