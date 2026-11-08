# Performance

## Design budgets

- Thumbnail cache: ~96 MiB, memory-only, evicted under pressure.
- App-issued concurrent thumbnail requests: at most 4 on local volumes, 2 on remote.
- Blocking image or I/O work on the main actor: zero.
- Live cells and retained buffers: proportional to the viewport plus one prefetch viewport, not the catalog size.
- Idle work after scan and prefetch: effectively zero. No redraw loop, no polling timer, no eager decode of every file.
- Priorities in order: time to first thumbnails, smooth scrolling, minimal disk reads, bounded memory and concurrency, UI responsiveness under load, negligible idle CPU.

Measure window presentation, first discovered batch, first thumbnail, visible-viewport population, scan completion, fast-scroll responsiveness, cache warm versus cold behavior, peak and steady RSS, idle CPU, and concurrent request count. Do not ship a benchmarking UI.

## Measurements

Measured 2026-10-22 on macOS 27.0.1, arm64, Swift 6.4, macOS 27.0 SDK, with 18 CPU cores. The release app ran three times against a 2,000-item, 7.8 MB corpus: 1,000 varied PNGs, 980 PNG copies, and 20 one-second H.264 movies, plus hidden, packaged, linked, text, and audio entries. `perf-gate-ready` returned 0 before and after each run.

These launch numbers come from the original flattened grid, now selected with `--recursive`. Browser mode lists one directory and has not been measured separately.

After a five-second settle, CPU and RSS were sampled ten times over 20 seconds; physical footprint was sampled during each run. Latency values below are the median and range across three launches. This is a recorded baseline, not a comparison or speedup claim.

| Metric | Median | Range |
| --- | ---: | ---: |
| Window shown | 97.6 ms | 95.8–112.2 ms |
| First batch | 120.3 ms | 118.3–135.1 ms |
| Scan complete | 209.2 ms | 208.3–223.7 ms |
| First image displayed | 156.9 ms | 154.5–171.5 ms |
| Initial viewport settled | 181.0 ms | 176.7–191.8 ms |

Each run scanned 2,000 items in 16 batches and started 33 thumbnail requests. Maximum concurrent requests were 4 and recorded thumbnail cache cost was 9,216,000 bytes in every run. The first-image time ends when a real thumbnail is displayed. The initial viewport settles when each visible request completes; failed requests also settle and may leave placeholders.

The CPU means per run were 0.10%, 0.22%, and 0.07%; individual samples ranged from 0.0% to 1.5%. RSS medians were 100,960–105,096 KiB. Physical footprint at capture was 44–45 MB, with peaks of 50–51 MB.

| Gate | Observed | Limit | Result |
| --- | ---: | ---: | --- |
| First image | 156.9 ms median | 400 ms | Pass |
| Initial viewport settled | 181.0 ms median | 1,500 ms | Pass |
| Scan 2,000 items | 209.2 ms median | 1,000 ms | Pass |
| Concurrent thumbnail requests | 4 in each run | 4 | Pass |
| Thumbnail cache cost | 9,216,000 bytes | 100,663,296 bytes | Pass |
| Idle CPU mean | 0.07–0.22% | 0.5% | Pass |
| Steady RSS | 100,960–105,096 KiB medians | 500 MiB | Pass |
| Second-pass RSS growth | Not measured | 20% | Unmeasured |

Scrolling and second-pass memory growth were not measured. The three launches did not compare decoder implementations or establish whether Quick Look dominates thumbnail latency.

## Scanner comparison at 20,000 items

`swift scripts/make-corpus.swift .work/corpus-20k 20000` creates 10,000 varied PNGs, 9,800 PNG copies, 20 one-second H.264 movies, and 180 movie copies. The corpus occupied 97 MB. The scanner ran in release mode against the same corpus on each side. Five runs per side alternated between `f544159` and the per-entry failure handling change. `perf-gate-ready` returned 0 before each run and after each run with `-1`.

Every run found 20,000 media items in 157 batches. The scanner used a no-op batch sink, so the measurements cover discovery and batching without collection-view updates, thumbnail requests, or Quick Look. The baseline was `f544159`; the candidate was `b9021ed`.

| Metric | Baseline | Candidate |
| --- | ---: | ---: |
| Scan time median | 436.4 ms | 432.9 ms |
| Scan time range | 429.8–440.7 ms | 431.0–439.1 ms |
| Maximum resident set size | 30.1 MiB | 30.1 MiB |

The time ranges overlap. The scanner change shows no measurable performance difference on this corpus. `time -l` recorded 0.36–0.37 seconds of user CPU and 0.04–0.05 seconds of system CPU per scan. This serial discovery run used about one core.

The run did not measure window launch, thumbnail generation, forward or reverse scrolling, cache reuse, Quick Look during discovery, or memory growth during scrolling. Those UI measurements remain open.

The release `view` executable grew from 621,200 bytes at `f544159` to 652,600 bytes after these changes. The repository has no executable-size limit.

## Workflow benchmarks

`ViewBenchmarks` (Sources/ViewBenchmarks) measures user-facing waits against realistic corpora instead of micro-loops. The release build is required — optimized code only, as `swift test` builds debug code and measures differently. Run it through `make bench` (always release) or explicitly:

```sh
swift run -c release ViewBenchmarks -- \
  --browser-corpus .work/corpus-2k --recursive-corpus .work/corpus-20k --samples 5
```

It prints JSON lines (median, p95, min/max, RSS before/after/delta per workflow) to stdout; building in debug prints a warning to stderr but still runs.

Workflows: `browser-list` (list one directory + materialise the catalog), `catalog-interact` (5000-item select/range/remove/insert), `recursive-scan` (full discovery with batch sink), `scroll-prefetch` (sliding 24-visible + 48-prefetch viewport over the corpus scheduler), and `thumb-decode` (CGImageSource decode of 100 varied PNGs). Do not ship a benchmarking UI; fixture generation stays outside the timed regions.

Measured 2026-11-08 on arm64, macOS 27, release build, five samples per workflow. Browser listing used the first 100-item group folder of a 2,000-item corpus; recursive/scroll/decode used a fresh 20,000-item PNG corpus generated offline (mixed 400×300 / 300×400 / 500×500 / 1200×120 fills).

| Workflow | Scale | Median | p95 | RSS delta |
| --- | ---: | ---: | ---: | ---: |
| Browser list | 100-item directory | 5.5 ms | 10.9 ms | +6.2 MiB |
| Catalog interact | 5,000 items | 13.8 ms | 15.2 ms | +2.4 MiB |
| Recursive scan | 20,000 items | 1,372 ms | 1,385 ms | +32.3 MiB |
| Scroll/prefetch | 2,000-key sliding window | 206 ms | 208 ms | +4.5 MiB |
| Thumbnail decode | 100 PNGs | 9.3 ms | 12.4 ms | +0.2 MiB |

These are recorded workflow baselines, not comparison claims. The recursive scan is the outlier worth watching: it sits above the 1,000 ms launch budget for 2,000 items and grows with corpus size, so fast-scroll and per-batch collection-view updates need their own measurements before claiming a speedup. Steady-state RSS stayed near 50 MiB in this run; peak footprint and idle CPU still need the launches above for a full picture.
