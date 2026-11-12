# AGENTS.md

`view` is a native macOS image and video browser in Rust, using AppKit through `objc2`.

## Commands

- Build: `cargo build --profile dist --bin app --features app --locked`.
- Core tests: `cargo test --lib --locked`.
- App interactions: `cargo test --features app --test app_regression --locked`.
- UI parity: `cargo test --test ui_parity --locked`.
- Full suite when requested: `cargo test --profile dist --features app --locked`.
- Lint: `make lint`. Package: `sh scripts/package-app.sh`.

One `NSWindow` hosts all tabs. Catalog selection uses paths; indexes are presentation positions.
Rename, undo, and redo use exclusive moves that refuse to replace an existing destination.
Keep the video poster visible until the selected item is ready for display.

[Architecture](docs/architecture.md) owns the runtime, build, packaging, and test boundaries.
[Performance](docs/performance.md) owns workloads, targets, and historical measurements.
[Trash operation ownership](docs/rfc/VIEW-3-trash-operation-ownership.md) records the session-owned operations and window-wide undo adopted for file mutations.
