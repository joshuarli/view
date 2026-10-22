# view — repository rules

- Use Swift 6.4, the macOS 27 SDK, Swift 6 language mode, and warnings as errors. SwiftPM only; no dependencies, no Xcode project, no generators.
- `viewCore` and its tests stay AppKit-free. UI, Quick Look, and trash behavior live in `Sources/view`, main-actor isolated.
- Run `swift build`, `./scripts/test.sh`, and `./scripts/build-app.sh` from the repository root. Package the app with `./scripts/build-app.sh`; launch it with `./bin/view <dir>`. Plain `swift test` cannot find the Command Line Tools' Swift Testing macro plugin, so always use the wrapper.
- Generated app bundles, build products, and all scratch/fixture state stay untracked: `.build/`, `View.app/`, and `.work/`.
- Prefer fewer, larger files with clear isolation boundaries over one type per file.
