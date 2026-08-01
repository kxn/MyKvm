#!/usr/bin/env bash
# 用临时负向夹具验证 noVNC web 资源策略（sh 版本，对应 test-web-assets.ps1）

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=scripts/web-assets-tools.sh
source "$SCRIPT_DIR/web-assets-tools.sh"

# ---------------------------------------------------------------------------
# assert_throws_like 对应 Assert-ThrowsLike：
# 运行给定的命令字符串，断言其以非零退出码失败，且 stderr 合并输出匹配模式。
# 用法：assert_throws_like <描述> <正则模式> <命令...>
# ---------------------------------------------------------------------------
assert_throws_like() {
    local name="$1"
    local pattern="$2"
    shift 2
    local output exit_code

    set +e
    output=$("$@" 2>&1)
    exit_code=$?
    set -e

    if [[ "$exit_code" -eq 0 ]]; then
        echo "$name succeeded but failure was expected" >&2
        echo "output: $output" >&2
        return 1
    fi
    # 与 PowerShell 的 -notmatch 一致，匹配不区分大小写。
    if ! grep -qiE "$pattern" <<<"$output"; then
        echo "$name output did not match '$pattern': $output" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# write_utf8_file 对应 Set-Utf8File：以无 BOM 的 UTF-8 写入文件，自动创建父目录。
# 用法：write_utf8_file <路径> <内容>
# ---------------------------------------------------------------------------
write_utf8_file() {
    local path="$1"
    local content="$2"
    mkdir -p "$(dirname "$path")"
    printf '%s' "$content" >"$path"
}

# ---------------------------------------------------------------------------
# write_utf8_file_raw 对应 PowerShell here-string 夹具：
# 直接把 stdin 写入文件，不做任何转义/插值。
# 用法：write_utf8_file_raw <路径>
# ---------------------------------------------------------------------------
write_utf8_file_raw() {
    local path="$1"
    mkdir -p "$(dirname "$path")"
    cat >"$path"
}

# ---------------------------------------------------------------------------
# new_test_root 对应 New-TestRoot：在系统临时目录下创建唯一子目录路径。
# ---------------------------------------------------------------------------
new_test_root() {
    local tmp_root root
    tmp_root="$(get_temp_root)"
    root="$tmp_root/my-ipkvm-web-assets-$(date +%s)-$$-$RANDOM"
    assert_safe_temporary_path "$root"
    printf '%s\n' "$root"
}

# ---------------------------------------------------------------------------
# new_novnc_fixture 对应 New-NoVncFixture：构造 noVNC 包夹具与 npm 元数据。
# 输出 package 目录路径。
# ---------------------------------------------------------------------------
new_novnc_fixture() {
    local root="$1"
    local package="$root/package"

    write_utf8_file "$package/AUTHORS" "fixture authors"
    write_utf8_file "$package/LICENSE.txt" "fixture license"
    write_utf8_file "$package/docs/LICENSE.MPL-2.0" "fixture MPL"
    write_utf8_file "$package/vendor/pako/LICENSE" "fixture pako license"
    write_utf8_file "$package/core/crypto/des.js" "fixture BSD notice"
    write_utf8_file "$package/core/rfb.js" "export default class RFB {}"
    write_utf8_file_raw "$package/package.json" <<'EOF'
{
  "name": "@novnc/novnc",
  "version": "1.7.0",
  "license": "MPL-2.0",
  "dependencies": {}
}
EOF

    write_utf8_file_raw "$root/npm-metadata.json" <<'EOF'
{
  "version": "1.7.0",
  "gitHead": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e",
  "dist": {
    "tarball": "https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz",
    "integrity": "sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA==",
    "shasum": "7f832cf07c66475a81a25708b8e5299a5c4efec5"
  }
}
EOF

    write_utf8_file_raw "$root/npm-attestations.json" <<'EOF'
{
  "attestations": [
    {
      "predicateType": "https://slsa.dev/provenance/v1",
      "repository": "https://github.com/novnc/noVNC",
      "commit": "63107bd06d9e1f6136ff21aeda8cd62cbf0d433e"
    }
  ]
}
EOF

    printf '%s\n' "$package"
}

# ---------------------------------------------------------------------------
# new_browser_lock_fixture 对应 New-BrowserLockFixture
# 输出 browser-tests 目录路径。
# ---------------------------------------------------------------------------
new_browser_lock_fixture() {
    local root="$1"
    local browser_root="$root/browser-tests"

    write_utf8_file_raw "$browser_root/package.json" <<'EOF'
{
  "name": "my-ipkvm-browser-tests",
  "private": true,
  "type": "module",
  "devDependencies": {
    "playwright-core": "1.62.1"
  }
}
EOF
    write_utf8_file_raw "$browser_root/package-lock.json" <<'EOF'
{
  "name": "my-ipkvm-browser-tests",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "my-ipkvm-browser-tests",
      "devDependencies": {
        "playwright-core": "1.62.1"
      }
    },
    "node_modules/playwright-core": {
      "version": "1.62.1",
      "resolved": "https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz",
      "integrity": "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw==",
      "dev": true,
      "license": "Apache-2.0",
      "engines": {
        "node": ">=20"
      }
    }
  }
}
EOF

    printf '%s\n' "$browser_root"
}

# ---------------------------------------------------------------------------
# 主流程
# ---------------------------------------------------------------------------
root="$(new_test_root)"
mkdir -p "$root"

fixture_root=""
cleanup() {
    if [[ -n "${fixture_root:-}" && -d "$fixture_root" ]]; then
        # 再次断言仍位于临时目录下，避免误删仓库
        assert_safe_temporary_path "$fixture_root"
        rm -rf -- "$fixture_root"
    fi
}
fixture_root="$root"
trap cleanup EXIT

novnc_root="$root/novnc"
package_root="$(new_novnc_fixture "$novnc_root")"
manifest_path="$novnc_root/manifest.sha256"
write_web_asset_manifest "$package_root" "$manifest_path"

# 基线：所有夹具均合法
assert_web_asset_tree "$package_root" "$manifest_path"
assert_novnc_package \
    "$package_root" \
    "$novnc_root/npm-metadata.json" \
    "$novnc_root/npm-attestations.json"

# 篡改 rfb.js 内容 -> 哈希不匹配
rfb_path="$package_root/core/rfb.js"
write_utf8_file "$rfb_path" "tampered"
assert_throws_like "tampered rfb.js" \
    "hash mismatch.*core/rfb\.js" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_web_asset_tree "$1" "$2"' \
    "$SCRIPT_DIR" "$package_root" "$manifest_path"
write_utf8_file "$rfb_path" "export default class RFB {}"

# 删除 AUTHORS -> 缺失
authors_path="$package_root/AUTHORS"
rm -f "$authors_path"
assert_throws_like "missing AUTHORS" \
    "missing.*AUTHORS" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_web_asset_tree "$1" "$2"' \
    "$SCRIPT_DIR" "$package_root" "$manifest_path"
write_utf8_file "$authors_path" "fixture authors"

# 增加意外文件 unexpected.js -> 意外
extra_path="$package_root/unexpected.js"
write_utf8_file "$extra_path" "unexpected"
assert_throws_like "unexpected file" \
    "unexpected.*unexpected\.js" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_web_asset_tree "$1" "$2"' \
    "$SCRIPT_DIR" "$package_root" "$manifest_path"
rm -f "$extra_path"

# package.json 版本被改 -> 校验失败
package_json_path="$package_root/package.json"
valid_package_json="$(cat "$package_json_path")"
write_utf8_file "$package_json_path" "${valid_package_json//\"1.7.0\"/\"1.7.1\"}"
assert_throws_like "package version mismatch" \
    "package version" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_novnc_package "$1" "$2" "$3"' \
    "$SCRIPT_DIR" "$package_root" "$novnc_root/npm-metadata.json" "$novnc_root/npm-attestations.json"
write_utf8_file "$package_json_path" "$valid_package_json"

# npm-metadata.json 的 gitHead 被改 -> 校验失败
metadata_path="$novnc_root/npm-metadata.json"
valid_metadata="$(cat "$metadata_path")"
write_utf8_file "$metadata_path" \
    "${valid_metadata//63107bd06d9e1f6136ff21aeda8cd62cbf0d433e/0000000000000000000000000000000000000000}"
assert_throws_like "metadata gitHead mismatch" \
    "gitHead" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_novnc_package "$1" "$2" "$3"' \
    "$SCRIPT_DIR" "$package_root" "$metadata_path" "$novnc_root/npm-attestations.json"
write_utf8_file "$metadata_path" "$valid_metadata"

# 删除 vendor/pako/LICENSE -> 必需文件缺失
pako_license="$package_root/vendor/pako/LICENSE"
rm -f "$pako_license"
assert_throws_like "missing pako license" \
    "required noVNC file.*vendor/pako/LICENSE" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_novnc_package "$1" "$2" "$3"' \
    "$SCRIPT_DIR" "$package_root" "$metadata_path" "$novnc_root/npm-attestations.json"
write_utf8_file "$pako_license" "fixture pako license"

# 浏览器锁夹具基线
browser_root="$(new_browser_lock_fixture "$root")"
browser_package="$browser_root/package.json"
browser_lock="$browser_root/package-lock.json"
assert_browser_package_lock "$browser_package" "$browser_lock"

# 改 registry source -> 失败
valid_lock="$(cat "$browser_lock")"
write_utf8_file "$browser_lock" \
    "${valid_lock//https:\/\/registry.npmjs.org\/playwright-core\//https:\/\/example.invalid\/playwright-core\/}"
assert_throws_like "registry source mismatch" \
    "registry source" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_browser_package_lock "$1" "$2"' \
    "$SCRIPT_DIR" "$browser_package" "$browser_lock"
write_utf8_file "$browser_lock" "$valid_lock"

# 加入未批准的包 node_modules/unapproved -> 失败
write_utf8_file_raw "$browser_lock" <<EOF
{
  "name": "my-ipkvm-browser-tests",
  "lockfileVersion": 3,
  "requires": true,
  "packages": {
    "": {
      "name": "my-ipkvm-browser-tests",
      "devDependencies": {
        "playwright-core": "1.62.1"
      }
    },
    "node_modules/unapproved": {
      "version": "1.0.0",
      "resolved": "https://registry.npmjs.org/unapproved/-/unapproved-1.0.0.tgz",
      "integrity": "sha512-invalid",
      "license": "MIT"
    },
    "node_modules/playwright-core": {
      "version": "1.62.1",
      "resolved": "https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz",
      "integrity": "sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw==",
      "dev": true,
      "license": "Apache-2.0",
      "engines": {
        "node": ">=20"
      }
    }
  }
}
EOF
assert_throws_like "unapproved package" \
    "unapproved package" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_browser_package_lock "$1" "$2"' \
    "$SCRIPT_DIR" "$browser_package" "$browser_lock"
write_utf8_file "$browser_lock" "$valid_lock"

# ---------------------------------------------------------------------------
# tar 安全用例（对应 assert_safe_tar_entries）
# 用法：run_tar_case <描述> <期望 pass|reject> <names 内容> <verbose 内容>
# ---------------------------------------------------------------------------
run_tar_case() {
    local name="$1"
    local expect="$2"
    local names="$3"
    local verbose="$4"
    local names_file="$root/$name.names"
    local verbose_file="$root/$name.verbose"

    printf '%s' "$names" >"$names_file"
    printf '%s' "$verbose" >"$verbose_file"

    if [[ "$expect" == "pass" ]]; then
        assert_safe_tar_entries "$names_file" "$verbose_file"
        echo "[$name] accepted as expected"
    else
        assert_throws_like "$name" "unsafe tar (path|entry type)" \
            bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_safe_tar_entries "$1" "$2"' \
            "$SCRIPT_DIR" "$names_file" "$verbose_file"
        echo "[$name] rejected as expected"
    fi
}

run_tar_case "valid-tar" "pass" \
    $'package/\npackage/core/rfb.js\n' \
    $'drwxr-xr-x  0 0 0 0 Jan 01 00:00 package/\n-rw-r--r--  0 0 0 1 Jan 01 00:00 package/core/rfb.js\n'

run_tar_case "dotdot-tar" "reject" \
    $'package/../outside\n' \
    $'-rw-r--r--  0 0 0 1 Jan 01 00:00 package/../outside\n'

run_tar_case "drive-tar" "reject" \
    $'C:/outside\n' \
    $'-rw-r--r--  0 0 0 1 Jan 01 00:00 C:/outside\n'

run_tar_case "symlink-tar" "reject" \
    $'package/link\n' \
    $'lrwxr-xr-x  0 0 0 0 Jan 01 00:00 package/link -> outside\n'

run_tar_case "hardlink-tar" "reject" \
    $'package/hardlink\n' \
    $'hrw-r--r--  0 0 0 0 Jan 01 00:00 package/hardlink link to outside\n'

# ---------------------------------------------------------------------------
# 临时路径安全用例（对应 Assert-SafeTemporaryPath）
# ---------------------------------------------------------------------------
assert_throws_like "non-temporary path" \
    "outside the system temporary directory" \
    bash -c 'set -e; source "$0/web-assets-tools.sh"; assert_safe_temporary_path "$1"' \
    "$SCRIPT_DIR" "$SCRIPT_DIR/not-temporary"
echo "[non-temporary path] rejected as expected"

echo "Web asset policy tests passed."
