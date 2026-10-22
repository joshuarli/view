# Performance measurements

## O1 baseline

The three recorded runs used the release app on macOS 27.0.1, arm64, Swift 6.4, and the macOS 27.0 SDK. The machine has 18 CPU cores.

The corpus contains 2,000 media items: 1,000 varied-color PNGs, 980 PNG copies, and 20 one-second H.264 movies. It also contains text and audio noise, a hidden image, a package image, and file and directory symlinks. The corpus occupies 7.8 MB.

The app was built with `TMPDIR="$PWD/.work/tmp" ./scripts/build-app.sh`. Each run launched `View.app/Contents/MacOS/view` in a foreground session with `VIEW_METRICS=1`. The readiness gate returned 0 before and immediately after each run.

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
