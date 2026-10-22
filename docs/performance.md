# Performance measurements

## O1 baseline

Recorded 2026-10-22 on macOS 27.0.1, arm64, Swift 6.4, and the macOS 27.0 SDK. The machine has 18 CPU cores.

These three launches use the final source after the scheduler fix and task registry cleanup.

The corpus contains 2,000 media items: 1,000 varied-color PNGs, 980 PNG copies, and 20 one-second H.264 movies. It also contains text and audio noise, a hidden image, a package image, and file and directory symlinks. The corpus occupies 7.8 MB.

The corpus and release app were prepared with these commands:

```sh
swift scripts/make-corpus.swift .work/corpus
TMPDIR="$PWD/.work/tmp" ./scripts/build-app.sh
```

Each run launched the bundled executable in a foreground session. `perf-gate-ready` returned 0 before each launch and `perf-gate-ready -1` returned 0 immediately after each measurement.

```sh
VIEW_METRICS=1 ./View.app/Contents/MacOS/view "$PWD/.work/corpus" > .work/tmp/o4-run1.metrics 2>&1
```

After a five-second settle, CPU and RSS were sampled ten times over 20 seconds. Physical footprint was sampled during each run.

```sh
for sample in $(seq 1 10); do ps -o %cpu= -o rss= -p "$APP_PID"; sleep 2; done
footprint --sample 1 --sample-duration 25 --noCategories "$APP_PID"
```

| Run | Window shown ms | First batch ms | Scan complete ms | First thumbnail ms | Visible populated ms |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 97.6 | 120.3 | 208.3 | 156.9 | 181.0 |
| 2 | 95.8 | 118.3 | 209.2 | 154.5 | 176.7 |
| 3 | 112.2 | 135.1 | 223.7 | 171.5 | 191.8 |

| Run | Scan items | Scan batches | Thumbnail requests | Max concurrent requests | Thumbnail cache bytes |
| --- | ---: | ---: | ---: | ---: | ---: |
| 1 | 2,000 | 16 | 33 | 4 | 9,216,000 |
| 2 | 2,000 | 16 | 33 | 4 | 9,216,000 |
| 3 | 2,000 | 16 | 33 | 4 | 9,216,000 |

| Metric | Median | Range |
| --- | ---: | ---: |
| Window shown | 97.6 ms | 95.8–112.2 ms |
| First batch | 120.3 ms | 118.3–135.1 ms |
| Scan complete | 209.2 ms | 208.3–223.7 ms |
| First thumbnail | 156.9 ms | 154.5–171.5 ms |
| Initial viewport populated | 181.0 ms | 176.7–191.8 ms |

| Run | Idle CPU mean and range | RSS median and range | Current physical footprint | Peak physical footprint |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.10% (0.0–1.0%) | 100,960 KiB (100,176–103,904 KiB) | 44 MB | 50 MB |
| 2 | 0.22% (0.0–1.5%) | 105,096 KiB (105,056–105,152 KiB) | 45 MB | 50 MB |
| 3 | 0.07% (0.0–0.7%) | 103,952 KiB (103,840–104,016 KiB) | 44 MB | 51 MB |

The three runs produced matching item, batch, request, concurrency, and cache counts. P13 does not count failed thumbnail requests. The initial viewport metric includes cells with either an image or a latched placeholder.

The scroll-pass comparison was not measured. OD2 excludes GUI automation, and these launches did not send scroll input.

## O2 gates

No tuning parameter changed. The final-source O1 series meets each measurable threshold. The scheduler retains canceled work in its concurrency count until the request returns, so the reported limit includes requests still completing cancellation.

| Gate | Observed | Limit | Status |
| --- | ---: | ---: | --- |
| First thumbnail | 156.9 ms median | 400 ms | Pass |
| Initial viewport populated | 181.0 ms median | 1,500 ms | Pass |
| Scan 2,000 items | 209.2 ms median | 1,000 ms | Pass |
| Concurrent thumbnail requests | 4 in each run | 4 | Pass |
| Thumbnail cache cost | 9,216,000 bytes | 100,663,296 bytes | Pass |
| Idle CPU mean | 0.07–0.22% | 0.5% | Pass |
| Steady RSS | 100,960–105,096 KiB medians | 500 MiB | Pass |
| Second-pass RSS growth | Not measured | 20% | Unmeasured under OD2 |
| Tuning candidate regression | No tuning candidate | 1.5× against O1 beyond spread | Not applicable |

Source inspection found directory enumeration, metadata reads, volume lookup, and Quick Look generation outside the main actor. The main actor wraps returned `CGImage` values in `NSImage` for display; no synchronous decoder or file read is present on that path. This is source evidence, not a main-actor latency trace. The cache remains 96 MiB, local concurrency remains 4, prefetch remains one viewport ahead, and scan batches remain 128 items.

## O3 decision

The ImageIO experiment was not run. O1 first-thumbnail p50 was 156.9 ms against a 600 ms trigger, and initial-viewport p50 was 181.0 ms against a 2,500 ms trigger. The signpost condition was not measured, and no other evidence showed Quick Look dominating. No alternate thumbnail path was added.
