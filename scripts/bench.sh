#!/bin/sh
set -eu

: "${VIEW_BENCH_SMALL:?set VIEW_BENCH_SMALL to a small media directory}"
: "${VIEW_BENCH_TYPICAL:?set VIEW_BENCH_TYPICAL to a typical media directory}"
: "${VIEW_BENCH_LARGE:?set VIEW_BENCH_LARGE to a large media tree}"

mkdir -p .work/tmp
cargo build --profile dist --bin app --features app
bench_app="$PWD/.work/tmp/view-bench-app-$$"
cp target/dist/app "$bench_app"
trap 'unlink "$bench_app"' EXIT
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
export VIEW_BENCH_APP="$bench_app"
cargo bench --profile dist --bench view --features benchmark --no-run

case "${1:-}" in
    --test|--list|--help)
        cargo bench --profile dist --bench view --features benchmark -- "$@"
        exit $?
        ;;
    advance_carousel_preview|generate_video_thumbnail|launch_and_paint_first_image|open_directory_and_render_grid|open_small_directory|open_typical_directory|recursive_media_scan|resize_and_render_grid|scroll_and_render_grid)
        scenario=$1
        shift
        ;;
    *)
        printf '%s\n' 'specify one benchmark name, --test, or --list' >&2
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
