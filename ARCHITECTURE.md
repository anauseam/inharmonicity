# Architecture

How this program is built, for someone contributing. One section per crate:
`tuner-core` carries the runtime, the codemap, the contracts a change must not
break, the decisions that shaped it and the methods it stands on; `tuner-gui`
and `tuner-lab` carry their own maps and the contracts that bind them alone.
What the program is *for* is the [README](README.md); the full text of each
contract is in [`docs/internals/`](docs/internals/); the evidence behind
every number is in [`reports/`](reports/).

## Overview

```text
┌─────────────┐         ┌─────────────────────┐         ┌─────────────────────┐
│ 1 · Audio   │         │ 2 · DSP             │         │ 3 · Worker          │
│ sound card  │ samples │ 43 hops/s           │ capture │ on demand           │
│ clean audio │────────►│ gate · name the key │────────►│ (F₀, B) for one key │
│             │   ring  │ track · strobe      │  1.5 s  │ curve for all 88    │
└─────────────┘         └─┬───────────────────┘         └─┬───────────────────┘
                          │ frame,        ▲ targets,      │ (F₀, B), ▲ recompute
                          │ newest wins   │ settings,     │ curve    │
                          ▼               │ arm/cancel    ▼          │
                        ┌─────────────────┴──────────────────────────┴────────┐
                        │ 4 · GUI                                             │
                        │ 60 frames/s                                         │
                        │ profile · settings · display · autosave to disk     │
                        └─────────────────────────────────────────────────────┘
```

Each box is a thread, with what paces it on its second line: all threads in the top row are for `tuner-core`, and the bottom row is for `tuner-gui`. No arrow waits for a reply.

Audio in, tuning-targets out. Each hop, the DSP pipeline decides whether a note is
sounding and has settled, names the key, and reports the strobe's beat phase
against that key's targets. When a capture is armed, a settled note is recorded
and handed to a background Worker, which extracts the partials, fits the
stiff-string equation $f_n = n F_0 \sqrt{1 + B n^2}$ (stiffness lifts a string's
partials above the harmonic series by the inharmonicity coefficient $B$) and
records the key's $(F_0, B)$ in the instrument's `InharmonicityProfile`. The
curve engines turn a profile into per-key targets, and the strobe reads against
them. The measurement, the strobe and the ET reference mode are
instrument-agnostic; all current tuning curves are based on Rigaud's dual-bridge
$B_\xi$ piano model. On a piano $B$ is smallest around the mid bass, rises steeply
through the treble to more than two orders of magnitude more at the top, and rises
again into the deep bass; each end is a bridge term in the model.

`tuner-core` is **headless**. No GUI code, no GUI-specific
types, no dependencies on `iced`, `egui`, or any frontend framework. Any future
frontend (a CLI, a WebAssembly app, a mobile app, or an alternative GUI) must be able to
consume the crate as-is. `tuner-gui` depends on `tuner-core`. The reverse never
holds. `tuner-lab` holds the measurement harnesses, ships in nothing, and depends
on `tuner-core` only.

**Four threads, six wait-free crossings.** The audio callback conditions and
buffers; the analysis thread runs the DSP pipeline once per hop; a single
background Worker does the heavy, non-real-time measurement; the GUI renders.
The threads, and what crosses between them (each crossing's contract is in
[`thread-crossings.md`](docs/internals/thread-crossings.md)):

```text
audio::spawn_analysis_thread(source, dump_dir)  →  HostHandle
   (builds an AudioPipeline, moves it to the analysis thread, keeps the ports)
        │                         │
        ▼                         ▼
    Analysis Thread           Frontend Thread
    ┌─────────────────┐       ┌────────────────────────────────┐
    │ Gatekeeper      │       │ HostHandle                     │
    │  Silence /      │       │  .pipeline_handle.atomics ← rw │ (config + runtime observations)
    │  Unstable /     │       │  .frame_rx          ← read     │ (FrameOutput: viz + strobe angles)
    │  Stable, +onset │       │  .worker_rx         ← recv     │ (WorkerOutput: measurements + curves)
    │       ↓         │       │  .send_curve_job()  → send     │ (WorkerJob: curve recompute)
    │ Engine (F0 DSP) │       │  .send_dump_dir()   → send     │ (WorkerJob: dump directory)
    │       ↓         │       │  .profiles          → send     │ (template updates → DSP; gated off)
    │ Strobe (tap)    │       │  .strobe_refs       → send     │ (strobe references → DSP)
    │       ↓         │       │  .capture_commands  → send     │ (Arm / Cancel → DSP)
    │ Capture Accum.  │       └────────────────────────────────┘
    │   (AudioPool)   │
    │       ↓         │
    │ FrameOutput     │ ─────────────► triple_buffer (lossy, freshest hop)
    └───────┬─────────┘
            │ crossbeam SPSC (CapturePayload)   [DSP → Worker]
            ▼
      Worker Thread   ◄─── crossbeam SPSC (WorkerJob: curve recompute, dump dir)  [UI → Worker]
      ┌───────────────────┐
      │ High-res FFT+CSPE │ ← 65536-pt + shifted frame   (captures serviced first;
      │ MAT (f₀,B solver) │ ← partials + inharmonicity     curve recompute when idle)
      │ Curve engines a–d │ ← CurveBundle (cold, ≈ 1.4 s)
      │ Diagnostics I/O   │ ← analysis.json + audio.raw
      └────────┬──────────┘
               │ crossbeam SPSC (WorkerOutput: measurements + curves) → Frontend
               │ recycles buffers → AudioPool, then clears capture_in_flight → DSP
               ▼
```

A host with its own audio thread calls `AudioPipeline::new` directly instead,
keeps the `PipelinePorts` it returns — the same atomics, receivers and producers,
plus the `worker_job_tx` the `HostHandle` keeps private — and takes each hop's
`FrameOutput` from `push_audio`'s return value rather than from a triple buffer.

The thread split also partitions **state**, and that partition is what makes the
real-time side safe to reset at any moment:

- **Threads 1–2 hold transient state.** The Engine's lock and partial trackers; the Gatekeeper's readings and states; the
  Strobe's accumulated beat phase, its sliding fits and its baseband record; the
  pipeline's own `CaptureState`. We call it the *transient* state because it is
  **re-derivable from the audio**. The next note rebuilds it within a few hops, so
  losing it costs nothing but a moment's display. Silence resets the Engine's lock
  and trackers and the Gatekeeper's verdict. The Strobe deliberately holds its
  accumulated phase and rate, which only a change of reference clears. An armed
  capture stays armed until triggered or cancelled.
- **Thread 3 holds none.** The Worker is the one genuinely stateless stage. Each
  capture and each curve job is a pure function of its payload. Its only
  standing field is the dump directory the frontend hands it.
- **Thread 4 holds the persistent state.** The `InharmonicityProfile`, the app
  settings, and the open session are the program's memory, and are the one
  thing no other thread can reconstruct. A measurement not written down means
  striking the string again with the piano still in front of you. It reaches disk
  through autosave, which performs an atomic file write and takes one `.bak` file per session.

As mentioned previously the Worker has a dump directory. It writes files regarding each capture's raw audio and analysis. These are
**diagnostic output, not program state**. Nothing in the app reads one back and they only exist for the offline harnesses (`tuner-lab/`). The frontend
can switch them off by handing the Worker `None`. Where they land is thread-4
policy (`WorkerJob::SetDumpDir`), so `tuner-core` stays headless and resolves no
paths of its own.

## `tuner-core`

### How it runs

The pipeline, hop by hop:

```text
      samples
         │
┌────────┼──── AudioPipeline::process_cola_hop, once per 1024-sample hop ──────┐
│        ▼                                                                     │
│ ┌──────────────┐     ┌───────────────────┐                                   │
│ │ CircularFifo │────►│ 1 ProcessingFrame ├────────────────────────┐          │
│ └──────────────┘     └─────────┬─────────┘                        │          │
│                                ▼                                  │          │
│                       ┌─────────────────┐                         │          │
│                       │ 3 Gatekeeper    │                         │          │
│                       └────────┬────────┘                         │          │
│           ┌── GateResult ──────┼───────────────────┐              │          │
│           │                    ▼                   │              │          │
│           │           ┌─────────────────┐          │              │          │
│           │           │ 5 Engine        │          │              │          │
│           │           └────────┬────────┘          │              │          │
│           │   ┌─ PitchResult ──┴───────────────┐   │              │          │
│           ▼   ▼                                ▼   ▼              ▼          │
│ ┌─────────────────────┐       ┌──────────────────────┐   ┌─────────────────┐ │
│ │ 6 capture lifecycle │──────►│ 7 FrameOutput        │◄──│ 5b Strobe (tap) │ │
│ └──────────┬──────────┘       └──────────┬───────────┘   └─────────────────┘ │
└────────────┼─────────────────────────────┼───────────────────────────────────┘
             ▼                             ▼
  CapturePayload → Worker         triple buffer → GUI
```

Every arrow inside the box is `AudioPipeline` handing one component's return
value to the next; no component calls another. The numbers are the steps of the
per-hop table in [`hot-path.md`](docs/internals/hot-path.md). Capture takes the Engine's key and its f₀, which seeds MAT. Not
drawn: steps 0, 2 and 4 (the queues, the config atomics, the magnitude spectra),
and capture's other two inputs, the hop's Arm / Cancel and the Worker's in-flight
flag.

**The Gatekeeper and the Engine.** What the gate's verdict does to the Engine,
each hop:

```text
┌─ 3 Gatekeeper ───────────────┐ GateResult   ┌─ 5 Engine ─────────────────────┐
│ Silence   RMS EMA under the  ├─ reset all ─►│ [A] Discovery                  │
│           silence threshold  │              │     TWM over the bass peaks;   │
│ onset     flux (NHWRSF) over ├─ drop lock ─►│     one vote per Stable hop,   │
│           its threshold      │              │     locks at 7 of the last 8   │
│ Stable    stability over its ├─ vote ──────►│                                │
│           threshold, 4 hops  ├──────────┐   ├──────────── ▼ lock ────────────┤
│ Unstable  not yet settled    ├─ track ──┴──►│ [B] Tracking                   │
│ An onset is a per-hop flag,  │              │     a Goertzel phase vocoder   │
│ not a state.                 │              │     on each tracked partial    │
└──────────────────────────────┘              └────────────────────────────────┘
```

The pipeline hands the Engine each hop's `GateResult`, its state and onset flag.
Silence sends the Engine back to discovery with its trackers cleared; an onset
drops the lock and skips its hop. Only Stable hops vote, and once the lock lands
every hop that is neither silent nor an onset is tracked. A key the user selects
skips the vote and locks at once. The full state rules are in `gatekeeper.rs`'s
module doc.

**The Gatekeeper and capture.** What the same verdicts do to the capture
lifecycle:

```text
┌─ 3 Gatekeeper ─┐      ┌─ 6 capture lifecycle · CaptureState ─────────────────┐
│                │      │ Idle                                                 │
│                │      │  │  Arm, from the GUI                                │
│                │      │  ▼                                                   │
│ onset          ├─────►│ Armed      an onset starts the diagnostic record,    │
│                │      │            opening with about 350 ms of pre-roll;    │
│ Silence        ├─────►│            silence: nothing starts until an onset    │
│ Stable         ├─────►│  │  the first Stable hop after that onset            │
│                │      │  ▼                                                   │
│                │      │ Recording  the measured record, the one MAT analyses │
│ Silence        ├─────►│  │  full, or silent at the default length            │
│                │      │  ▼                                                   │
│                │      │ Processing a CapturePayload on its way to the Worker │
│                │      │  │  the Worker clears its in-flight flag             │
│                │      │  ▼                                                   │
│                │      │ Idle                                                 │
└────────────────┘      └──────────────────────────────────────────────────────┘
```

Capture fills two buffers from the pool: the diagnostic record, which is only
ever written to disk, and the measured record, the one MAT analyses. Unstable
changes nothing here, and a Cancel from the GUI sends Armed or Recording back to
Idle.

**How the hot path stays allocation-free.** Every buffer it touches is allocated
once, up front, and none of this relies on the OS raising the thread's priority:

- **The Elastic Ring Buffer:** A lock-free circular buffer connecting Thread 1 and Thread 2. Acts as an elastic shock absorber if the OS briefly suspends the processing thread. Audio keeps accumulating for up to ≈ 371 ms (`RING_BUFFER_CAPACITY`); past that the callback drops samples rather than block. In normal running the analysis thread drains it as fast as the callback fills it, so the capacity adds no delay; it only sets how long a stall can last before audio drops, for 64 KiB of memory.
- **Lock-Free Object Pool (`AudioPool`):** Pre-allocated pool of `Box<[f32]>` buffers, each sized to `CAPTURE_MAX_SAMPLES` (5 s at 44.1 kHz) so a runtime change to the capture length never allocates on the audio thread. A capture borrows two: one at the onset for the diagnostic full-event record (pre-roll, strike and decay), and one at `Stable` for the note itself, filled to the current target (1.5 s by default). Both pass to the background Worker, which recycles them into the pool when finished.
- **`ProcessingFrame`:** Scratch buffers owned by `AudioPipeline`, for zero-allocation per-frame DSP. All fields are `Box<[T]>`, allocated once in `AudioPipeline::new()` via `vec![..].into_boxed_slice()`, and never resized. Includes dedicated `treble_magnitude_buffer` (1024 bins) and `bass_magnitude_buffer` (4096 bins) for the Dual-Track FFT paths. The Engine reads from these directly; there is no per-frame heap allocation anywhere in the discovery + tracking chain.
- **`CircularFifo` (COLA):** Owned by `AudioPipeline`. A `Box<[f32]>` ring buffer holding `BASS_WINDOW_SIZE` (8192) samples. Every time it gathers a `HOP_SIZE` (1024-sample) hop (half the 2048-sample treble window, an eighth of the 8192-sample bass window), `push_audio` runs one pipeline frame. Invisible to a frontend: the analysis thread `spawn_analysis_thread` starts is what calls `push_audio`.

### Codemap

The pipeline and its components sit at the top of `src/`; `algorithms/` holds
the stateless DSP math they call.

```text
tuner-core/
├── src/
│   ├── algorithms.rs             stateless DSP building blocks; the curve modules are cold path
│   ├── algorithms/
│   │   ├── curves.rs             curve engines (a)–(d), built on rigaud, giordano, whittaker
│   │   ├── discovery.rs          split search: an 88-key TWM scan, then refine the best few
│   │   ├── giordano.rs           Giordano's sensory-dissonance octave width (Plomp–Levelt)
│   │   ├── mat.rs                Median-Adjustive Trajectories: joint (f₀, B) on a CSPE map
│   │   ├── metrics.rs            RMS, EMA, spectral flux (NHWRSF), spectral sparsity
│   │   ├── peaks.rs              peak picking and masking, the coarse read, the line search
│   │   ├── rigaud.rs             Rigaud's inharmonicity (B_ξ) and octave-type (ρ_φ) model
│   │   ├── spectral.rs           FFT, CSPE, Jacobsen–Candan refinement, Goertzel
│   │   ├── twm.rs                Two-Way Mismatch f₀ scoring (Maher & Beauchamp 1994)
│   │   └── whittaker.rs          Whittaker smoother and its banded least-squares solver
│   ├── audio.rs                  CPAL input (DC-blocked), analysis-thread host, DSP constants
│   ├── cola.rs                   CircularFifo: the FIFO the overlapping windows read from
│   ├── engine.rs                 f₀ detection: names the key, M-of-N lock, partial tracking
│   ├── gatekeeper.rs             signal validator: each hop is Silence, Unstable or Stable
│   ├── lib.rs                    crate root; FrameOutput, the per-hop result a frontend reads
│   ├── models.rs                 domain types: notes, measurements, profile, curve, templates
│   ├── pipeline.rs               AudioPipeline + PipelinePorts: the hop, atomics, captures
│   ├── strobe.rs                 fixed-reference Goertzel bank: beat phase per partial (a tap)
│   ├── strobe/
│   │   ├── band_slope.rs         the band's rotation rate in Hz, by sliding least squares
│   │   └── unison.rs             a baseband ring per reference (experimental)
│   ├── synth.rs                  additive resynthesis of a tuning curve to audio (cold path)
│   ├── tests/*_tests.rs          unit tests: FFT, CSPE, Jacobsen, device config, peaks
│   └── worker.rs                 background thread: measures captures, recomputes curves
├── tests/                        integration tests, on the public API
│   ├── mat_b_recovery.rs         MAT's (f₀, B) recovery against known synthetic B
│   ├── profile_schema_compat.rs  profiles saved before a field rename still load
│   └── unison_resolution.rs      the unison line estimator against synthetic truth
└── benches/                      criterion: hot-path cost against one hop
    ├── discovery_cost.rs         split discovery's per-frame cost, against one hop
    └── strobe_cost.rs            the strobe bank's per-hop cost, against one hop
```

### Invariants (briefly)

The contracts a change must not break. The full text of
each lives where it binds.

**The hot path is allocation-free and non-blocking.** Everything reached from
`AudioPipeline::process_cola_hop`: Gatekeeper, Engine, Strobe and every
`algorithms/*` call, plus the audio callback. The Worker and the GUI may
allocate freely. → [`hot-path.md`](docs/internals/hot-path.md)

**One entry point, pure components.** Every hop runs exactly one function,
`process_cola_hop`; the components return by value and only the pipeline syncs
to the atomics. `audio.rs` calling into `Engine` is what this prevents.
→ [`hot-path.md`](docs/internals/hot-path.md)

**The chain is sacred; taps are deletable.** FFT front end → Gatekeeper →
Engine → `FrameOutput`, with the capture limb to the Worker, has one shape;
inserting a stage needs a design note and a *Decisions* entry. Everything else
in the hop is a tap, and removing one must leave gating, detection and
measurement bit-identical. The Strobe is a tap.
→ [`hot-path.md`](docs/internals/hot-path.md)

**Six crossings, and no seventh by accident.** Cross-thread state moves only
over the six wait-free crossings; a second payload of the same class is a new
message type, never a new channel.
→ [`thread-crossings.md`](docs/internals/thread-crossings.md)

**Split / Handle.** The `split()` convention `ringbuf` and `crossbeam_channel`
share: construction hands back one half that moves to the analysis thread —
`AudioPipeline`, which owns every real-time component and is the only thing
that mutates them — and one the caller keeps, `PipelinePorts`, or the
`HostHandle` that `spawn_analysis_thread` folds them into. Neither side needs
the other's internals.
→ [`thread-crossings.md`](docs/internals/thread-crossings.md)

**Layering.** Stateless DSP math lives in `algorithms/`, domain types in
`models.rs`, and cross-hop state on the component that owns it.
→ [`layering.md`](docs/internals/layering.md)

### Decisions

The choices that shape the system as a whole: what was decided, why, what would
reopen it, and where the evidence is. An entry that cites no report rests on the
argument at its pointer. The complete decision log, constants included,
is the report index in [`reports/README.md`](reports/README.md).

**Scope is struck and plucked stiff-string instruments.** Every current subsystem
relies on two things about the sound: a decaying excitation, and partials that
follow the stiff-string equation, of which the ideal string is the $B = 0$ case.
An instrument that breaks either needs a subsystem replaced. Reopens if the
program takes on sustained excitation (bowed, blown) or modal percussion, whose
modes are no stretched harmonic series; each is its own decision.
→ [report 0004](reports/retiring/0004-instrument-scope.md)

**Heavy DSP runs off-thread, on one Worker.** MAT, the high-resolution FFT and
the curve engines would exceed the real-time budget inline; one thread suffices
because captures are serialised at the source. →
[`thread-crossings.md`](docs/internals/thread-crossings.md) §5, and the Worker's
own module doc.

**The pipeline is locked to 44.1 kHz.** Buffer sizes, window lengths and every
gate's timing are dimensioned for it; a dynamic rate is a planned overhaul, not
a parameter. → [`hot-path.md`](docs/internals/hot-path.md), *Sample-rate handling*.

**Two FFTs run every hop: 2048 for the treble, 8192 for the bass.** Frequency
resolution is bought with time: the 8192-sample window separates the bass
partials, and lags the sound by 186 ms; the 2048-sample window keeps the gate
and the display within 46 ms of it. The cost is that latency, not the compute,
and both run every hop so the hop's control flow never varies. Reopens if what
reads the bass spectrum no longer needs that resolution.

**`CaptureState` has one owner.** The pipeline holds the capture's state as a
plain `enum` and makes every transition itself; the GUI asks for one
(crossing 4) and reads the result (crossing 2), and the Worker clears one
`AtomicBool` when its work is done (crossing 5). One writer means no transition
can race another, and every move is in one function to read. Reopens if a
lifecycle operation ever needs a synchronous answer. The chain is walked in
[`thread-crossings.md`](docs/internals/thread-crossings.md).

**`synth` is cold-path; audio-out is a future seventh crossing.** Resynthesis
runs on no pipeline thread and owns no stream; when playback is built, the
output stream lives in `audio`, not the GUI. →
[`thread-crossings.md`](docs/internals/thread-crossings.md), *Cold-path
modules*.

### DSP foundations

Published methods, used as published: where a mechanism or a constant is ours,
its guard comment says so, states what it rests on and cites the report. Where
a method was chosen over rivals, its entry says why and what would reopen it.
The rule, and the test any added heuristic must pass, is in
[`layering.md`](docs/internals/layering.md).

The major DSP components and their foundations:

- **Sustain Stability Detection**: `inverse_participation_ratio`, $N/N_{\text{eff}}$ — the inverse of Bell & Dean's (1970) participation ratio, the $\ell^2/\ell^1$ sparsity measure Hurley & Rickard (2009) define. Used as the sustain gate.
- **Onset Detection**: `nhwrsf` — half-wave-rectified L1 spectral flux (Masri 1996; Bello et al. 2005; Dixon 2006). Heuristic edits: the ≈ 43 Hz–10 kHz band, and dividing by the frame's in-band magnitude sum, which makes the flux independent of gain
- **Peak Extraction** (Discovery): Candan (2015) Jacobsen complex-domain estimator
- **Note Discovery**: Maher & Beauchamp (1994) Two-Way Mismatch, scored on spectral peaks and searched coarse to fine; its amplitude terms ($q$, $r$) are ours, tuned by an NSGA-II sweep (report 0006). A stiff string's f₀ is a parameter of its partial series, not the frequency of any one line, and in the bass only the series separates neighbouring keys: at A0 a semitone is 1.6 Hz against a 5.4 Hz bin, and the gap grows with partial number. TWM is a published series scorer that has worked here so far, not one shown to be best; MAT stays on the Worker. Reopens on a labelled in-scope corpus with trustworthy truth, on polyphony, or on systematic *B*-mismatch in the discovery residuals (report 0005)
- **Detection thresholds**: Kay (1998) Neyman–Pearson amplitude threshold, with σ taken to be the silence threshold — discovery's peak gate, the tracker's per-partial gate and the strobe bank's per-reference gate. Nothing measures the room's noise; that threshold is 1.5 × the loudest ambient RMS seen at calibration, or whatever the operator sets
- **Sympathetic Noise Rejection**: `mask_peaks` — our own critical-band masking heuristic (empirically validated in report 0002; the global magnitude gate adapts Cano 1998), with a Duan et al. (2010) topological ceiling on the reverse TWM error term
- **Temporal lock**: M-of-N binary integration at (7, 8), a published nonparametric rule — Schwartz (1956) / Shnidman (1998), validated on two instruments in report 0010. The attack transient wins a first-to-3 race; a window that outlasts it recovers the note body's plurality, the largest single lever measured on the discovery side. Reopens on the post-tuning re-replay; release and re-lock hysteresis are deliberately unspecified
- **Partial tracking** (Locked): Goertzel phase vocoder — McAulay & Quatieri (1986) instantaneous frequency, with an adaptive re-centring of each partial's evaluation frequency that is ours, after Dolson (1986)
- **Inharmonicity**: Hodgkinson (2009) Median-Adjustive Trajectories (MAT) — serial trajectory growth, with Short & Garcia (2006) Complex Spectral Phase Evolution (CSPE) sub-bin refinement
- **Tuning curve** (cold path): Rigaud, David & Daudet (2013) parametric inharmonicity-and-tuning model — the $B_\xi$ fit and the $\rho_\varphi$ octave-type curve; Giordano (2015) sensory-dissonance octave-width recipe (Plomp–Levelt roughness in the Sethares parametrization) as the perceptual layer; Whittaker (1923) / Eilers (2003) smoother for the per-key residual
- **Strobe display** (`strobe.rs`): a fixed-reference Goertzel bank that accumulates per-partial beat phase against the curve targets on the DSP thread — a software strobe (Goertzel 1958 finalization giving exact hop-to-hop phase, audit 08); references below ≈ 86 Hz use a 4096-sample window, the long-window rule
- **Coarse spectral readout** (`peaks::coarse_read`, folded into the strobe): a bounded, ordered-statistic CFAR-gated (Rohling 1983) magnitude search at the nominated reference partial — the strobe's out-of-range fallback when the phase band aliases (report 0011)

`rigaud.rs`, `giordano.rs` and `whittaker.rs` each hold one cited method, composed by `curves.rs` — the same shape `discovery.rs` has over `twm.rs`, and for the same reason: a cited method stays pure and auditable in its own file, and the orchestrator is where they are combined.

## `tuner-gui`

The reference frontend. It is the half that separates if the crate split happens;
where that boundary falls is still open (`TODO.md`).

### Codemap

```text
tuner-gui/src/
├── advisory.rs                   which curve flags mark a measurement suspect; their wording
├── app.rs                        the iced app: Message, TunerApp's state, AppDisplayData
├── app/
│   └── strobe.rs                 strobe lock, the references sent to the bank, strobe state
├── calibration.rs                state and per-tick logic behind the three threshold panels
├── library.rs                    where profiles, settings and capture dumps live; the listing
├── main.rs                       entry point
├── session.rs                    the open instrument's profile, its file, when it is written
├── views.rs                      the screens, each arranging widgets into a panel or page
├── views/
│   ├── curve_select.rs           curve gallery (a)–(d), (b) and (c) withheld; sets the curve
│   ├── inspector_view.rs         one key's measurements, the curve's verdict, drop or re-measure
│   ├── library_view.rs           saved profiles, and the open one's identity form
│   ├── main_view.rs              the tuning screen: sidebar, then two columns of panels
│   ├── rms_calibration.rs        silence-threshold panel: the room's level against it
│   ├── settings_view.rs          settings sidebar and the panel each entry opens
│   ├── sidebar.rs                control column: button groups, capture and undo
│   ├── sustain_calibration.rs    sustain-stability panel: the live trace against it
│   └── transient_calibration.rs  onset-threshold panel: spectral flux against it
├── widgets.rs                    the drawing surfaces the views compose
└── widgets/
    ├── cent_meter.rs             needle on a ±50 ¢ bar, coloured by distance from target
    ├── curve_plot.rs             a curve's d(m) across the compass; sparkline, key picker
    ├── envelope.rs               scrolling smoothed-RMS trace against the silence threshold
    ├── guitar_strings.rs         six open-string buttons, a debug stand-in for the keys
    ├── piano_keyboard.rs         88 keys: detected, selected, doubted measurements marked
    ├── seismograph.rs            scrolling trace of one metric against its threshold
    ├── spectrum_plot.rs          the current frame's magnitude spectrum, a bar per bin
    ├── strobe_display.rs         the strobe band: a ring turned by the beat phase
    └── unison_display.rs         a note's resolved strings as markers on a cents axis
```

### Invariants

**GUI layering.** The widgets are stateless renderers, the views compose them,
and `app` is the state hub with no DSP in it. This half binds the frontend
only, and travels with it if the crate split happens (`TODO.md`).
→ [`layering.md`](docs/internals/layering.md)

## `tuner-lab`

The measurement instruments; every mode and what it reproduces is in
[`tuner-lab/README.md`](tuner-lab/README.md).

`tuner-lab` holds the measurement harnesses: a harness that asserts is a test, one
that times is a bench, and one that reports lives here
([`style.md`](docs/internals/style.md), *Where tests live*). It ships in nothing —
`publish = false`, and the workspace's `default-members` leave it out of a plain
`cargo build`. It stays a member of this workspace rather than becoming a repository of its
own: it drives `tuner-core`'s public API and reproduces the reports here, so a
separate repository would pin it to a commit and turn every API change into two.
The two settings above are what keep it out of the product, and they already do.

### Codemap

```text
tuner-lab/
├── src/
│   ├── capture.rs                finds capture sets; names the key a capture belongs to
│   ├── curve/                    cargo lab curve: the tuning-curve engines
│   │   ├── auralize.rs           each engine's curve rendered to WAV, to judge by ear
│   │   ├── compare.rs            all four engines on one regen dump, side by side
│   │   └── mod.rs                mode list and dispatch
│   ├── engine/                   cargo lab engine: discovery and the auto lock
│   │   ├── dump.rs               what the STFT, peak picker and TWM saw, per frame
│   │   ├── lock.rs               the shipped auto lock, end to end; also from-onset
│   │   ├── mod.rs                mode list and dispatch
│   │   ├── nsga2.rs              synthetic dataset and fitness for the NSGA-II sweep
│   │   └── reach.rs              how far a note can be detuned and still be named
│   ├── figure.rs                 the PNG canvas and the pieces the charts share
│   ├── gatekeeper/               cargo lab gatekeeper: the signal validator
│   │   ├── dump.rs               per-frame gate metrics to gatekeeper.csv
│   │   ├── mod.rs                mode list and dispatch
│   │   ├── plot.rs               the gate's verdict drawn, and the wait to Stable
│   │   ├── replay.rs             one pass through the gate, shared by dump and plot
│   │   └── sparsity.rs           our sparsity ratio against Mounir's NINOS² variants
│   ├── gates/                    cargo lab gates: the detection thresholds
│   │   ├── ambient.rs            the ambient σ at the three hot-path gates that use it
│   │   ├── coarse.rs             the coarse read's gate: profile, pfa, refset, verify, ab
│   │   └── mod.rs                mode list and dispatch
│   ├── main.rs                   CLI: the six subsystems
│   ├── mat/                      cargo lab mat: the Worker's (f₀, B) estimator
│   │   ├── mod.rs                mode list and dispatch
│   │   ├── recovery.rs           MAT against known synthetic B, 1×–25× the prior
│   │   ├── regenerate.rs         regen: per-key partials re-derived from kept audio
│   │   ├── repeats.rs            capture-to-capture noise in what the curves consume
│   │   └── validate.rs           measured B per key against the prior; also offset
│   ├── raw.rs                    reads the raw f32 capture dumps
│   ├── regen.rs                  the mat regen schema, and the rules for consuming it
│   ├── strobe/                   cargo lab strobe: the bank and the displayed reading
│   │   ├── isolation.rs          the unison panel against mute-isolation truth
│   │   ├── mod.rs                mode list, dispatch, and the shared bank driver
│   │   ├── readout.rs            the displayed reading against truth: twelve modes
│   │   └── replay.rs             the shipped bank over real captures
│   └── truth.rs                  the offline reference: hi-res DFT truth, YIN, gate replicas
└── scripts/                      Python drivers and post-processing
    ├── audit_captures.py         audits the regenerated repeat-capture set
    ├── isolation_truth.py        the isolation set's truth side, beside strobe isolation
    ├── optimize_twm.py           Optuna NSGA-II driver over engine nsga2 --serve
    ├── plot_curves.py            draws curve compare --json as one image
    ├── plot_engine.py            draws one capture's goertzel.csv
    ├── replay_lock_rules.py      M-of-N lock-rule replay over cached dumps
    ├── test_engine_all.py        gate and engine dumps over a set; lock pass/fail
    └── validate_config.py        scores a TWM constant set against the captures
```

### Invariants

**The lab reads `tuner-core`'s public API, and never `tuner-gui`.** A harness
that reached into `tuner-gui` would measure the frontend's copy of a decision
rather than the core's: a decision worth replaying offline belongs in
`tuner-core`, where the app and the lab read the one implementation. A harness
that needs something private has found a `tuner-core` API question.

**The capture sets are validation-only.** Two instruments cannot select a
configuration, and only `capture-sets.md`, the reports and the audits quote a
raw tally; this file, the README and the rest of `docs/internals/` state results
without one. The one contract that binds claims rather than code.
→ [`capture-sets.md`](docs/internals/capture-sets.md),
[`CONTRIBUTING.md`](CONTRIBUTING.md)

## Status

How settled each module is, and whether more work is expected. Every one is built
and running; what each is waiting on is in [TODO.md](TODO.md).

| Module | Status |
| --- | --- |
| `pipeline.rs` — AudioPipeline orchestrator, shared atomics, capture lifecycle | 🟢 Stable |
| `gatekeeper.rs` — signal validator (Silence / Unstable / Stable), pure DSP | 🟢 Stable |
| `engine.rs` — TWM discovery + Goertzel phase tracking | 🔬 R&D |
| `worker.rs` — background (f₀, B) measurement (single thread) | 📐 Provisional |
| `algorithms/curves.rs` + `rigaud.rs`/`giordano.rs`/`whittaker.rs` — tuning-curve engines (a)–(d) | 📐 Provisional |
| `strobe.rs` + `strobe/` — manual-mode strobe: beat phase and its band-slope rate | 📐 Provisional |
| `synth.rs` — offline curve → audio resynthesis (cold path, no audio stream) | 📐 Provisional |
| `algorithms/peaks.rs` — peak extraction and masking, the coarse read, the line search | 🧩 Extensible |
| `algorithms/twm.rs` — Two-Way Mismatch scoring | 🔬 R&D |
| `app.rs` + `app/` — GUI state hub | 🧩 Extensible |

**Legend** — **Mature**: solved, unlikely to change · 🟢 **Stable**: complete for
its role · 📐 **Provisional**: functional now, but a required feature is not built
on it yet · 🧩 **Extensible**: works today, could grow, no commitment either way ·
🚧 **In Development**: functional, more features actively coming · 🔬 **R&D**:
algorithm still being developed and validated. (The two "done" tiers follow the
PyPI convention, where Mature ranks above Stable.)

Orchestration is settled. The **curve layer** and the **strobe** are complete and
wired together, and stay Provisional until a professional tuner has judged the
result on an instrument. **synth** is cold-path: it renders a curve to audio
offline and owns no audio stream. The **Engine** and **twm** are the open research front — discovery
locks reliably, but the deep bass is inharmonicity-limited and gated on an in-tune
instrument ([report 0006](reports/0006-discovery-refinement-validation.md)). The
**Gatekeeper** is stable, with three open items in `TODO.md`: its onset and bypass
flags are one signal, silence closes it off from its own metrics, and the silence
threshold standing in for a noise floor is the next major piece of work.

## Pointers

| Doc | Purpose |
| --- | --- |
| [README.md](README.md) | What the project is, what it does, how to build and run it |
| [docs/internals/](docs/internals/) | The binding contracts: crossings, hot path, layering, style, capture sets |
| [reports/](reports/) | The reports, the methods standard, the audits, and the report index |
| [reports/tuning-curve-grounding.md](reports/tuning-curve-grounding.md) | What the tuning curve rests on, and how strongly |
| [docs/design/](docs/design/) | Proposals still being argued, and exploratory sketches |
| [TODO.md](TODO.md) | The backlog, with what each item is blocked on |
| [CONTRIBUTING.md](CONTRIBUTING.md) | How to contribute: build, lint, and the evidence rules |
| [LICENSE](LICENSE) | Licensing |
