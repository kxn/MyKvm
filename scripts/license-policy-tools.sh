#!/usr/bin/env bash
# 依赖许可证策略公共工具（sh 版本，对应 license-policy-tools.psm1）

set -euo pipefail

REQUIRED_CARGO_DENY_VERSION="0.20.2"
CARGO_DENY_INSTALL_COMMAND="cargo install --locked --version 0.20.2 cargo-deny"

get_required_cargo_deny_version() {
    printf '%s\n' "$REQUIRED_CARGO_DENY_VERSION"
}

assert_cargo_deny_version() {
    local version_output="$1"
    local version

    version=$(printf '%s\n' "$version_output" |
        sed -nE 's/^cargo-deny[[:space:]]+([0-9]+\.[0-9]+\.[0-9]+)([[:space:]].*)?$/\1/p' |
        head -n 1)

    if [[ -z "$version" ]]; then
        echo "无法解析 cargo-deny 版本。请执行：$CARGO_DENY_INSTALL_COMMAND" >&2
        return 1
    fi

    if [[ "$version" != "$REQUIRED_CARGO_DENY_VERSION" ]]; then
        echo "cargo-deny 版本不符：期望 $REQUIRED_CARGO_DENY_VERSION，实际 $version。请执行：$CARGO_DENY_INSTALL_COMMAND" >&2
        return 1
    fi

    printf '%s\n' "$version"
}

get_cargo_deny_executable() {
    local command_path
    local version_output

    command_path=$(command -v cargo-deny || true)
    if [[ -z "$command_path" ]]; then
        echo "未找到 cargo-deny。请执行：$CARGO_DENY_INSTALL_COMMAND" >&2
        return 1
    fi

    if ! version_output=$("$command_path" --version 2>&1); then
        echo "cargo-deny --version 执行失败，退出码：$?" >&2
        return 1
    fi

    assert_cargo_deny_version "$version_output" >/dev/null
    printf '%s\n' "$command_path"
}
