# view

A fast native macOS image and video browser, launched from a terminal.

```sh
swift build -c release
./scripts/build-app.sh
./bin/view ~/Pictures
```

`./bin/view` resolves the directory to an absolute path, rejects missing or non-directory roots, and launches `View.app` as a detached native application. With no argument it uses the current directory. Set `VIEW_APP` to point at another `View.app`.

Paths and layout:

- `Sources/viewCore` — AppKit-free logic: launch validation, scanning, catalog ordering, thumbnail scheduling.
- `Sources/view` — AppKit application: window, grid, Quick Look, trash.
- `Tests/viewCoreTests` — Swift Testing suites for `viewCore`.
- `Info.plist`, `scripts/build-app.sh` — app bundle assembly and ad-hoc signing.
- `plan.md` — the build campaign's execution plan.

Requires Swift 6.4 and the macOS 27 SDK (Command Line Tools are sufficient).

Test with `./scripts/test.sh` (it adds the Command Line Tools' Swift Testing macro plugin path; plain `swift test` fails on that plugin without it). Build artifacts go to `.build/`; `View.app` is generated and ignored. Scratch and fixture state belongs under `.work/`.
