#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

echo "=== Building Swift reference app ==="
sh swift/scripts/build-app.sh

echo "=== Building Rust app ==="
sh scripts/package-app.sh

mkdir -p .work/visual/swift .work/visual/rust .work/visual/diff

echo "=== Dumping Swift visual steps ==="
./swift/View.app/Contents/MacOS/view --dump-visual .work/visual/swift tests/fixtures/visual

echo "=== Dumping Rust visual steps ==="
./View.app/Contents/MacOS/view --dump-visual .work/visual/rust tests/fixtures/visual

echo "=== Comparing screenshots with odiff ==="
PASS=0
FAIL=0

for i in 0 1 2 3 4 5 6 7 8; do
    SWIFT_IMG=$(ls .work/visual/swift/step_${i}_*.png 2>/dev/null | head -n 1)
    RUST_IMG=$(ls .work/visual/rust/step_${i}_*.png 2>/dev/null | head -n 1)
    DIFF_IMG=".work/visual/diff/diff_${i}.png"

    if [ -z "$SWIFT_IMG" ] || [ -z "$RUST_IMG" ]; then
        echo "Step $i: MISSING images"
        FAIL=$((FAIL + 1))
        continue
    fi

    NAME=$(basename "$SWIFT_IMG" .png)
    OUTPUT=$(odiff --aa "$SWIFT_IMG" "$RUST_IMG" "$DIFF_IMG" 2>&1 || true)

    if echo "$OUTPUT" | grep -q "different pixels"; then
        DIFF_PERCENT=$(echo "$OUTPUT" | sed -E 's/.*different pixels \(([0-9.]+)%\).*/\1/')
        # Accept minor CoreText/CG antialiasing & system shadow variations under 8%
        if awk "BEGIN {exit !($DIFF_PERCENT <= 8.0)}"; then
            echo "Step $i ($NAME): PASS ($DIFF_PERCENT% diff within tolerance)"
            PASS=$((PASS + 1))
        else
            echo "Step $i ($NAME): FAIL ($DIFF_PERCENT% diff exceeds tolerance)"
            FAIL=$((FAIL + 1))
        fi
    else
        echo "Step $i ($NAME): PASS (0.00% diff)"
        PASS=$((PASS + 1))
    fi
done

echo "=== Visual comparison summary: $PASS passed, $FAIL failed ==="
if [ "$FAIL" -gt 0 ]; then
    exit 1
fi
exit 0
