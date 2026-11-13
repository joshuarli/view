# Architecture

One AppKit window hosts multiple tabs. Each `Tab` owns a root directory, browser mode, catalog, selection, navigation history, preview session, and availability state. `AppState` owns the tabs, the active index, one session-wide trash history, and the set of in-flight trash operations. A shared grid and carousel render the active tab.

## Catalog and navigation

`list_browser` and `scan_recursive` in `src/lib.rs` discover media. Browser listings put directories first by natural name, followed by files in descending size order. Recursive traversal visits child directories through a stack and emits each directory's files in reverse natural name order. It does not globally sort relative paths.

Navigation synchronously replaces a tab's catalog through `load_listing` and `show_listing` in `src/bin/app.rs`. `load_listing` carries a path-aware `ListingError` from `src/lib.rs` on failure and applies the tab's media filter — images, videos, or both — which folders always bypass. A failed listing keeps the previous catalog and marks the tab unavailable; the grid shows the error instead of a listing. There are no incremental scan batches, scan generations, or background directory workers. `after_navigation` updates the title, layout, Quick Look, scroll position, and tab bar.

After a trash mutation rebuilds a tab's catalog, the selection advances to the item that followed the deleted focus, clamped to the surviving listing; the grid document view shrinks to the new content size and the scroll origin is clamped into it.

`Catalog` stores selection membership, anchor, and focus by path. `PreviewSession` supplies Quick Look's URL list and selected index. Panel navigation updates catalog focus. Selection membership is sparse after insertion and across trash/undo cycles. Highlighting, dragging, rename reconciliation, and trash targets use membership; anchor and focus remain only for range gestures and restoration ordering.

There is no persistent catalog or filesystem watcher. Navigation rereads a directory; merely activating a tab keeps its existing catalog. `NSUserDefaults` stores the chosen view mode, the media filter, and `NSWindow` uses a frame autosave name.

## Images and video

The application owns `ThumbStore` in `src/bin/app.rs`, with FIFO eviction against `THUMBNAIL_CACHE_LIMIT` (96 MiB of estimated image data). Image and movie thumbnail keys include path, requested pixel size, file size, and modification time. Movie request bookkeeping and completion paths carry the same version; completions reject results for files whose version changed in flight and evict superseded versions of the same path.

Still images use ImageIO thumbnails with orientation transforms. `draw_rect` gathers visible cache misses; `decode_cg_thumbnails` joins concurrent decodes before drawing continues. A single preview miss decodes synchronously. This work therefore blocks the main event loop until it finishes.

Movies use asynchronous `QLThumbnailGenerator` requests, deduplicated by path, pixel size, and file version. Requests have no app-managed concurrency limit or navigation cancellation. Completions consult the file's current version before caching and refresh the current carousel selection.

`src/bin/app/player.rs` owns one reusable `AVPlayer`, its view, the selected item, and KVO observation. Loading a different movie removes the old item's status observer and replaces the player item. The poster stays visible until both the item and its display are ready. Space toggles playback or pending autoplay; a failed item can fall back to Quick Look. Grid mode and non-video selections release the item. Quick Look remains available through E for videos.

The exported `Scheduler`, `ThumbnailCache`, `TrashService`, and PNG decoder helpers in `src/lib.rs` have separate core tests. The application does not use them for thumbnail scheduling, cache ownership, or trash execution. Its still-image path does not call `decode_png` or `decode_png_native`.

## File operations and lifecycle

`trash_selected` calls `NSWorkspace.recycleURLs` with the selected items and receives original-to-Trash mappings asynchronously. Each operation carries its captured inputs; completion applies every returned mapping — successful moves included with a non-null error — removes only the moved files from affected tabs, records one window-wide transaction, shows the error, and keeps failed files in place. Navigation stays available during the move; undo and redo refuse to race a pending move of the same paths. After every trash, undo, and redo, each affected tab's listing is rebuilt from disk; a tab whose root cannot be listed shows an unavailable message instead of a stale or empty listing. The ownership design is in [the ownership design](rfc/VIEW-3-trash-operation-ownership.md).

Rename, undo, and redo call `renamex_np` with `RENAME_EXCL`. Existing destinations are preserved, and cross-volume moves fail. Failed undo/redo entries stay on their original history stack for retry. Rename reloads all tabs, relocates roots and navigation paths, preserves sparse selections by relocating each selected path, updates original paths in trash history, and marks a tab unavailable when its reload fails.

AppKit state belongs to the main thread. Quick Look thumbnail callbacks marshal successful results to the main queue. ImageIO workers own their `CGImage` until joined. The app has no termination coordinator that drains pending trash or thumbnail requests; SIGINT and SIGTERM call `exit(0)`.

AppKit provides native window, menu, focus, drag, Quick Look, and playback controls. Replacing it would require rebuilding those behaviors as well as the custom grid.

## Build and package

Build with the nightly pinned in `rust-toolchain.toml`. The `app` feature selects the native host. The `dist` profile enables fat LTO and retains debug information.

```sh
cargo build --profile dist --bin app --features app --locked
./target/dist/app ~/Pictures
cargo run --profile dist --bin app --features app -- --recursive ~/Pictures
sh scripts/package-app.sh
make install
```

Packaging creates `View.app`, copies `Info.plist` and the executable, and applies an ad-hoc signature. `make install` copies that bundle to `/Applications`; override the destination with `INSTALL_DIR=...`. Launch the installed executable as `/Applications/View.app/Contents/MacOS/view [directory]`. The manually dispatched release workflow publishes an Apple Silicon executable ZIP and checksums.

## Testing

The commands are in [AGENTS.md](../AGENTS.md). Native dependencies and framework linkage require macOS; this repository does not currently provide a Linux test configuration.

`tests/integration.rs` exercises discovery and core catalog, scheduling, and trash helpers. `tests/ui_parity.rs` checks shortcuts, restore conflicts, native drag-item construction, and Quick Look movie thumbnails. Those helper tests do not exercise the application's asynchronous trash callback.

The `app` feature enables `tests/app_regression.rs`. It launches the real AppKit process for navigation, selection, Quick Look, layout, drag/drop, rename, undo/redo, mixed-result trash completion, sparse selection across trash/undo/rename, active-tab file drops, same-path movie replacement, and inline playback scenarios. File restoration uses an isolated simulated Trash. `--player-test` checks initial playback, pause, item changes, and release on grid mode; it does not check replay after the item ends. Test fixtures live beneath `.work/tmp` and child processes are reaped by the test harness.
