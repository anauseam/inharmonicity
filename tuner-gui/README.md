# tuner-gui

Iced-based GUI frontend for the inharmonicity tuner.

## Structure

├── examples/                   # Standalone visual sandboxes
│   ├── shared/mod.rs           # Shared audio testing utility
│   ├── dashboard_test.rs       # Composite widget area integration test
│   ├── spectrogram_test.rs     # Isolated spectrogram widget test
│   └── cent_meter_test.rs      # Isolated cent meter widget test
├── src/
│   ├── main.rs                 # Binary entry point
│   ├── lib.rs                  # Crate configuration
│   ├── app.rs                  # Application central state, dispatcher, and triple buffer receiver
│   ├── calibration.rs          # Wait-free noise floor & onset baseline calculation logic
│   ├── views.rs                # View module public exports
│   ├── views/                  # Layouts orchestrating domain-level components
│   │   ├── main_view.rs        # Main composed layout (widgets + sidebar)
│   │   ├── settings_view.rs    # Settings configuration tab router
│   │   ├── rms_calibration.rs  # Noise floor envelope and manual threshold slider
│   │   └── transient_calibration.rs # Live seismograph scope for onset capture
│   ├── widgets/                # Independent UI drawing components
│   │   ├── cent_meter.rs       # Cent deviation meter
│   │   ├── envelope.rs         # RMS envelope viewer (time-domain)
│   │   ├── partials_display.rs # Harmonic partials display
│   │   ├── piano_keyboard.rs   # Interactive 88-key piano
│   │   ├── seismograph.rs      # High-speed flux progression display
│   │   └── spectrogram.rs      # Frequency spectrum visualization
│   └── utils/                  # Shared UI helpers (sidebar, timers)
└── Cargo.toml

## Real-Time Settings & Calibration

Unlike traditional setups that span background audio streams exclusively for configuring thresholds, `tuner-gui` handles its physical calibration (noise floors and transients) natively in its front-end loop without allocating CPAL instances.

### How it works

1. **Wait-Free Dispatch System:** Standard `AudioPipeline` analytics (such as exponentially-smoothed RMS histories and Normalized Flux values) are naturally broadcast out to standard atomics inside `PipelineHandle.atomics.runtime`.
2. **Tick Countdown Measurement:** Instead of invoking a heavyweight multi-threaded workflow when hitting "Recalibrate", `calibration.rs` just triggers a continuous tick-polling routine during the 60 FPS update loop. The GUI natively monitors the baseline audio emissions coming directly from the triple buffer's mathematical nodes and applies strict constants on exact `WARMUP_FRAMES` arrays to derive true optimal floors.
3. **Memory Mapped Control:** Altering the transient configurations directly mutates `PipelineHandle.atomics.config`, creating an immediate wait-free parameter override in the active audio processor.

Because the DSP logic relies on decoupled configuration pipelines, the GUI controls real-time adjustments precisely without halting or refreshing active captures.
