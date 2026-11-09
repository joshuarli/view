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

## Partially ported

- Scheduler: demand deduplication/preemption bookkeeping only; the Swift
  async runner (concurrency limit, generation-stale discards, Task
  cancellation, cache-hit sink) is modelled, not executed end-to-end.
- PNG decode: ImageIO via FFI in benchmarks; the pure-Rust inflater is
  kept for reference only.
- Discovery: package/hidden via extension/dot heuristics (Swift uses
  `URLResourceValues` flags and UTType conformance incl. content-type
  sniffing).
- Undo/redo: core history + service exist; end-to-end restore observes
  the AppKit layer that wires file transactions to the catalog.

## Not ported (GUI, next phase)

- Window/grid/carousel views, scroll-follow, loading spinner, full-path
  title, view-mode and frame persistence.
- Keyboard map (row-step arrows, Space, Backspace/Return/Delete families),
  history, drag-out, context menu, hover/selection polish.
- Quick Look panel (Space) and video preview.
- Menus, focus restoration, Trash window integration (trash sound, sheets).

## GUI design

Two candidates were compared:

1. **Native AppKit via `objc2-app-kit`** — keeps `NSCollectionView`,
   `QLPreviewPanel`, menus/key equivalents, drag sessions, and focus
   behavior identical to Swift. Cost: unsafe interop, ~10 crate
   dependency surface, dev speed.
2. **Custom windowing (winit/egui/Metal, like `~/d/play`)** — avoids
   AppKit but reimplements grid virtualization, text, scroll, menus,
   Quick Look, accessibility, and window integration from scratch.

The AppKit option wins on behavior fidelity and long-term parity; the
custom host wins on dependency weight only. Next phase builds a second
binary (`src/bin/app.rs`) on the AppKit path, wired to the core above,
behind the same `dist` profile as the benchmark.

## Milestones to close the gap

1. Window + `NSCollectionView` grid/carousel on `objc2-app-kit`,
   backing-scale aware, backed by `Catalog`/scheduler state.
2. Keyboard + scroll-follow + focus/loading/empty states + title.
3. History, menus (Edit/View), context menu, drag-out.
4. TrashController wiring (recycle/restore + undo/redo) and
   `QLPreviewPanel` Quick Look.
5. View-mode/window-frame persistence + release `View.app` bundle.
