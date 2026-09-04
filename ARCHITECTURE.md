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
- **Lock-Free Object Pool (`AudioPool`):** Pre-allocated pool of `Box<[f32]>` buffers, each sized to `CAPTURE_MAX_SAMPLES` (5 s at 44.1 kHz) so a runtime change to the capture length never allocates on the audio thread. Thread 2 borrows one to record a stable note, fills it to the current target (1.5 s by default), and passes it to the background worker, which recycles it back to the pool when finished.
- **`ProcessingFrame`:** Thread-local scratch buffers for zero-allocation per-frame DSP. All fields are `Box<[T]>` — allocated once in `AudioPipeline::new()` via `vec![..].into_boxed_slice()`, never resized. Includes dedicated `treble_magnitude_buffer` (1024 bins) and `bass_magnitude_buffer` (4096 bins) for the Dual-Track FFT paths. The Engine reads from these directly — no per-frame heap allocation anywhere in the discovery + tracking chain.
- **`CircularFifo` (COLA):** Owned by `AudioPipeline`. A `Box<[f32]>` ring buffer that accumulates samples and triggers a new FFT + pipeline frame on every 50% hop. Invisible to `tuner-gui` — the GUI only calls `pipeline.push_audio(&[f32])`.

### Threading Model

The thread split also partitions **state**, and that partition is what makes the
real-time side safe to reset at any moment:

- **Threads 1–2 hold transient state.** The Engine's lock, its M-of-N window and
  partial trackers; the Gatekeeper's EMAs and its five-state machine; the
  Strobe's accumulated beat phase, its sliding fits and its baseband record; the
  pipeline's own `CaptureState`. What makes that state _transient_ is that all of
  it is **re-derivable from the audio**: silence resets it and the next note
  rebuilds it within a few hops, so losing it costs nothing but a moment's
  display.
- **Thread 3 holds none.** The Worker is the one genuinely stateless stage —
  each capture and each curve job is a pure function of its payload. Its only
  standing field is the dump directory the frontend hands it.
- **Thread 4 holds the persistent state.** The `InharmonicityProfile`, the app
  settings, and the open session are the program's memory, and they are the one
  thing no other thread can reconstruct: a measurement not written down means
  striking the string again with the piano still in front of you. It reaches disk
  through autosave — an atomic file write, with one `.bak` taken per session
  before the first.

The Worker also writes files: each capture's raw audio and analysis. These are
**diagnostic output, not program state** — nothing in the app reads one back,
they exist for the offline harnesses (`tuner-lab/`), and the frontend
can switch them off by handing the Worker `None`. Where they land is thread-4
policy (`WorkerJob::SetDumpDir`), so `tuner-core` stays headless and resolves no
paths of its own.

The four threads:

#### Thread 1: The Audio Stream

This thread is the high-speed hardware ingestor and signal conditioner.

- **Action:** Continuously captures raw audio from the microphone at 44,100 Hz. Each sample passes through a `DcBlocker` (single-pole high-pass IIR, α = 0.995; its corner sits above A0, which is a measured trade — see [the DC blocker section](#why-the-dc-blocker-corner-sits-above-a0)) to remove hardware-dependent DC offset, then is pushed into the Elastic Ring Buffer. This guarantees every downstream consumer sees a zero-mean signal regardless of microphone, audio interface, or OS driver.
- **Rule:** This thread performs zero allocations and no analysis. The DC blocker is the only computation — one multiply and two additions per sample. Its job is to guarantee pristine, zero-mean data throughput.

#### Thread 2: The Audio Processing Pipeline

This thread constantly consumes data from the Elastic Ring Buffer and executes a deterministic DSP pipeline — `push_audio` feeds the COLA fifo, and every hop runs `process_cola_hop` — to calculate the fundamental frequency ($f_0$).

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
    │ [4] RELEASE (decay)     │                 │     (Goertzel Phase     │
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
                  │ crossbeam SPSC (WorkerOutput)
                  ▼
              GUI (Thread 4)
```

- **The Gatekeeper (Signal Validator & 5-State Logic):** An always-running traffic cop monitoring the signal envelope. It evaluates stability via a `GateResult` return value, executing a 5-stage state machine. The pipeline reads the Gatekeeper's `SignalState` and uses it to drive capture accumulation:
  - _State 0 (IDLE / Silence Gating):_ Uses a dynamic RMS baseline with Exponential Moving Average (EMA) to bypass heavy DSP during periods of noise or silence.
  - _State 1 (ATTACK):_ Uses Normalized Half-Wave Rectified Spectral Flux (NHWRSF) to detect hammer strikes. Sends onset pulse to the Engine to begin pitch detection.
  - _State 2 (TRANSIENT):_ A one-frame buffer state that resolves the `transient_active` flag once NHWRSF drops back below its threshold. Because the entire hammer-string transient is shorter than the 46.4ms FFT window, physical recovery happens rapidly. State 2 serves purely to allow a clean NINOS2 entry on the subsequent frame.
  - _State 3 (HARMONIC DECAY):_ Uses NINOS2 (Normalized Identification of Note Onset based on Spectral Sparsity) to monitor the signal. After the State 2 delay clears, NINOS2 ignores volume swells and identifies the "Golden Window" of pure, stable harmonic decay for capture. It enforces a secondary `required_stable_frames` threshold (e.g. 4 frames) to gracefully bridge any remaining chaos that the fixed delay missed.
  - _State 4 (RELEASE):_ A record ends either at the latched fill target or when the Gatekeeper's envelope decays back to `Silence` — its verdicts bracket the record at both ends, since `Stable` is what started it. The pipeline owns the transition: it dispatches the `CapturePayload` to Thread 3 via a bounded crossbeam channel and moves `CaptureState` to `Processing`, all without blocking the real-time pipeline. A capture recording past the shipped length ignores the decay stop, since the audio past it is the point; the operator can drop such a take mid-record.
- **The Engine (TWM Discovery + Goertzel Phase Tracking):** A pitch detection chain that operates as an independent state machine, **synchronously reset** by the Gatekeeper's onset pulse but otherwise decoupled from the Gatekeeper's internal transient delays.
  - **Discovery Phase (State: Unlocked):** Identifies the fundamental frequency from the 8192-pt bass FFT buffer using the canonical Two-Way Mismatch algorithm.
    1. **Peak Extraction:** Sub-bin peaks are extracted using the Jacobsen complex-domain estimator. To establish a statistical minimum magnitude for Additive White Gaussian Noise (AWGN) rejection, a dynamic Neyman-Pearson threshold is computed against the pipeline's dynamic noise floor and acts as a floor gate. _(Note: Because the piano's acoustic noise floor during an active note is vastly higher than the room's silence threshold, this AWGN boundary is mathematically sound but practically negligible in effect)._
    2. **Peak Masking:** A two-stage masking process is applied: first, a `-30 dB` relative global magnitude floor removes absolute structural noise, followed by our own proportional critical-band masking heuristic to aggressively drop sympathetic tonal noise and structural intermodulation distortion.
    3. **TWM Scoring:** The surviving peaks are scored against 88 pre-computed inharmonicity-stretched `KeyProfile` arrays. The algorithm evaluates both forward error and reverse error with psychoacoustic frequency weighting ($f^{-0.5}$). To prevent unbounded error accumulation from distant noise, the Measured-to-Predicted error is topologically bounded using a piecewise ceiling.
    4. **Temporal Tracking:** The lock is confirmed by **M-of-N binary integration** over the per-frame discovery winner on _stable_ frames — the first key to win ≥ M of the last N stable frames latches (refined default 7-of-8). This is a bounded, allocation-free window that outlasts the attack transient a first-to-win race could otherwise lock onto; confirmation transitions the Engine to the Tracking Phase. (Note: The historic Viterbi hidden Markov model was permanently excised because its path cost persistence caused unrecoverable sub-harmonic locks following noisy hammer strikes).
  - **Tracking Phase (State: Locked):** Once a key is locked, the engine switches to per-partial Goertzel analysis on 1024-sample segments to refine the tuning measurement.
    1. **Phase Vocoder:** Phase differences between consecutive hops are unwrapped to yield instantaneous frequency estimates.
    2. **Amplitude SNR Gate:** A Neyman-Pearson threshold (derived from a Generalized Likelihood Ratio Test) compares the Goertzel unnormalized magnitude against the noise floor. Partials that fail this threshold are rejected as noise.
    3. **Adaptive Seed Feedback:** For partials that survive the SNR gate, a slow Exponential Moving Average adapts the theoretical Goertzel tracking seed toward the measured instantaneous frequency, so the tracker stays locked onto detuned strings without losing coherent integration energy. Its rate is deliberately slow enough to be outrun by a fast pitch raise, which is what the coarse readout covers.
    4. **f0 Reconstruction:** The engine exclusively uses Partial 1 to drive the primary Cent Meter. This intentionally avoids averaging higher partials, which carry an $n^2$ systematic cents error when the theoretical inharmonicity profile ($B_{profile}$) diverges from the physical string ($B_{true}$).
- **Output:** Pushes a `FrameOutput` structure every hop, containing the treble magnitude spectrum, sub-cent accurate $f_0$, real-time partial frequencies, and the strobe's per-reference angle, rate and resolved unison lines, to the UI thread via a wait-free `triple_buffer`.

Once the Gatekeeper detects silence, it closes the gate by sending the `is_silence` flag to the Engine to force an immediate state reset and prevent pitch detection from running on background noise.

The **Strobe** is drawn in its hop position (step 5b, after the Engine) but is a parallel _tap_, not a stage: it reads the hop's audio plus the UI-pushed curve references, and it and the Engine write `FrameOutput` independently. It consumes nothing from the Engine and nothing downstream consumes it — removing it leaves gating, detection, and measurement bit-identical (the authoritative per-hop step list is [`docs/internals/03-dsp-pipeline.md`](docs/internals/03-dsp-pipeline.md); the stage-vs-tap rule is [`01-architecture.md`](docs/internals/01-architecture.md)).

#### Thread 3: The Background Worker

This is a single detached worker thread spawned at pipeline construction inside `AudioPipeline::new()`. It blocks on a `select!` over two inputs — capture buffers from the DSP thread and background jobs from the UI — waking when either arrives. **Captures are serviced first**; background jobs run only when no capture is pending.

- **Action (capture):** When the pipeline dispatches a filled capture buffer, the worker:
  1. Performs a high-resolution power-of-two FFT on the captured audio (up to 65,536 points), plus a one-sample-shifted frame, and derives a CSPE (Short & Garcia 2006) super-resolution per-bin frequency map.
  2. Takes the note **identity** from the payload — it does not re-identify the note. In **Auto Mode** that identity is the Engine's real-time TWM discovery lock (`latched_auto_key`); in **Manual Mode** it is the user-selected key.
  3. Seeds from the Engine's Goertzel-tracked _f₀_ (or the key's Equal-Temperament frequency if the tracker never locked).
  4. Runs MAT (Median-Adjustive Trajectories — serial trajectory growth, reading partial frequencies from the CSPE map) to jointly estimate the partials, the refined _f₀_, and the inharmonicity coefficient ($B$) via the median of pairwise partial combinations.
  5. Writes diagnostic files (`audio.raw` + `analysis.json`) to the open instrument's dump directory — `diagnostics/<identity.id>/`, so captures follow the instrument they were taken on rather than its name.
- **Action (curve job):** When the UI requests a tuning-curve recompute (`WorkerJob::Curve`, carrying a trust-filtered `CurveInput` snapshot), the worker runs all curve engines and returns a `CurveBundle`. Jobs are latest-wins (a `generation` counter drops superseded bundles); the read-only snapshot means a curve recompute can never race the profile a `KeyMeasurement` writes.
- **Output:** Sends a `WorkerOutput` back to the GUI over one shared result channel — `Measurement(KeyMeasurement)` (partials, $f_0$, $B$) per capture, `Curve(Box<CurveBundle>)` per recompute. After a capture it recycles the audio buffers into the `AudioPool`, then clears `capture_in_flight` — which is what ends the capture lifecycle — and only then sends the measurement, so a consumer that arms on it finds the lifecycle already finished. Curve jobs touch neither the pool nor the flag.

#### Thread 4: The UI Thread (The Visual Renderer)

This is the graphical interface thread operating at 60 FPS.

- **Action:** Consumes the high-speed stream of `FrameOutput` structures from Thread 2 via the `triple_buffer` to drive the instantaneous tuning visualizers (spectrogram, cents-deviation, keyboard). Drains `WorkerOutput` results from the Worker via the `worker_rx` receiver: a `Measurement` is **appended** to its key's list in the `InharmonicityProfile`, which then auto-saves — appending rather than replacing is what stops an unattended Auto-mode capture displacing a trusted one, since `active()` reads the newest _trusted_ entry (and, when measured-B discovery seeding is enabled, the recompiled template goes back to the live engine via the `profiles` producer — crossing #4); a `Curve` bundle is stashed in UI state to drive the curve display. On any trusted-set edit (capture merge, undo, profile load) it enqueues a `WorkerJob::Curve` for the Worker to recompute the curve off-thread (recompute-on-load; the curve is never persisted). Reads/writes configuration (e.g., silence threshold, target key) and polls runtime observations (e.g., smoothed RMS for the Envelope Viewer) via `Arc<PipelineAtomics>`.

#### Cross-Thread Communication Topology

Because `tuner-core` enforces strict zero-allocation, wait-free real-time audio
constraints, it relies on a rigidly defined topology for inter-thread message
passing. There are **six** sanctioned crossings. Several carry more than one
payload, so the rows below group by crossing: a second payload of the same class
is a new message type, never a new channel.

| # | Pathway | Primitive | Direction | Purpose |
| --- | --- | --- | --- | --- |
| **1** | Hardware Capture | `ringbuf` SPSC | Stream (1) → DSP (2) | Lossless elastic buffer for incoming raw audio. |
| **2** | Structural Output | `triple_buffer` | DSP (2) → UI (4) | `FrameOutput` — lossy per-hop viz and telemetry, plus anything the DSP integrates across hops. |
| **3** | DSP Parameters | `Arc<Atomic*>` | UI (4) ↔ DSP (2) | Wait-free configuration and metric reads/writes. |
| **4** | Template Update | `ringbuf` SPSC | UI (4) → DSP (2) | Recompiled `KeyProfile` (measured $B$) into the engine's templates. |
| **4** | Strobe References | `ringbuf` SPSC | UI (4) → DSP (2) | `StrobeRefUpdate` — curve targets + coarse partial — into the `Strobe`. |
| **4** | Capture Commands | `ringbuf` SPSC | UI (4) → DSP (2) | `CaptureCommand`: `Arm(ArmRequest)` — fill target + string declaration — and `Cancel`. |
| **5** | Capture Dispatch | crossbeam SPSC (bounded) | DSP (2) → Worker (3) | `CapturePayload` containing pooled audio buffer + metadata. |
| **5** | Buffer Recycling | Lock-Free Object Pool | DSP (2) ↔ Worker (3) | Recycled `Box<[f32]>` buffers at the `CAPTURE_MAX_SAMPLES` ceiling — zero allocation during capture. |
| **5** | Capture Completion | `AtomicBool` | Worker (3) → DSP (2) | `capture_in_flight`: the pipeline raises it as it dispatches, the Worker clears it once the buffers are home. |
| **5** | Worker Results | crossbeam SPSC (bounded) | Worker (3) → UI (4) | `WorkerOutput`: `Measurement(KeyMeasurement)` per capture, `Curve(CurveBundle)` per recompute. |
| **6** | Worker Jobs | crossbeam SPSC (bounded) | UI (4) → Worker (3) | `WorkerJob`: curve recomputes (`CurveInput` snapshot, latest-wins) and the dump directory. |

The channel-by-channel contract is documented in
[02-cross-thread-communication.md](docs/internals/02-cross-thread-communication.md).
What's important here is that the choices are deliberate: each one
sidesteps a category of latency or correctness problem (OS mutexes,
allocator contention, lossy MPMC fallback) that would compromise the
real-time guarantee.

#### The capture lifecycle: one chain across three crossings

Every other piece of state in the program lives on one crossing. A capture is
the exception — it is a chain, and following it is the clearest way to see why
the six are shaped as they are.

`CaptureState` — Idle → Armed → Recording → Processing → Idle, plus Recording →
Idle when the operator drops a take — is **not** shared state. The pipeline owns
it as a plain `enum` field and makes every transition itself, on the Gatekeeper's
verdicts in-thread (`Stable` starts a record, `Silence` ends one) and the
Worker's completion flag. The three threads with a stake in it each get what
their need actually is:

- The UI **asks** for a transition (`Arm`, `Cancel`) on crossing **4**. It never
  writes the state, so the display cannot claim a state the DSP disagrees with.
- The UI **reads** the state off `FrameOutput` on crossing **2** — a one-way
  per-hop snapshot, which is exactly what a lossy buffer is for.
- The Worker **ends** it on crossing **5**, by clearing `capture_in_flight`
  once the pooled buffers are home. That is the one fact the pipeline cannot
  observe for itself.

Ownership is what makes the chain safe: an out-of-sequence transition has no
code path to come from, because every move is an assignment through
`&mut self`. The argument for that choice is under Design decisions below.

## DSP Philosophy: Analytical Algorithms

Piano acoustics (inharmonicity, phantom partials, beating unisons) are complex but well-documented in the literature. Rather than inventing custom heuristics or magic-number thresholds to handle edge cases, the pipeline relies on established, peer-reviewed math.

For example, to handle spectral peaks distorted by beating unisons, it's tempting to write a custom heuristic that measures lobe asymmetry and throws out bad peaks. Instead, we lean on the math: the Hodgkinson (2009) MAT algorithm naturally discards those bad measurements by taking the median of all paired inharmonicity coefficients.

**The Topological Scrutiny Test:** If a heuristic or empirical constant _must_ be introduced, it must define or alter the geometric shape of the information (e.g., scale-invariant frequency ratios or error curve exponents) rather than acting as a fragile, environment-dependent threshold. See [04-algorithms-and-models.md](docs/internals/04-algorithms-and-models.md) for the full standard.

The major DSP components and their foundations:

- **Transient Stability Detection**: `ninos2` — a spectral-sparsity ratio of our own design (an $N/N_{\text{eff}}$ participation-ratio form; _not_ Mounir 2021's NINOS², per faithfulness-audit-05)
- **Onset Detection**: Normalized Half-Wave Rectified Spectral Flux (NHWRSF) — via spectral difference
- **Peak Extraction** (Discovery): Candan (2015) Jacobsen complex-domain estimator
- **Note Discovery**: Maher & Beauchamp (1994) Two-Way Mismatch
- **Detection thresholds**: Kay (1998) Neyman–Pearson floor against the running noise estimate — discovery's peak gate and the tracker's per-partial gate
- **Sympathetic Noise Rejection**: `mask_peaks` — our own critical-band masking heuristic (empirically validated in ADR 0002; the global magnitude gate adapts Cano 1998), with a Duan et al. (2010) topological ceiling on the reverse TWM error term
- **Temporal lock**: M-of-N binary integration — Schwartz (1956) / Shnidman (1998), validated on two instruments in ADR 0010
- **Partial tracking** (Locked): Goertzel phase vocoder — McAulay & Quatieri (1986) instantaneous frequency, with Dolson (1986) adaptive seed feedback
- **Inharmonicity**: Hodgkinson (2009) Median-Adjustive Trajectories (MAT) — serial trajectory growth, with Short & Garcia (2006) Complex Spectral Phase Evolution (CSPE) sub-bin refinement
- **Tuning curve** (cold path): Rigaud, David & Daudet (2013) parametric inharmonicity-and-tuning model — the $B_\xi$ fit and the $\rho_\varphi$ octave-type curve; Giordano (2015) sensory-dissonance octave-width recipe (Plomp–Levelt roughness in the Sethares parametrization) as the perceptual layer; Whittaker (1923) / Eilers (2003) smoother for the per-key residual
- **Strobe display** (`strobe.rs`, a pipeline _tap_, not a chain stage): a fixed-reference Goertzel bank that accumulates per-partial beat phase against the curve targets on the DSP thread — a software strobe (Goertzel 1958 finalization giving exact hop-to-hop phase, audit-08), deep-bass references on a 4096-sample window (R3)
- **Coarse spectral readout** (`peaks::coarse_read`, folded into the strobe): a bounded, ordered-statistic CFAR-gated magnitude search at the nominated reference partial — the strobe's out-of-range fallback when the phase band aliases (ADR 0011)
- **Unison assist** (`peaks::resolve_lines` + `strobe::unison`, the same tap): a zoom FFT (Lyons ch. 13) over the per-reference complex baseband the strobe's own Goertzel already produces, resolving a note's individual strings as separate lines — Rohling's OS-CFAR again for admission, with a _sliding local_ reference window rather than the coarse read's flanking one, and Candan (2015) Eq. 1 for sub-bin refinement (ADR 0012)

Each of those is one file named for the one method it implements (`rigaud.rs`, `giordano.rs`, `whittaker.rs`), composed by `curves.rs` — the same shape `discovery.rs` has over `twm.rs`, and for the same reason: a cited method stays pure and auditable in its own file, and the orchestrator is where they are combined.

If the pipeline produces bad data, the fix is usually to implement the mathematically complete version of the algorithm rather than adding a clamp or a safety bound.

## What the tuning curve is grounded on

Two claims get conflated when people ask whether a tuning curve is "right", and
this project can make only one of them. **Building a curve fitted to a measured
piano is a solved mechanism. Deciding which of the curves it can build is the
best one is not, and may not be solvable at all.** This section separates them,
because the first is defensible in detail and the second is honestly open.

### Where the instrument is characterised, and how strongly

The measure of how much a given piano — rather than the model — determines its
own curve is how far the shipped engine (d) departs from engine (a), the pure
parametric prior. Measured on instrument 2, a Young Chang F-108B upright:

| register | median \|d − a\| | max | what drives it |
| --- | --- | --- | --- |
| bass A0–B2 | **13.64 ¢** | 30.50 ¢ | 24–32 partials/key, $B$ repeatable to 0.14 % |
| tenor C3–G#4 | 1.86 ¢ | 4.89 ¢ | 0.16 % repeatability |
| mid A4–A#5 | 0.74 ¢ | 1.11 ¢ | 0.48 % repeatability |
| upper B5–G#6 | 1.12 ¢ | 1.14 ¢ | shrinkage handing over to the prior |
| top A6–C8 | 0.52 ¢ | 0.97 ¢ | model; 0.02 ¢ at C8 |

Read plainly: **the bottom five and a half octaves are this instrument's curve;
the top two are a model.** The residual in the top register is not information —
it is the smoother's tail reaching up from where the data stops.

Everything instrument-specific enters through four paths: the per-key measured
$B$ (inverse-variance shrunk, ADR 0009), the bass asymptote $\xi = (s_B, y_B)$
fitted by L1 to this piano's keys, the interval rows engine (d) builds wherever
both endpoints carry trustworthy $B$, and the amplitude-informed display partials.
The treble asymptote and the octave-type curve $\rho_\varphi$ are the two things
that are _not_ instrument-specific.

### How a measurement becomes curve $B$: two structures, not one

**Admission.** A capture enters the curve when it is trusted (manual mode, not a
partial unison), its $B$ is finite and positive, it carries at least two
partials, and Rigaud's Eq. 20 can solve an $F_0$ from them. That test is binary
and it is the only binary test: a 3-partial C8 is admitted exactly as a
32-partial A1 is. How much each is _believed_ is decided downstream, and
continuously.

**Structure 1 — the $B_\xi$ model (Rigaud Eqs. 7–8).** Each bridge's $B$ is an
exponential in key number, so on a log axis each is a straight line, and the
model is their sum:

$$B_\xi(m) = e^{s_B m + y_B} + e^{s_T m + y_T}$$

The treble pair $(s_T, y_T)$ is fixed at the paper's cross-piano values ("Why
the top octave is modelled, not measured", below). The bass pair is fitted to the
instrument by least absolute deviations in $\ln B$ (Eq. 29 — an L1 fit, chosen
by the paper because it behaves like a median: one wild key barely moves it)
over **every admitted key, with no cutoff.** Treble keys enter and cannot move
it, not because they are blocked but because the model gives them no lever: a
key's residual only responds to $(s_B, y_B)$ in proportion to the bass line's
share of $B_\xi$ there, and on instrument 2 that share is

| A0 | A1 | A2 | A#2 | A3 | A4 | A5 | A6 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 98 % | 90 % | 56 % | 52 % | 16 % | 2.6 % | 0.4 % | 0.1 % |

so the fit is effectively decided by A0–A3 and the crossover sits at B2.
Admitting everything costs nothing and avoids inventing a register cutoff, which
[04](docs/internals/04-algorithms-and-models.md) forbids. With no admitted keys
at all the model falls back to the medium-piano default pair. Two parameters:
this is the smooth backbone, not the curve.

**Structure 2 — the per-key blend (ADR 0009; ours, not Rigaud's).** Every
admitted key's curve-side $B$ is the precision-weighted combination of its own
measurement and the model's value at that key:

$$\ln B_\text{curve} = w\,\ln B_\text{meas} + (1-w)\,\ln B_\xi,\qquad
w = \frac{\sigma_p^2}{\sigma_p^2 + \sigma_m^2}$$

This is the textbook inverse-variance combination — the posterior mean of a
Gaussian prior and a Gaussian measurement — and what makes it more than a
formula is that **both variances are measured, not chosen.** $\sigma_m$ is the
capture's own repeat scatter, which falls steeply with the number of partials
the capture held; $\sigma_p$ is how far this piano's real strings sit from the
smooth model, self-calibrated per instrument from the keys whose measurement
noise is negligible. Both laws and their constants are
[ADR 0009](docs/adr/0009-repeat-capture-noise-decomposition.md).

The weight then asks one question per key: is this capture's noise smaller than
the real per-key deviation it is trying to resolve? A 32-partial bass capture
answers yes decisively ($w \approx 0.998$); a 4-partial treble capture answers no
($w \approx 0.06$) and is handed back to the model almost entirely; a key with no
capture at all takes $w = 0$. Nothing switches at a threshold —
"measurement-dominated" ($w \ge \tfrac12$, about seven partials) only grades keys
for the consumers below, and $B$ itself stays continuous across it. This replaced
a hard partial-count cutoff, and with it the boundary artifacts ADR 0007 had
flagged.

One limit to state plainly: $\sigma_m$ is a repeat *precision*, not an accuracy.
A self-consistent wrong series — a comb the seed mis-numbered — repeats perfectly
and enters at $w \approx 0.999$. The blend does not guard against that and
cannot; MAT's seed tolerance upstream does.

**The §2 eviction.** With the blended $B$ at both ends of every octave, Eq. 6
gives the beatless stretch of each octave pair. To first order in $B$ it is
negative only when $B_L(4\rho^2 - 1) < B_U(\rho^2 - 1)$ — the upper key would
need more than four to five times the lower key's $B$ within one octave (and at
$\rho = 1$ it can never happen at all: a 2:1 octave on a stiff string is always
stretched). Real pianos rise by at most ~3× per octave in the treble and *fall*
going down the bass, so a negative stretch certifies a wrong $B$ rather than a
strange string. The procedure: among the pair's measurement-dominated keys,
evict the one further from the model (larger $|\ln(B/B_\xi)|$) — hand it back
to $B_\xi$, mark it excluded — then re-check every pair, because an evicted key
is the lower note of one octave and the upper of another, until no negative
pair remains (tolerance 0.01 ¢). Flag-and-exclude, never clamp. On instrument 2
it evicts nothing. Separately, `finish()` re-checks the *final* curve for
$d(m{+}12) < d(m)$ and flags it, without fixing it.

**Who consumes which.** Engine (a) reads the model only: its Eq.-6 chain runs on
$B_\xi$, and it is the backbone every other engine starts from. Engines (b),
(c) and (d) all read the blend — (b) and (c) in their per-key chains, and
(d) in every interval row (`interval_width_cents(curve_b[m], curve_b[u], …)`),
admitted only where both endpoints are measurement-dominated and weighted by the
measured partial amplitudes. The blend is what makes the shipped curve this
piano's; the model is what fills in wherever the blend has nothing to say.

### Why the top octave is modelled, not measured

$B$ is estimated from how far partial $n$ sits above $n f_1$, so a capture
holding more partials resolves it better. Above about A6 the partials are not
there to hold. Measured on instrument 2, relative to each key's own
fundamental, a treble string's second partial sits **28 dB down** and its
fourth **50 dB down**, while a bass string puts *more* energy into its upper
partials than into $f_1$. A treble string radiates almost everything through the
one component that carries no inharmonicity information at all — and that
35–55 dB swing, not the sample rate or the band edge, is the binding constraint.

So the top two octaves are modelled rather than measured. Three things make that
a designed outcome rather than a gap:

- **The handover is continuous.** The blend above already weights every key by
  how well its own capture resolved $B$, so a treble key contributes exactly in
  proportion to what it resolved and the model supplies the rest. There is no
  register threshold to sit on the wrong side of, and nothing switches.
- **The borrowed half of $B_\xi$ is the half that is standardized.** Rigaud
  fixes the treble pair $(s_T, y_T)$ across pianos and fits only the bass pair
  per instrument, and the reason is physical rather than statistical: treble
  string design in this range is not constrained by the size of the case, so it
  is common across instruments, while the size constraint lands on the bass
  bridge — which is exactly the pair we do fit. That borrowed pair agrees with
  Young 1952's independent physics-based derivation to 1.9 %, and both of our
  uprights read its slope within ~2 SE, in the direction their own estimator
  bias predicts. Any error in it is worth **under ~3 ¢ at C8**.
- **The readout does not inherit the uncertainty at all.** Above key 48 the
  strobe and the coarse read both target $n = 1$, and
  $f_1^\ast = f_{ET}\cdot 2^{d/1200}$ contains no $B$. A wrong treble $B$ moves
  the curve; it cannot move the number the operator reads at the pin.

What it costs is bounded and confined to the curve: sweeping treble $B$ across
0.5×–2× of the model moves engine (d)'s C8 target by 29.9 ¢. Why the partials
are missing, why a higher sample rate would not recover them, and what has been
measured against it are in
[ADR 0009](docs/adr/0009-repeat-capture-noise-decomposition.md) analysis 7; the
check of the borrowed asymptote against our own instruments is in
[faithfulness-audit-06](docs/audits/faithfulness-audit-06-b-prior.md).

### Why this cannot over-tighten a string

Tension goes as $f^2$, so a target $\Delta$ cents from a string's current pitch
implies $\Delta T/T = 2^{2\Delta/1200} - 1$. The curve's own targets span
**−19.9 ¢ (A0) to +37.5 ¢ (C8)**, so the curve by itself never asks more than
**+4.4 %** of a string already at ET pitch, and the bass targets are _below_
pitch, i.e. loosening. Measured against instrument 2's as-found state with the
shipped engine, the mean change is **about +0.5 % across the instrument**
(median move +0.1 ¢); the largest single move is a top-octave string at roughly
+9–11 %, almost all of it the string's own drift, on keys whose as-found pitch
scatters ±15 ¢ between repeat captures. Piano wire is designed to sit at roughly
half to two-thirds of its breaking tension, so even that worst case moves a
healthy string from ~60 % to ~66 %.

Most of any large move is the string's accumulated drift, not the curve. The
hazards this software does not change are a large overall pitch raise (which the
A440-only limit can silently prescribe on a flat piano), and over-pulling past
the target on short treble strings where a small pin rotation covers many
cents.

### $\rho$ has a hard floor and a soft ceiling

$\rho$ indexes _which_ partial pair is made beatless: partial $2\rho$ of the
lower note against partial $\rho$ of the upper, so $\rho = 1$ is the 2:1 octave,
2 is 4:2, 3 is 6:3.

**$\rho < 1$ asks for a partial below the fundamental**, which does not exist.
Numerically it also under-stretches into compressed octaves (at
$\rho \le 0.25$ the A0–A1 stretch goes negative), which is exactly the
condition the §2 detector treats as an artifact. `StretchPreset::Low`
therefore floors at 1, and any future arbitrary-$\rho$ control must too.

The upper bound is musical, not mechanical, and binds far sooner. $\rho = 3$
already places A7 at **+78 ¢** — absurd by any tuning standard — at a tension
excursion of only +9.5 %. Tension does not become the binding concern until
$\rho \approx 6$–8, where A7 sits one to two semitones sharp. **Clamp to
$\rho \in [1, 3]$ on musical grounds; breakage is nowhere near.**

Where $\rho$ actually binds is the treble, and there it is unconstrained by
data: engine (c)'s calibration accepts **zero $\rho$ points above F4** (key 44),
and running the Eq.-6 chain at fixed treble $\rho$ puts A7 at +24.2 ¢
($\rho = 1$, pure 2:1) / +29.3 (shipped) / +46.3 ($\rho = 2$, pure 4:2) — a
**17 ¢ span, larger than the whole $B$ uncertainty.** Rigaud's own ±1 variants
(`StretchPreset::{Low, Mean, High}`, §IV.C.2) cover exactly that range. They
exist in `CurveParams` and are deliberately **not yet wired**, because choosing
between them is a listening judgment: the control waits on the auralization path
rather than shipping as an unexplained knob. `Mean` is the conservative default.

One property to know before it is exposed: **the presets are not symmetric in
the treble.** `Low` is $\rho - 1$ floored at 1, and treble $\rho$ is already
1.11, so `Low` lands on the floor — A7 = +24.2 ¢ against `Mean`'s +29.3 ¢, a
5 ¢ step — while `High` reaches ≈ +47 ¢, an 18 ¢ step. In the bass, where
$\rho \approx 4.4$, the same ±1 is symmetric. The floor, not the preset, is
what compresses the low side up top.

### The limits, and what compensates for each

Four limits, each argued above:

- **Treble $B$ is not measurable** (its partials are 30–60 dB below the
  fundamental) — compensated by continuous inverse-variance shrinkage toward a
  borrowed asymptote that has itself been checked.
- **The readout does not inherit that uncertainty** — in that register the
  strobe and the coarse read target a partial that carries no $B$, so a wrong
  treble $B$ moves the curve, never the number the operator reads at the pin.
- **A bad capture cannot silently poison the curve** — a measurement implying a
  negative octave stretch is definitionally an estimator artifact, and the §2
  eviction hands that key back to the fit.
- **The octave type $\rho$ is not measurable even in principle** — it encodes
  which interval you prefer to make beatless, and it is worth more at the top of
  the piano than the whole $B$ uncertainty.

That last one is not compensated, and it feeds straight into the open question below.

### What is still open

Curve _selection_, and it is a sharper problem than "we need more data".

The engines split into two families in the bottom octave. At A0: the
octave-chain engines land at **(a) −50.4 ¢, with (b) and (c) within a cent of it**; the
multi-interval ones at **(d) −19.9 (shipped), (d) pure-12ths −14.3**. The
mechanism is plain — (a)–(c) walk the Eq.-6 chain, each step setting the
beatless width for the prescribed $\rho \approx 4.4$ down there, which with the
bass's large $B$ yields 11.2–13.6 ¢/oct of stretch; (d) least-squares a
compromise across octaves, twelfth, double octave and tempered fifths/fourths
at once, landing at 3.55 ¢/oct. **A ~31 ¢ disagreement on the lowest notes.**

It is not noise: ADR 0009 analysis 4 resampled which capture feeds each key over
24 draws and measured (d)-Balanced's bass curve SD at **0.02 ¢** — the gap is
three orders above it.

**Both available criteria are circular, in opposite directions.** Beat-rate
coherence favours (d) decisively (bass 2:1 median 0.16 Hz against (a)'s 0.68;
4:2 0.11 against 1.08) — but 2:1/4:2/6:3 are (d)'s own objective. Leave-one-key-out
favours the chain engines just as decisively ((d) 14.68 ¢ bass against (b)'s
0.42) — but its reference _is_ the Eq.-6 chain value. Each metric is one
family's objective. A cross-check on intervals in neither objective — major
thirds, sixths, minor thirds — cannot reach: across the temperament region all
five engines are indistinguishable (8.29–8.87 Hz median third rate, 1–2
reversals), and in the deep bass those intervals are not aural tests.

So no further computation on the existing data resolves it. Three things could:
an **aural reference tuning** measured and compared (the ground truth
[the tuning-curve design note](docs/design/tuning-curve-design.md) §11 records
as absent), a **listening test** — which is what `synth.rs` and the `auralize`
harness exist for — or a beat-salience measure belonging to neither family's
objective, which has not been designed.

**What the GUI offers is a subset of what the worker computes.** The bundle
keeps all five engines — they cost little, and they are how the disagreement
above stays visible to the offline harnesses — but a selector asking the operator
to choose among five curves asks them to settle a question this project has not
settled. Engines (b) and (c) are therefore not offered as tuning targets while
their validity is open; (d)-Balanced is the shipped one.

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

The tuning-curve engines share that thread for the same reason — engine (c)'s
Giordano dissonance scans alone take ~1.3 s — and captures are serviced ahead of
them, because measurement latency is user-facing mid-session and a curve
recompute is not.

A single worker thread is sufficient because captures are
deliberately serialised at the source: a stable note is held for
1.5 s before the next one can be captured, and the worker finishes
well within that window. Adding more worker threads would buy
nothing and would complicate buffer recycling.

### One owner for `CaptureState`, not a shared state machine

Three threads have a stake in the capture lifecycle — the UI arms and
cancels, the DSP thread runs the record, the Worker finishes it — so the
obvious shape is a shared `AtomicU8` whose transitions are partitioned
between them by convention. That puts a state machine in shared memory,
and the only way to defend one is to check every transition at runtime.

`CaptureState` is instead a plain `enum` the pipeline owns. Arming is a
**command** the UI sends over a `ringbuf` (the Capture Commands row
above); the Worker reports completion by clearing one `AtomicBool` — the
one fact the pipeline cannot observe for itself. With one thread making
every transition, an out-of-sequence move has no code
path to come from — the compiler enforces through `&mut self` what a
`compare_exchange` could only detect afterwards. The state reaches the
UI on `FrameOutput`, which is where a one-way per-hop snapshot belongs
anyway.

The general rule this is an instance of: **when a state machine can have
one owner, give it one owner** — hardening shared mutation is the weaker
move. The transition table is in
[02-cross-thread-communication.md](docs/internals/02-cross-thread-communication.md).

### Hardcoded 44.1kHz Sample Rate Architecture

The pipeline is statically locked to a 44,100 Hz sample rate. This is not just a surface-level parameter; it is deeply baked into the zero-allocation memory layout and temporal math of the DSP chain:

- **Static Buffer Sizes**: The `AudioPool`'s buffers are allocated once at startup to a sample-count ceiling derived from 44.1 kHz, and every capture length is a sample count against that rate.
- **Time/Frame Conversions**: Gatekeeper thresholds are counted in DSP frames. Each frame analyses a 2048-sample window, but frames arrive one per 1024-sample hop, so a threshold like `required_stable_frames = 4` converts to a duration only through that hop rate.
- **Math Constants**: Parameters like EMA smoothing alphas (`rms_ema_alpha = 0.1`) and the Gatekeeper's timing thresholds are calibrated against 44.1kHz timing and frequency bin widths.

Attempting to change the sample rate dynamically would require migrating away from fixed-size arrays to dynamic allocations on the audio hot-path, breaking the core real-time guarantees. While we currently rely on the host OS audio daemon (e.g., PipeWire or CoreAudio) to resample native hardware inputs down to 44.1kHz, this is strictly a temporary stopgap. Full dynamic sample rate support is planned for the future, but it requires a complex architectural overhaul; shipping a robust, working pipeline at a fixed rate remains the immediate priority.

One thing a higher rate would **not** buy is a measured top-octave $B$ for the curve: the partials it would admit are weaker than the ones already failing ([ADR 0009](docs/adr/0009-repeat-capture-noise-decomposition.md) analysis 7). It would still be worth having for analysis — more treble partials on record, even ones too quiet to move the curve — which is a reason to build it, just not that one.

### Why the DC blocker corner sits above A0

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
fundamental — the per-bin/per-octave noise floor in [TODO.md](TODO.md),
or an instrument that genuinely radiates it (both validation pianos are uprights,
the weak case). If that happens, change the **order**, not α, and note three
things: cascaded biquads are needed rather than one pole; conditioning at
`fc/fs ≈ 5.7e-4` puts the poles at radius ≈ 0.9965, where f32 direct-form I is
marginal (use transposed direct-form II or f64 state); and more filter state
multiplies the single-state stereo defect recorded in [TODO.md](TODO.md).

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
| [docs/audits/](docs/audits/)       | Faithfulness audits of each ported algorithm               |
| [TODO.md](TODO.md)                 | The backlog, with what each item is blocked on             |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute (build, lint)                            |
| [LICENSE](LICENSE)                 | Licensing                                                  |
