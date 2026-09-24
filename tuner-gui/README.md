# tuner-gui

The reference frontend: an `iced` 0.14 application over `tuner-core`. It holds
the program's persistent state — the open instrument, its profile and files, the
app settings — and none of the signal processing: it reads the pipeline's output
and sends it commands over the crossings.

```bash
cargo run -p tuner-gui
```

How to use the app is in the [project README](../README.md).

## Where to look

- **The module map** is the `tuner-gui/src/` table in
  [ARCHITECTURE.md](../ARCHITECTURE.md).
- **The rules this crate keeps** — widgets render and own no state, views
  compose them, `app` is the one state hub and holds no DSP — are the
  `tuner-gui` half of [`layering.md`](../docs/internals/layering.md).
- **How it talks to `tuner-core`** is the UI side of
  [`thread-crossings.md`](../docs/internals/thread-crossings.md).
- **What each module does** is its module doc:
  `cargo doc -p tuner-gui --no-deps --document-private-items --open`.
