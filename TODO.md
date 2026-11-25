Prefix: VIEW
Next: 13
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


### VIEW-12 · P3 · Quitting during a trim export can leave a temporary file

- **Area:** Lifecycle
- **Where:** `src/bin/app/trim.rs:332`, `src/bin/app.rs:2862`
- **Problem:** Export writes to a uniquely named temporary directory and only removes it when the operation reaches `finish()`. Closing the window or sending SIGTERM while the export is in flight terminates the app without calling `finish()`, leaving a `.view-trim-*` directory beside the destination. The original is never damaged, but the temporary bytes are not reclaimed until manual cleanup.
- **Recommendation:** Add a termination path that refuses immediate termination while an export is active, lets the export and validation finish, and then removes the temporary directory. Keep it bounded so a hung export cannot trap the app; cover it with a native test that quits mid-export.
- **Fix with:** lifecycle-concurrency
- **Effort:** M
