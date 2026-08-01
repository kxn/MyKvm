#!/usr/bin/env bash
# 运行真实浏览器验证：构建浏览器夹具二进制并驱动 Playwright 访问 noVNC（sh 版本，对应 verify-browser.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BROWSER_TEST_ROOT="$REPOSITORY_ROOT/browser-tests"

# Python 解释器解析（既定模式优先 python3，Windows 下回退到 py 启动器）
get_python3() {
    if command -v python3 >/dev/null 2>&1; then
        printf '%s\n' "python3"
    elif command -v py >/dev/null 2>&1; then
        printf '%s\n' "py"
    else
        echo "未找到 python3，请先安装 Python 3" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# run_checked 对应 Invoke-CheckedCommand：打印步骤名，失败时抛出。
# 用法：run_checked <名称> <命令...>
# ---------------------------------------------------------------------------
run_checked() {
    local name="$1"
    shift
    echo "==> $name"
    "$@"
}

# ---------------------------------------------------------------------------
# get_fixture_executable 对应 Get-FixtureExecutable：
# 编译 ipkvm-browser-fixture，从 cargo 的 JSON 消息中解析出唯一的可执行文件路径。
# ---------------------------------------------------------------------------
get_fixture_executable() {
    local py executable
    local -a lines=()
    py="$(get_python3)"

    # 注意：即便诊断信息输出到 stderr，cargo 仍以 0 退出码表示成功。
    mapfile -t lines < <(cargo build \
        -p ipkvm-headless \
        --features browser-fixture \
        --bin ipkvm-browser-fixture \
        --message-format=json-render-diagnostics)

    executable="$(
        "$py" - "${lines[@]}" <<'PY'
import json
import os
import sys

found = []
for line in sys.argv[1:]:
    line = line.strip()
    if not line:
        continue
    try:
        message = json.loads(line)
    except json.JSONDecodeError:
        continue
    if (
        message.get("reason") == "compiler-artifact"
        and message.get("target", {}).get("name") == "ipkvm-browser-fixture"
        and message.get("executable")
    ):
        found.append(os.path.abspath(message["executable"]))

unique = sorted(set(found))
if len(unique) != 1:
    raise SystemExit(
        f"Expected one browser fixture executable, got {len(unique)}"
    )
print(unique[0])
PY
    )"

    if [[ ! -f "$executable" ]]; then
        echo "Browser fixture executable does not exist: $executable" >&2
        return 1
    fi
    printf '%s\n' "$executable"
}

# ---------------------------------------------------------------------------
# assert_fixture_feature_boundary 对应 Assert-FixtureFeatureBoundary：
# 用 cargo metadata 断言 ipkvm-browser-fixture 仅要求 browser-fixture 特性。
# ---------------------------------------------------------------------------
assert_fixture_feature_boundary() {
    local py json
    py="$(get_python3)"

    json=$(cargo metadata --format-version 1 --no-deps)

    "$py" - "$json" <<'PY'
import json
import sys

data = json.loads(sys.argv[1])
headless = [p for p in data.get("packages", []) if p.get("name") == "ipkvm-headless"]
if len(headless) != 1:
    raise SystemExit("Expected one ipkvm-headless package in Cargo metadata")

targets = [
    t for t in headless[0].get("targets", []) if t.get("name") == "ipkvm-browser-fixture"
]
if len(targets) != 1:
    raise SystemExit("Expected one ipkvm-browser-fixture target")

required = targets[0].get("required-features", [])
if len(required) != 1 or required[0] != "browser-fixture":
    raise SystemExit(
        "Browser fixture must require exactly the browser-fixture feature"
    )
PY
}

# 保存/恢复环境变量的状态，避免污染调用者。
had_fixture_path=
previous_fixture_path=
if [[ -n "${IPKVM_BROWSER_FIXTURE+x}" ]]; then
    had_fixture_path=1
    previous_fixture_path="$IPKVM_BROWSER_FIXTURE"
fi

cleanup() {
    if [[ -n "$had_fixture_path" ]]; then
        export IPKVM_BROWSER_FIXTURE="$previous_fixture_path"
    else
        unset IPKVM_BROWSER_FIXTURE
    fi
}
trap cleanup EXIT

cd "$REPOSITORY_ROOT"

node_version=$(node -p "process.versions.node")
node_major="${node_version%%.*}"
if [[ "$node_major" -lt 20 ]]; then
    echo "Node.js 20 or newer is required, got $node_version" >&2
    exit 1
fi

run_checked "Install locked browser test dependency" \
    npm ci --ignore-scripts --prefix "$BROWSER_TEST_ROOT"

assert_fixture_feature_boundary
export IPKVM_BROWSER_FIXTURE="$(get_fixture_executable)"

run_checked "Run real browser verification" \
    node "$BROWSER_TEST_ROOT/novnc-browser.mjs"

echo "Real browser verification passed."
