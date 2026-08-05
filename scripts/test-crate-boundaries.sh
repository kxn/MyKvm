#!/usr/bin/env bash
set -euo pipefail

metadata="$(cargo metadata --format-version 1 --no-deps)"
for package in \
    ipkvm-device \
    ipkvm-headless \
    ipkvm-headless-app \
    ipkvm-headless-demo \
    ipkvm-browser-fixture \
    ipkvm-desktop-core; do
    [[ "$metadata" == *"\"name\":\"$package\""* ]] || {
        echo "Missing workspace package: $package" >&2
        exit 1
    }
done

[[ "$metadata" != *"ipkvm-desktop-iced-spike"* ]] || {
    echo "Retired iced spike remains in workspace" >&2
    exit 1
}

tree_for() {
    cargo tree -p "$1" --edges normal
}

headless_tree="$(tree_for ipkvm-headless)"
fixture_tree="$(tree_for ipkvm-browser-fixture)"
desktop_core_tree="$(tree_for ipkvm-desktop-core)"

[[ "$headless_tree" != *serialport* && "$headless_tree" != *nokhwa* && "$headless_tree" != *"windows v"* ]] || {
    echo "Headless library leaks a hardware backend" >&2
    exit 1
}
[[ "$fixture_tree" != *serialport* && "$fixture_tree" != *nokhwa* && "$fixture_tree" != *"windows v"* ]] || {
    echo "Browser fixture leaks a hardware provider" >&2
    exit 1
}
[[ "$desktop_core_tree" != *serialport* && "$desktop_core_tree" != *nokhwa* && "$desktop_core_tree" != *iced* && "$desktop_core_tree" != *eframe* && "$desktop_core_tree" != *egui* ]] || {
    echo "Desktop core leaks UI or hardware dependencies" >&2
    exit 1
}

echo "Crate boundary checks passed."
