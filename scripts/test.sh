#!/bin/sh
set -eu

cd "$(dirname "$0")/.."

plugins=$(dirname "$(xcrun --find swiftc)")/../lib/swift/host/plugins/testing
exec swift test -Xswiftc -plugin-path -Xswiftc "$(cd "$plugins" && pwd)" "$@"
