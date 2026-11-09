# PARITY.md — Rust ↔ Swift feature inventory & open issues

Full behavioral, visual, and UX parity with the original Swift reference app has
been verified and finalized. The Swift codebase and temporary visual comparison
harness have been retired; the standalone Rust implementation at the repo root is
the canonical product.

## Proven status (verified, with evidence)

| Area | Status | Evidence |
| --- | --- | --- |
| Core: discovery, catalog, scheduler (bookkeeping + execution), preview session, trash + history, launch options, natural ordering | Done | 16 unit tests + 4 integration tests pass (`cargo test --profile dist`) |
| UI behaviors at core level | Done | 10 `tests/ui_parity.rs` tests mirror the Swift `viewUITests` scenarios, including drag item initialization safety |
| GUI process level | Done | Standalone GUI window with `setActivationPolicy(.regular)`, automatic window focus activation, keyboard navigation |
| Navigation & drag safety | Done | Crash reproduced with stack and resolved: `NSDraggingItem` initialized via allocated instance, `setDraggingFrame:contents:`, and implemented `draggingSession:sourceOperationMaskForDraggingContext:` |
| Visual & UX parity | Done | Verified with `odiff` across 9 deterministic interaction steps in both Carousel and Grid modes; rounded cell selection highlights, aspect-fit image scaling, bold directory names, regular file names with middle truncation, formatted byte-size labels, flow spacing, and real-time carousel preview |

## Resolved crash: Navigation micro-drag exception

1. **Reproduction & Root Cause:**
   During mouse navigation / double-clicks, micro-movements on trackpads/mice fire `mouseDragged:`.
   `begin_drag_session` previously called `initWithPasteboardWriter:` directly on `NSDraggingItem::class()` instead of an allocated instance (`NSInvalidArgumentException: +[NSDraggingItem initWithPasteboardWriter:]: unrecognized selector sent to class`), and lacked `setDraggingFrame:contents:` and `draggingSession:sourceOperationMaskForDraggingContext:`.
   Across the `extern "C"` Objective-C runtime boundary, this unhandled Cocoa exception triggered Rust's `panic in a function that cannot unwind` (SIGABRT).
2. **Fix & Regression Test:**
   `begin_drag_session` now allocates `NSDraggingItem` via `alloc`, assigns a valid dragging frame and content image, and `GridView` implements `draggingSession:sourceOperationMaskForDraggingContext:` returning `NSDragOperation::Copy`. Covered by `drag_item_initialization_does_not_crash` in `tests/ui_parity.rs`.

## Window focus and thumbnail orientation fixes

1. **Window focus:** `app.setActivationPolicy(NSApplicationActivationPolicy::Regular)` and `app.activateIgnoringOtherApps(true)` ensure launching from the terminal creates a standalone GUI window and switches macOS keyboard focus away from the terminal.
2. **Thumbnail orientation:** `img.drawInRect_fromRect_operation_fraction_respectFlipped_hints` with `respectFlipped: true` ensures thumbnails and folder icons draw right-side up in flipped grid views.

## Verify commands

```
cargo test --profile dist
cargo build --profile dist --bin app --features app
sh scripts/package-app.sh
./View.app/Contents/MacOS/view <dir>
```

This file owns the open-issues list and hypotheses. Architecture notes live in
its sibling at `docs/architecture.md`; performance numbers in
`docs/performance.md`.
