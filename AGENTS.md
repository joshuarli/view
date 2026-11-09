# AGENTS.md

`view` is a native macOS image and video browser in Rust (AppKit via `objc2`), and the only Swift in this repository is historical naming in comments.

Build: `cargo build --profile dist --bin app --features app`
Test: `cargo test --profile dist --test ui_parity` (ports the Swift `viewUITests` scenarios; see the Testing section of [docs/architecture.md](docs/architecture.md)) and `cargo test --profile dist` for the full suite.
Package: `sh scripts/package-app.sh`; launch: `./target/dist/app [directory]`
Performance baselines: [docs/performance.md](docs/performance.md)
Architecture and module overview: [docs/architecture.md](docs/architecture.md)

Invariants: one `NSWindow` owns browsing; the catalog is in-memory and authoritative (trash derives visibility from it); `decode_png` is the pure-Rust pixel path, `decode_png_native` is ImageIO rendering.
