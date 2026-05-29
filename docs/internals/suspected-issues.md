# Suspected Issues — Descriptive Context

This file documents **suspected-but-unreproduced** hazards in the codebase and the defensive code that addresses them. Unlike the rules in `docs/internals/`, this file is mainly descriptive and provides historical context for certain workarounds.

---

## CPAL/ALSA shutdown-segfault workaround

**Location:** `tuner-gui/src/app.rs` (the exit-on-close-request path) and `tuner-core/src/audio.rs`.

**What we suspect:** Shutting down the CPAL/ALSA audio stream without a clean drop can trigger a segmentation fault during application exit.

**Why we suspect it:** During early development, forcing an exit without cleanly joining/dropping the stream threads caused a segfault on Linux under ALSA. The GUI's exit-on-close-request path was added defensively to ensure the audio pipeline drops completely before the main process exits.

**Reproduction status:** Unverified against current CPAL/ALSA versions. The fault may no longer reproduce.

**Action if refuted:** If a future maintainer can verify that closing the app via standard means (without the dedicated exit-on-close-request path) no longer triggers a segfault under ALSA, the explicit drop workaround can be removed.
