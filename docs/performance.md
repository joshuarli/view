# Performance

These targets describe interactive browsing on an Apple Silicon Mac. They are
engineering goals, not automated release gates. Compare changes on the same
machine and unchanged corpus. The current baseline misses the interactive
rendering targets.

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

Supply three stable, local, read-only datasets. Paths are not included in the
benchmark report. Use diverse real images and videos, not empty files or copies
of a single image. Keep the files unchanged between comparisons.

- `VIEW_BENCH_SMALL`: a directory with at least 20 visible items. Its first
  sorted item must be an image for the launch benchmark.
- `VIEW_BENCH_TYPICAL`: a directory with at least 100 visible items, a video,
  and two adjacent images in the app's sort order. It drives grid and carousel
  workloads.
- `VIEW_BENCH_LARGE`: a tree with at least 1,000 discoverable media files. It
  drives recursive scanning.

A corpus can contain private material. Benchmarks read its files and can show
them in an AppKit window. Keep reports and diagnostic output private if they
contain paths or error messages.

## Run

Set the corpus paths, then check that every workload completes:

```sh
export VIEW_BENCH_SMALL=/path/to/small
export VIEW_BENCH_TYPICAL=/path/to/typical
export VIEW_BENCH_LARGE=/path/to/large
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
cargo build --profile dist --bin app --features app
target/dist/app --benchmark-memory "$VIEW_BENCH_TYPICAL"
```

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

## Baseline

On 2026-11-09, an Apple M1 with 8 GiB RAM and macOS 27.2 ran five independent
guarded processes per workload, with five samples per process. The values below
are the median of the five run medians; the range is the spread of those run
medians. The small folder held 42 images. The typical folder held 157 images and
20 videos. The recursive tree held roughly 3,300 media files. The typical image
median was about 230 KiB. This is one warm-cache corpus, not a universal device
or workload baseline.

| Workflow | Median ms | Range ms | Rust allocations | Rust bytes |
| --- | ---: | ---: | ---: | ---: |
| Launch and paint first image | 180.3 | 178.2–182.8 | n/a | n/a |
| List small directory | 0.119 | 0.115–0.123 | 171 | 6,729 |
| List typical directory | 0.376 | 0.372–0.380 | 712 | 22,329 |
| Recursive scan | 17.2 | 16.8–17.7 | 113,927 | 3,573,906 |
| Open and paint grid | 29.0 | 27.7–33.1 | 860 | 2,548,403 |
| Advance carousel image | 53.3 | 51.0–56.7 | 45 | 4,405,124 |
| Generate video thumbnail | 30.7 | 30.4–34.8 | 27 | 1,260,524 |
| Resize and paint grid | 31.3 | 30.2–32.2 | 148 | 2,526,074 |
| Scroll and paint grid | 117.6 | 117.1–140.3 | 86 | 10,454,659 |

One production-app memory run on the same typical directory sampled a maximum
of 86.6 MiB RSS after a frame and 84.7 MiB at the end. A separate diagnostic
profile of grid scrolling showed ImageIO JPEG decoding on the main drawing path.
The old Swift comparison numbers and synthetic scheduler and decoder timings do
not measure these workflows and are not comparable to this baseline.

The production `dist` binary was 738,000 bytes before this benchmark work and
744,576 bytes afterward. The added command-line instrumentation accounts for
the application change. Rustybench and its dependencies are enabled only by the
`benchmark` feature. Production build-time impact has not been measured.
