# tuner-gui

Iced-based GUI frontend for the inharmonicity tuner.

## Structure

├── examples/                   # Standalone visual sandboxes
│   ├── shared/mod.rs           # Shared audio testing utility
│   ├── dashboard_test.rs       # Composite widget area integration test
│   ├── spectrogram_test.rs     # Isolated spectrogram widget test
│   └── cent_meter_test.rs      # Isolated cent meter widget test
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
