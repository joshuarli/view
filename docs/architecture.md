# Architecture

One AppKit window hosts multiple tabs. Each `Tab` owns a root directory, browser mode, catalog, selection, navigation history, preview session, and trash history. `AppState` owns the tabs and active index. A shared grid and carousel render the active tab.

## Catalog and navigation

`list_browser` and `scan_recursive` in `src/lib.rs` discover media. Browser listings put directories first by natural name, followed by files in descending size order. Recursive traversal visits child directories through a stack and emits each directory's files in reverse natural name order. It does not globally sort relative paths.

Navigation synchronously replaces a tab's catalog through `load_listing` and `show_listing` in `src/bin/app.rs`. There are no incremental scan batches, scan generations, or background directory workers. Listing errors currently become empty vectors. `after_navigation` updates the title, layout, Quick Look, scroll position, and tab bar.

`Catalog` stores selection membership, anchor, and focus by path. `PreviewSession` supplies Quick Look's URL list and selected index. Panel navigation updates catalog focus. Selection membership can be sparse after insertion, even though some UI consumers still reconstruct a contiguous range. The open defects are in [TODO.md](../TODO.md).

There is no persistent catalog or filesystem watcher. Navigation rereads a directory; merely activating a tab keeps its existing catalog. `NSUserDefaults` stores the chosen view mode, and `NSWindow` uses a frame autosave name.

## Images and video

The application owns `ThumbStore` in `src/bin/app.rs`, with FIFO eviction against `THUMBNAIL_CACHE_LIMIT` (96 MiB of estimated image data). Still-image keys include path, requested pixel size, file size, and modification time. Movie requests and cached results currently omit file metadata.

Still images use ImageIO thumbnails with orientation transforms. `draw_rect` gathers visible cache misses; `decode_cg_thumbnails` joins concurrent decodes before drawing continues. A single preview miss decodes synchronously. This work therefore blocks the main event loop until it finishes.

Movies use asynchronous `QLThumbnailGenerator` requests, deduplicated by path and pixel size. Requests have no app-managed concurrency limit, generation check, or navigation cancellation. Completions cache the result and refresh the current carousel selection.

`src/bin/app/player.rs` owns one reusable `AVPlayer`, its view, the selected item, and KVO observation. Loading a different movie removes the old item's status observer and replaces the player item. The poster stays visible until both the item and its display are ready. Space toggles playback or pending autoplay; a failed item can fall back to Quick Look. Grid mode and non-video selections release the item. Quick Look remains available through E for videos.

The exported `Scheduler`, `ThumbnailCache`, `TrashService`, and PNG decoder helpers in `src/lib.rs` have separate core tests. The application does not use them for thumbnail scheduling, cache ownership, or trash execution. Its still-image path does not call `decode_png` or `decode_png_native`.

## File operations and lifecycle

`trash_selected` calls `NSWorkspace.recycleURLs` and receives original-to-Trash mappings asynchronously. It currently chooses the first tab whose catalog contains a mapped path and records that tab's undo transaction. A non-null error causes the completion to return without applying its successful mappings. These limitations and cross-directory undo behavior are covered by the [draft ownership design](rfc/VIEW-3-trash-operation-ownership.md).

Rename, undo, and redo call `renamex_np` with `RENAME_EXCL`. Existing destinations are preserved, and cross-volume moves fail. Failed undo/redo entries stay on their original history stack for retry. Rename reloads all tabs, relocates roots and navigation paths, and updates original paths in trash history.

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

The `app` feature enables `tests/app_regression.rs`. It launches the real AppKit process for navigation, selection, Quick Look, layout, drag/drop, rename, undo/redo, and inline playback scenarios. File restoration uses an isolated simulated Trash. `--player-test` checks initial playback, pause, item changes, and release on grid mode; it does not check replay after the item ends. Test fixtures live beneath `.work/tmp` and child processes are reaped by the test harness.
