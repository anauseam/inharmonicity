---
description: How to build, test, and lint the inharmonicity workspace
---

# Building and Verifying the Project

// turbo-all

## 1. Full workspace build

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo build 2>&1 | tail -20
```

## 2. Run tuner-core tests

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo test -p tuner-core 2>&1
```

## 3. Run tuner-gui tests (if any)

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo test -p tuner-gui 2>&1
```

## 4. Clippy linting (tuner-core)

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo clippy -p tuner-core -- -W clippy::all 2>&1 | head -40
```

## 5. Clippy linting (tuner-gui)

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo clippy -p tuner-gui -- -W clippy::all 2>&1 | head -40
```

## 6. Check for dead code / unused imports

```bash
cd /home/indigo/Projects/anauseam/programs/inharmonicity && cargo build 2>&1 | grep "warning:" | head -20
```

## Notes

- Always build the full workspace (not just one crate) to catch cross-crate breakage.
- GUI examples can be run individually: `cargo run -p tuner-gui --example <name>`
- The main GUI application: `cargo run -p tuner-gui`
