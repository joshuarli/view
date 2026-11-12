Prefix: VIEW
Next: 12
### VIEW-3 · P2 · Trash completion and undo follow mutable tab catalogs

- **Area:** File operations
- **Where:** `src/bin/app.rs:481`, `src/bin/app.rs:518`, `src/bin/app.rs:572`, `src/bin/app.rs:808`, `src/bin/app.rs:899`
- **Problem:** Trash captures selected paths but later chooses the first tab containing any returned path, then reconstructs the transaction from that tab's current catalog. With two tabs on /A, trashing /A/x from the second assigns history to the first and leaves the active catalog stale. Navigate away before completion and the success can be discarded because no tab matches. Separately, trash x in /A, navigate to /B, then Undo: x is restored to /A but inserted into /B's catalog at the saved index. Other tabs are not reconciled. Portable probes of the verbatim completion and undo bookkeeping reproduced all three state errors; the undo probe used a temporary-directory rename adapter, not macOS Trash. Native timing and presentation remain untested.
- **Recommendation:** Give each mutation an owner and a captured item snapshot independent of current listings; retain its history when tabs navigate or close, and reconcile all affected catalogs after trash, undo, and redo without inserting foreign-root entries. Preserve exclusive rename conflict handling. Use the linked draft to decide window-wide versus per-tab undo. Verify duplicate tabs, navigation and tab closure before completion, and undo after changing directories.
- **Fix with:** lifecycle-concurrency
- **Effort:** M
- **Design:** [Draft RFC](docs/rfc/VIEW-3-trash-operation-ownership.md)


### VIEW-4 · P2 · Partial trash failures discard successful moves

- **Area:** File operations
- **Where:** `src/bin/app.rs:497`, `src/bin/app.rs:510`, `src/bin/app.rs:513`
- **Problem:** The recycle completion returns immediately when NSError is non-null, discarding every successful original-to-Trash mapping and showing no error. Apple's [recycle contract](https://developer.apple.com/documentation/appkit/nsworkspace/recycle(_:completionhandler:)) explicitly permits successful mappings together with an error for the files that failed. A batch containing a movable file and a failing file therefore leaves the moved file visible and omits its Undo record. A portable probe of the verbatim completion, supplied a successful mapping and had_error=true, retained the catalog item and produced no history. Native NSWorkspace execution was not run.
- **Recommendation:** Apply and record every returned success even when the batch has an error, retain failed files in the catalog, and present the error to the user. Add a controlled mixed-result completion test asserting catalog membership, visible failure, and a working undo for the successful subset. This branch repair can land before the broader ownership work.
- **Fix with:** error-model
- **Effort:** S


### VIEW-5 · P2 · Range reconstruction expands sparse selections

- **Area:** Selection
- **Where:** `src/lib.rs:243`, `src/lib.rs:321`, `src/bin/app.rs:1024`, `src/bin/app.rs:742`, `src/bin/app.rs:1900`, `src/bin/app.rs:481`
- **Problem:** Trash B from A/B/C, select A through C in the remaining catalog, then Undo B. The catalog correctly keeps selected_all=[A,C], but selection_bounds spans A/B/C. Drawing and dragging use that span, so B is highlighted and exported despite not being selected. Rename also reconstructs inactive-tab selections with select_range(anchor,focus); renaming an unrelated file in another tab expands the sparse selection to [A,B,C], and a later Trash includes B. Portable probes of the real Catalog methods and rename bookkeeping reproduced both the span mismatch and the selection expansion. Native drawing/dragging was not run.
- **Recommendation:** Use actual selection membership for highlighting, drag payloads, click preservation, and selection restoration. Keep anchor/focus for range gestures rather than deriving membership from them. Preserve individual selected paths through rename/reload and add the remove-select-undo-rename sequence with assertions for highlighted paths, drag payloads, and trash targets.
- **Fix with:** make-invalid-states-unrepresentable
- **Effort:** M


### VIEW-6 · P2 · Directory read errors appear as empty folders

- **Area:** Discovery
- **Where:** `src/lib.rs:148`, `src/lib.rs:175`, `src/bin/app.rs:1092`, `src/bin/app.rs:808`
- **Problem:** list_browser propagates entry metadata failures, and scan_recursive aborts on any nested read_dir failure after possibly collecting readable files. load_listing converts either error into Vec::default(), which navigation commits as a successful empty catalog. An exact-source probe with visible.png and one mode-000 child directory returned PermissionDenied from scan_recursive and zero items from load_listing. This reproduces the loss of usable listing state on a real filesystem under a non-root user; AppKit error presentation was not run.
- **Recommendation:** Carry the path-aware Result through the app boundary and visibly distinguish failure from a successful empty listing. Preserve the previous catalog when navigation fails. If recursive discovery is changed to return partial results, make skipped paths explicit rather than silently dropping them. Cover an unreadable child and a failed navigation without conflating either with an empty folder.
- **Fix with:** error-model
- **Effort:** M


### VIEW-7 · P2 · Dropping onto the active tab skips the view refresh

- **Area:** Navigation
- **Where:** `src/bin/app.rs:972`, `src/bin/app.rs:890`, `src/bin/app.rs:875`, `src/bin/app.rs:1585`
- **Problem:** A tab-bar drop calls navigate_tab_to_path, which changes the target tab's root/catalog/selection and then calls activate_tab. activate_tab returns immediately when that index is already active, so the title, carousel, grid layout, Quick Look and tab bar receive no after_navigation refresh. A portable probe of the exact navigation and activation functions changed the active root with zero refresh-hook calls; an inactive-tab control made one call. The current app regression covers an inactive target. Native pixels were not observed, but the missing refresh dispatch is reproduced.
- **Recommendation:** Refresh the displayed tab after a successful mutation even when activation is a no-op. Keep one navigation-completion path for both active and inactive targets. Extend the drop scenario to assert the active tab's title, selected preview, layout and Quick Look after dropping a different folder and a file.
- **Fix with:** isolate-effects
- **Effort:** S


### VIEW-8 · P2 · The benchmark target still uses removed AppState fields

- **Area:** Performance tooling
- **Where:** `src/bin/app/benchmarks.rs:77`, `src/bin/app/benchmarks.rs:81`, `src/bin/app/benchmarks.rs:120`, `src/bin/app/benchmarks.rs:171`, `src/bin/app.rs:63`, `benches/view.rs:1`, `scripts/bench.sh:11`
- **Problem:** The tab refactor moved root, mode, view_mode and catalog into Tab, but prepare_window and several benchmark operations still access those fields directly on AppState. Every benchmark build includes this module, so none of the documented benchmark commands can compile. A focused compiler probe combining the verbatim AppState definition and obsolete setup accesses failed with four E0609 unknown-field errors. The complete AppKit benchmark build was not run on Linux. Current performance claims cannot be refreshed until the harness uses the current application state.
- **Recommendation:** Initialize a real tab and migrate all benchmark setup/operations to the app's current tab accessors. Preserve the real window, grid and player paths and existing timing boundaries. On macOS, compile the benchmark target without running measurements, verify the runner locates the emitted executable, then exercise a named workflow and its correctness assertions before collecting new baselines.
- **Fix with:** refactor-benchmarks
- **Effort:** S


### VIEW-9 · P2 · Movie thumbnails survive replacement of their source file

- **Area:** Thumbnail cache
- **Where:** `src/bin/app.rs:298`, `src/bin/app.rs:1172`, `src/bin/app.rs:1233`, `src/bin/app.rs:1267`, `src/bin/app.rs:1916`
- **Problem:** Both movie drawing paths pass None for file size and modification time, and thumbnail_completed stores that same metadata-free key. Replacing or editing a movie at the same path then navigating away and back rereads its catalog metadata but still reuses the old grid/poster image until eviction or restart. An older outstanding request can also populate that unversioned key. A probe of the actual cache functions retained the old image token after a real file replacement; supplying the new metadata would have missed. The NSImage value was a stand-in, so native Quick Look rendering and edit notifications remain untested.
- **Recommendation:** Carry a file version through movie lookup, pending requests and completion, and reject obsolete results. Invalidate the current item/poster when a known edit changes that version. Add a same-path replacement test and an out-of-order completion test that prove neither grid nor carousel can display the previous file version.
- **Fix with:** lifecycle-concurrency
- **Effort:** M


### VIEW-10 · P3 · Space has no replay transition after a movie ends

- **Area:** Playback
- **Where:** `src/bin/app/player.rs:211`, `src/bin/app/player.rs:198`, `src/bin/app.rs:2941`
- **Problem:** Once a finite movie reaches EOF, toggle only sees rate()==0 and calls play(); there is no seek or end-of-item handling. Apple's [AVFoundation playback guide](https://developer.apple.com/library/archive/documentation/AudioVideo/Conceptual/AVFoundationPG/Articles/02_Playback.html) states that play has no effect at the end until the playhead is repositioned. The source and API contract support a missing replay transition, but this is unproven in the live AVPlayerView host and is kept at P3 pending native confirmation. The existing player regression toggles play/pause before EOF.
- **Recommendation:** First run the short fixture to EOF and press Space in the macOS app. If confirmed, distinguish ended from paused, seek to the beginning before replay, and ensure a stale seek completion cannot start a newly selected item. Extend the real player scenario through end, replay, and navigation during that transition.
- **Fix with:** lifecycle-concurrency
- **Effort:** S


### VIEW-11 · P3 · Recursive discovery emits files in reverse natural order

- **Area:** Discovery
- **Where:** `src/lib.rs:175`, `src/lib.rs:187`, `src/bin/app.rs:1092`
- **Problem:** scan_recursive sorts directory entries by natural name, then iterates them in reverse. Reversing is useful for the directory stack, but files are emitted immediately in that reversed order and before queued children. The real-function probe returned z.png, visible.png, a.png, b/c.png instead of natural relative-path order. The previous README promised natural relative-path order; the refreshed docs describe the current behavior. This makes recursive browsing inconsistent with the app's natural-name policy. Existing discovery coverage has only one surviving file.
- **Recommendation:** Produce a defined natural relative-path order, either by sorting the final media list or by correcting the traversal without reversing emitted files. Add a mixed root-file/subdirectory fixture with numeric filenames and assert the full output order.
- **Fix with:** os-boundaries
- **Effort:** S
