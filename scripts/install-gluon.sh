#!/bin/sh
# Install the gluon toolchain (meson, ninja, gluon-build) from the latest
# prerelease of joshuarli/gluon into a per-workspace directory. Override the
# destination with GLUON_DIR; it defaults to .work/gluon.
set -eu
cd "$(dirname "$0")/.."
GLUON_DIR="${GLUON_DIR:-$PWD/.work/gluon}"
REPO="${GLUON_REPO:-joshuarli/gluon}"
mkdir -p "$GLUON_DIR/bin"

case "$(uname -sm)" in
    "Darwin arm64") triple="aarch64-apple-darwin" ;;
    "Linux x86_64") triple="x86_64-unknown-linux-musl" ;;
    *) echo "install-gluon: unsupported platform: $(uname -sm)" >&2; exit 1 ;;
esac

tmpdir=$(mktemp -d)
trap 'rm -rf "$tmpdir"' EXIT
cd "$tmpdir"
tag=$(gh release list --repo "$REPO" --limit 1 --json tagName -q '.[0].tagName' 2>/dev/null || true)
if [ -z "${tag:-}" ]; then
    echo "install-gluon: no release found for $REPO" >&2
    exit 1
fi
gh release download "$tag" --repo "$REPO" --clobber --pattern "gluon-$triple.zip"
gh release download "$tag" --repo "$REPO" --clobber --pattern "SHA256SUMS"
name="gluon-$triple.zip"
if command -v shasum >/dev/null 2>&1; then
    grep -F "$name" SHA256SUMS | shasum -a 256 -c -
else
    grep -F "$name" SHA256SUMS | sha256sum -c -
fi
unzip -o "$name"
for bin in meson ninja gluon-build; do
    test -f "$bin" || { echo "install-gluon: missing $bin in $name" >&2; exit 1; }
    cp "$bin" "$GLUON_DIR/bin/$bin"
    chmod +x "$GLUON_DIR/bin/$bin"
done
echo "installed gluon ($triple) to $GLUON_DIR/bin"
"$GLUON_DIR/bin/gluon-build" --version
