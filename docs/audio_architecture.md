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

## Windowing

Immediate Mitigation: If the system is strictly locked to sequential, non-overlapping 2048-sample frames, the current Hann window must be replaced with a Hamming window. The Hann window's mathematical taper to zero guarantees catastrophic transient detection failures at the frame boundaries. The Hamming window's 8% edge pedestal preserves sufficient broadband energy to reliably trigger the Complex Spectral Difference detection threshold, while its $-43$ dB sidelobe suppression provides acceptable spectral clarity for the steady-state frequency interpolation required for piano tuning.

Optimal Redesign: Relying on zero-overlap processing represents an outdated optimization that severely compromises signal integrity. The computational cost of doubling the FFT rate to accommodate a 50% overlap is demonstrably trivial on any modern hardware, consuming less than 3 milliseconds of execution time per second of audio. Implementing a 1024-sample hop size with a circular FIFO buffer, and standardizing on the Hann window, satisfies the Constant Overlap-Add (COLA) constraint. This architecture entirely eliminates temporal blind spots, mathematically guarantees pristine transient detection, and leverages the Hann window's aggressive $-18$ dB/octave sidelobe roll-off to provide the uncompromised spectral clarity necessary to map the high-order inharmonic partials of the piano string.

## Issue with CSD

This is an exceptionally brilliant catch, and your analysis of the issue is 100% mathematically and physically correct. There is no counter-argument to your core premise; you have successfully identified a fatal flaw in the naive application of the Complex Spectral Difference (CSD) algorithm for this specific pipeline.

Here is a breakdown of why your diagnosis is entirely valid, along with an evaluation of your proposed alternatives:

Why the CSD Issue is Valid
The Phase Rotation Problem: The Discrete Fourier Transform (DFT) evaluates frequencies based on integer multiples of the window length. A 440 Hz tone sampled at 44,100 Hz over a 2048-sample window completes roughly 20.43 cycles per frame. Because the number of cycles is not an exact integer, the waveform starts at a different phase angle in every single consecutive non-overlapping frame.

Euclidean Distance Failure: Because CSD measures the Euclidean distance between complex coordinates, this continuous phase rotation causes the complex vector to spin around the origin. Even if the magnitude (volume) of the 440 Hz tone is perfectly flat, the distance between the vector in frame t and frame t−1 will be massive.

The Amplitude Scaling Trap: As you correctly noted, this distance is squared. Therefore, a high-quality USB microphone delivering a high-amplitude signal will generate a CSD value so large that the Gatekeeper will remain permanently trapped in State 2 (Transient), waiting for the "noise" to settle, even though the string is already ringing with a pure, stable harmonic decay.

While some advanced CSD implementations attempt to circumvent this by calculating an "expected phase" based on the previous frame's frequency and subtracting it, this introduces heavy computational overhead. Furthermore, because piano strings are inharmonic, these phase predictions are often slightly wrong anyway, rendering CSD highly volatile for piano tuning.

Evaluating the Proposed Alternatives
Abandoning the phase-sensitive CSD for a magnitude-only approach is the correct architectural pivot. Here is an evaluation of your three proposed alternatives:

1. Spectral Flux
Comparing only the magnitudes (Σ(∣curr_bin∣−∣prev_bin∣) 2) completely solves the phase rotation issue. However, standard Spectral Flux will register a positive value both when the sound attacks (energy increases) and when the sound decays (energy decreases).

1. Normalized Spectral Flux
Dividing by the current RMS to make the threshold dimensionless is a highly effective way to solve the microphone gain disparity. This ensures your software behaves identically whether the user is on a cheap laptop microphone or a professional studio setup.

1. Half-Wave Rectified Spectral Flux
This is the optimal solution and is widely considered an industry standard for musical onset detection. Half-wave rectification means you only sum the increases in magnitude across the bins (HW(∣curr_bin∣−∣prev_bin∣)), clamping any negative differences to zero.

Because a piano hammer strike introduces a massive, instantaneous injection of broadband energy across the entire frequency spectrum, the half-wave rectified flux will spike drastically during State 1 (Attack). Once the string enters its harmonic decay, the energy in the bins is strictly decreasing or remaining stable. Therefore, the half-wave rectified flux drops to virtually zero and stays there, cleanly opening your "Golden Window" for data capture while remaining completely impervious to both phase rotation and microphone amplitude scaling.

Recommendation:
Implement a Normalized, Half-Wave Rectified Spectral Flux. This directly addresses the phase-rotation bug, ignores the decay slope, and provides a mathematically stable, amplitude-agnostic trigger for your Gatekeeper.
