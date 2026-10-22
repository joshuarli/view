# Performance measurements

## O1 baseline

Recorded 2026-10-22 on macOS 27.0.1, arm64, Swift 6.4, and the macOS 27.0 SDK. The machine has 18 CPU cores.

The corpus contains 2,000 media items: 1,000 varied-color PNGs, 980 PNG copies, and 20 one-second H.264 movies. It also contains text and audio noise, a hidden image, a package image, and file and directory symlinks. The corpus occupies 7.8 MB.

The corpus and release app were prepared with these commands:

```sh
swift scripts/make-corpus.swift .work/corpus
TMPDIR="$PWD/.work/tmp" ./scripts/build-app.sh
```

Each run launched the bundled executable in a foreground session. The readiness gate returned 0 before and immediately after each run.

```sh
VIEW_METRICS=1 ./View.app/Contents/MacOS/view "$PWD/.work/corpus" > .work/tmp/o1-run1.metrics 2>&1
```

After a five-second settle, CPU and RSS were sampled ten times over 20 seconds. Physical footprint was sampled during each run.

```sh
for sample in $(seq 1 10); do ps -o %cpu= -o rss= -p "$APP_PID"; sleep 2; done
footprint --sample 1 --sample-duration 25 --noCategories "$APP_PID"
```

| Run | Window shown ms | First batch ms | Scan complete ms | First thumbnail ms | Visible populated ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 97.7 | 122.2 | 215.2 | 153.6 | 181.9 |
| 2 | 95.7 | 118.0 | 205.0 | 146.0 | 171.5 |
| 3 | 95.0 | 116.4 | 202.8 | 144.3 | 168.6 |

| Run | Scan items | Scan batches | Thumbnail requests | Max concurrent requests | Thumbnail cache bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,000 | 16 | 33 | 4 | 9,216,000 |
| 2 | 2,000 | 16 | 33 | 4 | 9,216,000 |
| 3 | 2,000 | 16 | 33 | 4 | 9,216,000 |

| Metric | Median | Range |
| --- | ---: | ---: |
| Window shown | 95.7 ms | 95.0–97.7 ms |
| First batch | 118.0 ms | 116.4–122.2 ms |
| Scan complete | 205.0 ms | 202.8–215.2 ms |
| First thumbnail | 146.0 ms | 144.3–153.6 ms |
| Initial viewport populated | 171.5 ms | 168.6–181.9 ms |

Idle CPU was sampled ten times over 20 seconds after each viewport settled. RSS samples use `ps -o rss=`. `footprint --sample 1 --sample-duration 25 --noCategories` recorded physical footprint during each run.

| Run | Idle CPU mean and range | RSS median and range | Current physical footprint | Peak physical footprint |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.00% (0.0–0.0%) | 103,904 KiB (103,872–103,968 KiB) | 44 MB | 49 MB |
| 2 | 0.05% (0.0–0.4%) | 103,992 KiB (103,984–104,080 KiB) | 44 MB | 53 MB |
| 3 | 0.09% (0.0–0.6%) | 105,024 KiB (104,976–105,072 KiB) | 45 MB | 54 MB |

The O1 result is a baseline, not a speedup claim. The three runs produced matching item, batch, request, concurrency, and cache counts. The P13 metrics do not count failed thumbnail requests. The initial viewport metric includes cells with either an image or a latched placeholder.

The scroll pass comparison was not measured. OD2 excludes GUI automation, and these launches did not send scroll input. An initial background launch exited before emitting metrics, so it was discarded. The three recorded launches used foreground sessions.

## O2 gates

The O1 values met each measured O2 threshold. After the scheduler cancellation fix, one additional gated release launch checked startup behavior and the concurrency metric. It produced 156.8 ms first-thumbnail time, 177.9 ms initial viewport time, 211.4 ms scan completion, 4 maximum concurrent requests, and 9,216,000 cache bytes. Its timings stayed within 1.03× of the O1 ranges. This single check is a regression check, not a new baseline distribution.

| Gate | O1 result | Post-fix check | Limit | Status |
| --- | ---: | ---: | ---: | --- |
| First thumbnail | 146.0 ms median | 156.8 ms | 400 ms | Pass |
| Initial viewport populated | 171.5 ms median | 177.9 ms | 1,500 ms | Pass |
| Scan 2,000 items | 205.0 ms median | 211.4 ms | 1,000 ms | Pass |
| Concurrent thumbnail requests | 4 in each run | 4 | 4 | Pass |
| Thumbnail cache cost | 9,216,000 bytes | 9,216,000 bytes | 100,663,296 bytes | Pass |
| Idle CPU mean | 0.00–0.09% | 0.01% | 0.5% | Pass |
| Steady RSS | 103,904–105,024 KiB medians | 100,064 KiB median | 500 MiB | Pass |
| Second-pass RSS growth | Not measured | Not measured | 20% | Unmeasured under OD2 |

Source inspection found directory enumeration, metadata reads, volume lookup, and Quick Look generation outside the main actor. The main actor wraps returned `CGImage` values in `NSImage` for display; there is no synchronous decoder or file read on that path. This is source evidence, not a main-actor latency trace. The scroll-memory gate needs owner verification because measuring it requires UI input.

No tuning parameter changed. The baseline already meets the measured latency, memory, and idle CPU thresholds. The cache remains 96 MiB, local concurrency remains 4, prefetch remains one viewport ahead, and scan batches remain 128 items. The post-fix sample did not exceed 1.5× any O1 timing, and its values were within the observed spread except first-thumbnail time, which was 2.1% above the O1 maximum.

## O3 decision

The ImageIO experiment was not triggered. O1 first-thumbnail time was 146.0 ms p50 against a 600 ms trigger, and initial viewport time was 171.5 ms p50 against a 2,500 ms trigger. No signpost trace was collected to test the alternate Quick Look dominance condition, and no second thumbnail path was added.
