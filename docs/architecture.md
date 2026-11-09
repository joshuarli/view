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
          +-- QLPreviewPanel (Space)
          |
          +-- NSWorkspace.recycle (Cmd+Delete or Ctrl+Delete)
```

The app owns no persistent database. Files trashed by the app are removed from the in-memory catalog; there is no watcher or rescan. Browser mode replaces the catalog on navigation and keeps the thumbnail cache across directories.

## Modules

`src/lib.rs` has no AppKit dependency except the `decode_png_native` FFI:

- Discovery: shared natural-name ordering, package/hidden/symlink skipping, extension surface + content sniffing for extensionless files, browser ordering (directories first, size descending) and recursive scan (stable natural relative-path order).
- Catalog: flat ordered array plus stable selection by URL. Append preserves selection; removing the last selected entry picks the neighbor at the removal index, else the final item.
- PreviewSession: stable snapshot of URLs plus the selected index for Quick Look. Survives scan appends and trash removals by identity, never by stale index.
- Trash: `TrashService` guards one in-flight operation per URL and only removes from the catalog after `recycle` confirms. `restore` moves a trashed file back.
- TrashHistory: undo/redo stacks for trash operations. Each transaction records the items, their trash locations, their catalog positions, and the selection before trashing.
- Scheduler: demand bookkeeping (visible first, prefetch, generations, cancel) plus `execute_pending` worker-limited execution returning `(key, generation)` completion pairs.
- ThumbnailCache: byte-costed FIFO-eviction cache with a 96 MiB default limit. Keys include path, kind, pixel size, and scale.
- GridCommand: Space, Ctrl+Delete, Cmd+Z / Cmd+Shift+Z undo/redo, Shift+arrows history, UpLevel, ActivateDirectory.
- decode_png: pure-Rust 8-bit PNG decode (colour types 0/2/4/6, non-interlaced) via flate2. `decode_png_native` is the ImageIO path via `CGImageSource` + `CGBitmapContext`.

`src/bin/app.rs` is the AppKit binary (feature `app`):

- Owns `NSApplication`, the window, the main menu, and shutdown. Termination waits for scan cancellation, thumbnail cancellation, and any pending trash.
- Custom grid view owns the catalog and both content modes. Browser mode lists one directory, applies each listing atomically, and navigates into folders or to the parent with a scan-generation guard; going up selects the folder it came from. Recursive mode keeps incremental inserts per batch. Both share selection by identity, demand updates from visible plus scroll-direction prefetch, and scroll-into-view after trash and panel navigation. Handles Space, Return, Delete, Cmd+Delete, Cmd+Up, and Ctrl+Delete in `keyDown`.
- QLPreviewPanel: the grid view is the panel's data source. Space and double-click toggle via menu and `keyDown`; the panel steps through items while key.
- Trash: trashes the selection in one guarded operation, plays the cached system trash sound once on success, and records the batch for undo (Cmd+Z) and redo (Cmd+Shift+Z). Undo moves files back and reinserts those still browsed; destination conflicts stay on the undo stack for retry, and permanently missing trashed files are dropped.

## Concurrency

UI, catalog, scheduler bookkeeping, and Quick Look state are main-thread. Thumbnail generation runs through `QLThumbnailGenerator` on its own queue; the demand scheduler executes under worker limits with generation-based stale-discards. State that must outlive the session is committed in the repository; `View.app` is built from the `dist` profile.
