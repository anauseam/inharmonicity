# Project Rules — Overview

These rules describe the conventions, hard constraints, and design decisions
that have actually solidified in `inharmonicity`. The project is fast-moving
and experimental; the rule set captures only what has stopped moving.

## What this project is

An open-source piano-tuning application written in Rust. The headless DSP
engine (`tuner-core`) performs real-time pitch detection plus async
inharmonicity measurement; the GUI (`tuner-gui`) is an `iced`-based
frontend. Long-form motivation lives in [README.md](../../README.md);
design rationale lives in [ARCHITECTURE.md](../../ARCHITECTURE.md); this
directory is the structural layer underneath.

## The threading model

```text
CPAL audio callback  ──ringbuf SPSC──►  Analysis thread (DSP)
                                          │
                                          │  CircularFifo (COLA)
                                          ▼
                                  AudioPipeline::process_cola_hop()
                                          │
                                          ├──► Gatekeeper  (signal validator)
                                          ├──► Engine      (TWM + Goertzel F0)
                                          ├──► Strobe      (fixed-ref beat phase)
                                          └──► capture accumulation
                                                  │
                                                  │  crossbeam SPSC (captures)
                                                  ▼
                                          Worker thread (async DSP)
                                                  │        ▲
                                                  │        │  crossbeam SPSC
                                                  │        │  (jobs: curve recompute)
                                                  ▼        │
                                          GUI thread (iced)
                                                  ▲
                                                  │  triple_buffer (FrameOutput)
```

Four threads, six sanctioned wait-free crossings. See
[02-cross-thread-communication.md](02-cross-thread-communication.md) for
the channel-by-channel contract.

## Hot path

The "hot path" is the set of code that runs on every audio sample or every
DSP hop. We keep it allocation-free and non-blocking to stay within the
realtime budget; see [03-dsp-pipeline.md](03-dsp-pipeline.md) for the
binding constraints.

- **Thread 1 (audio stream).** The CPAL input callback in `audio.rs`. DC
  blocking plus the ringbuf push.
- **Thread 2 (analysis).** The body of `spawn_analysis_thread` in
  `audio.rs`, which drains the ringbuf and calls
  `AudioPipeline::push_audio` → `AudioPipeline::process_cola_hop`.
  Everything reached transitively from `process_cola_hop` — Gatekeeper,
  Engine, and the `algorithms/*` functions invoked by them — is hot.

The **Worker thread** (`worker.rs`) and the **GUI thread** (`tuner-gui`)
run async to the hot path. They may heap-allocate freely.

## File map — `tuner-core/src/`

| Concern | File(s) |
| --- | --- |
| Crate root, `FrameOutput` | `lib.rs` |
| CPAL capture, DC blocking, audio thread | `audio.rs` |
| COLA overlapping-frame sliding window | `cola.rs` |
| Pipeline mediator, shared atomics, AudioPool | `pipeline.rs` |
| Signal validator (5-state) | `gatekeeper.rs` |
| F0 detection (TWM + Goertzel) | `engine.rs` |
| Strobe bank (fixed-reference beat phase) | `strobe.rs` |
| Async background worker, `CurveBundle` | `worker.rs` |
| Stateless DSP math | `algorithms/{spectral,peaks,twm,discovery,mat,metrics,curves,rigaud,giordano,whittaker}.rs` |
| Offline curve auralization (resynthesis) | `synth.rs` |
| Domain types and lookup tables | `models.rs` |
| Developer CLI tools & testing harnesses | `examples/` |

## File map — `tuner-gui/src/`

| Concern | File(s) |
| --- | --- |
| Application entry, state hub, message handling | `app.rs`, `main.rs` |
| View composition | `views/{main_view,settings_view,curve_select,rms_calibration,transient_calibration,ninos2_calibration}.rs` |
| Stateless widgets | `widgets/{cent_meter,curve_plot,envelope,partials_display,piano_keyboard,seismograph,spectrogram}.rs` |
| Calibration logic | `calibration.rs` |
| Shared view helpers | `utils/view_utils.rs` |

## Guidelines

```text
docs/internals/
├── 00-overview.md                       (this file)
├── 01-architecture.md                   crate boundaries, ownership, Split/Handle
├── 02-cross-thread-communication.md     the six wait-free crossings
├── 03-dsp-pipeline.md                   hot-path constraints
├── 04-algorithms-and-models.md          algorithms/ vs models/ layout
├── 05-style.md                          Rust style, allocation idioms
└── 06-capture-sets.md                   the validation data: what it is, how to consume it
```

```text
docs/adr/
├── 0001-mobo-tuning.md                        TWM parameter optimization methodology
├── 0002-twm-peak-masking-validation.md        First full-compass 8/8 pass validation
├── 0003-gatekeeper-rejection-of-sfm.md        Gatekeeper: SFM rejected as a signal gate
├── 0004-instrument-scope.md                   Instrument scope (inharmonic framework)
├── 0005-discovery-algorithm-class.md          Discovery class: peak-domain model scoring
├── 0006-discovery-refinement-validation.md    TWM calibration & validation (Draft, living)
├── 0007-tuning-curve-regularization-geometry.md  Curve engines: boundary reversion & gauge
├── 0008-giordano-layer-fidelity-derived-weights.md  Engine (c)/(d): derived weights, 1-SE rule
└── 0009-repeat-capture-noise-decomposition.md Repeat-capture σ model; ln-B shrinkage
```

Each file is self-contained, and section headings are stable enough to be
cited from outside.
