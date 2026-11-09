# Rust port (core only)

Core port of `viewCore` for head-to-head benchmarking against `ViewBenchmarks`.
The AppKit GUI, Quick Look, and panel controllers are not ported.

Toolchain is pinned by `../../rust-toolchain.toml` (nightly-2026-09-15).
Profiles mirror `~/d/ish/Cargo.toml` (`release` thin-LTO, `dist` fat-LTO),
and dist is the profile used for the final comparison:

```sh
cargo run --release --bin bench -- --browser-corpus ../corpus-2k/group-00 --recursive-corpus ../corpus-20k --samples 5
# final comparison profile (fat LTO, single codegen unit):
cargo run --profile dist --bin bench -- --browser-corpus ../corpus-2k/group-00 --recursive-corpus ../corpus-20k --samples 5
```

Measure both sides with release/dist and report medians as JSON (median, p95, RSS deltas). Swift side stays at `swift run -c release ViewBenchmarks`.
