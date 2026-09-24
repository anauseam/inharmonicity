# Inharmonicity: An Electronic Tuner for Inharmonic Stringed Instruments

![Inharmonicity Interface](images/interface-screenshot.png)

Inharmonicity is an open-source tuner built for the measurement and tuning of **stiff stringed inharmonic** instruments. Inharmonic instruments are not traditionally tuned via equal temperament (ET), and must be tuned in a "stretched" manner, where each key is tuned to a specific pitch that is slightly sharp or flat relative to ET. When measuring an instrument the program generates a tuning curve based on the characteristics of each key. Then, using real-time pitch tracking, the user can tune a key against this curve.

The **current focus is the piano**, though the measurement and the strobe tuner are designed to **generalize to any stiff string**. Full-piano validation is ongoing, and other stiff stringed instruments will benefit from the work in the future.

For a detailed WIP overview of the algorithms used, see the [Anauseam documentation](https://docs.anauseam.org/project-docs/inharmonicity-tuner).

> [!IMPORTANT]
> **Project Status — pre-release, and usable**
>
> The full path for piano tuning is built and working: measure each note, compute
> the instrument's stretch curve, tune each string to it on a strobe tuner. No binary has
> shipped yet, and the efficacy has not been validated by a professional. Treat the project as a capable alpha rather than a finished product.
>
> Current limits:
>
> - **Manual mode only.** You pick each key yourself. Captures taken in Auto mode
>   are kept but never count toward the curve, and the strobe tuner needs a selected key,
>   so a whole compass captured in Auto gives a curve with no measurements in it.
> - **A suspect capture is caught by eye, not automatically.** The measurement
>   inspector shows every retained measurement of a key and lets you drop or
>   re-measure it, and the curve marks the keys it doubts, but the app never
>   rejects a capture on its own.
> - **The top of the keyboard is modelled, not measured.** Up there a string
>   radiates almost nothing but its fundamental, so there are no overtones left to
>   measure its inharmonicity from and the curve follows a standardized treble model that most pianos share
>   instead. That is a limit on what can be heard, not a fault in the piano, but
>   it does mean the app cannot check the model against the instrument in front of
>   it, and the top octave's targets rest on it.
> - **Which curve is "best" is not settled.** *Curve Select* offers three curves.
>   They agree through the middle of the keyboard but pull well apart in the
>   lowest octave, and no measurement will ever say which is right since it can be subjective. *Multi-interval ·
>   Balanced* is the default.
> - **No pitch-raise over-pull targets.**
> - **A440 only**, no user-adjustable temperaments. On a piano sitting well below
>   pitch the app therefore prescribes a full pitch raise with no way to tune the
>   instrument to itself instead.
>
> The full backlog, with what each item is blocked on, is in [TODO.md](TODO.md).

## Getting Started

### Building and Running

Building needs a Rust toolchain (`rustup`, stable). On Linux it also needs
`pkg-config` and the ALSA development files the audio library links against:
`libasound2-dev` on Debian and Ubuntu, `alsa-lib-devel` on Fedora.

```bash
# Clone the repository
git clone https://github.com/anauseam/inharmonicity.git
cd inharmonicity

# Build the project
cargo build --release

# Run the GUI application
cargo run --release -p tuner-gui
```

Build in release. A debug build is too slow to keep up with the microphone and
drops audio.

> [!NOTE]
> **Pre-built binaries are coming.** Building from source is the only route
> today. Now that the measure → curve → strobe path is complete, a tagged
> release with a compiled executable is planned so the tuner is usable without
> a Rust toolchain.

### Tuning a piano

Work in **Manual mode**: only a key you selected counts toward the tuning
curve. Automatic note detection is still being validated, so a capture taken in
Auto mode is kept but left out.

1. **Calibrate.** The app listens to the room for a few seconds at launch to
   learn how loud it is; keep quiet until `Calibrating…` clears.
2. **Select a key** on the *Key select* keyboard (clicking it again returns to
   automatic detection).
3. **Measure.** Enable *Measurement Mode* (it arms straight away) and strike
   the note. The capture button tracks the take: *Armed* → *Capturing…* →
   *Processing…* → *Ready*. Press *Ready* to arm the next one; pressing it while
   *Armed* cancels instead. *Undo* takes back the last capture.
4. **Watch the curve form** in the *Curve Plot* panel (unmeasured keys follow
   the model, so it settles as the compass fills in).
5. **Tune** in the *Strobe* panel: stationary band = in tune, direction = sharp
   or flat, and it shows a cents number instead when the string is too far off
   to read. The curve locks on entry so targets cannot shift mid-pass.

Measurements autosave to the open instrument as they land and the curve is recomputed rather than stored. The open instrument is
named beside the title; **Settings → Instrument Library** switches or creates
one, so check the name before measuring a different piano.

If for some reason you want to tune to equal temperament (using a different instrument), switch the sidebar's reference toggle to **Ref: ET**
for a pure equal-temperament strobe with no stretch curve.

Every capture also writes its audio and its analysis to a diagnostics folder in
the app's data directory — `~/.local/share/inharmonicity/diagnostics/` on Linux,
the equivalent on macOS and Windows — one folder per instrument. The exact path
is printed at startup. That is there for development; nothing in the app reads it
back, and nothing clears it, so it grows until you delete it.

## Interface

Everything below is built and in the app today; planned features live in
[TODO.md](TODO.md).

### Features

#### Measurement

- **Per-note inharmonicity measurement** — a struck note is captured and
  analysed for how far its overtones stray from whole-number multiples of its
  fundamental.
- **Measurement inspector** — every capture kept for a key, with the curve's
  verdict on each; drop one, or take the key again. *Undo* reverts the last
  capture.
- **Suspect-measurement flags** — a key whose measurement disagrees with the
  octave around it is marked on the keyboard, the curve and the strobe, and left
  out of the curve.

#### Tuning curve

- **Per-instrument stretch curve** — built from the piano's own measurements,
  with unmeasured keys following a model. It redraws after every capture, and
  doubles as a key picker.
- **Curve selection** — three curve styles side by side: a smooth model of the
  whole piano, a balanced curve that puts octaves first (the default), or one
  that favours pure twelfths.

#### Tuning

- **Strobe tuner** — a band that stands still on target and drifts one way for
  sharp, the other for flat. Measured keys are read on the overtone chosen for
  them, unmeasured keys on the fundamental, and a string too far off for the band
  to follow gets a cents readout instead.
- **Curve lock** — targets freeze for the length of a tuning pass, so a later
  measurement cannot move a string already tuned.
- **Cent meter and live spectrum** — color-coded distance from the target,
  beside the current frame's spectrum.
- **ET reference mode** — plain equal temperament with no stretch curve, for any
  instrument.

#### Instruments

- **Instrument library** — make, model, serial number, owner and notes per
  piano, with search, sort, duplicate and delete.
- **Autosave** — measurements, the chosen curve and the instrument's settings are
  written as they change, and the last instrument reopens at launch.

#### Calibration

- **Room check** — run at launch to learn how loud a sound must be to stand out
  from the room, re-runnable or settable by hand.
- **Detection thresholds** — live traces for how sharp a strike must be to count
  as one and how steady a tone must be before it is measured, each with a slider,
  both saved with the instrument.

> [!TIP]
> **Graphics Issues? Check Your Vulkan Drivers**
>
> This application uses `iced` with the `wgpu` backend (Vulkan on Linux). If you experience
> invisible widgets, flickering, or blank panels, the most common cause is stale or
> incompatible Vulkan drivers. Ensure your GPU drivers are fully up-to-date before
> reporting rendering bugs.

## For developers

The workspace is three crates. **`tuner-core`** is the headless engine — audio
input, the real-time DSP pipeline and the background measurement — with no GUI
dependency, so any frontend can drive it. **`tuner-gui`** is the frontend built
on it ([its README](tuner-gui/README.md)). **`tuner-lab`** holds the measurement
harnesses that reproduce the reports, and ships in nothing.

- [ARCHITECTURE.md](ARCHITECTURE.md) — the codemap, the threading model, how
  settled each module is, and the decisions that shape the system.
- [reports/](reports/) — the measurements those decisions rest on, indexed by
  what each one decided.
- [CONTRIBUTING.md](CONTRIBUTING.md) — how to propose a change, and the rules a
  change resting on a measurement has to meet.
- [tuner-lab/README.md](tuner-lab/README.md) — the harnesses, and the capture
  format they read.
- [TODO.md](TODO.md) — the backlog, with what each item is blocked on.

## License

Mozilla Public License 2.0. See [LICENSE](LICENSE).
