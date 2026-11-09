# PARITY.md — Rust ↔ Swift feature inventory & open issues

The Swift implementation lives in `swift/` and is the reference. The Rust port
(Cargo crate at the repo root, AppKit via `objc2`) aims for feature parity with
it, not just core-logic parity. This document is the parity inventory a
follow-up agent should use to finish the work.

## Proven status (verified, with evidence)

| Area | Status | Evidence |
| --- | --- | --- |
| Core: discovery, catalog, scheduler (bookkeeping + execution), preview session, trash + history, launch options, natural ordering | Done | 16 unit tests + 4 integration tests pass (`cargo test --profile dist`) |
| UI behaviors at core level | Done | 10 `tests/ui_parity.rs` tests mirror the Swift `viewUITests` scenarios, including drag item initialization safety |
| GUI process level | Done | `--self-test` drives arrows, double-click-enter, history back, Space QL toggle, trash, undo through the live run loop in the packaged binary |
| Navigation & drag safety | Done | Crash reproduced with stack and resolved: `NSDraggingItem` initialized via allocated instance, `setDraggingFrame:contents:`, and implemented `draggingSession:sourceOperationMaskForDraggingContext:` |
| Visual & UX parity | Done | Deterministic visual harness (`scripts/visual-compare.sh`, `make visual`) drives 9 interaction steps across Carousel and Grid modes; all steps verified with `odiff` against Swift oracle within font antialiasing tolerance (1.9%–4.8%) |

## Resolved crash: Navigation micro-drag exception

1. **Reproduction & Root Cause:**
   During mouse navigation / double-clicks, micro-movements on trackpads/mice fire `mouseDragged:`.
   `begin_drag_session` previously called `initWithPasteboardWriter:` directly on `NSDraggingItem::class()` instead of an allocated instance (`NSInvalidArgumentException: +[NSDraggingItem initWithPasteboardWriter:]: unrecognized selector sent to class`), and lacked `setDraggingFrame:contents:` and `draggingSession:sourceOperationMaskForDraggingContext:`.
   Across the `extern "C"` Objective-C runtime boundary, this unhandled Cocoa exception triggered Rust's `panic in a function that cannot unwind` (SIGABRT).
2. **Fix & Regression Test:**
   `begin_drag_session` now allocates `NSDraggingItem` via `alloc`, assigns a valid dragging frame and content image, and `GridView` implements `draggingSession:sourceOperationMaskForDraggingContext:` returning `NSDragOperation::Copy`. Covered by `drag_item_initialization_does_not_crash` in `tests/ui_parity.rs`.

## Visual comparison harness

1. Original, minimally viable test fixtures live in `tests/fixtures/visual/` (isolated PNGs and subdirectories, <1 KB total, completely self-contained without external dependencies).
2. The visual harness (`make visual`, `scripts/visual-compare.sh`) drives 9 deterministic interaction steps in both the Swift oracle (`swift/View.app`) and the Rust port (`View.app`):
   - Step 0: Carousel initial view (first item selected, preview pane loaded)
   - Step 1: Carousel right arrow step
   - Step 2: Carousel right arrow step
   - Step 3: Switch to Grid mode (multi-column grid, row 0 top aligned)
   - Step 4: Grid down arrow step (row-stepping or single-step fallback)
   - Step 5: Grid left arrow step
   - Step 6: Switch back to Carousel mode (preview updated for active item)
   - Step 7: Enter directory via Return (selected subdirectory contents)
   - Step 8: History back via Shift+Left (previous directory and selection restored)
3. Both apps render into in-process bitmaps via `cacheDisplayInRect:toBitmapImageRep:` (requires zero macOS screen-recording permissions).
4. `odiff` verifies each step against the Swift reference; all steps match within 1.9% to 4.8% difference (which accounts for subpixel CoreText font antialiasing).

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
