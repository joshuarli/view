# view

A native macOS image and video browser built with SwiftPM.

- Build: `swift build`
- Release: `make release` (packages `View.app` via `scripts/build-app.sh`)
- Test: `./scripts/test.sh` (loads the Command Line Tools' Swift Testing plugin)
- Package: `./scripts/build-app.sh`; launch: `./bin/view [directory]`
- Performance measurements and limits: [docs/performance.md](docs/performance.md)
- Architecture and module overview: [docs/architecture.md](docs/architecture.md)
