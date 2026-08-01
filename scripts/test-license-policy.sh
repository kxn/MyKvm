#!/usr/bin/env bash
# 用临时负向夹具验证许可证策略（sh 版本，对应 test-license-policy.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPOSITORY_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

# shellcheck source=scripts/license-policy-tools.sh
source "$SCRIPT_DIR/license-policy-tools.sh"

assert_succeeded() {
    local name="$1"
    shift
    local output
    local exit_code

    if output=$("$@" 2>&1); then
        return 0
    fi
    exit_code=$?
    echo "$name failed with exit code $exit_code: $output" >&2
    return 1
}

assert_rejected() {
    local name="$1"
    local expected_exit="$2"
    local pattern="$3"
    shift 3
    local output
    local exit_code

    set +e
    output=$("$@" 2>&1)
    exit_code=$?
    set -e

    if [[ "$exit_code" -eq 0 ]]; then
        echo "$name succeeded but failure was expected" >&2
        return 1
    fi
    if [[ "$exit_code" -ne "$expected_exit" ]]; then
        echo "$name returned $exit_code, expected $expected_exit: $output" >&2
        return 1
    fi
    if ! grep -qE "$pattern" <<<"$output"; then
        echo "$name output did not match '$pattern': $output" >&2
        return 1
    fi
}

new_path_dependency_fixture() {
    local root="$1"
    local dependency_license="$2"

    mkdir -p "$root/dependency/src" "$root/src"

    cat >"$root/Cargo.toml" <<EOF
[package]
name = "policy-fixture-app"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-fixture-dependency = { path = "dependency" }

[workspace]
members = ["dependency"]
EOF

    cat >"$root/dependency/Cargo.toml" <<EOF
[package]
name = "policy-fixture-dependency"
version = "0.1.0"
edition = "2024"
license = "$dependency_license"
publish = false
EOF

    printf 'pub fn app() {}\n' >"$root/src/lib.rs"
    printf 'pub fn dependency() {}\n' >"$root/dependency/src/lib.rs"
}

new_git_dependency_fixture() {
    local root="$1"
    local dependency_root="$root/git-dependency"
    local consumer_root="$root/git-consumer"

    mkdir -p "$dependency_root/src" "$consumer_root/src"

    cat >"$dependency_root/Cargo.toml" <<'EOF'
[package]
name = "policy-git-dependency"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false
EOF

    printf 'pub fn dependency() {}\n' >"$dependency_root/src/lib.rs"

    git -C "$dependency_root" init >/dev/null
    git -C "$dependency_root" config user.name "my_ipkvm policy test"
    git -C "$dependency_root" config user.email "policy-test@invalid.local"
    git -C "$dependency_root" add .
    git -C "$dependency_root" commit -m "fixture" >/dev/null

    local dependency_uri
    dependency_uri="file://$(realpath "$dependency_root")"

    cat >"$consumer_root/Cargo.toml" <<EOF
[package]
name = "policy-git-consumer"
version = "0.1.0"
edition = "2024"
license = "MIT"
publish = false

[dependencies]
policy-git-dependency = { git = "$dependency_uri" }

[workspace]
EOF

    printf 'pub fn consumer() {}\n' >"$consumer_root/src/lib.rs"
    printf '%s\n' "$consumer_root"
}

if [[ "$(get_required_cargo_deny_version)" != "0.20.2" ]]; then
    echo "Required version is not 0.20.2" >&2
    exit 1
fi

if [[ "$(assert_cargo_deny_version "cargo-deny 0.20.2")" != "0.20.2" ]]; then
    echo "Expected version was rejected" >&2
    exit 1
fi

if assert_cargo_deny_version "cargo-deny 0.20.1" >/dev/null 2>&1; then
    echo "Expected version mismatch was accepted" >&2
    exit 1
fi

if assert_cargo_deny_version "not a version" >/dev/null 2>&1; then
    echo "Unparseable version was accepted" >&2
    exit 1
fi

deny_config="$REPOSITORY_ROOT/deny.toml"
if [[ ! -f "$deny_config" ]]; then
    echo "deny.toml was not found at repository root" >&2
    exit 1
fi

cargo_deny=$(get_cargo_deny_executable)
fixture_root=$(mktemp -d "${TMPDIR:-/tmp}/my-ipkvm-license-policy.XXXXXX")

cleanup() {
    if [[ -n "${fixture_root:-}" && -d "$fixture_root" ]]; then
        rm -r -- "$fixture_root"
    fi
}
trap cleanup EXIT

path_fixture="$fixture_root/path-dependency"
new_path_dependency_fixture "$path_fixture" "BSD-3-Clause"

assert_succeeded "Generate allowed fixture lock file" \
    cargo generate-lockfile \
    --manifest-path "$path_fixture/Cargo.toml" \
    --offline

assert_succeeded "Allowed license fixture" \
    "$cargo_deny" \
    --config "$deny_config" \
    --manifest-path "$path_fixture/Cargo.toml" \
    --locked \
    check licenses sources

new_path_dependency_fixture "$path_fixture" "GPL-3.0-only"
assert_rejected "Rejected license fixture" 4 "rejected|GPL-3\\.0-only" \
    "$cargo_deny" \
    --config "$deny_config" \
    --manifest-path "$path_fixture/Cargo.toml" \
    --locked \
    check licenses sources

git_consumer=$(new_git_dependency_fixture "$fixture_root")

assert_succeeded "Generate Git fixture lock file" \
    cargo generate-lockfile \
    --manifest-path "$git_consumer/Cargo.toml"

assert_rejected "Rejected Git source fixture" 8 "source-not-allowed|git-source-underspecified|file://" \
    "$cargo_deny" \
    --config "$deny_config" \
    --manifest-path "$git_consumer/Cargo.toml" \
    --locked \
    check sources

echo "Dependency license policy tests passed."
