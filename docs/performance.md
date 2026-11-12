# Performance

These targets describe interactive browsing on an Apple Silicon Mac. They are
engineering goals, not automated release gates. The measurements below are
historical results from 2026-11-09, before the tab and later playback changes.
They do not establish the current revision's latency, memory use, or binary size.

## Targets

| Workflow | Median target |
| --- | ---: |
| Launch and paint the first image | ≤ 250 ms |
| List a small directory | ≤ 2 ms |
| List a typical directory | ≤ 5 ms |
| Recursively scan the large tree | ≤ 100 ms |
| Open a directory and paint the grid | ≤ 50 ms |
| Advance an image in the carousel and paint | ≤ 16.7 ms |
| Generate a warm video thumbnail | ≤ 100 ms |
| Resize and paint the grid | ≤ 16.7 ms |
| Scroll and paint the grid | ≤ 16.7 ms |

For carousel, resize, and scroll, also aim for p95 ≤ 33.3 ms. Rustybench 0.4
reports medians but does not expose the samples needed to calculate p95, so this
tail target is not yet measurable by the harness. Aim for at most ten Rust
allocations and 64 KiB allocated per interactive operation. These allocation
goals exclude AppKit, ImageIO, and other native allocations.

The existing memory budget is 500 MiB resident while browsing. The memory
workload reports the largest of 81 resident-set snapshots, not the true peak.
It draws 80 grid positions in one app process and reports the final snapshot.
It does not wait for asynchronous video thumbnails to settle.

## Corpus

The harness writes its corpus under `.work/bench-corpus` and reuses that tree
while the `READY` stamp matches the generator. Benchmarks read this tree.

`small` and `typical` hold generated 1920×1080 JPEGs at quality 0.55, each
with its own noise seed. `small` holds 42, and the largest sorts first for
launch. `typical` holds 160 plus `tests/fixtures/carousel.mp4`, and it drives
the grid, carousel, video, and memory workloads. `large` holds 3,300 distinct
small JPEGs in 33 directories, and it drives recursive scanning.

Generation runs before the timed samples. The movie bytes are the fixture the
Quick Look test already uses.

## Run

The benchmark target currently cannot compile: `prepare_window` and other
workloads in `src/bin/app/benchmarks.rs` still access fields moved from
`AppState` into `Tab`. Repair those references before using these commands.
The production app and benchmark are separate targets.

The runner also requires `fd` and the external `perf-gate-ready` command.
After repairing the harness, check workload completion with:

```sh
sh scripts/bench.sh --test
```

Run one workflow per process. This prevents thumbnail state from one workload
from warming another. For example:

```sh
sh scripts/bench.sh scroll_and_render_grid --format json --sample-count 100 --sample-size 1 --min-time 0
```

Use `sh scripts/bench.sh --list` for the names. The script builds a production
`dist` app and a separate Rustybench target, then checks `perf-gate-ready`
immediately before and after the timed run. Exit status 2 means the gate was
busy; discard that run. Run at least five independent processes and compare
medians and their spread. For before/after comparisons, interleave the two
versions. Record failures as failures, not timings.

To measure resident memory in the production app, run:

```sh
sh scripts/bench.sh memory
```

The script builds the `dist` app, writes the corpus, and runs
`--benchmark-memory` on `typical`. `VIEW_BENCH_APP` is set by the script for
the launch benchmark only.

The JSON output reports frame count, maximum sampled RSS, and final RSS in
bytes. Rustybench 0.4 from crates.io does not report macOS RSS; the upstream
runner now supports it, but the app-process check remains necessary because the
launch benchmark runs a child process.

## Workload boundaries

Directory benchmarks time the app's listing and recursive discovery functions.
The grid benchmarks use the app's real `GridView`, scroll view, window layout,
and AppKit drawing. They repaint into an AppKit bitmap with a warm filesystem
cache and drain an autorelease pool after each operation. Opening includes directory listing, layout, and synchronous grid paint;
scrolling moves by 24 items and paints the visible viewport; resizing alternates
between 800 and 1,000 points wide. The carousel sends a right-arrow event
between adjacent images and paints the strip and preview. The video benchmark
requests a real Quick Look thumbnail and waits for completion on the main run
loop. System thumbnail caches may be warm.

Launch includes starting and exiting the production app process, listing the
small directory, and synchronously painting the initial carousel preview. It
does not measure compositor presentation or a settled viewport. Rustybench's
allocation numbers for this workload belong to the parent runner, so do not use
them as app allocation measurements. The in-process allocation counts cover
Rust allocations on the benchmark thread, not native framework allocations or
background work. The synchronous grid measurements do not wait for all video
thumbnails to finish. None of these measurements represents a cold disk cache.

## Historical baseline

The table below is the private-corpus measurement from 2026-11-09, before
concurrent thumbnail decode. The harness no longer reads that disk. Compare a
generated-corpus run only with the generated-corpus section.

On that date, an Apple M1 with 8 GiB RAM and macOS 27.2 ran five independent
guarded processes per workload, with five samples per process. The two paint
paths alternated, and each process passed `perf-gate-ready` immediately before
and after the timed work. The values below are the median of the five run
medians. The range is the spread of those run medians. Rustybench reported
process CPU time as unsupported, so these times are wall-clock medians. The
small folder held 42 images. The typical folder held 157 images and 20 videos.
The recursive tree held roughly 3,300 media files. The typical image median
was about 230 KiB. This is one warm-cache corpus, not a universal device or
workload baseline.

| Workflow | Median ms | Range ms | Rust allocations | Rust bytes |
| --- | ---: | ---: | ---: | ---: |
| Launch and paint first image | 177.6 | 175.4–180.1 | n/a | n/a |
| List small directory | 0.115 | 0.114–0.126 | 171 | 6,729 |
| List typical directory | 0.366 | 0.362–0.378 | 712 | 22,329 |
| Recursive scan | 16.2 | 16.1–17.1 | 113,927 | 3,573,906 |
| Open and paint grid | 8.3 | 8.1–9.3 | 859 | 39,925 |
| Advance carousel image | 31.6 | 31.3–32.6 | 57 | 2,438 |
| Generate video thumbnail | 9.3 | 6.2–20.3 | 30 | 1,022 |
| Resize and paint grid | 9.1 | 9.0–9.1 | 142 | 6,082 |
| Scroll and paint grid | 82.7 | 82.5–85.1 | 201 | 17,685 |

Open and resize repeat one viewport inside the process, so those medians are
warm cache hits after the first paint. Scroll advances 24 items per sample, so
its median is a mostly new page. Against the previous paint path in the same
session, launch, both directory lists, and the recursive scan stayed inside
the earlier spread. Scroll moved from 98.8 ms (96.2–126.4) to 82.7 ms, and
its Rust bytes moved from 10,454,659 to 17,685. Resize moved from 26.9 ms
(25.7–27.7) to 9.1 ms. Open moved from 27.6 ms (26.8–29.8) to 8.3 ms. Carousel
moved from 52.1 ms (49.1–55.6) to 31.6 ms. The video row moved from 29.7 ms
(27.0–37.4) to 9.3 ms because thumbnail completion repaints the grid, and that
repaint no longer reads each visible file into a Rust buffer. The video
request is still a 480-point Quick Look thumbnail at scale 2.

An unreported sample of the scroll benchmark, taken after the timed runs,
spent about half of its main-thread paint samples in
`CGImageSourceCreateThumbnailAtIndex`, most of them in AppleJPEG. That serial
decode is why the table above stays near 83 ms. A warm resize of cells already
in the cache is 9.1 ms.

Concurrent decode of visible cache misses shipped after that table. On the
same retired corpus, five interleaved processes of five samples moved scroll
from 82.731 ms (82.529–121.718) to 27.614 ms (26.530–27.806). Carousel measured
31.142 ms (30.977–31.230) before that change and 31.107 ms (31.014–32.405)
after it. Resize measured 8.955 ms (8.922–9.845) before and 9.178 ms
(9.081–9.682) after. The carousel and resize ranges overlap across the two
builds. Scroll's ranges do not. Carousel Rust allocations rose from 51–57
(2,122–2,438 bytes) to 70–74 (3,913–4,119 bytes), still under the 64 KiB aim.

Five production-app memory runs on the typical directory sampled a median
maximum of 114.2 MiB RSS (range 114.1–114.2) and a median final RSS of 113.7
MiB (range 113.7–113.8). The previous paint path sampled 86.4 MiB maximum and
84.5 MiB final. The increase is the retained display-sized thumbnails. Those
runs were below the 500 MiB budget. The memory workload does not wait for video
thumbnails.

The production `dist` binary, with the debug info that the `dist` profile
keeps, was 866,176 bytes before this paint change and 886,912 bytes after.
Rustybench and its dependencies are enabled only by the `benchmark` feature.
Production build-time impact has not been measured. Binary size after
concurrent decode has not been remeasured.

## Historical generated-corpus results

On 2026-11-09 the same Apple M1 ran that revision's `dist` bench on the generated
corpus. Five processes of five samples were interleaved with a rejected
point-size build. The rows below are the retained build from that comparison.
Each value is the median of the five run medians, and the range is the spread of
those medians.

| Workflow | Median ms | Range ms | Rust allocations | Rust bytes |
| --- | ---: | ---: | ---: | ---: |
| Advance carousel image | 35.760 | 34.777–36.576 | 52 | 2,949 |
| Resize and paint grid | 8.012 | 7.930–8.476 | 82 | 3,712 |
| Scroll and paint grid | 50.235 | 48.725–53.257 | 154–160 | 12,204–18,702 |

Launch, both directory lists, the recursive scan, open, and the video
thumbnail have no five-process measurement on this corpus.

One instrumented scroll process is excluded from the table. Its first draw
decoded 20 cells in 49–53 ms. Its second draw blitted those cells in about
8 ms from the cache. One instrumented carousel advance decoded the new
1920×1080 frame in 11–12 ms. The medians above include that decode and the
paint after it. Resize stays under 16.7 ms on a warm cache. Scroll and
carousel stay over 16.7 ms because the timed region decodes each new JPEG.

A backing-scale point size for each still was the other side of the same
interleave. Scroll measured 49.217 ms (48.925–57.450), carousel 35.692 ms
(35.520–37.238), and resize 8.157 ms (7.799–8.354). Each range overlaps the
current-code side.

Removing the grid layer was measured later, while another build was using the
machine. Scroll measured 55.248 ms (54.979–64.825) against a same-interleave
current-code side of 52.507 ms (51.007–63.628). Resize measured 9.159 ms
(8.286–10.195) against 9.052 ms (8.502–9.137). Carousel measured 35.767 ms
(34.910–35.925) against 35.775 ms (35.387–38.593). The ranges overlap.
Sampled maximum RSS was 106.33 MiB (106.22–106.36) against 106.06 MiB
(105.81–106.38). Treat that resident-set pair as a busy-host sample.
