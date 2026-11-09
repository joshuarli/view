# PARITY.md — Rust ↔ Swift feature inventory & open issues

The Swift implementation lives in `swift/` and is the reference. The Rust port
(Cargo crate at the repo root, AppKit via `objc2`) aims for feature parity with
it, not just core-logic parity. This document is the parity inventory a
follow-up agent should use to finish the work.

## Proven status (verified, with evidence)

| Area | Status | Evidence |
| --- | --- | --- |
| Core: discovery, catalog, scheduler (bookkeeping + execution), preview session, trash + history, launch options, natural ordering | Done | 15 unit tests + 4 integration tests pass (`cargo test --profile dist`) |
| UI behaviors at core level | Done | 9 `tests/ui_parity.rs` tests mirror the Swift `viewUITests` scenarios |
| GUI process level | Done | `--self-test` drives arrows, double-click-enter, history back, Space QL toggle, trash, undo through the live run loop in the packaged binary (PASS on fixtures + the real tree) |
| Navigation | Works | Double-click into subdirs verified on the real tree |
| Crash fixes landed | Done | Drag-on-click overlap removed (`mouseDragged:` + re-entry guard); `from_raw().unwrap()` panic surface; QL mutex poison recovery; CGBitmapContext + synchronize decode fix; key-watcher hook emits full backtrace |

## Hard facts / known issues

1. **The original user-reported navigation panic was never reproduced.** Ran
   against `/Volumes/p/infolog-wip/` and 30+ subdirectories with interactive
   double-clicks: no abort. The trace the user pasted was stripped
   (`panic=abort`, symbols removed → `thread::panicking` line only).
2. **`decode_png_native` only shape-asserts in unit tests.** It renders
   real RGBA pixels through `CGBitmapContextCreate` + `CGContextDrawImage`
   + `CGContextSynchronize`, but pixel equality vs the pure decoder is only
   asserted for the top-left sample; orientation/bottom-up ordering may be
   wrong for rows other than the first.
3. **Range selection** (`selection_anchor`/`selection_range`) was added late
   and verified only via the summary in earlier commits plus render code
   inspection. No dedicated test.
4. **`TrashService`** has no rust-side process-level open/finder/reveal/QuickLook
   integration tests beyond the self-test lifecycle slice (trash→undo).

## Hypotheses for the remaining crash (ranked)

Use these on the next pass. Each includes what to do to falsify it.

### H1: Drag-session re-entry during navigation — CONFIRMED-EMONG defect, likely crash path
Every `mouseDown:` opened an `NSDraggingSession`; the **double-click** that
enters a directory re-entered `beginDraggingSessionWithItems` while the first
session was still live. The session-begin goroutine on the dispatch entry
guarded by `DRAG_ACTIVE` now prevents that (fix in `cb7cb08`). Evidence
against: a synthetic double-click pass succeeded in self-tests pre-fix too,
and the old binary couldn't be crashed headlessly — but note `eventTriplets`
synthetic timing may not reproduce a real rapid interaction. Falsify by
rebuilding pre-fix (`git checkout 0c6f395^ -- src/bin/app.rs`) and running with
`RUST_BACKTRACE=full` under an environment that reliably delivers real double
clicks.

### H2: QL thumbnail handler panics on cache poison
The QuickLook completion handler mutates `THUMB_CACHE`/`THUMB_PENDING`
behind `Mutex`. If a handler thread panics once, the guard is poisoned and
every subsequent `.lock().unwrap()` aborts. Fix landed (`unwrap_or_else`
recovering from poison), but if a handler still panics the visibility of
that is now surfaced via the panic hook.

### H3: `Retained::from_raw(ptr).unwrap()` in drag init
`NSDraggingItem::initWithPasteboardWriter` … `Retained::from_raw(ptr).unwrap()`
in `begin_drag_session` was an immediate abort if the message returned null.
Now graceful. Same shape existed in the QL request creation path.

### H4: ImageIO context state in free-running processes
`decode_png_native` renders zero-filled results from freestanding processes
even with a valid `CGBitmapContext` (we probed with `CGContextFillRect` at
increasing parameter counts). The app path — the actual product — is the
same function running on the app's runloop with an `NSAutoreleasePool`
context, which seems to work. Differential between the two environments was
never pinned down.

### H5: `clickCount > 2` or rapid quadruple clicks
`mouse_down` only treats `clickCount >= 2` as "open"; any clicks after rapid
re-entry could race stale `back`/`forward` pops. Untested; low probability.

## What to do next (ordered)

1. **Get the actual user stack.** Build with symbols (`strip = "symbols"` in
   `Cargo.toml` `dist` profile) and have the reporter run:
   ```
   RUST_BACKTRACE=full ./View.app/Contents/MacOS/view /Volumes/p/infolog-wip/
   ```
   then paste the trace here. The panic hook (installed in `main`) now dumps
   a full `std::backtrace` before any abort, so the site will name itself.
2. **If the crash is reproducible:** run that stack down to the file/line,
   fix the confirmed call site, add a regression self-test step, re-run
   `cargo test --profile dist` + `./View.app/Contents/MacOS/view --self-test .work/navtest`.
3. **Golden harness for interactive drive:** the current `--self-test` covers
   arrows/double-click/history/QL/trash/undo only. Extend it with:
   - double-clicks at the first N directory cells from random start points
   - rapid repeat clicks while a QL thumbnail loads (exercises H2)
   - drag-over events (`mouseDragged:`) issued mid-drag with a moving point
4. **Make freestanding decode verifiable:** load one corpus PNG through
   `decode_png_native` in the packaged app (e.g. under the runloop, not under
   `cargo test`'s spawned threads), print bytes to compare. Expect bottom-up
   RGBA and fix orientation using the same routine the app uses.
5. **Add the missing range test** (`selection_range` after Shift+Click and
   arrows) and a trash/undo through the self-test which already has trash
   steps but no explicit assertion on `items().len()`.

## Verify commands

```
cargo test --profile dist
cargo build --profile dist --bin app --features app
sh scripts/package-app.sh
./View.app/Contents/MacOS/view --self-test <dir>
RUST_BACKTRACE=full ./View.app/Contents/MacOS/view <dir>   # crash site capture
```

This file owns the open-issues list and hypotheses. Architecture notes live in
its sibling at `docs/architecture.md`; performance numbers in
`docs/performance.md`.
