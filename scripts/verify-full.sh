#!/usr/bin/env bash
# 本机全量门禁（sh 版本，对应 verify-full.ps1）：快速门禁 + 全量编译检查。
# 合并前运行本脚本；仅开发迭代时可用 ./scripts/verify.sh 快速门禁替代。

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

run_check() {
    local name="$1"
    shift
    echo "==> $name"
    "$@"
}

cd "$REPOSITORY_ROOT"

run_check "Run quick verification gate" "$SCRIPT_DIR/verify.sh"
run_check "Run workspace tests" cargo test --workspace --all-features
run_check "Run Clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
run_check "Build Rust documentation" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps

echo "Full verification passed."
