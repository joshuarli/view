Prefix: VIEW
Next: 3

### VIEW-1 · P2 · Grid selection anchor and range stay stale after catalog edits

- **Area:** Types
- **Where:** `src/bin/app.rs:52-54`, `src/bin/app.rs:353-358`, `src/bin/app.rs:397-408`, `src/bin/app.rs:435`, `src/bin/app.rs:534-535`, `src/bin/app.rs:787`, `src/bin/app.rs:1090-1095`, `src/bin/app.rs:1100-1102`, `src/bin/app.rs:1948-1949`
- **Problem:** The grid selection is three fields: `selected`, `selection_anchor`, and `selection_range`. A click sets all three (`1100-1102`). Trash removes the item and writes only `selected = 0` (`353-358`). Redo does the same (`435`). Undo inserts the restored items and writes none of them (`397-408`). Launch sets `selected` and leaves the anchor `None` (`1948-1949`). Drawing treats a cell as selected when it is `selected` or when it lies in `selection_range` and the anchor is set (`787`). Drag uses the range whenever the anchor is set (`534-535`). After a click on index 2 and a trash of that item, index 0 and the file that slid into index 2 are both highlighted, and a drag exports the file at the stale index rather than the primary selection. Shift-click then extends from the stale anchor (`1090-1095`): with the anchor past the end of the list, shift-clicking the remaining primary cell highlights every remaining row. The same launch state makes the first shift-click drop the highlighted row and select only the clicked cell, because a missing anchor starts a new range. Eleven other writers assign the three fields together; these four do not. `Catalog` in `src/lib.rs:235` already stores the selection by path and repairs it in `remove`.
- **Recommendation:** Replace the index triple with `Catalog`'s path selection. Route trash, undo, redo, launch, and shift-click through `select`, `select_range`, and `remove` so a catalog edit cannot leave a second cursor behind.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S

### VIEW-2 · P2 · Quick Look keeps a second index and the data source substitutes /

- **Area:** Types
- **Where:** `src/bin/app.rs:446-453`, `src/bin/app.rs:1150-1155`, `src/bin/app.rs:1167-1175`, `src/bin/app.rs:1187-1195`, `src/bin/app.rs:353-358`, `src/bin/app.rs:1948-1949`, `src/lib.rs:854-924`
- **Problem:** Quick Look's current item is `QLPreviewPanel.currentPreviewItemIndex`, copied from `selected.unwrap_or(0)` only when the panel opens (`1153-1154`). The observer copies that index back into the grid only while it is in range (`1168-1175`). Trash, redo, navigation, and launch replace `items` and `selected` and never write the panel index. Space still opens the panel when nothing is selected (`446-453`). The data source then indexes the live `items` vector and, for a missing index, returns the directory `/` (`1193-1195`). Open Quick Look on the last item and trash it: the grid selection becomes 0 while the panel index stays on the old length, and the next preview query for that index is `/`. The same query is `/` for an empty catalog, because launch leaves `selected` as `None` and begin still forces index 0. `PreviewSession` (`src/lib.rs:854`) already snapshots URLs and drops indexes the catalog no longer contains. The app does not use it.
- **Recommendation:** Drive the panel from `PreviewSession`. Reload it after every catalog edit, close it when the session is empty, and delete the `/` fallback.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** S
