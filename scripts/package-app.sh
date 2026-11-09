#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
cargo build --profile dist --bin app --features app
app=View.app
rm -rf "$app"
install -d "$app/Contents/MacOS"
install -m 644 Info.plist "$app/Contents/Info.plist"
install -m 755 target/dist/app "$app/Contents/MacOS/view"
codesign --force --sign - "$app"
echo "$PWD/$app"
