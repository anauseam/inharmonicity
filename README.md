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

### Threading Model

- **GUI Thread**: Main Iced application thread handling user interface
- **Audio Thread (System)**: Real-time system audio lock-free extraction from capture device
- **Audio Processing Thread**: Dedicated worker polling audio data and running expensive DSP operations
- **Communication**: Lock-free asynchronous `ringbuf` transfers from audio to processing threads; standard `crossbeam` channels transfer `AnalysisResult` data back to the GUI
- **Real-time Processing**: Fast 5ms buffer polling loop providing fluid ~46ms GUI update intervals

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
