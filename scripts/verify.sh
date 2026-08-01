#!/usr/bin/env bash
# 本机一键自动化验证（sh 版本，对应 verify.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

run_check() {
    local name="$1"
    shift
    echo "==> $name"
    "$@"
}

check_tracked_text_encoding() {
    local files
    mapfile -t files < <(
        git ls-files -- \
            "*.json" "*.md" "*.ps1" "*.psm1" "*.py" "*.rs" "*.sh" "*.toml" "*.yaml" "*.yml" \
            "AGENTS.md" "Cargo.lock"
    )

    python3 - "${files[@]}" <<'PY'
import pathlib
import sys

for relative_path in sys.argv[1:]:
    path = pathlib.Path(relative_path)
    if not path.is_file():
        continue

    data = path.read_bytes()
    if data.startswith(b"\xef\xbb\xbf"):
        raise SystemExit(f"{relative_path} contains a UTF-8 BOM")
    try:
        data.decode("utf-8")
    except UnicodeDecodeError as exc:
        raise SystemExit(f"{relative_path} is not valid UTF-8: {exc}") from exc
PY
}

cd "$REPOSITORY_ROOT"

echo "==> Check text encoding"
check_tracked_text_encoding

run_check "Test dependency license policy" "$SCRIPT_DIR/test-license-policy.sh"
run_check "Check dependency licenses and sources" "$SCRIPT_DIR/verify-licenses.sh"
run_check "Check Rust formatting" cargo fmt --all --check
run_check "Run workspace tests" cargo test --workspace --all-features
run_check "Run Clippy" cargo clippy --workspace --all-targets --all-features -- -D warnings
run_check "Build Rust documentation" env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --all-features --no-deps
run_check "Check working tree diff" git diff --check
run_check "Check staged diff" git diff --cached --check

echo "Local verification passed."
