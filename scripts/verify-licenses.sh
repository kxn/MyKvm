#!/usr/bin/env bash
# 检查当前锁定依赖图的许可证和来源（sh 版本，对应 verify-licenses.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# shellcheck source=scripts/license-policy-tools.sh
source "$SCRIPT_DIR/license-policy-tools.sh"

cargo_deny=$(get_cargo_deny_executable)

cd "$REPOSITORY_ROOT"
"$cargo_deny" --locked check licenses sources

echo "Dependency licenses and sources passed."
