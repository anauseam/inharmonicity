# Inharmonicity — a Professional Tuner for Inharmonic Stringed Instruments

![Inharmonicity Interface](images/interface-screenshot.png)

An open-source, professional-grade tuner built in Rust for **inharmonic stringed instruments**. With real-time audio analysis, pitch detection, and cent-deviation measurement, it measures each string's inharmonicity and drives a manual-mode strobe display for tuning each string to its own partial targets. The **current focus is the piano** for which it computes a per-instrument stretch curve; an instrument-agnostic ET reference mode already works, and the inharmonicity measurement and strobe **generalize to any stiff string**. Full-piano validation is ongoing, and other stiff stringed instruments will benefit from the work in the future.

For a detailed WIP overview of the algorithms used, see the [Anauseam documentation](https://docs.anauseam.org/project-docs/inharmonicity-tuner/00_intro).
For the design rationale and open observations, see [ARCHITECTURE.md](ARCHITECTURE.md). For contribution guidelines, see [CONTRIBUTING.md](CONTRIBUTING.md).

> [!IMPORTANT]
> **Project Status — pre-release, and usable**
>
> The full path a piano tuning needs — measure each string, compute the
> instrument's stretch curve, tune each string to it on a strobe — is built and
> works. No binary has shipped yet, and the tuner has not yet been validated by
> tuning a piano end to end, so treat it as a capable alpha rather than a
> finished product.
>
> Current limits:
>
> - **Manual mode only** — Auto-mode captures are excluded from the curve by design.
> - **The noise floor is re-measured every launch** (it belongs to the room, not
>   the instrument).
> - **No per-key inspector.** Undo reverts the last capture; a suspect
>   measurement cannot yet be reviewed or dropped, and the flags that mark one
>   are computed but not shown.
> - **Unisons are set by ear** — no unison-beat readout.
> - **No pitch-raise over-pull targets.**
> - **A440 only**, no user-adjustable temperaments.
>
> The full backlog, with what each item is blocked on, is in [TODO.md](TODO.md).

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

> [!NOTE]
> **Pre-built binaries are coming.** Building from source is the only route
> today. Now that the measure → curve → strobe path is complete, a tagged
> release with a compiled executable is planned so the tuner is usable without
> a Rust toolchain.

### Tuning a piano

Work in **Manual mode** — you name each key, which is the only provenance the
tuning curve accepts.

1. **Calibrate.** The app measures the room's noise floor for ~3 s at launch;
   keep quiet until `Calibrating…` clears.
2. **Select a key** on the *Key select* keyboard (clicking it again returns to
   automatic detection).
3. **Measure.** Enable *Measurement Mode*, press capture, strike the string.
   1.5 s of stable decay is captured and $f_0$/$B$ measured off-thread. *Undo*
   reverts the last capture.
4. **Watch the curve form** in the *Curve Plot* panel — unmeasured keys follow
   the model, so it settles as the compass fills in.
5. **Tune** in the *Strobe* panel: stationary band = in tune, direction = sharp
   or flat, and it falls back to a coarse cents number when the string is too
   far off to read. The curve locks on entry so targets cannot shift mid-pass.

Measurements autosave to the open instrument's profile as they land — there is
nothing to save, and the curve is recomputed rather than stored. The instrument
is named beside the title; **Settings → Instrument Library** switches or creates
one, so check the name before measuring a different instrument.

For a non-piano instrument, switch the sidebar's reference toggle to **Ref: ET**
for a pure equal-temperament strobe with no stretch curve.

> [!IMPORTANT]
> **Capture in Manual mode.** Automatic note discovery is still under
> validation, so captures taken in Auto mode are recorded as untrusted and are
> **excluded from the tuning curve** by design. A full compass captured in Auto
> yields a model-only curve with no measurements in it.

Every capture also writes its raw audio and analysis to a `diagnostics/` folder
in the per-user data directory (path printed at startup; not yet pruned). That
data exists for development — the format and the offline harnesses that read it
are documented in [`tuner-core/examples/README.md`](tuner-core/examples/README.md).

## Interface

An `iced` 0.14 GUI. Everything below is built and in the app today; planned
features live in [TODO.md](TODO.md).

### Features

- **Spectrogram Visualization**: Real-time frequency spectrum display
- **Cent Meter**: Visual tuning accuracy indicator with color-coded feedback
- **Piano Keyboard Select**: 88-key piano interface with click-to-select frequency functionality
- **Inharmonicity Measurement**: Capture and analyze piano-specific inharmonicity characteristics
- **Tuning Curve Display**: Live per-key stretch curve plot, recomputed off-thread as you capture
- **Manual-Mode Strobe**: Tune each string to its own per-partial curve targets — a fine phase band with an OS-CFAR coarse readout for out-of-range strings, per-key Smart-Partials selection, and a curve lock that freezes targets for a pass
- **ET Reference Mode**: Instrument-agnostic pure-equal-temperament strobe (no stretch curve) for non-piano use
- **Profile Management**: Per-instrument profiles, autosaved as you measure, with a searchable library and the last instrument reopened at launch
- **Transient Detection Calibration**: Manual and automatic transient detection calibration
- **Noise Floor Calibration**: Manual and automatic noise floor calibration
- **NINOS2 Stability Calibration**: Live oscilloscope with adjustable threshold for tuning the NINOS2 tonal stability gate

> [!TIP]
> **Graphics Issues? Check Your Vulkan Drivers**
>
> This application uses `iced` with the `wgpu` backend (Vulkan on Linux). If you experience
> invisible widgets, flickering, or blank panels, the most common cause is stale or
> incompatible Vulkan drivers. Ensure your GPU drivers are fully up-to-date before
> reporting rendering bugs.

See [tuner-gui](tuner-gui/README.md) for more information.

## Architecture

### Project Structure

```text
inharmonicity/
├── tuner-core/                     # Headless audio processing & analysis (no GUI code)
│   ├── src/
│   │   ├── algorithms/             # Stateless DSP building blocks
│   │   │   ├── spectral.rs         # FFT, Hann windowing, magnitude spectrum, CSPE + Jacobsen sub-bin estimators
│   │   │   ├── peaks.rs            # Spectral peak extraction (Jacobsen sub-bin) + SMS masking + OS-CFAR coarse read
│   │   │   ├── twm.rs              # Canonical Two-Way Mismatch (Primary F0 discovery)
│   │   │   ├── mat.rs              # Median-Adjustive Trajectories — faithful serial (f₀,B) estimator w/ CSPE
│   │   │   ├── metrics.rs          # RMS, EMA, NHWRSF, NINOS2 signal metrics
│   │   │   ├── discovery.rs        # Split discovery search (Stage A 88-key scan → Stage B refine)
│   │   │   ├── curves.rs           # Tuning-curve engines (a)–(d) — orchestrates the three leaves below
│   │   │   ├── rigaud.rs           # Rigaud parametric inharmonicity + tuning model (B_ξ fit, ρ_φ, F₀)
│   │   │   ├── giordano.rs         # Giordano sensory-dissonance octave-width recipe (Plomp–Levelt/Sethares)
│   │   │   └── whittaker.rs        # Whittaker smoother + shared banded LS solver
│   │   ├── cola.rs                 # CircularFifo — COLA circular FIFO for overlapping frame analysis
│   │   ├── models.rs               # Domain types: Note, Partial, KeyMeasurement, KeyProfile, InharmonicityProfile (schema v1)
│   │   ├── pipeline.rs             # AudioPipeline mediator — Dual-FFT unconditional execution
│   │   ├── engine.rs               # F0 Engine — TWM Discovery + M-of-N lock + Goertzel Phase Tracking
│   │   ├── strobe.rs               # Manual-mode strobe: fixed-reference beat phase + OS-CFAR coarse readout (pipeline tap)
│   │   ├── gatekeeper.rs           # 5-state signal validator (DSP only, no shared state)
│   │   ├── worker.rs               # Background worker for heavy offline DSP
│   │   ├── audio.rs                # CPAL audio capture, stream management, DC blocking
│   │   ├── synth.rs                # Offline additive resynthesis of a tuning curve → audio (cold-path, no audio stream)
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
    │       ↓         │       │   .worker_rx       ← recv    │ (WorkerOutput: measurements + curves)
    │ Engine (F0 DSP) │       │   .worker_job_tx   ← send    │ (WorkerJob: curve recomputes)
    │       ↓         │       │   .profiles        ← send    │ (template updates → DSP; gated)
    │ Strobe (tap)    │       │   .strobe_refs     ← send    │ (strobe references → DSP)
    │       ↓         │       └──────────────────────────────┘
    │ Capture Accum.  │              ↑ polls via Arc<Atomic*>
    │   (AudioPool)   │──────────────┘ triple_buffer (FrameOutput: viz + strobe angles)
    │       ↓         │
    └───────┬─────────┘
            │ crossbeam SPSC (CapturePayload)   [DSP → Worker]
            ▼
      Worker Thread   ◄─── crossbeam SPSC (WorkerJob: curve recompute)  [UI → Worker]
      ┌───────────────────┐
      │ High-res FFT+CSPE │ ← 65536-pt + shifted frame   (captures serviced first;
      │ MAT (f₀,B solver) │ ← partials + inharmonicity     curve recompute when idle)
      │ Curve engines a–d │ ← CurveBundle (cold, ~1.3 s)
      │ Diagnostics I/O   │ ← analysis.json + audio.raw
      └────────┬──────────┘
               │ crossbeam SPSC (WorkerOutput: measurements + curves) → Frontend
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
  - `worker_rx` — crossbeam SPSC receiver for `WorkerOutput` results from the Worker: `Measurement(KeyMeasurement)` per capture and `Curve(CurveBundle)` per curve recompute (one enum stream, so a new result kind is a variant, not a new channel)
  - `worker_job_tx` — crossbeam SPSC sender for `WorkerJob` background requests to the Worker (today curve recomputes; latest-wins). The GUI uses `HostHandle::send_curve_job` so the crossbeam types stay out of the frontend crate
  - `profiles` — `ringbuf` SPSC producer for pushing recompiled inharmonicity templates back to the live engine (UI → DSP; the measured-B discovery path is **gated off** by default — see [TODO.md](TODO.md))

  The single-owner endpoints (`worker_rx`, `worker_job_tx`, `profiles`, `strobe_refs`) cannot be cloned: `spawn_analysis_thread()` folds them, with the handle, into a `HostHandle`.

A frontend contributor just calls `AudioPipeline::new()`, gets a `PipelinePorts`, and never needs to know about Gatekeeper internals, EMA calculations, or lock management.

The pipeline also manages the **`WorkerManager`** (`worker.rs`), which owns a single dedicated background thread for computationally expensive offline DSP. When the pipeline's capture accumulator fills a 1.5-second buffer (or silence is detected), it dispatches a `CapturePayload` to the worker via a bounded crossbeam channel. The worker runs a high-resolution FFT and CSPE map, takes the note identity from the pipeline (the Engine's real-time discovery lock in Auto mode, or the user-selected key in Manual mode — it does not re-identify the note), and runs MAT to extract the partials and jointly refine the fundamental and the inharmonicity coefficient ($B$). The result is sent to the frontend via `worker_rx`, and the audio buffer is recycled back into the `AudioPool`. The same thread also serves **tuning-curve recomputes** on request (a `WorkerJob` from the UI → a `CurveBundle` back): the curve engines are cold-path but slow (Giordano engine (c) ~1.3 s), so they run here rather than on the GUI thread, with **captures always serviced first**. A single thread is sufficient because captures are infrequent (one stable note at a time), the algorithms complete before the next capture could arrive, and curve jobs are latest-wins (a burst of edits collapses to one recompute).

> [!NOTE]
> **Module Status**
>
> Every module below is built and running; the status reflects how settled each
> one is and whether more work is expected. All are `tuner-core` except
> `app.rs`, which is the GUI's state hub.
>
> | Module | Status |
> | --- | --- |
> | `pipeline.rs` — AudioPipeline orchestrator + shared state | 🟢 Stable |
> | `gatekeeper.rs` — 5-state signal validator (pure DSP) | ✅ Mature |
> | `engine.rs` — TWM discovery + Goertzel phase tracking | 🔬 R&D |
> | `worker.rs` — background (f₀, B) measurement (single thread) | 📐 Provisional |
> | `curves.rs` + `rigaud.rs`/`giordano.rs`/`whittaker.rs` — tuning-curve engines (a)–(d) | 📐 Provisional |
> | `strobe.rs` — manual-mode strobe (fixed-ref beat phase + OS-CFAR coarse readout, Path A) | 📐 Provisional |
> | `synth.rs` — offline curve → audio resynthesis (cold-path, no audio stream) | 📐 Provisional |
> | `peaks.rs` — Jacobsen sub-bin peak extraction + OS-CFAR coarse read | 🧩 Extensible |
> | `twm.rs` — canonical Two-Way Mismatch scoring | 🔬 R&D |
> | `app.rs` — GUI state hub | 🚧 In Development |
>
> **Legend** — ✅ **Mature**: solved, unlikely to change · 🟢 **Stable**: complete for its role · 📐 **Provisional**: functional now, but a specific required feature isn't built on it yet · 🧩 **Extensible**: works today, could grow, no commitment either way · 🚧 **In Development**: functional, more features actively coming · 🔬 **R&D**: algorithm still being developed and validated. (The two "done" tiers follow the PyPI convention where Mature ranks above Stable.)
>
> Reading the table: gating and orchestration are settled. The **curve layer**
> and the **strobe** are complete and wired together, and stay Provisional only
> until a piano has actually been tuned to a curve. **synth** is cold-path — it
> renders a curve to audio offline for the auralization A/B and owns no audio
> stream. The **Engine** and **twm** are the open research front: discovery
> locks reliably but the deep bass is inharmonicity-limited, gated on a second
> instrument ([ADR 0006](docs/adr/0006-discovery-refinement-validation.md)). What
> each module is still waiting on is in [TODO.md](TODO.md).

## License & Contact

This project is licensed under the terms specified in the LICENSE file.
