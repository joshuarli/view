# view

A native macOS image and video browser with folder tabs, a thumbnail grid, and a carousel preview. Folders sort first by natural name. Files follow by descending size, then natural name.

## Install and launch

Download the Apple Silicon binary from [Releases](https://github.com/joshuarli/view/releases), unzip it, and make it executable:

```sh
chmod +x view-aarch64-apple-darwin
./view-aarch64-apple-darwin ~/Pictures
./view-aarch64-apple-darwin --recursive ~/Pictures
```

For a `View.app` bundle, follow [source installation](docs/architecture.md#build-and-package). The bundle declares macOS 27.0 as its minimum version. With no directory argument, the executable browses the current directory. Unknown options produce a `view: ...` error and exit status 1.

Browser mode lists one directory per tab. Recursive mode flattens the directory tree without showing folders. Its current traversal emits files in reverse natural name order within each directory, followed by child directories. Hidden names, symlinks, and recognized package directories are skipped.

## Controls

| Interaction | Behavior |
| --- | --- |
| Click; Shift-click | Select an item; extend the selection from its anchor |
| Double-click or Return on a folder | Enter the folder |
| Arrow keys | Move selection and scroll it into view |
| Backspace or Delete | Move the selection to Trash, including with Cmd or Ctrl held; focus advances to the next item |
| Cmd+Up | Go to the parent folder in browser mode |
| Cmd+Z; Cmd+Shift+Z | Undo; redo Move to Trash |
| Shift+Left; Shift+Right | Navigate back; forward |
| Space on a video | Toggle inline playback in the carousel; from the grid, enter the carousel and request playback |
| Space on another item | Toggle Quick Look |
| E | Toggle Quick Look for the selection, including videos; pauses inline playback |
| Cmd+T; Cmd+W | Open a tab for the current folder; close the active tab when another remains |
| Cmd+minus; Cmd+plus | Decrease; increase thumbnail size |
| Images; Videos; Both | Toolbar control filters the listing to images, videos, or both |
| Drag selected thumbnails | Copy the files to another application |
| Drop a folder or media file | Browse the folder or select the file; the tab bar also accepts drops |
| Right-click an item | Open, show in Finder, copy its path, rename, move to Trash, or undo/redo |
| Cmd+Q | Quit |

The toolbar and View menu switch between carousel and grid. The app remembers the chosen view mode. The toolbar also switches the listing between images, videos, and both; the app remembers that choice too. Folders stay visible in browser mode regardless of the filter. Video previews retain a poster until a frame is ready. While loading, Space toggles the request to play when ready. After a video load fails, Space can open Quick Look instead. Quick Look shows Markup for images and Trim for videos; edits are written to the original file.

Catalogs and trash history last for the current session. There is no filesystem watcher, so changes made elsewhere do not automatically refresh an open listing. Search, tags, a persistent media library, and a custom editor are outside the app's scope.

Report problems through [Issues](https://github.com/joshuarli/view/issues). No license file is included in this repository.
