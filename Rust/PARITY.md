# Parity inventory — Rust port

Reference: the Swift app (`Sources/view`, `Sources/viewCore`) plus its
Tests (`Tests/viewCoreTests`, `Tests/viewUITests`).

## Ported (core, unit-covered)

- natural ordering, browser ordering (directories first, size descending)
- browser listing and recursive scan (extension-based type detection)
- Catalog append/select/range/insert/remove (Swift order/selection contract)
- LaunchOptions + RootDirectory resolution (tilde, `--`, unknown options)
- GridCommand/Undo/Redo shortcuts
- PreviewSession snapshot/update
- TrashService move/restore errors + TrashHistory stack
- Bounded ThumbnailCache (cost + FIFO eviction)

## Ported (GUI, `src/bin/app.rs`, `objc2-app-kit`)

- Window + scroll view + custom grid view (grid + carousel modes, full-path
  title, view-mode persistence in NSUserDefaults, autosaved frame
  `ViewWindow`).
- Carousel strip with preview region; edge-follow scroll-into-view on
  keyboard/mouse selection.
- Keyboard map: arrows (row-step), Shift+Left/Right history, Backspace
  (up a level), Cmd+Backspace (trash), Cmd+Z / Cmd+Shift+Z (undo/redo),
  Space (Quick Look via `qlmanage -p`), Return/double-click (enter dirs).
- Right-click context menu: Open, Show in Finder, Copy Path, Move to
  Trash, Undo, Redo.
- Edit menu (Undo/Redo/Move to Trash with key equivalents) + View menu
  (Carousel/Grid) + app menu (Quit).
- Drag-out via `NSDraggingSession` (pasteboard file URL item).
- Trash via `NSWorkspace recycleURLs:`, undo/redo restore through
  `TrashHistory`, no play badges.
- SIGINT/SIGTERM handlers exit 0 (direct-binary Ctrl-C parity).
- App bundle: `scripts/package-app.sh` builds `dist`-profile binary into
  `View.app` with the repo `Info.plist`, signed ad-hoc.

## Partially ported

- Scheduler: demand deduplication/preemption bookkeeping only; the Swift
  async runner (concurrency limit, generation-stale discards, Task
  cancellation, cache-hit sink) is modelled, not executed end-to-end.
- PNG decode: ImageIO via FFI in benchmarks; the pure-Rust inflater is
  kept for reference only.
- Discovery: package/hidden via extension/dot heuristics (Swift uses
  `URLResourceValues` flags and UTType conformance incl. content-type
  sniffing).
- Preview thumbnails: images decode via `NSImage`; movies show the film
  symbol. `qlmanage -p` covers full Quick Look, but grid movie thumbnails
  are not generated (Swift uses Quick Look thumbnail generation).
- Trash end-to-end tested manually only (recycle/restore path);
  automated UI tests are not ported.

## GUI design

Native AppKit via `objc2-app-kit` was chosen over custom windowing
(winit/egui/Metal): it keeps `NSCollectionView`-equivalent grid behavior,
menus/key equivalents, drag sessions, and focus behavior identical to
Swift, at the cost of unsafe interop and a larger dependency surface.

## Milestones — complete

1. Window + grid/carousel on `objc2-app-kit`, backed by `Catalog` state.
2. Keyboard + scroll-follow + title + view-mode/frame persistence.
3. History, menus (Edit/View), context menu, drag-out.
4. Trash wiring (recycle/restore + undo/redo) and Space Quick Look.
5. Release `View.app` bundle + SIGINT/SIGTERM Ctrl-C parity.

## Verify

```
cargo test --profile dist --lib
cargo build --profile dist --bin app --features app
sh scripts/package-app.sh
./View.app/Contents/MacOS/view <dir>
```
