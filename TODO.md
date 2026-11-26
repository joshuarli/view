Prefix: VIEW
Next: 13

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
