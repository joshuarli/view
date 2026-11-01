# Architecture

```
Terminal: ./bin/view [--recursive] [directory]
          |
          v
      View.app (NSApplication, one NSWindow)
          |
          v
   Directory Listing (browser, off-main)  |  Recursive Media Scanner (--recursive, off-main)
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
          |      +-- QLThumbnailGenerator (unified image/video/folder icon)
          |      +-- Memory-only NSCache (~96 MiB)
          |
          +-- QLPreviewPanel (Space)
          |
          +-- NSWorkspace.recycle (Cmd+Delete or Ctrl+Delete)
```

The app owns no persistent database. Files trashed by the app are removed from the in-memory catalog; there is no watcher or rescan. Browser mode replaces the catalog on navigation and keeps the thumbnail cache across directories.

## Modules

`viewCore` has no AppKit dependency:

- `Discovery/DirectoryEntry.swift`: shared resource-key set, required-metadata validation, natural-name ordering, and root-read error mapping for both scanners.
- `Discovery/DirectoryLister.swift`: browser-mode traversal of one level. Skips dot-hidden, resource-hidden, package, and symbolic-link entries, emits supported images and movies plus navigable directories, and orders directories first by natural name, then files by descending size with unknown sizes last and natural-name ties. Never computes recursive folder sizes.
- `Discovery/DirectoryScanner.swift`: iterative depth-first traversal. Sorts entries before descending so batches arrive in stable natural relative-path order. Skips symlink directories, hidden entries (resource flag or dot-prefixed name, so hidden directories are never traversed even where volume metadata is unreliable), and packages. Accepts `UTType` conformances to `.image` and `.movie`, never audio-only. Emits immutable `Sendable` batches of 128 and obeys cancellation.
- `Discovery/PathOrder.swift`: numeric, case-insensitive natural ordering with a UTF-8 tiebreak.
- `Discovery/MediaItem.swift`: file identity plus kind (image, movie, or directory), modification date, and size. No pixels.
- `RootDirectory.swift`: resolves the CLI argument to an absolute URL, expanding `~`. Rejects missing and non-directory roots with a terminal error.
- `LaunchOptions.swift`: parses `[--recursive] [directory]` and selects browser mode by default.
- `Catalog.swift`: flat ordered array plus stable selection by URL. The selection is an anchor-first list: plain clicks replace it, Shift+Click extends the inclusive span from the anchor (last plain click). Append preserves selection; removing the last selected entry picks the neighbor at the removal index, else the final item.
- `PreviewSession.swift`: stable snapshot of URLs plus the selected index for Quick Look. Survives scan appends and trash removals by identity, never by stale index.
- `Trash.swift`: `TrashService` guards one in-flight operation per URL and only removes from the catalog after `recycle` confirms.
- `Thumbnails/ThumbnailScheduler.swift`: priority queue with visible first, one viewport of scroll-direction prefetch, deduplication, per-key generations to discard stale completions, and cancellation propagation. Advances on demand completion, never on a timer.
- `Thumbnails/ThumbnailCache.swift`: `NSCache` with explicit byte-cost accounting and a 96 MiB default limit. Keys include path, kind, pixel size, and scale.
- `Thumbnails/ThumbnailKey.swift`: request identity.
- `GridCommand.swift`: Space and Ctrl+Delete shortcuts.

`view` is `@MainActor` AppKit:

- `ViewApp.swift`: owns `NSApplication`, the window, the main menu, and shutdown. Termination waits for scan cancellation, thumbnail cancellation, and any pending trash.
- `Grid/GridViewController.swift`: owns the catalog and both content modes. Browser mode lists one directory, applies each listing atomically, and navigates into folders or to the parent with a scan-generation guard; going up selects the folder it came from. Recursive mode keeps incremental inserts per batch. Both share selection by identity, demand updates from visible plus scroll-direction prefetch, and scroll-into-view after trash and panel navigation. Forwards Quick Look responder-chain methods to the preview controller and handles Space, Return, Delete, Cmd+Delete, Cmd+Up, and Ctrl+Delete in `keyDown` (intercepted at the collection view so the grid has focus).
- `Grid/ThumbnailCell.swift`: reusable item with aspect-fit image, play badge for movies, folder placeholder, selection tint, and name/size captions in browser mode. Never does I/O.
- `PreviewPanelController.swift`: owns the `PreviewSession` and the `QLPreviewPanel` data source and delegate. Space and double-click toggle via menu and `keyDown`; Up/Down step through items while the panel is key. Caches per-item `displayState` so scrubbing back and revisiting restores instantly. `updateController` refreshes the responder-chain controller on selection change and before showing. `previewPanel(_:handle:)` handles Space, Escape, Backspace, Delete, and Ctrl+Delete while the panel is key. In browser mode Backspace closes the panel and Cmd+Delete trashes; in recursive mode Backspace keeps its original trash behavior. Selection stays consistent in both directions.
- `ThumbnailCoordinator.swift`: bridges demand to the scheduler at the current backing scale, serves cache hits, requests folder icons through Quick Look's icon representation, and recreates the scheduler when the local/remote volume class changes. Records lightweight `VIEW_METRICS` signposts.
- `TrashController.swift`: trashes every selected URL in one guarded operation, applies a single batched collection-view removal so a range disappears in one pop, plays the cached system trash sound once on success, and surfaces the first failure as a sheet.

## Concurrency

UI, catalog, scheduler bookkeeping, and Quick Look state are `@MainActor`. Traversal and Quick Look decoding run off-main; batches cross as `Sendable` values. No `@preconcurrency`, no `nonisolated(unsafe)`, no shared mutable globals, no polling loops. The build uses Swift 6 language mode with warnings as errors.
