#!/bin/sh
set -eu

mkdir -p .work/tmp
export TMPDIR="$PWD/.work/tmp"
cargo build --profile dist --bin app --features app
bench_app="$PWD/.work/tmp/view-bench-app-$$"
cp target/dist/app "$bench_app"
trap 'unlink "$bench_app"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export VIEW_BENCH_APP="$bench_app"
cargo bench --profile dist --bench view --features benchmark --no-run
bench_bin=$(fd -t x --glob 'view-*' target/dist/build/view-core -x stat -f '%m %N' | sort -n | awk 'END { print $2 }')
if [ -z "$bench_bin" ]; then
    printf '%s\n' 'benchmark binary was not built' >&2
    exit 1
fi

case "${1:-}" in
    --test|--list|--help)
        cargo bench --profile dist --bench view --features benchmark -- "$@"
        exit $?
        ;;
    memory)
        typical=$("$bench_bin" --write-corpus | awk 'NR == 2 { print }')
        "$bench_app" --benchmark-memory "$typical"
        exit $?
        ;;
    advance_carousel_preview|generate_video_thumbnail|launch_and_paint_first_image|open_directory_and_render_grid|open_small_directory|open_typical_directory|recursive_media_scan|resize_and_render_grid|scroll_and_render_grid)
        scenario=$1
        shift
        ;;
    *)
        printf '%s\n' 'specify one benchmark name, memory, --test, or --list' >&2
        exit 1
        ;;
esac

if ! perf-gate-ready; then
    printf '%s\n' 'perf gate deferred: host busy' >&2
    exit 2
fi
set +e
cargo bench --profile dist --bench view --features benchmark -- "view::app::benchmarks::$scenario" --exact "$@"
result=$?
perf-gate-ready -1
gate=$?
set -e
if [ "$gate" -ne 0 ]; then
    printf '%s\n' 'perf gate deferred: host busy; discard this run' >&2
    exit 2
fi
exit "$result"
