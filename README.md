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
│   │   │   ├── spectral.rs         # FFT, Hann windowing, magnitude spectrum, CSPE + Jacobsen sub-bin estimators
│   │   │   ├── peaks.rs            # Spectral peak extraction (Jacobsen sub-bin) + SMS masking
│   │   │   ├── twm.rs              # Canonical Two-Way Mismatch (Primary F0 discovery)
│   │   │   ├── mat.rs              # Median-Adjustive Trajectories — faithful serial (f₀,B) estimator w/ CSPE
│   │   │   ├── metrics.rs          # RMS, EMA, NHWRSF, NINOS2 signal metrics
│   │   │   ├── tuning.rs           # Cent deviation, inharmonicity-compensated frequencies
│   │   │   └── inharmonicity.rs    # Deprecated B-fit; reserved for the future inharmonic tuning curve
│   │   ├── cola.rs                 # CircularFifo — COLA circular FIFO for overlapping frame analysis
│   │   ├── models.rs               # Domain types: Note, Partial, KeyMeasurement, KeyProfile, profiles
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
AudioPipeline::new()  →  (AudioPipeline, PipelinePorts)
        │                         │
        ▼                         ▼
    Audio Thread              Frontend Thread
    ┌─────────────────┐       ┌──────────────────────────────┐
    │ Gatekeeper      │       │ PipelinePorts                │
    │   (5-state SM)  │       │   .handle.atomics  ← rw/ro   │ (config + runtime observations)
    │       ↓         │       │   .worker_rx       ← recv    │ (KeyMeasurement from Worker)
    │ Engine (F0 DSP) │       │   .profiles        ← send    │ (template updates → DSP; gated)
    │       ↓         │       └──────────────────────────────┘
    │ Capture Accum.  │              ↑ polls via Arc<Atomic*>
    │   (AudioPool)   │              │
    │       ↓         │──────────────┘ triple_buffer (FrameOutput)
    └───────┬─────────┘
            │ crossbeam SPSC (CapturePayload)
            ▼
      Worker Thread
      ┌───────────────────┐
      │ High-res FFT+CSPE │ ← 65536-pt + shifted frame
      │ MAT (f₀,B solver) │ ← partials + inharmonicity
      │ Diagnostics I/O   │ ← analysis.json + audio.raw
      └────────┬──────────┘
               │ crossbeam SPSC (KeyMeasurement) → Frontend
               │ returns buffer → AudioPool (recycled)
               ▼
```

The pipeline–GUI relationship follows the **Split / Handle pattern** (the same convention used by `crossbeam_channel`, `ringbuf::split()`, `std::thread::spawn`):

- **`AudioPipeline`** is moved to the audio thread. It owns the pure DSP components (`Gatekeeper`, `Engine`) and is the **only** thing that mutates the pipeline's internal state. After calling each DSP component's `process_frame()`, the pipeline reads their returned `GateResult` and syncs observations to the shared atomic state. It also manages inline capture accumulation — when `CaptureState` is `Recording`, it copies each hop's newest samples into a pooled buffer.
- **`PipelinePorts`** is kept by the frontend (everything `new()` hands back once the pipeline is moved to the audio thread). It bundles:
  - `handle` — a cloneable `PipelineHandle` carrying `Arc<PipelineAtomics>`:
    - `atomics.config` — wait-free reading and writing of configuration values (e.g., silence threshold, target key)
    - `atomics.runtime` — wait-free polling of runtime observations (e.g., smoothed RMS for the Envelope Viewer)
    - `atomics.capture_state` — baton-pass `AtomicU8` governing the capture lifecycle across three threads
  - `worker_rx` — crossbeam SPSC receiver for `KeyMeasurement` results from the Worker thread
  - `profiles` — `ringbuf` SPSC producer for pushing recompiled inharmonicity templates back to the live engine (UI → DSP; the measured-B discovery path is **gated off** by default — see [Project Work in Progress](#project-work-in-progress))

  The single-owner endpoints (`worker_rx`, `profiles`) are single-owned: `spawn_analysis_thread()` folds them, with the handle, into a `HostHandle`.

A frontend contributor just calls `AudioPipeline::new()`, gets a `PipelinePorts`, and never needs to know about Gatekeeper internals, EMA calculations, or lock management.

The pipeline also manages the **`WorkerManager`** (`worker.rs`), which owns a single dedicated background thread for computationally expensive offline DSP. When the pipeline's capture accumulator fills a 1.5-second buffer (or silence is detected), it dispatches a `CapturePayload` to the worker via a bounded crossbeam channel. The worker runs a high-resolution FFT and CSPE map, takes the note identity from the pipeline (the Engine's real-time discovery lock in Auto mode, or the user-selected key in Manual mode — it does not re-identify the note), and runs MAT to extract the partials and jointly refine the fundamental and the inharmonicity coefficient ($B$). The result is sent to the frontend via `worker_rx`, and the audio buffer is recycled back into the `AudioPool`. A single thread is sufficient because captures are infrequent (one stable note at a time) and the algorithms are fast enough to complete before the next capture could arrive.

> [!NOTE]
> **Module Status**
>
> Every `tuner-core` module is built and running; the status reflects how settled
> each one is and whether more work is expected.
>
> | Module | Status |
> | --- | --- |
> | `gatekeeper.rs` — 5-state signal validator (pure DSP) | ✅ Mature |
> | `pipeline.rs` — AudioPipeline orchestrator + shared state | 🟢 Stable |
> | `worker.rs` — background (f₀, B) measurement (single thread) | 📐 Provisional |
> | `peaks.rs` — Jacobsen sub-bin peak extraction | 🧩 Extensible |
> | `app.rs` — GUI state hub | 🚧 In Development |
> | `engine.rs` — TWM discovery + Goertzel phase tracking | 🔬 R&D |
> | `twm.rs` — canonical Two-Way Mismatch scoring | 🔬 R&D |
>
> **Legend** — ✅ **Mature**: solved, unlikely to change · 🟢 **Stable**: complete for its role · 📐 **Provisional**: functional now, but a specific required feature isn't built on it yet — v1 isn't done until it is · 🧩 **Extensible**: works today, could grow, no commitment either way · 🚧 **In Development**: functional, more features actively coming · 🔬 **R&D**: algorithm still being developed and validated. (The two "done" tiers follow the PyPI convention where Mature ranks above Stable.)
>
> Reading the table: the **Gatekeeper** (onset/stability detection) is essentially done. The **pipeline** is complete as the DSP orchestrator — the last structural piece is a hook to integrate the inharmonic-curve calculation once it exists. The **Worker** measures (f₀, B) today, but the curve solver it must also host is a required v1 feature, not yet built. **Peaks** works and hosts the codebase's one empirical gate (every attempt to replace it with a smarter detector has so far lost to it) — a future improvement, not a commitment. **app.rs** is functional, with more widgets to come (e.g. the strobe tuner). The **Engine** and **twm** are the open research front: automatic TWM discovery locks reliably but is not finished — bass is inharmonicity-limited and gated on a second instrument (see [`docs/adr/0006-discovery-refinement-validation.md`](docs/adr/0006-discovery-refinement-validation.md)).

## Iced 0.14.0 UI

### Features

- **Spectrogram Visualization**: Real-time frequency spectrum display
- **Cent Meter**: Visual tuning accuracy indicator with color-coded feedback
- **Piano Keyboard Select**: 88-key piano interface with click-to-select frequency functionality
- **Inharmonicity Measurement**: Capture and analyze piano-specific inharmonicity characteristics
- **Profile Management**: Save and load piano tuning profiles with JSON persistence
- **Transient Detection Calibration**: Manual and automatic transient detection calibration
- **Noise Floor Calibration**: Manual and automatic noise floor calibration
- **NINOS2 Stability Calibration**: Live oscilloscope with adjustable threshold for tuning the NINOS2 tonal stability gate

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

- **Dynamic Sample Rate**: The `Engine` now carries a `sample_rate` field (set from the resolved stream rate in `spawn_analysis_thread`) instead of hard-coding it. But the rate is not yet genuinely dynamic: the capture path still forces 44,100 Hz at the CPAL request (`open_input_stream`'s `with_sample_rate(SAMPLE_RATE)`, relying on the OS to resample), `CapturePayload` still passes a literal `44100` to the Worker, and the `AudioPool` buffer sizes, the COLA window, and the Gatekeeper timing constants are all dimensioned for 44.1 kHz. Enabling true dynamic-rate operation means dropping the forced request, plumbing the rate into `CapturePayload`, and deriving those sizes/timings from the negotiated rate. The `03-dsp-pipeline.md` guideline asks new code to read the rate from a single source of truth so this stays a single-point change.

### Engine TODOs

- **TWM Parameter Optimization — second-instrument validation**: The canonical Two-Way Mismatch constants ($q=1.4, r=0.5, \rho=0.33$) were calibrated by Maher & Beauchamp for general audio. A Multi-Objective Bayesian Optimization (MOBO) framework was built and run to retune them for piano inharmonicity against a generative synthetic dataset; the adopted `TwmConfig::default()` is the resulting conservative tuned set ($p=0.5, q=3.88, r=1.426, \rho=0.298, \lambda=18$), with the canonical values kept only as the math-regression guard. The open item is validation on a **second instrument** — the current results rest on one piano ($n=1$). Methodology: [`docs/adr/0001-mobo-tuning.md`](docs/adr/0001-mobo-tuning.md); findings and threats-to-validity: [`docs/adr/0006-discovery-refinement-validation.md`](docs/adr/0006-discovery-refinement-validation.md).
- **TWM Bass-Lock Bias (Sub-Harmonic Locks)**: Real-world testing shows the engine occasionally locks to extreme bass candidates (e.g. D#4 → A#0, F#3 → A0, D2 → E1). This is a fundamental TWM scoring problem: because A#0's harmonic series is so dense across the mid-range spectrum, many mid-range peaks accidentally align with its predicted partials, and the $f^{-0.5}$ weighting makes those alignments very cheap. While the excision of the Viterbi module removed the "stickiness" of these false locks, the fundamental scoring bias remains. MOBO retuning and several structural penalty terms (deadzone, Duan non-peak, Emiya smoothness) were tried and did not resolve it (ADR 0006): the residual is a *wrong-inharmonicity ($B$) template* problem in the bass — the fixed Rigaud prior mis-shapes the deep-bass templates — which scoring constants cannot fix. The lever is a per-key $B$ measurement applied only to its own key's template, gated on validation with a second, in-tune instrument.
- **Per-Bin Noise Floor Estimation**: The Neyman-Pearson Amplitude SNR gate relies on a global, broadband noise floor (`self.noise_floor`) calculated during silence. Real acoustic noise is colored (1/f pink noise, HVAC hum, soundboard modes). This global scalar can inadvertently pass noise at bass frequencies (where 1/f noise is loudest) or inappropriately kill real signals at treble frequencies. A future architectural fix should calculate and maintain a *per-bin* (or per-octave) noise baseline by actively tracking the Goertzel magnitude of tuning curve targets during Gatekeeper silence periods.
- **Inharmonic Curve Calculation**: A solver to calculate the optimal tuning stretch (inharmonic curve) for the entire piano so that the harmonics of the bass align with the fundamentals of the treble. It runs offline (worker/GUI side) over the persisted profile — the cross-thread profile transfer it depends on is already in place — so the remaining work is the solver itself: a faithful port of a published stretch / beat-minimization method, not yet built.
- **Measured-B Discovery Seeding (built, gated OFF)**: The pathway that seeds the live discovery templates with the Worker's measured per-key $B$ — over the sanctioned UI → DSP `ringbuf` channel (crossing #4) — is fully implemented but disabled by default via the `pipeline::APPLY_MEASURED_B_TO_DISCOVERY` flag. Real-capture validation on the one available instrument showed it *regresses* lock accuracy (the highest-ratio bass keys, with measured $B$ at 18–25× the prior, broke), consistent with the measured bass $B$ being over-estimated on an out-of-tune upright with no reference to check it against. Flip the flag to re-enable once a second, in-tune instrument validates the measurement. Full write-up: [`docs/adr/0006-discovery-refinement-validation.md`](docs/adr/0006-discovery-refinement-validation.md).

### Worker TODOs

- **MAT serial-vs-simultaneous on a second instrument**: The offline MAT solver (`worker.rs` → `mat.rs`) was rewritten as a faithful Median-Adjustive-Trajectories joint $(f_0, B)$ estimator with CSPE (Complex Spectral Phase Evolution) sub-bin refinement, validated offline by the new `validate_mat` CLI tool. The paper's serial trajectory growth is the shipped default; an all-partials "simultaneous" order is kept as a labeled fallback. A second, in-tune instrument is needed to confirm the serial order generalizes — at which point the simultaneous fallback is removed and the paper's tighter §2.4 peak-detection band can be re-enabled (it was fragile on the one out-of-tune upright).
- **CaptureState `compare_exchange`**: The current baton-pass relies on convention (each thread writes only its owned transitions). Switching to `compare_exchange` would enforce correct ordering at the atomic level and prevent a category of future bugs.

### GUI TODOs

- **Diagnostics Importer**: Build a mechanism in the GUI to directly import an offline-generated `analysis.json` into the `InharmonicityProfile`. This serves as a developer/diagnostic tool to allow us to easily load test outputs from our offline CLI harnesses into the GUI for visual inspection and graph plotting.
- **Persist calibration values into the profile**: The GUI calibration module computes the silence / noise-floor threshold, the NHWRSF onset threshold, and the NINOS2 stability threshold, but these currently live only in the runtime config atomics (`atomics.config.*`) and are lost on restart. Persist them into the saved profile (the GUI owns the profile — `tuner-core` is DSP-only) so loading a profile restores its calibration, not just its per-key $B$ measurements.

### Known Issues

- **Noisy Environments**: The engine uses a `-30 dB` relative amplitude threshold to filter sympathetic noise before TWM evaluation. If used in a noisy rehearsal environment where the Signal-to-Noise Ratio (SNR) drops below 30 dB, the noise will bleed past the mask and may cause stability issues.
- **MAT $f_0$ vs the tracked $f_0$**: MAT now jointly refines $(f_0, B)$ with CSPE sub-bin refinement and serial-growth (no more negative/impossible bass $B$). Its primary product is the per-key inharmonicity $B$; the Worker still reports the Goertzel-tracked $f_0$ as the key's `measured_f0` by design (the real-time tracker is the authority on pitch), with MAT's jointly-refined $f_0$ recorded in `analysis.json` for diagnostics. Final accuracy of $B$ still awaits validation on a second, in-tune instrument.
- **Stereo DC Blocker**: The `DcBlocker` currently uses a single state variable. If a stereo audio stream is ingested, the interleaved channels will corrupt the 1-pole IIR filter. The app either needs to explicitly force CPAL to Mono or support stereo state tracking.

### Follow-up Documentation & Verification

- **Review the `unsafe` byte-slice transmute in `worker.rs::write_diagnostics`.** Used to serialize the captured `f32` audio buffer to `audio.raw`. Functionally correct but worth reviewing whether `bytemuck` or a safe-wrapper crate could replace the raw `from_raw_parts` call without a performance regression.

### Architecture Goals (No ETA)

- **`models/` growth pattern**: The current `models.rs` is a single file. It will eventually become a `models/` directory with submodules (`models/note.rs`, `models/partial.rs`, …) once the single file becomes uncomfortable. There is no specific threshold beyond "when it stops feeling right." Documented in `04-algorithms-and-models.md`.
- **Shared DSP/stream constants module**: The cross-cutting DSP constants — `SAMPLE_RATE`, `WINDOW_SIZE`, `BASS_WINDOW_SIZE`, `HOP_SIZE`, `RING_BUFFER_CAPACITY` — currently live in `audio.rs` (the CPAL/hardware module), which everyone imports for them. Because `models::KeyProfile` needs `SAMPLE_RATE` to derive its Nyquist cutoff, the pure-data layer is forced to reach up into the hardware module (a small, accepted `models ↔ audio` cycle). Extract these constants into their own **descriptive leaf module** (a name better than `constants`/`dsp` — e.g. something that names the *role*) that `audio`, `models`, `engine`, and `pipeline` all import, so the data layer never depends on CPAL just to read a number. This is a deliberate full-codebase refactor (it touches every constant import), so it is intentionally *not* done via a re-export shim — left whole for a focused pass.
- **`ninos2` → `spectral_sparsity` rename**: The Gatekeeper's tonality gate is a spectral-sparsity ratio of our own design — the faithfulness audit ([`docs/audits/faithfulness-audit-05-metrics.md`](docs/audits/faithfulness-audit-05-metrics.md)) established it is *not* Mounir's NINOS² and relabeled its provenance — but the historical name persists across the metric function, the Gatekeeper config/telemetry fields, the `ninos2_calibration.rs` GUI view, `diagnose_gatekeeper`'s CSV headers, and `plot_gatekeeper.py`. Renaming is a mechanical ~11-file pass plus a historical-CSV header-compatibility decision, so it is deferred until the Gatekeeper is next reworked anyway.

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

Every capture will produce three files in a key-specific subdirectory within the `diagnostics/` folder (e.g., `diagnostics/key_001_A#0/`):

- `audio.raw`: The strictly causal, stable audio buffer that triggered the capture.
- `audio_full_event.raw`: The non-causal diagnostic buffer capturing ~348ms of pre-roll, the physical hammer strike, and the resulting decay.
- `analysis.json`: The analytical telemetry from the background worker.

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

To test the core DSP algorithms offline without launching the GUI, standalone Cargo examples are provided:

- **`diagnose_engine`**: A heavy-duty offline telemetry harness. It allows you to feed captured `.raw` audio files (`audio.raw` or `audio_full_event.raw`) back into the `tuner-core` engine to dump exactly what the STFT, peak extractor, and TWM algorithm observed at every frame into `spectrum.csv` and `peaks.csv`. When compiled with `--features telemetry`, it also writes `goertzel.csv` containing per-partial Goertzel amplitude and Neyman-Pearson threshold data for each tracking frame.

  ```bash
  cargo run --example diagnose_engine -- diagnostics/key_001_A#0/audio_full_event.raw
  ```

- **`validate_mat`**: Replays the Worker's MAT $(f_0, B)$ estimator over every `diagnostics/key_*/` capture and prints, per key, the measured inharmonicity vs. the Rigaud prior for **both** trajectory orders (serial and simultaneous) side by side, plus a ground-truth-free goodness-of-fit comparison. This is the offline tool used to validate the CSPE upgrade and to A/B the two MAT orders pending a second instrument.

  ```bash
  cargo run --release --example validate_mat
  ```

### License & Contact

This project is licensed under the terms specified in the LICENSE file.

For questions, suggestions, or collaboration opportunities, please contact [the team](mailto:contact@anauseam.org).
