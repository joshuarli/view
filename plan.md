# view — campaign plan

**Status:** ready to execute.
**Goal:** a native macOS 27 app, launched as `view [directory]`, that shows every image and video beneath that directory in one ordered, flattened thumbnail grid, with native Quick Look and Ctrl+Delete-to-Trash, and nothing else.
**Owner decisions:** OD1–OD3 below, dated 2026-10-22.

**Deliverables**

| ID | Deliverable |
| --- | --- |
| D1 | `view [dir]` launcher: absolute-root resolution, cwd default, useful terminal errors, detached app, arbitrary paths including spaces; `View.app` bundle. |
| D2 | Recursive scan of supported media into one deterministic, incremental, cancellable ordered catalog. |
| D3 | AppKit thumbnail grid: adaptive square cells, selection by item identity, video glyph, empty state, viewport-bounded live items. |
| D4 | Unified Quick Look thumbnail backend with a bounded, deduplicating, prioritized, cancellable coordinator and a 96 MiB memory cache. |
| D5 | Native Quick Look panel on Space, with adjacent navigation and grid selection kept consistent. |
| D6 | Ctrl+Delete moves the selected original to Trash, updates catalog/cache/selection/preview, and reports failures natively. |
| D7 | Swift 6 language mode with zero concurrency escape hatches; warnings as errors; tests for every behavior listed in Acceptance. |
| D8 | Measured performance, minimal architecture, short docs, and a final report with the plan's seven required sections. |

**Last accepted checkpoint:** I6 Hardening at `ace0fce` (shutdown drains scanner, thumbnail requests, and in-flight recycle work; full tests and release build pass).
**Provisional work:** none.
**Current steering decision:** parity before optimization; GUI correctness is owner-verified (OD2); perf measured and logged until P3.
**Next substantive milestone:** O1 Measure, with a generated corpus and three quiet-host measurement launches after checking `perf-gate-ready`.

## Charter

- Decide, log the call in the campaign log, and continue. Do not wait for approval inside the owner's grant (Execution boundary).
- Hard stops park only the affected item: an irreversible or outside-repo action, an effect the boundary forbids, or a loop that cannot check anything. Pushing is always forbidden.
- Parity before optimization. Every deliverable works end to end in P1; P3 and P4 change no feature. Optimization, size, and polish items live only in P3; a loop repair may come earlier when it unblocks delivery, with a stated exit test.
- Fix on the way, in its own commit, anything egregious met while working: a command over 3 minutes, a test suite over 60 seconds, RSS over 1 GiB, a crash on launch, or a regression over 2× the P0 baseline. Record everything else and leave it.
- The loop: `./scripts/test.sh --filter <Suite>` for an item, `./scripts/test.sh` plus `swift build -c release` at the end of each phase and at the close. Reuse a green result while the inputs it covers are unchanged; invalidate it when they change.
- Time-based gates (loop timing, O2 targets) run only when `perf-gate-ready` exits 0, and again with `-1` after the measurement; discard a run that overlapped load and say "perf gate deferred: host busy" instead of passing it. Memory, size, and correctness gates always run.
- If a documented Apple symbol is absent from the installed SDK, use the closest supported public API, keep the concurrency guarantees, and record the deviation in the campaign log and final report.
- Failure timebox: 2 attempts on an item, then park it with the evidence, the blocking prerequisite, and the retry condition. Requeue only when that condition changes.
- Width: one implementation lane. A verification lane may run only if a second agent is available, owns no implementation files, and does not touch shared files. Long-lived parallel lanes are wrong for this codebase size.
- Worktrees and scratch live under `.work/` (lane worktrees `.work/wt/<lane>`, scratch `.work/tmp/`). Never `/tmp`. `.work/` is excluded through `.git/info/exclude`.
- Every commit is Conventional (`type(scope): subject`, lowercase imperative subject ≤72 chars, why in the body, item ID in the body). Commit per accepted item; do not commit a red tree. Never push. Never run formatters. Never add or change CI.
- No code comments except the narrow exceptions in the repository rules; a comment never cites this plan.
- Contract changes (public API, schemas, permissions, invariants, persistence) are surfaced in the final report, not slipped in.
- Keep bookkeeping minimal: this plan, the campaign log, `docs/performance.md`, and the code. No provenance files, no second progress register.

## Decisions

### Owner-decided (2026-10-22)

- **OD1 — Git.** The repository was initialized; commit completed items locally, never push.
- **OD2 — GUI verification is skipped.** The owner exercises the running UI after delivery. The campaign proves behavior with compilation, unit tests, and measurement runs only. Do **not** build screenshots, UI automation, synthetic-event drivers, self-check modes, or an integration harness, and do not report GUI behavior as verified. Real launches happen only for measurement and must be reported as observations. This supersedes the integration block of the source spec and the "not a prototype" GUI expectations; everything else in the source spec stands.
- **OD3 — Performance.** Measure and log during parity, without gating. Gates apply in P3 only, against the recorded baseline.

### Prescribed

- **P1 Toolchain.** Swift 6.4 (`swiftlang-6.4.0.34.1`), macOS 27.0 SDK from the Command Line Tools, macOS 27.0.1 arm64. Build arm64 native; add no architecture slices. Xcode 27.1 is installed but unusable (license not accepted; accepting it needs sudo and is a hard stop). Use SwiftPM only; no `.xcodeproj`, no generators, no dependencies.
- **P2 Swift Testing wrapper.** Plain `swift test` cannot load `TestingMacros` from the CLT layout. Always run `./scripts/test.sh`, which adds the toolchain's `host/plugins/testing` path. Never weaken tests to work around this.
- **P3 Exact ordering.** Iterative depth-first pre-order. Within a directory, sort entries (files and subdirectories together) by name using `.numeric, .caseInsensitive, .forcedOrdering`, tie-broken by UTF-8 byte order of the name; emit a media entry when reached, descend when reached. Canonical vector — tree `A.png`, `a/c.png`, `a/d/1.png`, `frame2.png`, `frame10.png` yields the stream `a/c.png`, `a/d/1.png`, `A.png`, `frame2.png`, `frame10.png`. Delivered order is final; the catalog is never re-sorted.
- **P4 Classification and filtering.** One `contentsOfDirectory(at:includingPropertiesForKeys:)` per directory with `.isDirectoryKey, .isRegularFileKey, .isSymbolicLinkKey, .isPackageKey, .isHiddenKey, .nameKey, .contentTypeKey, .contentModificationDateKey, .fileSizeKey`, then one `resourceValues` read per entry. Skip dot-prefixed names, hidden entries, packages, and every symlink (directories and files); this makes cycles impossible and needs no cycle set. Accept `contentType` conforming to `UTType.image` or `UTType.movie`, never audio-only. No extension allowlist, no content reads, no dimension/duration/waveform/asset inspection at scan time. An unreadable nested directory is skipped and counted; an unreadable root is an error.
- **P5 Scan delivery.** A `nonisolated` scanner runs `@concurrent` and reports `[MediaItem]` batches of 64–128 (128 by default) through an `async` sink, so the consumer applies bounded backpressure. Cancellation is checked per entry and per directory and throws `CancellationError`. Already-delivered batches remain valid.
- **P6 Isolation split.** `viewCore` is AppKit-free: launch validation, media items, ordering, scanning, catalog state, thumbnail keys and scheduling, preview-session state, key-to-command mapping. `Sources/view` is `@MainActor` by default isolation and owns AppKit, Quick Look, Trash, and metrics. Do not add a service protocol or DI layer per responsibility.
- **P7 Quick Look quirks (verified in the SDK).** `QLPreviewPanelDataSource` and `QLPreviewPanelDelegate` are imported nonisolated; conform with isolated conformances (`extension X: @MainActor QLPreviewPanelDataSource`) rather than `@preconcurrency`. `QLPreviewPanel.shared()` returns `QLPreviewPanel?`. Preview items are `NSURL` values. `QLThumbnailGenerator` has **no** async overlay: bridge `generateBestRepresentation(for:completion:)` with `withCheckedThrowingContinuation` inside `withTaskCancellationHandler`, and cancel with `QLThumbnailGenerator.shared.cancel(request)`. Request type is `QLThumbnailGenerator.Request(fileAt:size:scale:representationTypes:)`; read `.cgImage` or `.nsImage` from the representation (`.cgImage` preferred).
- **P8 Thumbnails.** Quick Look only for now. Request a square of `160 * backingScaleFactor` pixels with `.thumbnail` representation types and icon mode off. Reject `.icon` representations: a request that yields only an icon counts as failed and shows the placeholder, because a decorated document icon is not a content thumbnail. Cache key: item URL, pixel size, scale, and `(modificationDate, fileSize)` version; cost `bytesPerRow * height` in an `NSCache` with `totalCostLimit = 96 MiB`. The cache is a small `viewCore` wrapper over Foundation's `NSCache` holding `CGImage`s, so eviction and accounting are unit-testable; the app converts to `NSImage` at display time. Limit 4 concurrent requests on local volumes, 2 elsewhere (`volumeIsLocalKey` read once at startup). Priority: visible, then one-viewport-ahead prefetch in the scroll direction, then nothing. Deduplicate by key; cancel on cell reuse, removal, or phase change; drop results whose generation or item identity is stale; latch a failed key for the session and show one quiet placeholder. No disk cache, no second decoding path, no restored full-resolution buffers.
- **P9 Grid.** `NSCollectionView` with `NSCollectionViewFlowLayout`, 160 pt square items, 10 pt gutters, 16 pt section insets, aspect-fit content inset 6 pt, an 11 pt `play.fill` secondary badge for videos, no captions, no toolbar/sidebar/inspector, one resizable window titled with the root's last path component. Selection: single, stable by item URL, drawn as a rounded fill of `selectedContentBackgroundColor` at 35% alpha. Data source: regular `NSCollectionViewDataSource` with `insertItems(at:)`/`deleteItems(at:)`, not diffable snapshots. Thumbnail completion updates only the affected item when visible. Arrow keys come from native collection-view handling. Empty catalog shows the root path in a centered secondary label.
- **P10 Quick Look control.** The window's content view controller is the panel controller: `acceptsPreviewPanelControl`, `beginPreviewPanelControl`, `endPreviewPanelControl`; it owns data source/delegate only between begin and end, and never touches a panel another responder controls. Begin snapshots the catalog's URLs and the selected index; `previewPanel(_:previewItemAt:)` records the index **and updates the catalog selection immediately**, so the grid and any Ctrl+Delete stay consistent while previewing. Catalog mutation while open updates the snapshot and calls `reloadData`; an empty catalog closes the panel. Space toggles from a main-menu item with key equivalent `" "` and no modifiers, and trash from a menu item with explicit target = the content view controller, so both work whether the grid or the panel is key; menu validation disables them without a selection. Escape stays native.
- **P11 Trash.** A menu item with key equivalent `"\u{8}"` and `.control` (never `.command`, never unmodified) triggers a `@MainActor` trash service in `viewCore` that owns the in-flight set, awaits an injected `recycle` closure, and mutates the catalog only after the returned mapping contains the URL. The app supplies `NSWorkspace.shared.recycle`, evicts cache entries, cancels demand, keeps Quick Look consistent, and shows an `NSAlert` sheet with `localizedDescription` on failure. Selection after removal is the item now at the removed index, else the previous one; no rescan. No second confirmation, no permanent deletion, no manual `~/.Trash` move.
- **P12 Interruptible identity checks.** Any work that crosses an `await` re-checks the requested item identity and a generation counter before applying a result. Cell reuse sets the cell's represented key; a late result for another key is discarded.
- **P13 Metrics.** Always-compiled, activated by `VIEW_METRICS=1`, one `VIEW_METRIC name=value` line per measurement on stderr, timed from process start with `ContinuousClock`. Names and meanings: `window-shown-ms` (window ordered front), `first-batch-ms` (first catalog batch applied), `scan-complete-ms` (scanner returned), `scan-items` (catalog count), `first-thumbnail-ms` (first thumbnail displayed in a cell), `visible-populated-ms` (every cell in the initial viewport holds an image or latched placeholder), `scan-batches` (catalog batches applied), `thumbnail-requests` (QL requests issued), `max-concurrent-thumbnail-requests` (high-water mark), `thumbnail-cache-bytes` (sum of costs after the initial viewport settles). No UI, no framework. Add `os_signpost` intervals in the same places. Measurement runs launch the bundled executable directly — `VIEW_METRICS=1 View.app/Contents/MacOS/view <dir> 2> <log>` — and the executor kills it afterwards; `bin/view` is not used for measurement because launchd discards stderr.
- **P14 Fixtures.** Tests build their own fixtures in `.work/tmp/unit-tests/`; measurement uses `swift scripts/make-corpus.swift <dir>` producing 1,000 varied-color 400×300 PNGs, 20 one-second H.264 clips, noise files, nested directories, and hidden/package/symlink decoys. Never use real user media as a test target.
- **P15 Launch.** The launcher is the checked-in POSIX script `bin/view`: resolve its own path through symlinks, find `View.app` beside the repo root or at `$VIEW_APP`, reject more than one argument, reject missing/non-directory roots on stderr with exit 1, canonicalize with `pwd -P`, then `open -n -a View.app --args <root>`. One new app instance per invocation; no dependence on a running instance or a live terminal. The app re-validates the root and exits 1 with a stderr message if it is unusable. No install step outside the repo.
- **P16 Bundle.** `View.app` assembled by `scripts/build-app.sh`, ad-hoc signed, `CFBundleIdentifier com.view`, executable `view`, `LSMinimumSystemVersion 27.0`, high-resolution capable, no sandbox, no entitlements, no icon, no custom URL types. `NSApplication` activation policy `.regular`.
- **P17 Docs.** `README.md` carries the exact commands; `docs/performance.md` carries measurement runs (date, machine, corpus, values, verdict). No other docs unless a real reader appears.

### Delegated (executor decides inside these bounds, logs the call)

- Internal data structures, file layout inside owned directories, and naming; batch size within 64–128 (128 unless a measured reason).
- Main-actor coordinator versus actor, as long as it is one coordinator and requests stay off the main thread.
- Placeholder artwork, alert phrasing, exact signpost names beyond P13, and whether the failure set is a `Set` or per-request flag.
- Cache limit tuning in P3 within 64–128 MiB, local concurrency within 3–6, and prefetch margin within one to two viewports.
- Whether `MediaKind` is an enum plus UTType identifier or just the identifier, provided kind is available without I/O.

## Baseline (measured 2026-10-22, this machine)

- macOS 27.0.1 (26A434), arm64; Swift 6.4; macOS 27.0 SDK; CLT toolchain. Two benign linker warnings about missing `CommandLineTools/Developer/...` search paths are inherited; they are not campaign regressions and are not errors.
- Cold `swift build`: 24.8 s. Warm `./scripts/test.sh`: 2.3 s (5 tests). Cold `./scripts/build-app.sh`: 19.0 s. Launch to visible window: ~1–2 s observed once.
- Known baseline failure: plain `swift test` fails to load `TestingMacros`; use `./scripts/test.sh`. No other known red.
- Capabilities: `CGWindowListCopyWindowInfo` works; `screencapture` and synthetic keystrokes are denied; System Events read-only queries work. GUI automation is therefore out of scope (OD2).
- Measurement path verified: `View.app/Contents/MacOS/view <dir>` launches the real window from a terminal with stderr attached (81 MB RSS, 18 MB physical footprint while idle), so `VIEW_METRICS` output is readable without launchd. `footprint <pid>` and `perf-gate-ready` are available. The host runs an OrbStack VM that can be hot; treat timing runs that fail the gate as deferred, not as failures.
- Reference numbers for perf: first real measurement lands in I3; P3 gates compare against it.

## Sequence

Items are capabilities, not tickets. Files listed are owned by that item; do not edit another item's files concurrently.

### P0 — ground (accepted)

- **S1 Scaffolding.** `Package.swift` (tools 6.4, macOS 27, Swift 6 mode, `NonisolatedNonsendingByDefault`, `defaultIsolation(MainActor.self)` for the app, warnings as errors), `Info.plist`, `Sources/viewCore/RootDirectory.swift`, `Sources/view/{ViewApp,RootViewController}.swift`, `bin/view`, `scripts/{build-app.sh,test.sh}`, `Tests/viewCoreTests/{RootDirectoryTests,TestDirectory}.swift`, `README.md`, `AGENTS.md`, `.gitignore`.
  Proof: `swift build`; `./scripts/test.sh` (5 tests); `./scripts/build-app.sh`; launcher error paths; one launch showing a 1100×760 window titled after the root.

### P1 — parity and completeness

- **I1 Discovery.** Prereq: S1. Files: `Sources/viewCore/Discovery/{MediaItem,PathOrder,DirectoryScanner}.swift`, `Tests/viewCoreTests/DiscoveryTests.swift`, fixtures in `TestDirectory.swift`.
  Implements D2 and P3–P5: root is the given directory; recursion flattens; filters exactly as P4; deterministic order; batches of 128 delivered with the first batch as soon as it fills; cancellation; unreadable nested directories skipped without crashing; duplicate basenames, spaces, Unicode, and odd-but-valid names preserved; no content reads, no watchers, no rescans, no global sort.
  Proof: `./scripts/test.sh --filter Discovery` covering empty, flat, nested, mixed media/noise, duplicate names, hidden entries, packages, symlink files and directories, a symlink cycle, unreadable directory, ordering vector, Unicode/whitespace names, cancellation mid-scan, and batch boundaries at the configured size (±1).
- **I2 Catalog and grid.** Prereq: I1. Files: `Sources/viewCore/Catalog.swift`, `Sources/view/{Grid/GridViewController,Grid/ThumbnailCell}.swift` (replacing `RootViewController` in the window), `Sources/view/ViewApp.swift` wiring, `Tests/viewCoreTests/CatalogTests.swift`.
  Implements D3 and P6/P9: append batches without re-sorting, selection by URL, neighbor selection on removal, empty state, incremental `insertItems`, cell reuse safety, viewport-bounded items, native arrow keys, resize without reload.
  Proof: `./scripts/test.sh --filter Catalog` (append preserves selection, remove selected picks next then previous, remove last yields empty, remove unrelated keeps identity, batch insert leaves order intact) and `--filter GridCommand` (Space maps to the preview toggle; modified Space does not).
- **I3 Thumbnails.** Prereq: I2. Files: `Sources/viewCore/Thumbnails/{ThumbnailKey,ThumbnailCache,ThumbnailScheduler}.swift`, `Sources/view/ThumbnailCoordinator.swift`, `Tests/viewCoreTests/SchedulerTests.swift`.
  Implements D4 and P8/P12/P13, including the scheduler's ten rules: demand queue with three priority tiers, dedupe, hard concurrency cap, cancellation release, reprioritization when scroll direction flips, stale-result discard, QL cancellation, failure latch, no per-file tasks, and advancement only on completion or demand change. Visible cells request even when prefetch callbacks never fire; demand updates come from prefetch callbacks plus clip-view bounds changes.
  Proof: `./scripts/test.sh --filter Scheduler` with an injected request closure (max concurrency, dedupe, priority order, cancellation frees a slot, bounded queue under 100 rapid direction flips, stale generation dropped, removed item cannot reappear, cache hit issues no request, failure requested once, cost-limit eviction drops oldest entries, removing an item evicts its keys); runtime metrics show `max-concurrent-thumbnail-requests ≤ 4`, `thumbnail-cache-bytes ≤ 100663296`, and non-zero `first-thumbnail-ms`.
- **I4 Quick Look.** Prereq: I2. Files: `Sources/view/PreviewPanelController.swift`, `Sources/viewCore/PreviewSession.swift`, `Tests/viewCoreTests/PreviewSessionTests.swift`.
  Implements D5 and P7/P10, including responder-chain ownership, snapshot lifetime, index recording, catalog-mutation handling, close synchronization, and native unavailable-preview behavior for corrupt files.
  Proof: `./scripts/test.sh --filter PreviewSession` (initial index, advancing index stays in range, deletion before the current index shifts it, deletion of the current item clamps and never points past the end, empty session yields no item, session never reads a stale index after mutation).
- **I5 Trash.** Prereq: I2, I4. Files: `Sources/viewCore/Trash.swift` (the recyclable operation that calls the catalog's removal), `Sources/view/TrashController.swift`, `Sources/view/Grid/GridViewController.swift` action wiring, `Tests/viewCoreTests/TrashTests.swift`.
  Implements D6 and P11, including the in-flight guard, post-success mutations only, cache/demand cancellation, selection continuity, Quick Look refresh, sheet error, and no rescan. The `viewCore` operation takes an injected recycle closure (`@Sendable (URL) async throws -> URL`), so success and failure paths are testable without touching the real Trash; the app supplies the `NSWorkspace.recycle` implementation.
  Proof: `./scripts/test.sh --filter Trash` (removal happens only after the recycle closure returns; a throwing closure leaves catalog, selection, and count unchanged; a duplicate attempt while in flight is refused; removal picks the next item then the previous; empty catalog after removal) plus `--filter GridCommand` (Ctrl+Delete maps to trash, Cmd+Delete and unmodified Delete do not). Real Trash behavior is owner-verified per OD2.

### P2 — hardening

- **I6 Hardening.** Prereq: I1–I5. Files: incrementally wherever the case belongs.
  Corrupt or unsupported media degrades to placeholder; unreadable root reports a native alert; filenames with newlines, quotes, emoji, and combining marks survive scanning, cell display, preview URLs, and trash; scanner error paths do not leak tasks; quitting cancels scan, scheduler, and in-flight QL requests. Update `README.md` and `AGENTS.md` with any command change.
  Proof: added unit cases per defect, `./scripts/test.sh` green, `swift build -c release` clean.

### P3 — optimization and polish (only after P1 is accepted)

- **O1 Measure.** Prereq: I6. Build the corpus, run 3 measured launches at the corpus root, and record every P13 metric plus peak/steady RSS (`footprint <pid>`) and idle CPU (`ps -o %cpu= -p <pid>` sampled 10× over 20 s after settle) in `docs/performance.md`. Compare scroll pass 1 vs pass 2 for growth. A run that overlaps host load is discarded per the charter; record the `perf-gate-ready` verdict with each run. The values recorded here are the gate reference.
- **O2 Tune.** Prereq: O1. Adjust cache limit (64–128 MiB), local concurrency (3–6), prefetch margin (1–2 viewports), and batch size within P8/P4 bounds, one change at a time, keeping only measured wins. Gates, measured against quiet-host runs: `first-thumbnail-ms ≤ 400`, `visible-populated-ms ≤ 1500`, `scan-complete-ms ≤ 1000` for the 2,000-item corpus, `max-concurrent-thumbnail-requests ≤ 4`, `thumbnail-cache-bytes ≤ 100663296`, idle CPU ≤ 0.5%, steady RSS ≤ 500 MiB, second-pass RSS growth ≤ 20%, zero blocking image or filesystem work on the main actor, and no regression over 1.5× against O1 beyond observed spread.
- **O3 ImageIO experiment (conditional, one attempt).** Run it only if O1 shows `first-thumbnail-ms` p50 > 600 or `visible-populated-ms` > 2500, or signposts show Quick Look dominating. Replace only the still-image request path, keep Quick Look for video, and keep the change only with ≥20% better visible-populated time and no RSS regression over 10%. Otherwise record the numbers and stop; do not keep a second path.
- **O4 Simplify.** Delete dead code, unused abstractions, and duplicate state; inline single-use types; confirm the app is still exactly a media browser. Answer in the log: can a file or class disappear; can an abstraction go; is more state kept than the UI needs; are caches redundant; is anything computed before it is needed; is any O(N) work repeated per batch; are images retained after scrolling away; does work outlive interest in its result; is AppKit being reimplemented; is concurrency earning its keep; has scope crept. Keep the AppKit grid unless measurement shows it materially fails; keep Quick Look as the only player.

### P4 — close

- **C1 Review.** Run `ultrareview` on the repository; fix P1 and P2 findings or record why not.
- **C2 Report.** Final report: implemented scope; exact build/run/test commands; file and module overview with reasons; test results and what was observed at runtime; measured performance; limitations and owner-verification list; whether further simplification is warranted.
- **C3 Handoff.** Leave the tree clean and committed; list the owner-verified checklist from Acceptance; no worktrees left unless clean, named, and needed.

## Acceptance

Every proof runs from the repository root, needs no owner, and fails loudly.

| Deliverable | Proof |
| --- | --- |
| D1 launcher | `./scripts/build-app.sh && ./bin/view .` exits 0 with a live app process; `./bin/view /nope` → stderr `no such directory`, exit 1; `./bin/view /etc/hosts` → `not a directory`, exit 1; `./bin/view a b` → usage, exit 2; launching from a directory containing spaces passes that exact absolute path, checked with `ps -o command=` on the app process. Window title and appearance are owner-verified. |
| D2 discovery | `./scripts/test.sh --filter Discovery` green, including every case in I1; at the O1 run, `scan-items` equals the corpus's media count and `scan-batches ≥ 8`. |
| D3 grid | `./scripts/test.sh --filter "Catalog|GridCommand"` green; a measurement launch reports a non-zero `first-batch-ms`, `scan-items`, and `scan-batches`; C1 review confirms reusable items and viewport-bounded live cells. |
| D4 thumbnails | `./scripts/test.sh --filter Scheduler` green; metrics show the concurrency cap, the cache ceiling, and a populated first screenful. |
| D5 Quick Look | `./scripts/test.sh --filter PreviewSession` green; runtime appearance, video playback, adjacent navigation, and Escape are **owner-verified** (OD2). |
| D6 trash | `./scripts/test.sh --filter "Trash|GridCommand"` green; failure path leaves state unchanged; real Trash contents and Put Back are **owner-verified** (OD2). |
| D7 concurrency | `swift build -c release` and `./scripts/test.sh` complete with zero warnings; `rg '@unchecked Sendable|nonisolated\(unsafe\)|@preconcurrency|Task\.detached|DispatchSemaphore|DispatchSourceTimer' Sources` returns nothing (a hit must be justified in the report, not silenced). |
| D8 performance | `docs/performance.md` holds the O1 and O2 runs and the O2 gate table; every gate passes or the miss is reported with numbers. |
| Report | `plan.md`'s required seven sections are present in the final report. |

Owner-verified after delivery (must not be claimed as campaign-verified): images render at correct aspect, video thumbnails and playback, Space/Escape/adjacent navigation, real Trash plus Finder Put Back, Retina crispness, resize without thumbnail regeneration, scroll smoothness, quit cancels pending work.

## Execution boundary

Authorized: read/write files in this repository; run builds, tests, inspections, and the generated corpus; launch and terminate `View.app`; initialize/commit locally (done) with Conventional commits; create fixtures only under `.work/`; trash only files this campaign created.

Forbidden: pushing; network access; sudo or accepting licenses; installing to `/Applications`, `~/.local/bin`, or anywhere outside this repository; modifying files outside this repository; deleting real user media or anything outside `.work/`; adding dependencies, CI, formatters, generators, or an Xcode project; building the GUI harness OD2 excludes; changing owner requirements.

Contract changes expected: none. If one becomes necessary, implement it only if it does not change user-visible behavior, then list it in the report.

## Non-goals

From the source spec, all out of scope: directory browsing/navigation/tree; search, filtering, tags, favorites, collections; library import/export/management; editing, cropping, rotating, transcoding; custom players, codecs, FFmpeg; slideshows, transitions, animated selection, decorative UI; AI/OCR/similarity/metadata indexing; preferences, config files, persistent libraries; background indexing/agents/daemons; networking, telemetry, analytics, updates; external runtime dependencies; handwritten Metal shaders or custom rendering without measured evidence.

Campaign-level: no CI, no benchmark framework or diagnostics UI, no GUI automation, no install script, no app icon, no sandboxing/localization/accessibility audit, no second decoder path without O3 evidence, no persistent thumbnail cache, no file watching or rescans.

## Reference

**Runtime shape.** `view [dir]` → `bin/view` resolves and validates → `open -n` launches `View.app` → app validates the same root → `@MainActor` window/grid → `@concurrent` scan → `@MainActor` catalog → `NSCollectionView` + thumbnail coordinator → `QLPreviewPanel` (Space) and `NSWorkspace.recycle` (Ctrl+Delete). No persistence, no network, no watchers.

**Verified SDK facts (checked against the installed macOS 27.0 SDK, not assumed).**

- `QLThumbnailGenerator.shared`, `generateBestRepresentation(for:completion:)`, `cancel(_:)`, `QLThumbnailGenerator.Request(fileAt:size:scale:representationTypes:)`, `QLThumbnailRepresentation.cgImage`/`.nsImage`. Completion-based only; the async bridge and cancellation shape are in P7.
- `QLPreviewPanel.shared()` is optional; `sharedPreviewPanelExists()`, `currentPreviewItemIndex`, `currentPreviewItem`, `reloadData()`, `dataSource`, `delegate`. Its protocols need isolated conformances.
- `NSWorkspace.shared.recycle([URL])` is `async throws -> [URL: URL]`.
- `NSCollectionViewPrefetching` is already main-actor annotated; `NSCollectionView` supports `insertItems(at:)`, `deleteItems(at:)`, `indexPathsForVisibleItems()`, native arrow navigation.
- `@concurrent` and `@MainActor` isolated conformances compile in Swift 6 language mode with `NonisolatedNonsendingByDefault`.
- `UTType.contentTypeKey` and `volumeIsLocalKey` resolve through `URLResourceValues` as used in P4/P8.

**Source reference (ideas only; no code copied, none vendored).** If any code is taken from them anyway, its license and attribution travel with it. Pixe (`ImageDirectoryWalker`, `ImageList`, `ThumbnailCache`, `GridLayout`, `InputHandler`; on-demand drawing) for recursive scanning and thumbnail caching patterns; MViewer (`PhotoLoader`, `FileNavigator`, `ViewerWindowController`) for bounded decoding and simple navigation; FlowVision for recursive media browsing at larger scale; Shoebox for a modern photo grid. All use Swift 5 mode or carry machinery outside this scope.

**File map after P1.** `Sources/viewCore/Discovery/{MediaItem,PathOrder,DirectoryScanner}.swift`, `Sources/viewCore/{Catalog,GridCommand,PreviewSession,Trash}.swift`, `Sources/viewCore/Thumbnails/{ThumbnailKey,ThumbnailCache,ThumbnailScheduler}.swift`, `Sources/view/{ViewApp,RootViewController,ThumbnailCoordinator,PreviewPanelController,TrashController}.swift`, `Sources/view/Grid/{GridViewController,ThumbnailCell}.swift`, `Tests/viewCoreTests/{Discovery,Catalog,Scheduler,PreviewSession,GridCommand,Trash,RootDirectory}Tests.swift`, `scripts/{build-app,test}`, `bin/view`, `Info.plist`, `README.md`, `AGENTS.md`; P3 adds `scripts/make-corpus` and `docs/performance.md`.

## Campaign log

- 2026-10-22 — Preflight. Verified toolchain, SDK symbols, launcher behavior, GUI/automation permissions, and loop timings. Scaffolding accepted at `21c798d` after build, 5 tests, app packaging, launcher error paths, and one windowed launch (1100×792 frame, title from root).
- 2026-10-22 — Owner answers recorded as OD1–OD3; source spec's GUI integration block superseded by OD2; terminology corrected to call media "items" (the app is `view`).
- 2026-10-22 — I1 accepted at `bc49a2b`; `./scripts/test.sh --filter Discovery` passed 9 tests covering ordering, filters, root and nested read failures, cancellation, and 127/128/129 batch boundaries. `PathOrder` uses the required comparison options and UTF-8 byte ordering for ties; the no-locale overload matches the canonical traversal vector. The public core surface is one static scan operation with immutable `Sendable` media items; next I2.
- 2026-10-22 — I2 accepted at `5a92004`; `./scripts/test.sh --filter 'Catalog|GridCommand'` passed 7 tests and `swift build` passed. The catalog appends in scanner order, owns URL selection and neighbor selection after removal, and drives incremental `NSCollectionView` inserts. `NSCollectionView` owns native arrow navigation and reuse; no GUI behavior is claimed verified under OD2. The public core API now includes catalog mutation and the Space command mapping; next I3.
- 2026-10-22 — I3 accepted at `5981c99`; `./scripts/test.sh --filter Scheduler` passed 11 tests and `swift build` passed. The scheduler bounds active requests, deduplicates and reprioritizes current demand, cancels stale work, and rejects stale results; the `NSCache` wrapper versions images by URL, size, scale, modification date, and file size with a 96 MiB ceiling. The Quick Look bridge rejects icon-only results and owns its cancellation task through completion. Runtime metrics and signposts are wired but measurement is deferred to O1 after I6; no GUI behavior is claimed verified under OD2. Next I4.
- 2026-10-22 — I4 accepted at `7cda6ba`; `./scripts/test.sh --filter PreviewSession` passed 6 tests and `swift build` passed. The window content controller owns panel control only through responder-chain callbacks; the session keeps a URL snapshot and updates the grid selection on Quick Look navigation. The Objective-C control callbacks are imported nonisolated, so they use `MainActor.assumeIsolated` at the AppKit callback boundary; this runtime-checked adapter keeps panel state main-actor isolated. No GUI behavior is claimed verified under OD2. Next I5.
- 2026-10-22 — I5 accepted at `8f3ee85`; `./scripts/test.sh --filter 'Trash|GridCommand'` passed 10 tests and `swift build` passed. The core service tracks in-flight URLs, requires the recycle mapping to contain the source URL, and only then calls the catalog-removal closure. The app uses `NSWorkspace.recycle`, removes the matching collection item, evicts thumbnail cache/demand, preserves catalog neighbor selection, refreshes the preview snapshot, and presents native failure sheets. The public core contract now includes `TrashService`, `TrashOutcome`, `TrashError`, and Ctrl+Delete mapping; no real Trash interaction was campaign-verified under OD2. Next I6.
- 2026-10-22 — I6 accepted at `ace0fce`; `./scripts/test.sh` passed all 49 tests and `swift build -c release` passed. Release output contains only the inherited Command Line Tools missing linker search-path warnings recorded in P1; no Swift warnings or errors appeared. Added exact URL identity checks for unusual names, sink-failure propagation, and scheduler shutdown coverage. App termination now awaits scan cancellation, scheduler/Quick Look request cancellation, and pending recycle completion. The root scanner's typed failure reaches the native alert path; actual panel, alert, thumbnail, and Trash appearance remain owner-verified under OD2. No command-line behavior changed. Next O1.

## For the owner afterwards

Read the final report, then verify by eye: `./scripts/build-app.sh && ./bin/view ~/Pictures`, check images/videos render, Space/Escape and adjacent navigation in Quick Look, Ctrl+Delete into Trash with Put Back, Retina crispness, resize, scroll smoothness, and idle CPU after settling. Nothing here gates the campaign; it is your acceptance pass.
