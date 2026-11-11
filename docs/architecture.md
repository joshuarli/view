# Architecture

```
Terminal: cargo run --profile dist --bin app --features app [--recursive] [directory]
          |
          v
      View.app (NSApplication, one NSWindow)
          |
          v
   Directory Listing (browser)  |  Recursive Media Scanner (--recursive)
          |
          v
   In-memory Media Catalog (flat, ordered)
          |
          v
    Custom grid view (~160pt cells, carousel strip in carousel mode)
          |
          +-- QLThumbnailGenerator (+ ImageIO fallback via decode_png_native)
          |      |
          |      +-- Bounded request scheduler (visible-first + prefetch)
          |      +-- Pure-Rust PNG decode (decode_png)
          |
          +-- AVPlayerView (inline video in the carousel preview, Space)
          |
          +-- QLPreviewPanel (Space for stills and unplayable items)
          |
          +-- NSWorkspace.recycle (Cmd+Delete or Ctrl+Delete)
```

The app owns no persistent database. Files trashed by the app are removed from the in-memory catalog; there is no watcher or rescan. Browser mode replaces the catalog on navigation and keeps the thumbnail cache across directories.

## Modules

`src/lib.rs` has no AppKit dependency except the `decode_png_native` FFI:

- Discovery: shared natural-name ordering, package/hidden/symlink skipping, extension surface + content sniffing for extensionless files, browser ordering (directories first, size descending) and recursive scan (stable natural relative-path order).
- Catalog: flat ordered array plus stable selection by URL. `selected` is the focus. A range keeps its anchor at the start of `selected_all` and its focus on the clicked path. Append preserves selection; removing the last selected entry picks the neighbor at the removal index, else the final item.
- PreviewSession: stable snapshot of URLs plus the selected index for Quick Look. Survives scan appends and trash removals by identity, never by stale index.
- Trash: `TrashService` guards one in-flight operation per URL and only removes from the catalog after `recycle` confirms. `restore` moves a trashed file back.
- TrashHistory: undo/redo stacks for trash operations. Each transaction records the items, their trash locations, their catalog positions, and the selection before trashing.
- Scheduler: demand bookkeeping (visible first, prefetch, generations, cancel) plus `execute_pending` worker-limited execution returning `(key, generation)` completion pairs.
- ThumbnailCache: byte-costed FIFO-eviction cache with a 96 MiB default limit (`THUMBNAIL_CACHE_LIMIT`). Keys include path, kind, pixel size, and scale. The grid keeps its own decoded images under that same limit.
- GridCommand: Space, Ctrl+Delete, Cmd+Z / Cmd+Shift+Z undo/redo, Shift+arrows history, UpLevel, ActivateDirectory.
- decode_png: pure-Rust 8-bit PNG decode (colour types 0/2/4/6, non-interlaced) via flate2. `decode_png_native` is the ImageIO path via `CGImageSource` + `CGBitmapContext`.

`src/bin/app.rs` is the AppKit binary (feature `app`), with inline-video state extracted to `src/bin/app/player.rs`:

- Owns `NSApplication`, the window, the main menu, and shutdown. Termination waits for scan cancellation, thumbnail cancellation, and any pending trash.
- Custom grid view owns the catalog and both content modes. Browser mode lists one directory, applies each listing atomically, and navigates into folders or to the parent with a scan-generation guard; going up selects the folder it came from. Recursive mode keeps incremental inserts per batch. Both share selection by identity, demand updates from visible plus scroll-direction prefetch, and scroll-into-view after trash and panel navigation. A cell paints a display-sized ImageIO thumbnail kept in the thumbnail cache. When several visible stills miss that cache, they are decoded on a concurrent queue and joined before painting continues. The file-size label comes from the catalog. Handles Space, Return, Delete, Cmd+Delete, Cmd+Up, and Ctrl+Delete in `keyDown`.
- QLPreviewPanel: the panel remains the preview surface for still images and for adjacent-item navigation, and the fallback when inline video playback is unavailable. Space toggles the panel for stills; for movies it opens the panel only when inline playback fails. The panel's items are the current `PreviewSession`, rebuilt from the catalog after the listing changes and closed when that session is empty. Stepping through items updates the grid selection. The grid reflows when the window changes size, and Cmd-minus / Cmd-plus resize thumbnails in both modes. Selected items can be dragged out as file URLs; dropping a folder opens it and dropping a media file selects it.
- Inline video: `src/bin/app/player.rs` owns the carousel preview player state (`AVPlayerView` backed by a reusable `AVPlayer`, item KVO observer, and main-thread-only store). Selecting a movie keeps the existing Quick Look thumbnail as a poster until the `AVPlayerItem` reports ready, then shows the player paused. Space toggles playback; while loading, Space toggles autoplay intent. In grid mode, Space switches to carousel and starts playback for a movie. Navigating to another item, entering a folder, or switching to grid mode releases the item. If the item fails to load, the player stays hidden, the poster remains, and Space falls back to the Quick Look panel.
- Trash: trashes the selection in one guarded operation and records the batch for undo (Cmd+Z) and redo (Cmd+Shift+Z). Undo and redo atomically rename files or folders without replacing an existing destination. Failed moves stay in history for retry; cross-volume moves are reported as failures.
- Rename: the context menu renames one file or folder in its existing directory without replacing another item. Open tabs, navigation, and trash history follow renamed paths.

## Why AppKit

The GUI uses `objc2-app-kit` (the same frameworks the original Swift app used) rather than custom windowing (winit/egui/Metal): it keeps window/menu/key-equivalent/drag-session/focus behavior identical, at the cost of unsafe interop and a larger dependency surface. Any swap reimplements virtualization, text, scroll, menus, Quick Look, accessibility, and window integration from scratch.

## Concurrency

UI, catalog, scheduler bookkeeping, and Quick Look state are main-thread. Thumbnail generation runs through `QLThumbnailGenerator` on its own queue; the demand scheduler executes under worker limits with generation-based stale-discards. State that must outlive the session is committed in the repository; `View.app` is built from the `dist` profile.

## Testing

- `cargo test --profile dist --lib` (unit) and `cargo test --profile dist` (unit + `tests/` integration + UI-parity suites).
- `cargo test --features app --test app_regression` exercises the live AppKit window, navigation and history, image orientation, Quick Look selection, layout, thumbnail completion, inline video playback, file URL drag/drop, and file undo/redo with an isolated simulated Trash. This test is not included unless the `app` feature is enabled. `--player-test` boots the real window, waits for the selected movie's player item to become ready, and asserts that Space toggles playback and that grid mode releases the player.
- `cargo test --profile dist --test ui_parity` checks core shortcuts and trash conflict handling, native drag-item construction, and movie thumbnail generation.
- `cargo build --profile dist --bin app --features app` builds the AppKit binary. [Performance](performance.md) describes the benchmark harness.
- `sh scripts/package-app.sh` builds the signed `View.app`. `View.app/Contents/MacOS/view --self-test <dir>` boots the real window and asserts key handling end-to-end through the live run loop; it exits nonzero on failure.
- SIGINT/SIGTERM exit 0, so Ctrl-C works when the binary is run directly.

This file owns the architecture, module map, concurrency model, and testing notes. Performance and baselines live in [docs/performance.md](performance.md).
