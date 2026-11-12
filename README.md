# view

A native macOS browser for images and videos, written in Rust on AppKit. It opens one folder at a time: subfolders and supported images and videos in a thumbnail grid, folders first, then files by descending size. Double-click or press Return on a folder to enter it; Backspace goes up. `--recursive` flattens a whole tree into one grid.

```sh
cargo build --profile dist --bin app --features app
./target/dist/app ~/Pictures
./target/dist/app --recursive ~/Pictures
```

## Interaction model

| Interaction | Behavior |
| --- | --- |
| Launch with a directory | Browse that folder |
| Double-click or Return | Enter a folder (browse mode) |
| Click thumbnail | Select item |
| Drag a thumbnail | Drag the file out of the app |
| Right-click | Context menu (Open, Show in Finder, Copy Path, Move to Trash, Undo, Redo) |
| Arrow keys | Move selection; grid/carousel window keeps the selection visible |
| Space | Toggle inline video playback in carousel mode; in grid mode, open the carousel and start playback for a video. For still images, or if a video cannot play inline, Space toggles the Quick Look panel. While a video is loading, Space toggles whether playback starts when ready. |
| E | Open the Quick Look panel for the selection (for a video, Quick Look's editing controls) |
| Backspace | Up one folder (browse mode) |
| Cmd+Backspace or Ctrl+Backspace | Trash the selection |
| Cmd+Z / Cmd+Shift+Z | Undo / redo Move to Trash |
| Shift+Left / Shift+Right | Back / forward through history |
| Cmd+Q | Quit |

Folders sort first by natural name. Files sort by descending size; unknown or equal sizes fall back to natural name order. `--recursive` lists every image and video beneath the root in natural relative-path order.

Carousel mode plays supported videos inline with a reusable `AVPlayer`, keeping a thumbnail poster visible until the player item is ready. Quick Look remains available for still-image preview, adjacent-item navigation, and as a fallback when inline playback is unavailable. Without a directory the launcher opens the current directory, and an unknown option exits with a one-line `view: ...` error. `View.app` can be rebuilt with `scripts/package-app.sh`; set `VIEW_APP` to point the launcher at another bundle.

## Scope

Search, tagging, libraries, editing, slideshows, AI indexing, preferences, background daemons, network access, and external dependencies are out of scope. Inline video playback is limited to the carousel preview and relies on system media frameworks.

## Layout

- `src/lib.rs`: discovery, catalog, thumbnails, trash, preview session, launch options. See [docs/architecture.md](docs/architecture.md).
- `src/bin/app.rs`: the AppKit window, grid, Quick Look panel, menus, drag-out.
- `benches/view.rs` and `src/bin/app/benchmarks.rs`: benchmark harness. See [docs/performance.md](docs/performance.md).
- `tests/`: integration and UI-parity suites.
- `scripts/package-app.sh`: packages `View.app` from a dist build.
- `Info.plist`: bundle metadata.

Build with `cargo build --profile dist --bin app --features app`, test with `cargo test --profile dist`, package with `sh scripts/package-app.sh`.

## Licensing

See `LICENSE` if present; otherwise this personal project is provided as-is.


For a one-shot build-and-install, use `make install` (installs `View.app` to `/Applications` by default; override with `INSTALL_DIR=...`).
