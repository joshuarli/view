# view

A native macOS browser for images and videos. By default it opens one folder at a time: subfolders and supported images and videos in a thumbnail grid, with folders first and files ordered by descending size. Double-click or press Return on a folder to enter it, and Backspace or Cmd+Up to go back up. `--recursive` instead flattens an entire tree into a single grid.

```sh
make release
./bin/view ~/Pictures
./bin/view --recursive ~/Pictures
```

## Interaction model

| Interaction | Behavior |
| --- | --- |
| Launch with directory | Browse that folder's subfolders and media |
| Double-click or Return on a folder | Enter the folder (browse mode) |
| Double-click or Return on media | Toggle native macOS Quick Look |
| Backspace or Cmd+Up | Go to the enclosing folder (browse mode) |
| Click thumbnail | Select item |
| Shift+Click | Select a range of items |
| Arrow keys | Navigate the grid |
| Cmd+Delete or Ctrl+Delete | Move the selection to Trash |
| Delete (in `--recursive` mode) | Move the selection to Trash |
| Space | Toggle native macOS Quick Look |
| Escape | Close Quick Look, when open |
| Cmd+Q | Quit |

Folders sort first by natural name. Files sort by descending size, with files of unknown size and equal size falling back to natural name order. `--recursive` restores the flattened grid: every image and video beneath the root, in natural relative-path order, where Delete trashes the selection as in earlier versions.

Normal trackpad and mouse scrolling and window resizing work. Quick Look handles image display and video playback, zooming, and adjacent-item navigation. If a folder has no images or videos, an empty state shows the folder path and how to go up. In browse mode, Backspace closes Quick Look while the panel is key.

Without a directory, the launcher opens the current directory. It expects `View.app` in the repository; set `VIEW_APP` to use another app bundle. Trashing plays the system trash sound. `view --help` prints usage.

## Non-goals

Search, tagging, libraries, editing, custom video playback, slideshows, AI indexing, preferences, background daemons, network access, and external dependencies are out of scope. The only destructive operation is moving a file or folder to Trash, which follows Finder semantics and is recoverable.

## Layout

- `Sources/viewCore`: directory listing and recursive scanner, catalog, thumbnails, trash, preview session, launch options. See [docs/architecture.md](docs/architecture.md).
- `Sources/view`: AppKit window, grid, Quick Look panel, coordinators.
- `Tests/viewCoreTests`: Swift Testing suites for discovery, catalog, launch options, scheduling, trash.
- `scripts/build-app.sh`: packages `View.app` from a release build. `make release` runs it.
- `bin/view`: terminal entry point. Resolves the root to an absolute URL and opens the app.
- `docs/performance.md`: design budgets and measured baselines.

Build with `swift build -c release`, test with `./scripts/test.sh`, package with `make release`.
