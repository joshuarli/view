#!/bin/sh
set -eu

cd "$(dirname "$0")/.."
config=${CONFIG:-release}
app=View.app

swift build -c "$config" --product view
rm -rf "$app"
install -d "$app/Contents/MacOS"
install -m 644 Info.plist "$app/Contents/Info.plist"
install -m 755 ".build/$config/view" "$app/Contents/MacOS/view"
codesign --force --sign - "$app"
echo "$PWD/$app"
