# Performance

## Budgets

| Metric | Target | Notes |
| --- | --- | --- |
| First image | ≤ 400 ms median | Open to first rendered image |
| Initial viewport settled | ≤ 1,500 ms | |
| Scan 2,000 items | ≤ 1,000 ms | |
| Steady RSS | ≤ 500 MiB | During browsing |
| Browser list (100 items) | ≤ 5 ms median | Hot metadata pass |
| Recursive scan (20,000) | ≤ 300 ms median | Hot traversal |

## Rust baseline (dist profile)

Toolchain: `nightly-2026-09-15` (`rust-toolchain.toml`), profiles mirror `~/d/ish/Cargo.toml` (`release` thin-LTO, `dist` fat-LTO single codegen-unit stripped). Dist is the reporting profile. Measured 2026-11-08 on arm64, macOS 27: browser listing on a 100-item group folder of a 2,000-item PNG corpus; recursive/scroll/decode on a 20,000-item PNG corpus. Five-plus samples per workflow via the bench harness (`src/bin/bench.rs`), printing JSON lines (median, p95, min/max, RSS before/after/delta).

Run:

```sh
cargo run --profile dist --bin bench -- --browser-corpus .work/corpus-2k/group-00 --recursive-corpus .work/corpus-20k --samples 7
```

| Workflow | Scale | Median | p95 | RSS delta |
| --- | --- | ---: | ---: | ---: |
| Browser list | group-00 (100 items) | 0.37 ms | 0.99 ms | +336 KiB |
| Catalog interact | 5,000 items | 12.87 ms | 13.16 ms | +1,456 KiB |
| Recursive scan | 20,000 items | 215–298 ms | 571–697 ms | +6,832 KiB |
| Scroll/prefetch | 20,000 items | 14–16 ms | 15–17 ms | +384 KiB to 0 |
| Thumb decode | single PNG | 26.6–27.5 ms pure-Rust | 28.5–29 ms | +12,976 KiB |

Thumb-decode uses the pure-Rust decoder (`decode_png`); the ImageIO path (`decode_png_native`) renders through `CGImageSource` + `CGBitmapContext` for system-backend comparisons.

## Measured app budgets

Steady RSS during browsing was 101–105 MiB medians (packaged app runs), with peaks near 50 MiB physical footprint in memory instrumentation. First image ≈ 157 ms, initial viewport ≈ 181 ms, 2,000-item scan ≈ 209 ms — all within budget. CPU means were 0.07–0.22% steady-state with 1.5% max samples.
