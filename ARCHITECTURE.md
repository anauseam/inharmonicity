# Architecture

This document is the narrative companion to the [README](README.md) and
the structural guidelines under [`docs/internals/`](docs/internals/). The
README explains _what the project does_ and how to run it; the guidelines
describe _the binding conventions_ a contributor must follow.
This file sits between them — _why_ the system is shaped the way it
is, with the design tradeoffs written down so future contributors
don't have to reverse-engineer them.

## Why this exists

Real strings are not perfect harmonic oscillators. Non-zero stiffness
stretches their partials away from integer multiples of the fundamental
by an inharmonicity coefficient $B$ — present on any stringed instrument
(a guitar included), and most dramatic on a piano, where it is small in
the mid-treble and very large in the bass and high treble. On a piano
the resulting "stretch curve" is what gives a properly tuned instrument
its rich, locked-in sound. Equal-temperament chromatic tuners ignore $B$
entirely — they assume partials line up cleanly on harmonic ratios — so
tuning a piano with one produces an instrument that is mathematically in
tune and musically wrong.

`inharmonicity` treats $B$ as a first-class measurement. It captures a
~1.5-second sample of each struck string, extracts the partials, fits a
stretched-string model, and records the per-key $B$ in an
`InharmonicityProfile` that drives the tuning display. The measurement,
the strobe, and an ET reference mode (pure equal temperament, no stretch
curve) are instrument-agnostic, but the **current focus is the piano**, where the
inharmonicity-compensated tuning curve (a piano-specific model: Rigaud
dual-bridge $B_\xi$, octave types, Railsback stretch across the 88-key
compass) matters most, and where all discovery/TWM validation has been
done. The result is a tuner that produces a musically correct instrument
rather than a chromatically correct one.

## System Overview

The system is split into two crates and four threads. The crate split
is a hard architectural constraint (see
[01-architecture.md](docs/internals/01-architecture.md)); the thread
layout is a consequence of needing to keep audio capture, real-time
DSP, heavy asynchronous DSP, and rendering all isolated from each other.

### Global Data Structures & Memory Management

To maintain real-time performance without relying on OS priority elevation, the core system completely avoids dynamic heap allocation during the audio hot-path by using pre-allocated, lock-free structures:

- **The Elastic Ring Buffer:** A lock-free circular buffer connecting Thread 1 and Thread 2. Acts as an elastic shock absorber — if the OS briefly suspends the processing thread, audio samples continue to accumulate safely without drops.
- **Lock-Free Object Pool (`AudioPool`):** Pre-allocated pool of `Box<[f32; 66150]>` arrays (1.5 seconds at 44.1 kHz). Thread 2 borrows an array to record a stable note and passes it to the background worker, which recycles it back to the pool when finished.
- **`ProcessingFrame`:** Thread-local scratch buffers for zero-allocation per-frame DSP. All fields are `Box<[T]>` — allocated once in `AudioPipeline::new()` via `vec![..].into_boxed_slice()`, never resized. Includes dedicated `treble_magnitude_buffer` (1024 bins) and `bass_magnitude_buffer` (4096 bins) for the Dual-Track FFT paths. The Engine reads from these directly — no per-frame heap allocation in the correlation + MAT chain.
- **`CircularFifo` (COLA):** Owned by `AudioPipeline`. A `Box<[f32]>` ring buffer that accumulates samples and triggers a new FFT + pipeline frame on every 50% hop. Invisible to `tuner-gui` — the GUI only calls `pipeline.push_audio(&[f32])`.

### Threading Model

The four threads:

#### Thread 1: The Audio Stream

This thread is the high-speed hardware ingestor and signal conditioner.

- **Action:** Continuously captures raw audio from the microphone at 44,100 Hz. Each sample passes through a `DcBlocker` (single-pole high-pass IIR, α = 0.995 — a **35 Hz** corner, `(1−α)·fs/2π`, which sits above A0 and costs −4.2 dB of its fundamental; measured as the right trade against sub-35 Hz rumble, see the Known Issues entry) to remove hardware-dependent DC offset, then is pushed into the Elastic Ring Buffer. This guarantees every downstream consumer sees a zero-mean signal regardless of microphone, audio interface, or OS driver.
- **Rule:** This thread performs zero allocations and no analysis. The DC blocker is the only computation — one multiply and two additions per sample — and is classified as signal _conditioning_, not signal analysis. Its job is to guarantee pristine, zero-mean data throughput.

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
    │ [0] IDLE (Silence Gate) │ is_new_onset    │ [A] Discovery Phase     │
    │ [1] ATTACK (NHWRSF Flux)│ ──────────────▶│     (Canonical TWM)     │
    │ [2] TRANSIENT (Wait)    │                 │                         │
    │ [3] STABILITY (NINOS2)  │                 │ [B] Tracking Phase      │
    │ [4] RELEASE             │                 │     (Goertzel Phase     │
    └────────────┬────────────┘                 │         Vocoder)        │
                 │                              └────────────┬────────────┘
                 │                              ┌────────────▼────────────┐
                 │                              │ Strobe  (tap, step 5b)  │
                 │                              │  reads hop + curve refs │
                 │                              │  adds strobe fields     │
                 │                              └────────────┬────────────┘
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
    │  High-Res FFT → CSPE map →   │
    │  MAT → (f₀, B) calc          │
    └─────────────┬────────────────┘
                  │ crossbeam SPSC (KeyMeasurement)
                  ▼
              GUI (Thread 4)
```

- **The Gatekeeper (Signal Validator & 5-State Logic):** An always-running traffic cop monitoring the signal envelope. It evaluates stability via a `GateResult` return value (replacing the old direct-field-read pattern), executing a 5-stage state machine. The pipeline reads the Gatekeeper's `SignalState` and uses it to drive capture accumulation:
  - _State 0 (IDLE / Silence Gating):_ Uses a dynamic RMS baseline with Exponential Moving Average (EMA) to bypass heavy DSP during periods of noise or silence.
  - _State 1 (ATTACK):_ Uses Normalized Half-Wave Rectified Spectral Flux (NHWRSF) to detect hammer strikes. Sends onset pulse to the Engine to begin pitch detection.
  - _State 2 (TRANSIENT):_ A one-frame buffer state that resolves the `transient_active` flag once NHWRSF drops back below its threshold. Because the entire hammer-string transient is shorter than the 46.4ms FFT window, physical recovery happens rapidly. State 2 serves purely to allow a clean NINOS2 entry on the subsequent frame.
  - _State 3 (HARMONIC DECAY):_ Uses NINOS2 (Normalized Identification of Note Onset based on Spectral Sparsity) to monitor the signal. After the State 2 delay clears, NINOS2 ignores volume swells and identifies the "Golden Window" of pure, stable harmonic decay for capture. It enforces a secondary `required_stable_frames` threshold (e.g. 4 frames) to gracefully bridge any remaining chaos that the fixed delay missed.
  - _State 4 (RELEASE):_ Caps the capture at 1.5 seconds. The pipeline detects completion (buffer full or silence decay), dispatches the `CapturePayload` to Thread 3 via a bounded crossbeam channel, and transitions `CaptureState` to `Processing` — all without blocking the real-time pipeline.
- **The Engine (TWM Discovery + Goertzel Phase Tracking):** A pitch detection chain that operates as an independent state machine, **synchronously reset** by the Gatekeeper's onset pulse but otherwise decoupled from the Gatekeeper's internal transient delays.
  - **Discovery Phase (State: Unlocked):** Identifies the fundamental frequency from the 8192-pt bass FFT buffer using the canonical Maher & Beauchamp (1994) Two-Way Mismatch algorithm.
    1. **Peak Extraction:** Sub-bin peaks are extracted using the Jacobsen complex-domain estimator (Candan 2015). To establish a statistical minimum magnitude for Additive White Gaussian Noise (AWGN) rejection, a dynamic Neyman-Pearson threshold (Kay 1998) is computed against the pipeline's dynamic noise floor and acts as a floor gate. _(Note: Because the piano's acoustic noise floor during an active note is vastly higher than the room's silence threshold, this AWGN boundary is mathematically sound but practically negligible in effect)._
    2. **Peak Masking:** A two-stage masking process is applied: first, a `-30 dB` relative global magnitude floor (an adaptation of Cano 1998's 40 dB rule) removes absolute structural noise, followed by our own proportional critical-band masking heuristic — empirically validated in ADR 0002, not a port of a published method — to aggressively drop sympathetic tonal noise and structural intermodulation distortion.
    3. **TWM Scoring:** The surviving peaks are scored against 88 pre-computed inharmonicity-stretched `KeyProfile` arrays. The algorithm evaluates both forward error and reverse error with psychoacoustic frequency weighting ($f^{-0.5}$). To prevent unbounded error accumulation from distant noise, the Measured-to-Predicted error is topologically bounded using a piecewise ceiling derived from Duan et al. (2010).
    4. **Temporal Tracking:** The lock is confirmed by **M-of-N binary integration** over the per-frame discovery winner on _stable_ frames — the first key to win ≥ M of the last N stable frames latches (refined default 7-of-8; Schwartz 1956 / Shnidman 1998, validated on two instruments in ADR 0010). This is a bounded, allocation-free window that outlasts the attack transient a first-to-win race could otherwise lock onto; confirmation transitions the Engine to the Tracking Phase. (Note: The historic Viterbi hidden Markov model was permanently excised because its path cost persistence caused unrecoverable sub-harmonic locks following noisy hammer strikes).
  - **Tracking Phase (State: Locked):** Once a key is locked, the engine switches to per-partial Goertzel analysis on 1024-sample segments to refine the tuning measurement.
    1. **Phase Vocoder:** Phase differences between consecutive hops are unwrapped to yield instantaneous frequency estimates (McAulay & Quatieri 1986).
    2. **Amplitude SNR Gate:** A Neyman-Pearson threshold (derived from a Generalized Likelihood Ratio Test) compares the Goertzel unnormalized magnitude against the noise floor. Partials that fail this threshold are rejected as noise.
    3. **Adaptive Seed Feedback:** For partials that survive the SNR gate, an Exponential Moving Average ($\alpha = 0.05$) adapts the theoretical Goertzel tracking seed toward the measured instantaneous frequency (Dolson 1986), ensuring the tracker stays locked onto detuned strings without losing coherent integration energy.
    4. **f0 Reconstruction:** The engine exclusively uses Partial 1 to drive the primary Cent Meter. This intentionally avoids averaging higher partials, which carry an $n^2$ systematic cents error when the theoretical inharmonicity profile ($B_{profile}$) diverges from the physical string ($B_{true}$).
- **Output:** Pushes a `FrameOutput` structure every hop, containing the treble magnitude spectrum, sub-cent accurate $f_0$, and real-time partial frequencies to the UI thread via a wait-free `triple_buffer`.

Once the Gatekeeper detects silence, it closes the gate by sending the `is_silence` flag to the Engine to force an immediate state reset and prevent pitch detection from running on background noise.

The **Strobe** is drawn in its hop position (step 5b, after the Engine) but is a parallel _tap_, not a stage: it reads the hop's audio plus the UI-pushed curve references, and it and the Engine write `FrameOutput` independently. It consumes nothing from the Engine and nothing downstream consumes it — removing it leaves gating, detection, and measurement bit-identical (the authoritative per-hop step list is [`docs/internals/03-dsp-pipeline.md`](docs/internals/03-dsp-pipeline.md); the stage-vs-tap rule is [`01-architecture.md`](docs/internals/01-architecture.md)).

#### Thread 3: The Background Worker

This is a single detached worker thread spawned at pipeline construction inside `AudioPipeline::new()`. It blocks on a `select!` over two inputs — capture buffers from the DSP thread and background jobs from the UI — waking when either arrives. **Captures are serviced first** (measurement latency is user-facing mid-session); background jobs run only when no capture is pending.

- **Action (capture):** When the pipeline dispatches a filled capture buffer, the worker:
  1. Performs a high-resolution power-of-two FFT on the captured audio (up to 65,536 points), plus a one-sample-shifted frame, and derives a CSPE (Short & Garcia 2006) super-resolution per-bin frequency map.
  2. Takes the note **identity** from the payload — it does not re-identify the note. In **Auto Mode** that identity is the Engine's real-time TWM discovery lock (`latched_auto_key`); in **Manual Mode** it is the user-selected key.
  3. Seeds from the Engine's Goertzel-tracked _f₀_ (or the key's Equal-Temperament frequency if the tracker never locked).
  4. Runs MAT (Median-Adjustive Trajectories — serial trajectory growth, reading partial frequencies from the CSPE map) to jointly estimate the partials, the refined _f₀_, and the inharmonicity coefficient ($B$) via the median of pairwise partial combinations.
  5. Writes diagnostic files (`audio.raw` + `analysis.json`) to the `diagnostics/` directory.
- **Action (curve job):** When the UI requests a tuning-curve recompute (`WorkerJob::Curve`, carrying a trust-filtered `CurveInput` snapshot), the worker runs all curve engines and returns a `CurveBundle`. This lives on the worker because engine (c)'s Giordano dissonance scans alone take ~1.3 s — far too slow for the GUI thread. Jobs are latest-wins (a `generation` counter drops superseded bundles); the read-only snapshot means a curve recompute can never race the profile a `KeyMeasurement` writes.
- **Output:** Sends a `WorkerOutput` back to the GUI over one shared result channel — `Measurement(KeyMeasurement)` (partials, $f_0$, $B$) per capture, `Curve(Box<CurveBundle>)` per recompute. After a capture it resets `CaptureState` to `Idle` and recycles the audio buffer into the `AudioPool`; curve jobs touch neither the baton nor the pool.

#### Thread 4: The UI Thread (The Visual Renderer)

This is the graphical interface thread operating at 60 FPS.

- **Action:** Consumes the high-speed stream of `FrameOutput` structures from Thread 2 via the `triple_buffer` to drive the instantaneous tuning visualizers (spectrogram, cents-deviation, keyboard). Drains `WorkerOutput` results from the Worker via the `worker_rx` receiver: a `Measurement` is **appended** to its key's list in the `InharmonicityProfile`, which then auto-saves — appending rather than replacing is what stops an unattended Auto-mode capture displacing a trusted one, since `active()` reads the newest _trusted_ entry (and, when measured-B discovery seeding is enabled, the recompiled template goes back to the live engine via the `profiles` producer — crossing #4); a `Curve` bundle is stashed in UI state to drive the curve display. On any trusted-set edit (capture merge, undo, profile load) it enqueues a `WorkerJob::Curve` for the Worker to recompute the curve off-thread (recompute-on-load; the curve is never persisted). Reads/writes configuration (e.g., silence threshold, target key) and polls runtime observations (e.g., smoothed RMS for the Envelope Viewer) via `Arc<PipelineAtomics>`.

#### Cross-Thread Communication Topology

Because `tuner-core` enforces strict zero-allocation, wait-free real-time audio constraints, it relies on a rigidly defined topology for inter-thread message passing:

| Pathway               | Primitive                | Direction                     | Purpose                                                                                                                                         |
| --------------------- | ------------------------ | ----------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| **Hardware Capture**  | `ringbuf` SPSC           | Stream (1) → DSP (2)          | Lossless elastic buffer for incoming raw audio.                                                                                                 |
| **Structural Output** | `triple_buffer`          | DSP (2) → UI (4)              | Lossy continuous viz telemetry (`FrameOutput`).                                                                                                 |
| **DSP Parameters**    | `Arc<Atomic*>`           | UI (4) ↔ DSP (2)              | Wait-free configuration and metric reads/writes.                                                                                                |
| **Capture Dispatch**  | crossbeam SPSC (bounded) | DSP (2) → Worker (3)          | `CapturePayload` containing pooled audio buffer + metadata.                                                                                     |
| **Buffer Recycling**  | Lock-Free Object Pool    | DSP (2) ↔ Worker (3)          | Recycled `Box<[f32; 66150]>` arrays — zero allocation during capture.                                                                           |
| **Capture Lifecycle** | `AtomicU8` (baton-pass)  | UI (4) → DSP (2) → Worker (3) | `CaptureState`: Idle → Armed → Recording → Processing → Idle.                                                                                   |
| **Worker Results**    | crossbeam SPSC (bounded) | Worker (3) → UI (4)           | `WorkerOutput`: `Measurement(KeyMeasurement)` per capture, `Curve(CurveBundle)` per recompute.                                                  |
| **Worker Jobs**       | crossbeam SPSC (bounded) | UI (4) → Worker (3)           | `WorkerJob`: curve-recompute requests (`CurveInput` snapshot, latest-wins).                                                                     |
| **Template Update**   | `ringbuf` SPSC           | UI (4) → DSP (2)              | Recompiled `KeyProfile` (measured $B$) into the engine's templates.                                                                             |
| **Strobe References** | `ringbuf` SPSC           | UI (4) → DSP (2)              | `StrobeRefUpdate` (curve targets + coarse partial) into the `Strobe`. Second instance of the Template-Update crossing class, not a new channel. |

The channel-by-channel contract is documented in
[02-cross-thread-communication.md](docs/internals/02-cross-thread-communication.md).
What's important here is that the choices are deliberate: each one
sidesteps a category of latency or correctness problem (OS mutexes,
allocator contention, lossy MPMC fallback) that would compromise the
real-time guarantee.

## DSP Philosophy: Analytical Algorithms

Piano acoustics (inharmonicity, phantom partials, beating unisons) are complex but well-documented in the literature. Rather than inventing custom heuristics or magic-number thresholds to handle edge cases, the pipeline relies on established, peer-reviewed math.

For example, to handle spectral peaks distorted by beating unisons, it's tempting to write a custom heuristic that measures lobe asymmetry and throws out bad peaks. Instead, we lean on the math: the Hodgkinson (2009) MAT algorithm naturally discards those bad measurements by taking the median of all paired inharmonicity coefficients.

**The Topological Scrutiny Test:** If a heuristic or empirical constant _must_ be introduced, it must define or alter the geometric shape of the information (e.g., scale-invariant frequency ratios or error curve exponents) rather than acting as a fragile, environment-dependent threshold. See `docs/internals/04-algorithms-and-models.md` for the full standard.

The major DSP components and their foundations:

- **Transient Stability Detection**: `ninos2` — a spectral-sparsity ratio of our own design (an $N/N_{\text{eff}}$ participation-ratio form; _not_ Mounir 2021's NINOS², per faithfulness-audit-05)
- **Onset Detection**: Normalized Half-Wave Rectified Spectral Flux (NHWRSF) — via spectral difference
- **Peak Extraction** (Discovery): Candan (2015) Jacobsen complex-domain estimator
- **Note Discovery**: Maher & Beauchamp (1994) Two-Way Mismatch
- **Sympathetic Noise Rejection**: `mask_peaks` — our own critical-band masking heuristic (empirically validated in ADR 0002; the global magnitude gate adapts Cano 1998), with a Duan et al. (2010) topological ceiling on the reverse TWM error term
- **Inharmonicity**: Hodgkinson (2009) Median-Adjustive Trajectories (MAT) — serial trajectory growth, with Short & Garcia (2006) Complex Spectral Phase Evolution (CSPE) sub-bin refinement
- **Tuning curve** (cold path): Rigaud, David & Daudet (2013) parametric inharmonicity-and-tuning model — the $B_\xi$ fit and the $\rho_\varphi$ octave-type curve; Giordano (2015) sensory-dissonance octave-width recipe (Plomp–Levelt roughness in the Sethares parametrization) as the perceptual layer; Whittaker (1923) / Eilers (2003) smoother for the per-key residual
- **Strobe display** (`strobe.rs`, a pipeline _tap_, not a chain stage): a fixed-reference Goertzel bank that accumulates per-partial beat phase against the curve targets on the DSP thread — a software strobe (Goertzel 1958 finalization giving exact hop-to-hop phase, audit-08), deep-bass references on a 4096-sample window (R3)
- **Coarse spectral readout** (`peaks::coarse_read`, folded into the strobe): a bounded, ordered-statistic CFAR-gated magnitude search at the nominated reference partial — the strobe's out-of-range fallback when the phase band aliases (ADR 0011)

Each of those is one file named for the one method it implements (`rigaud.rs`, `giordano.rs`, `whittaker.rs`), composed by `curves.rs` — the same shape `discovery.rs` has over `twm.rs`, and for the same reason: a cited method stays pure and auditable in its own file, and the orchestrator is where they are combined.

If the pipeline produces bad data, the fix is usually to implement the mathematically complete version of the algorithm rather than adding a clamp or a safety bound.

## Design decisions

A handful of decisions look arbitrary in isolation but were chosen
for specific reasons. They're recorded here so they don't get
re-litigated.

### Dual-Track FFT (2048 treble / 8192 bass) instead of one large FFT

Piano partials are sparse and inharmonically stretched. A single very
large FFT gives good bass resolution at the cost of a slow update
rate, which is unpleasant for tuning the treble where the perceived
target is changing in real time. A short FFT gives a snappy treble
response but cannot resolve closely spaced bass partials.

The pipeline runs both unconditionally: a short window for the
high-resolution-in-time treble path, and a long window for the
high-resolution-in-frequency bass path. The Engine routes between
them based on the candidate fundamental, so the chosen spectrum
always matches the region it's analysing. The cost is roughly twice
the FFT work per hop, which is well within the realtime budget on
modern hardware.

### TWM superseded the matched-filter / `templates.rs` approach

The early design used per-key sparse matched filters (the
`templates.rs` module, now retired) for $f_0$ discovery: a
pre-computed inharmonicity-stretched template per key, scored against
the live spectrum. This worked but it was rigid — the templates
needed re-baking whenever the stretched-partial model changed, and
the scoring function fought back against the long tail of weak high
partials.

The canonical Maher & Beauchamp (1994) Two-Way Mismatch algorithm
solved both problems. It scores measured-vs-predicted and
predicted-vs-measured in a single closed form with psychoacoustic
weighting; it doesn't need pre-baked templates beyond the key
profile; and the geometric and temporal consistency gates produce
markedly fewer false locks. `twm.rs` replaced the old matcher in the
Engine, and `templates.rs` was removed.

### Heavy DSP runs off-thread in the Worker

MAT, the high-resolution FFT, and the $B$ coefficient calculation are
expensive enough that running them inline on the analysis thread
would routinely exceed the realtime budget — especially during a
capture, which is exactly when the user expects the live display to
stay smooth. They are dispatched to a single dedicated worker thread
via crossbeam SPSC, processed asynchronously, and returned to the
GUI as a `KeyMeasurement`.

A single worker thread is sufficient because captures are
deliberately serialised at the source: a stable note is held for
1.5 s before the next one can be captured, and the worker finishes
well within that window. Adding more worker threads would buy
nothing and would complicate buffer recycling.

### `AtomicU8` baton-pass instead of a channel for `CaptureState`

`CaptureState` is read by three threads (GUI, DSP, Worker) and
written by all three at different points in its lifecycle. A
channel would require either a fan-out fabric (multiple receivers
need the same value) or polling discipline (each reader handles a
backlog). A single `AtomicU8` whose transitions are partitioned by
convention turned out to be much simpler: each thread writes only
the transitions it owns, and reads are cheap.

The current implementation is convention-only (plain `.store`/
`.load` with `Ordering::Relaxed`); hardening it to
`compare_exchange` is tracked as a follow-up in the README's
Work-in-Progress section.

### Hardcoded 44.1kHz Sample Rate Architecture

The pipeline is statically locked to a 44,100 Hz sample rate. This is not just a surface-level parameter; it is deeply baked into the zero-allocation memory layout and temporal math of the DSP chain:

- **Static Buffer Sizes**: The `AudioPool` uses fixed `[f32; 66150]` arrays exactly dimensioned for 1.5 seconds at 44.1kHz.
- **Time/Frame Conversions**: Gatekeeper threshold frames (e.g., `transient_timeout_frames = 20`) mathematically assume a ~46.4ms hop.
- **Math Constants**: Parameters like EMA smoothing alphas (`rms_ema_alpha = 0.1`) and the Gatekeeper's timing thresholds are calibrated against 44.1kHz timing and frequency bin widths.

Attempting to change the sample rate dynamically would require migrating away from fixed-size arrays to dynamic allocations on the audio hot-path, breaking the core real-time guarantees. While we currently rely on the host OS audio daemon (e.g., PipeWire or CoreAudio) to resample native hardware inputs down to 44.1kHz, this is strictly a temporary stopgap. Full dynamic sample rate support is planned for the future, but it requires a complex architectural overhaul; shipping a robust, working pipeline at a fixed rate remains the immediate priority.

### The DC blocker's corner sits at 35 Hz, above A0

The input conditioner is a one-pole high-pass with α = 0.995, whose −3 dB corner
is `(1−α)·fs/2π` ≈ **35 Hz**. That is _above_ A0's 27.5 Hz fundamental, which it
attenuates by 4.2 dB (3.3 dB at C1, 1.5 at A1, 0.4 by A2). For a tuner that
sets out to capture the whole bass register that looks wrong, so it was measured
rather than argued, and the corner is kept.

- **Restoring a 3.5 Hz corner (α = 0.9995) buys nothing and costs accuracy.** The
  bass fundamental gains 2–4 dB but remains 24–41 dB below the note's strongest
  partial — still under the −30 dB masking gate on the same 7 of 9 bass keys, so
  discovery sees no new partials. The missing bass fundamental is acoustic, not
  filter-induced. Meanwhile the CFAR reference cells that set the coarse readout's
  local noise estimate include this band (its deep-bass lower flank clamps at bin
  1), so the threshold rises 1–3 dB while the read's own reference partial at
  110 Hz gains nothing: measured, coarse availability falls 93.3 % → 87.4 % and
  error worsens 0.70 ¢ → 1.85 ¢.
- **A steeper filter is the better lever, and still not worth it.** Order — not α
  — is the axis that escapes the trade: a 3rd-order Butterworth at 25 Hz recovers
  2.4 dB at A0 while admitting slightly _less_ rumble, for 9 µs per 23 ms callback
  (0.04 % of one core), which is affordable. It was rejected on outcome: MAT's
  measured `B` moves by a median of **0.00 %** across 87 keys, and the coarse read
  by +0.3 points of availability. MAT tracks 30+ partials and the deep-bass
  fundamental was never in its fit, so the filter only shapes spectrum the
  estimator already ignores.

**What would reopen it.** A consumer that actually uses the bottom octave's
fundamental — the per-bin/per-octave noise floor under the README's Engine TODOs,
or an instrument that genuinely radiates it (both validation pianos are uprights,
the weak case). If that happens, change the **order**, not α, and note three
things: cascaded biquads are needed rather than one pole; conditioning at
`fc/fs ≈ 5.7e-4` puts the poles at radius ≈ 0.9965, where f32 direct-form I is
marginal (use transposed direct-form II or f64 state); and more filter state
multiplies the single-state stereo defect in the README's Known Issues.

Re-validating any change is possible **without re-recording**: the one-pole
inverts exactly (`x[n] = y[n] + x[n−1] − α·y[n−1]`, round-tripping to 1e-15
relative in f64 on real captures), so a candidate filter can be applied to the
existing capture sets by inverting this one first.

### The `synth` module is cold-path (curve auralization, no audio-out stream)

`tuner_core::synth` renders a computed `TuningCurve` to audio by **offline additive resynthesis** — placing each key's measured partials at the curve's target frequencies and summing them. Its purpose is _auralization_: hearing how a candidate stretch sounds before tuning a piano to it, since there is no ground-truth-free "best" curve (octave, fifth, and twelfth beats are mutually incompatible objectives — it is a listening judgment). Today the `auralize` example drives it to render a loudness-matched A/B set of WAVs.

This module is deliberately **not part of the real-time system**. It runs on none of the four threads above, touches no shared pipeline state, allocates freely, and owns **no audio stream** — it returns a `Vec<f32>` (or writes a WAV) and hands the level policy to the caller. It sits alongside the cold-path curve math, not the hot path; the four-thread model and the zero-allocation invariants are unaffected by it.

Playback through a speaker — the future GUI "hear the curve" feature — is a **separate, deferred** piece. When it is built, the audio **output** stream will live in `tuner_core::audio` (the single CPAL boundary), **not** in the GUI: `tuner-core` is headless and the GUI speaks only the six channels above. That stream is the mirror image of the capture crossing — a CPAL output callback (the real-time _consumer_) fills a `&mut [f32]` from a lock-free ring buffer whose _producer_ is the cold synth — so it is a documented **seventh cross-thread crossing**, exposed as an opt-in handle like `spawn_analysis_thread`, subject to the same wait-free/no-allocation callback discipline as the input path. Duplex (playing synthesized notes while the tuner is listening) is intentionally out of scope; capture and playback never run at once.

## Pointers

| Doc                                | Purpose                                                    |
| ---------------------------------- | ---------------------------------------------------------- |
| [README.md](README.md)             | What the project is, what it does, how to build and run it |
| [docs/internals/](docs/internals/) | Structural guidelines and conventions                      |
| [docs/adr/](docs/adr/)             | Architecture Decision Records and validation results       |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute (build, lint)                            |
| [LICENSE](LICENSE)                 | Licensing                                                  |
