# view

A native macOS browser for images and videos in a directory. It opens one window with a flattened thumbnail grid of every supported image and video found recursively beneath the supplied root. There is no sidebar, folder tree, library, or chrome.

```sh
make release
./bin/view ~/Pictures
```

## Interaction model

| Interaction | Behavior |
| --- | --- |
| Launch with directory | Recursively discover and display media |
| Click thumbnail | Select item |
| Double-click thumbnail | Toggle native macOS Quick Look |
| Shift+Click | Select a range of items |
| Arrow keys | Navigate the grid |
| Space | Toggle native macOS Quick Look |
| Delete or Ctrl+Delete | Move the selection to Trash |
| Escape | Close Quick Look, when open |
| Cmd+Q | Quit |

Normal trackpad and mouse scrolling and window resizing work. Quick Look handles image display and video playback, zooming, and adjacent-item navigation. If the directory contains no media, an empty state shows the root path instead of opening Finder.

Without a directory, the launcher opens the current directory. It expects `View.app` in the repository; set `VIEW_APP` to use another app bundle. Trashing plays the system trash sound.

## Non-goals

Directory browsing, search, tagging, libraries, editing, custom video playback, slideshows, AI indexing, preferences, background daemons, network access, and external dependencies are out of scope. The only destructive operation is moving a file to Trash, which follows Finder semantics and is recoverable.

## Layout

- `Sources/viewCore`: scanner, catalog, thumbnails, trash, preview session. See [docs/architecture.md](docs/architecture.md).
- `Sources/view`: AppKit window, grid, Quick Look panel, coordinators.
- `Tests/viewCoreTests`: Swift Testing suites for discovery, scheduling, catalog, trash.
- `scripts/build-app.sh`: packages `View.app` from a release build. `make release` runs it.
- `bin/view`: terminal entry point. Resolves the root to an absolute URL and opens the app.
- `docs/performance.md`: design budgets and measured baselines.

Build with `swift build -c release`, test with `./scripts/test.sh`, package with `make release`.
