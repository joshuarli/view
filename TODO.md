Prefix: VIEW
Next: 13

### VIEW-12 · P3 · Quitting during a trim export can leave a temporary file

- **Area:** Lifecycle
- **Where:** `src/bin/app/trim.rs:332`, `src/bin/app.rs:2862`
- **Problem:** Export writes to a uniquely named temporary directory and only removes it when the operation reaches `finish()`. Closing the window or sending SIGTERM while the export is in flight terminates the app without calling `finish()`, leaving a `.view-trim-*` directory beside the destination. The original is never damaged, but the temporary bytes are not reclaimed until manual cleanup.
- **Recommendation:** Add a termination path that refuses immediate termination while an export is active, lets the export and validation finish, and then removes the temporary directory. Keep it bounded so a hung export cannot trap the app; cover it with a native test that quits mid-export.
- **Fix with:** lifecycle-concurrency
- **Effort:** M
