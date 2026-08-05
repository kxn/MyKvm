#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

cd "$REPOSITORY_ROOT"

metadata="$(cargo metadata --format-version 1 --no-deps)"
python3 -c '
import json
import sys

metadata = json.load(sys.stdin)
if any(package["name"] == "ipkvm-desktop-iced-spike" for package in metadata["packages"]):
    raise SystemExit("workspace still contains ipkvm-desktop-iced-spike")
' <<< "$metadata"

tree="$(cargo tree --workspace --all-features)"
if printf '%s\n' "$tree" | grep -nE '(^|[├└]── )(eframe|egui) v'; then
    echo "workspace dependency tree still contains egui" >&2
    exit 1
fi

if grep -nE '^(eframe|wgpu)[[:space:]]*=' crates/ipkvm-desktop/Cargo.toml; then
    echo "ipkvm-desktop still declares egui UI dependencies" >&2
    exit 1
fi

test ! -e crates/ipkvm-desktop/src/main.rs || {
    echo "egui desktop binary entry still exists" >&2
    exit 1
}
test ! -e crates/ipkvm-desktop/src/app.rs || {
    echo "egui desktop UI source still exists" >&2
    exit 1
}
test ! -e crates/ipkvm-desktop-iced-spike || {
    echo "iced spike directory still exists" >&2
    exit 1
}

grep -qE '#!\[cfg_attr\(windows, windows_subsystem = "windows"\)\]' crates/ipkvm-desktop-iced/src/main.rs
test -f crates/ipkvm-desktop-iced/assets/icon.ico
test -f crates/ipkvm-desktop-iced/assets/icon.rc

echo "M5 desktop retirement gate passed."
