# Architecture

```
Terminal: ./bin/view [directory]
          |
          v
      View.app (NSApplication, one NSWindow)
          |
          v
   Recursive Media Scanner (off-main, UTType .image/.movie)
          |
          v
   In-memory Media Catalog (flat, ordered, @MainActor)
          |
          v
    NSCollectionView (FlowLayout, ~160pt cells)
          |
          +-- Thumbnail Coordinator
          |      |
          |      +-- Bounded request scheduler (4 local, 2 remote)
          |      +-- QLThumbnailGenerator (unified image/video)
          |      +-- Memory-only NSCache (~96 MiB)
          |
          +-- QLPreviewPanel (Space)
          |
          +-- NSWorkspace.recycle (Ctrl+Delete)
```

The app owns no persistent database. Files trashed by the app are removed from the in-memory catalog; there is no watcher or rescan.

## Modules

`viewCore` has no AppKit dependency:

- `Discovery/DirectoryScanner.swift`: iterative depth-first traversal. Sorts entries before descending so batches arrive in stable natural relative-path order. Skips symlink directories, hidden entries, and packages. Accepts `UTType` conformances to `.image` and `.movie`, never audio-only. Emits immutable `Sendable` batches of 128 and obeys cancellation.
- `Discovery/PathOrder.swift`: numeric, case-insensitive natural ordering with a UTF-8 tiebreak.
- `Discovery/MediaItem.swift`: file identity plus kind, modification date, and size. No pixels.
- `RootDirectory.swift`: resolves the CLI argument to an absolute URL, expanding `~`. Rejects missing and non-directory roots with a terminal error.
- `Catalog.swift`: flat ordered array plus stable selection by URL. Append preserves selection; removing the selection picks the next item, else the previous; removing the last item yields the empty state.
- `PreviewSession.swift`: stable snapshot of URLs plus the selected index for Quick Look. Survives scan appends and trash removals by identity, never by stale index.
- `Trash.swift`: `TrashService` guards one in-flight operation per URL and only removes from the catalog after `recycle` confirms.
- `Thumbnails/ThumbnailScheduler.swift`: priority queue with visible first, one viewport of scroll-direction prefetch, deduplication, per-key generations to discard stale completions, and cancellation propagation. Advances on demand completion, never on a timer.
- `Thumbnails/ThumbnailCache.swift`: `NSCache` with explicit byte-cost accounting and a 96 MiB default limit. Keys include path, pixel size, and scale.
- `Thumbnails/ThumbnailKey.swift`: request identity.
- `GridCommand.swift`: Space and Ctrl+Delete shortcuts.

`view` is `@MainActor` AppKit:

- `ViewApp.swift`: owns `NSApplication`, the window, the main menu, and shutdown. Termination waits for scan cancellation, thumbnail cancellation, and any pending trash.
- `Grid/GridViewController.swift`: `NSCollectionView` data source, delegate, and prefetching. Incremental inserts per batch, selection by identity, demand updates from visible plus scroll-direction prefetch. Forwards Quick Look responder-chain methods to the preview controller and handles Space and Ctrl+Delete in `keyDown`.
- `Grid/ThumbnailCell.swift`: reusable item with aspect-fit image, play badge for movies, and selection tint. Never does I/O.
- `PreviewPanelController.swift`: owns the `PreviewSession` and the `QLPreviewPanel` data source and delegate. Space toggles via menu and `keyDown`; `updateController` refreshes the responder-chain controller on selection change and before showing. `previewPanel(_:handle:)` handles Space, Escape, and Ctrl+Delete while the panel is key. Selection stays consistent in both directions.
- `ThumbnailCoordinator.swift`: bridges demand to the scheduler at the current backing scale, serves cache hits, and records lightweight `VIEW_METRICS` signposts.
- `TrashController.swift`: runs one trash operation at a time and surfaces failures as sheets.

## Concurrency

UI, catalog, scheduler bookkeeping, and Quick Look state are `@MainActor`. Traversal and Quick Look decoding run off-main; batches cross as `Sendable` values. No `@preconcurrency`, no `nonisolated(unsafe)`, no shared mutable globals, no polling loops. The build uses Swift 6 language mode with warnings as errors.
