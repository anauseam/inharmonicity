# Audio Architecture Overview

This document outlines the three-thread architecture of the `Inharmonicity` piano tuning application and how audio data journeys from the microphone to the GUI.

## 3-Thread Model

Inharmonicity has strict requirements: real-time audio capture must never block, CPU-heavy FFT processing must not interrupt audio polling, and the GUI must remain 60fps-smooth for the user. To guarantee this, the application is divided into three completely distinct threads.

### 1. CPAL Audio Thread (Real-time)

- **Role:** Retrieves raw floating-point samples directly from the operating system's audio loop (via ALSA, PulseAudio, etc.) at 44.1 kHz.
- **Strict Rule:** Must NEVER block, wait on a lock/mutex, or allocate memory. Doing so violates real-time deadlines and introduces audio crackling or dropouts.
- **Mechanism:** When the audio callback fires with an slice of e.g., 256 samples, the thread pushes the float values lock-free into the `ringbuf` Producer.
- **Location:** `tuner-core/src/audio.rs`

### 2. Audio Processing Thread (Background Worker)

- **Role:** Performs the heavy-lifting DSP calculations: transforming time-domain samples with FFT, analyzing pitch via PYIN algorithm, and calculating cent deviations.
- **Input Mechanism:** Iteratively sleeps for 5ms until the `ringbuf` Consumer contains at least 2048 samples (`BUFFER_SIZE`). It then pops exactly 2048 samples into a local vector lock-free.
- **Output Mechanism:** Wraps the outputs into an `AnalysisResult` struct and sends it over an unbounded `crossbeam_channel` to the main GUI thread.
- **Location:** Spawned entirely within `start_audio_processing` in `tuner-gui/src/main.rs`.

### 3. Iced GUI Thread (Main)

- **Role:** Handles user input, rendering graphics, and interpreting state.
- **Mechanism:** Polls the `crossbeam_channel` receiver every 16ms (60 FPS tick) to fetch any newly processed `AnalysisResult`s.
- **Location:** `tuner-gui/src/main.rs` and `tuner-gui/src/ui/`.

## Why `ringbuf` and `crossbeam_channel`?

You might wonder why we use a `ringbuf` for the audio capture and a `crossbeam_channel` for the processing results. 

`ringbuf` provides a **lock-free ring buffer** which allocates its memory entirely upfront and utilizes atomic pointers rather than Mutexes. It is standard practice in real-time audio because the OS audio thread can execute `producer.push_slice(...)` efficiently without any possible system blocking latency.

However, moving data from the Processing thread to the UI thread is far less real-time critical. Here, `crossbeam_channel` shines because it is easy, standard, and can serialize complex structures like vectors or string notes cleanly without low-level memory complications, perfectly serving 60 FPS update requirements.

## Buffer Sizing

By allocating the ring buffer in `main.rs` to inherently hold 8x the `BUFFER_SIZE` (a depth of 16,384 samples ≈ 371ms), we construct enough "runway." Even if the operating system randomly schedules the processing thread significantly late (by several hundred milliseconds), the CPAL callback has enough runway space to continue pushing samples lock-free without suffering a buffer overthrow `PushError`.
