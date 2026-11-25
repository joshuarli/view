# Roadmap

This is the single plan for this repository: all current and future work, and the handoff for resuming it. Campaigns are sections below, and their items interleave in **Sequence**. "Resume the roadmap" means continue from **Status**.

## Status

Native inline video trimming is complete and verified on macOS (2026-11-25). `T` or Trim Video… enters Apple's native trim UI for one ready local `.mp4`, `.m4v`, or `.mov` in the carousel. Trim offers a separately named copy or an explicit replace; cancel, a full-range selection, unsupported media, and any failure leave the original untouched. Passthrough export is validated before publication.

Verified: core, binary, app-interaction, UI-parity, and full `dist` suites; a native player regression covering `T`, cancel, native handle drag, Trim, replace confirmation, export, and player reload; `make lint`; `dist` build and packaging. A 1–4 second export of the checked-in H.264 fixture took 25 ms, produced 12,788 bytes, and reported exactly 1800/600 seconds at 640×360 with one video track.

Remaining verified limitations: boundaries can snap to encoded keyframes and edit lists; passthrough export cost scales with media size and storage; replacement is not undoable and refuses hard-linked sources; quitting during an export can leave its temporary directory because the app has no termination coordinator; the benchmark harness still cannot compile, so no fresh browsing-performance measurements accompany this campaign. See [performance](performance.md#trimming-measurements).
