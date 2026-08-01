#!/usr/bin/env bash
# noVNC web 资源校验公共工具（sh 版本，对应 web-assets-tools.psm1）

set -euo pipefail

# ---------------------------------------------------------------------------
# noVNC 版本/哈希策略常量（对应 web-assets-tools.psm1 第 4-36 行）
# ---------------------------------------------------------------------------
NOVNC_VERSION="1.7.0"
NOVNC_COMMIT="63107bd06d9e1f6136ff21aeda8cd62cbf0d433e"
NOVNC_TARBALL="https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz"
NOVNC_INTEGRITY="sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA=="
NOVNC_SHASUM="7f832cf07c66475a81a25708b8e5299a5c4efec5"
NOVNC_ARCHIVE_SIZE=155185
NOVNC_ARCHIVE_SHA256="32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903"
NOVNC_ARCHIVE_SHA512="b9c1093b1e13d9abc844295ea1d93b6286d98f93b619ad8078b7d0ebd03fca31bd1c76dadbe3c38304a437716db6b986fbbd191b1db5b0494d168b8ad77473c8"
NOVNC_METADATA_URL="https://registry.npmjs.org/@novnc%2Fnovnc/1.7.0"
NOVNC_ATTESTATIONS_URL="https://registry.npmjs.org/-/npm/v1/attestations/@novnc%2Fnovnc@1.7.0"
PLAYWRIGHT_VERSION="1.62.1"
PLAYWRIGHT_RESOLVED="https://registry.npmjs.org/playwright-core/-/playwright-core-1.62.1.tgz"
PLAYWRIGHT_INTEGRITY="sha512-wPYSwEBJY9GHraISXqyqtx0na0LpO3XEX7jNDhntbex7tzUS7kLnZsOlFruFJB4Hi/rhDMjXGqHewDZ68nYZVw=="

# ---------------------------------------------------------------------------
# Python 解释器解析（既定模式优先 python3，Windows 下回退到 py 启动器）
# ---------------------------------------------------------------------------
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
# SHA-256 计算（兼容 Linux 的 sha256sum 与 macOS 的 shasum -a 256）
# ---------------------------------------------------------------------------
compute_sha256() {
    local path="$1"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$path" | awk '{print $1}'
    else
        shasum -a 256 "$path" | awk '{print $1}'
    fi
}

# ---------------------------------------------------------------------------
# 路径规范化辅助
# ---------------------------------------------------------------------------

# get_path_prefix 对应 Get-PathPrefix：去掉结尾分隔符后补一个分隔符
get_path_prefix() {
    local path="$1"
    # 去掉所有结尾的 /（兼容反复追加的情况）
    while [[ "${path%/}" != "$path" ]]; do
        path="${path%/}"
    done
    printf '%s/\n' "$path"
}

# normalize_path 返回规范化的绝对路径（对应 .NET 的 Path.GetFullPath）
normalize_path() {
    local path="$1"
    local dir base
    if [[ -d "$path" ]]; then
        (cd "$path" && pwd) && return
    fi
    # 文件或尚不存在的路径：规范化其父目录后拼接文件名
    dir="$(cd "$(dirname "$path")" && pwd)"
    base="$(basename "$path")"
    printf '%s/%s\n' "$dir" "$base"
}

# assert_path_under_root 对应 Assert-PathUnderRoot：路径必须严格位于 root 之下
# 用法：assert_path_under_root <path> <root> <message>
assert_path_under_root() {
    local path="$1"
    local root="$2"
    local message="$3"
    local full_path full_root prefix

    full_path="$(normalize_path "$path")"
    full_root="$(normalize_path "$root")"
    prefix="$(get_path_prefix "$full_root")"

    # full_path 必须以 prefix 开头且严格不等于 full_root
    if [[ "$full_path" == "$full_root" ]]; then
        echo "$message: $full_path" >&2
        return 1
    fi
    if [[ "$full_path" != "$prefix"* ]]; then
        echo "$message: $full_path" >&2
        return 1
    fi
    printf '%s\n' "$full_path"
}

# get_temp_root 返回系统临时目录的规范化绝对路径（对应 .NET Path.GetTempPath）
get_temp_root() {
    local tmp
    tmp="${TMPDIR:-/tmp}"
    # 去掉结尾分隔符后规范化
    tmp="${tmp%/}"
    normalize_path "$tmp"
}

# assert_safe_temporary_path 对应 Assert-SafeTemporaryPath
assert_safe_temporary_path() {
    local path="$1"
    local temp_root
    temp_root="$(get_temp_root)"
    assert_path_under_root "$path" "$temp_root" \
        "Path is outside the system temporary directory" >/dev/null
}

# assert_safe_repository_target 对应 Assert-SafeRepositoryTarget
assert_safe_repository_target() {
    local path="$1"
    local repository_root="$2"
    local allowed_relative_root="$3"
    local allowed_root
    allowed_root="$(normalize_path "$repository_root/$allowed_relative_root")"
    assert_path_under_root "$path" "$allowed_root" \
        "Path is outside the approved repository target" >/dev/null
}

# ---------------------------------------------------------------------------
# noVNC 发布策略访问
# ---------------------------------------------------------------------------

# get_novnc_release_policy 对应 Get-NoVncReleasePolicy：输出键值清单
get_novnc_release_policy() {
    cat <<EOF
version=$NOVNC_VERSION
commit=$NOVNC_COMMIT
tarball=$NOVNC_TARBALL
integrity=$NOVNC_INTEGRITY
shasum=$NOVNC_SHASUM
archive_size=$NOVNC_ARCHIVE_SIZE
archive_sha256=$NOVNC_ARCHIVE_SHA256
archive_sha512=$NOVNC_ARCHIVE_SHA512
metadata_url=$NOVNC_METADATA_URL
attestations_url=$NOVNC_ATTESTATIONS_URL
EOF
}

# ---------------------------------------------------------------------------
# 清单（manifest）相关
# ---------------------------------------------------------------------------

# get_asset_relative_path 对应 Get-AssetRelativePath：返回 root 之下的相对路径（/ 分隔）
get_asset_relative_path() {
    local root="$1"
    local path="$2"
    local full_root prefix full_path

    full_root="$(normalize_path "$root")"
    full_path="$(assert_path_under_root "$path" "$full_root" "Asset is outside its root")"
    prefix="$(get_path_prefix "$full_root")"
    printf '%s\n' "${full_path#"$prefix"}"
}

# assert_safe_manifest_path 对应 Assert-SafeManifestPath：
# 拒绝反斜杠、NUL、绝对路径、盘符、空段、. 与 ..
assert_safe_manifest_path() {
    local path="$1"
    local segments segment

    if [[ -z "${path//[[:space:]]/}" ]]; then
        echo "Unsafe manifest path: $path" >&2
        return 1
    fi
    if [[ "$path" == *'\'* ]]; then
        echo "Unsafe manifest path: $path" >&2
        return 1
    fi
    # 注意：bash 变量无法承载 NUL 字节，故 [char]0 检查在 sh 中无可实现的等价物。
    if [[ "$path" == /* ]]; then
        echo "Unsafe manifest path: $path" >&2
        return 1
    fi
    if [[ "$path" =~ ^[A-Za-z]: ]]; then
        echo "Unsafe manifest path: $path" >&2
        return 1
    fi

    IFS='/' read -ra segments <<<"$path"
    if [[ ${#segments[@]} -eq 0 ]]; then
        echo "Unsafe manifest path: $path" >&2
        return 1
    fi
    for segment in "${segments[@]}"; do
        if [[ -z "$segment" || "$segment" == "." || "$segment" == ".." ]]; then
            echo "Unsafe manifest path: $path" >&2
            return 1
        fi
    done
}

# write_web_asset_manifest 对应 Write-WebAssetManifest：
# 遍历 root 下所有文件，计算 sha256 并按相对路径排序写入清单。
# 用法：write_web_asset_manifest <root> <manifest_path>
write_web_asset_manifest() {
    local root="$1"
    local manifest_path="$2"
    local full_root relative hash
    local -a entries=()

    full_root="$(normalize_path "$root")"

    while IFS= read -r -d '' file; do
        relative="$(get_asset_relative_path "$full_root" "$file")"
        assert_safe_manifest_path "$relative"
        hash="$(compute_sha256 "$file")"
        hash="${hash,,}"  # 转小写（bash 4+）
        entries+=("$hash  $relative")
    done < <(find "$full_root" -type f -print0)

    # 按相对路径排序（取每行 $2 之后的内容排序）
    local -a sorted=()
    if [[ ${#entries[@]} -gt 0 ]]; then
        mapfile -t sorted < <(printf '%s\n' "${entries[@]}" | LC_ALL=C sort -k2)
        printf '%s\n' "${sorted[@]}" >"$manifest_path"
    else
        : >"$manifest_path"
    fi
}

# read_web_asset_manifest 对应 Read-WebAssetManifest：
# 解析清单，逐行校验格式与 manifest 路径安全性，输出 "<path>\t<hash>" 行。
read_web_asset_manifest() {
    local manifest_path="$1"
    local line_no=0 line hash relative

    while IFS= read -r line || [[ -n "$line" ]]; do
        line_no=$((line_no + 1))
        # 用变量承载正则，避免在 [[ =~ ]] 中对空格的转义歧义。
        # 对应 PowerShell 的 ^([0-9a-fA-F]{64})  (.+)$（恰好两个空格）。
        pattern='^([0-9a-fA-F]{64})  (.+)$'
        if [[ ! "$line" =~ $pattern ]]; then
            echo "Invalid manifest line $line_no" >&2
            return 1
        fi
        hash="${BASH_REMATCH[1]}"
        relative="${BASH_REMATCH[2]}"
        assert_safe_manifest_path "$relative"
        # 去重检测通过输出后由调用方判定；这里返回 path\tlower(hash)
        printf '%s\t%s\n' "$relative" "${hash,,}"
    done <"$manifest_path"
}

# ---------------------------------------------------------------------------
# assert_web_asset_tree 对应 Assert-WebAssetTree
# 用法：assert_web_asset_tree <root> <manifest_path>
# ---------------------------------------------------------------------------
assert_web_asset_tree() {
    local root="$1"
    local manifest_path="$2"
    local full_root relative hash file
    local -A expected=()
    local -A actual=()

    full_root="$(normalize_path "$root")"

    # 读取期望清单
    local line key value
    while IFS=$'\t' read -r key value; do
        if [[ -n "${expected[$key]+x}" ]]; then
            echo "Duplicate manifest path: $key" >&2
            return 1
        fi
        expected["$key"]="$value"
    done < <(read_web_asset_manifest "$manifest_path")

    # 计算实际文件树
    while IFS= read -r -d '' file; do
        relative="$(get_asset_relative_path "$full_root" "$file")"
        if [[ -n "${actual[$relative]+x}" ]]; then
            echo "Duplicate web asset path: $relative" >&2
            return 1
        fi
        hash="$(compute_sha256 "$file")"
        actual["$relative"]="${hash,,}"
    done < <(find "$full_root" -type f -print0)

    # missing / mismatch
    for relative in "${!expected[@]}"; do
        if [[ -z "${actual[$relative]+x}" ]]; then
            echo "Web asset is missing: $relative" >&2
            return 1
        fi
        if [[ "${actual[$relative]}" != "${expected[$relative]}" ]]; then
            echo "Web asset hash mismatch: $relative" >&2
            return 1
        fi
    done

    # unexpected
    for relative in "${!actual[@]}"; do
        if [[ -z "${expected[$relative]+x}" ]]; then
            echo "Unexpected web asset: $relative" >&2
            return 1
        fi
    done
}

# ---------------------------------------------------------------------------
# assert_json_property_equals 对应 Assert-JsonPropertyEquals
# 用法：assert_json_property_equals <actual> <expected> <name>
# ---------------------------------------------------------------------------
assert_json_property_equals() {
    local actual="$1"
    local expected="$2"
    local name="$3"
    if [[ "$actual" != "$expected" ]]; then
        echo "Unexpected $name: expected '$expected', got '$actual'" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# assert_novnc_package 对应 Assert-NoVncPackage
# 用法：assert_novnc_package <package_root> <metadata_path> <attestations_path>
# ---------------------------------------------------------------------------
assert_novnc_package() {
    local package_root="$1"
    local metadata_path="$2"
    local attestations_path="$3"
    local required_files=(
        "AUTHORS"
        "LICENSE.txt"
        "docs/LICENSE.MPL-2.0"
        "vendor/pako/LICENSE"
        "core/crypto/des.js"
        "core/rfb.js"
        "package.json"
    )
    local relative path py

    py="$(get_python3)"

    for relative in "${required_files[@]}"; do
        path="$package_root/$relative"
        if [[ ! -f "$path" ]]; then
            echo "Missing required noVNC file: $relative" >&2
            return 1
        fi
    done

    # 解析 package.json 并校验字段。dependencies 必须为空对象。
    local pkg_name pkg_version pkg_license pkg_deps_empty
    read -r pkg_name pkg_version pkg_license pkg_deps_empty < <(
        "$py" - "$package_root/package.json" "$NOVNC_VERSION" <<'PY' | tr -d '\r'
import json
import sys

path, expected_version = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    data = json.load(handle)
deps = data.get("dependencies", {}) or {}
deps_empty = "1" if len(deps) == 0 else "0"
print(
    str(data.get("name", "")),
    str(data.get("version", "")),
    str(data.get("license", "")),
    deps_empty,
)
PY
    )

    assert_json_property_equals "$pkg_name" "@novnc/novnc" "package name"
    assert_json_property_equals "$pkg_version" "$NOVNC_VERSION" "package version"
    assert_json_property_equals "$pkg_license" "MPL-2.0" "package license"
    if [[ "$pkg_deps_empty" != "1" ]]; then
        echo "noVNC package has unexpected runtime dependencies" >&2
        return 1
    fi

    # 解析 npm-metadata.json
    local m_version m_githead m_tarball m_integrity m_shasum
    read -r m_version m_githead m_tarball m_integrity m_shasum < <(
        "$py" - "$metadata_path" <<'PY' | tr -d '\r'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
dist = data.get("dist", {}) or {}
print(
    str(data.get("version", "")),
    str(data.get("gitHead", "")),
    str(dist.get("tarball", "")),
    str(dist.get("integrity", "")),
    str(dist.get("shasum", "")),
)
PY
    )

    assert_json_property_equals "$m_version" "$NOVNC_VERSION" "npm metadata version"
    assert_json_property_equals "$m_githead" "$NOVNC_COMMIT" "npm metadata gitHead"
    assert_json_property_equals "$m_tarball" "$NOVNC_TARBALL" "npm metadata tarball"
    assert_json_property_equals "$m_integrity" "$NOVNC_INTEGRITY" "npm metadata integrity"
    assert_json_property_equals "$m_shasum" "$NOVNC_SHASUM" "npm metadata shasum"

    # 解析 npm-attestations.json：必须非空且含 SLSA provenance
    local has_slsa
    has_slsa="$(
        "$py" - "$attestations_path" <<'PY'
import json
import sys

with open(sys.argv[1], encoding="utf-8") as handle:
    data = json.load(handle)
attestations = data.get("attestations", []) or []
if len(attestations) == 0:
    print("empty")
    sys.exit()
for entry in attestations:
    if entry.get("predicateType") == "https://slsa.dev/provenance/v1":
        print("slsa")
        sys.exit()
print("no-slsa")
PY
    )"
    if [[ "$has_slsa" == "empty" ]]; then
        echo "npm attestation reference is empty" >&2
        return 1
    fi
    if [[ "$has_slsa" != "slsa" ]]; then
        echo "npm attestation reference lacks SLSA provenance" >&2
        return 1
    fi
}

# ---------------------------------------------------------------------------
# assert_browser_package_lock 对应 Assert-BrowserPackageLock
# 用法：assert_browser_package_lock <package_json_path> <package_lock_path>
# ---------------------------------------------------------------------------
assert_browser_package_lock() {
    local package_json_path="$1"
    local package_lock_path="$2"
    local py

    py="$(get_python3)"

    # 校验 package.json：devDependencies 必须且仅包含 playwright-core 固定版本
    "$py" - "$package_json_path" "$PLAYWRIGHT_VERSION" <<'PY'
import json
import sys

path, pinned = sys.argv[1], sys.argv[2]
with open(path, encoding="utf-8") as handle:
    data = json.load(handle)
deps = data.get("devDependencies", {}) or {}
keys = list(deps.keys())
if len(keys) != 1 or keys[0] != "playwright-core" or deps[keys[0]] != pinned:
    raise SystemExit(f"Browser package must pin only playwright-core {pinned}")
PY

    # 解析 package-lock.json
    "$py" - "$package_lock_path" "$PLAYWRIGHT_VERSION" "$PLAYWRIGHT_RESOLVED" "$PLAYWRIGHT_INTEGRITY" <<'PY'
import json
import sys

path, pinned, resolved, integrity = sys.argv[1:5]
with open(path, encoding="utf-8") as handle:
    data = json.load(handle)

lockfile_version = data.get("lockfileVersion")
if lockfile_version != 3:
    raise SystemExit(f"Unexpected npm lockfile version: expected '3', got '{lockfile_version}'")

packages = data.get("packages", {}) or {}
allowed = ["", "node_modules/playwright-core"]

for name in packages:
    if name not in allowed:
        raise SystemExit(f"Browser lock contains unapproved package: {name}")

for allowed_name in allowed:
    if allowed_name not in packages:
        raise SystemExit(f"Browser lock is missing package: {allowed_name}")

root = packages[""]
root_deps = root.get("devDependencies", {}) or {}
if root_deps.get("playwright-core") != pinned:
    actual = root_deps.get("playwright-core", "")
    raise SystemExit(
        f"Unexpected root playwright-core version: expected '{pinned}', got '{actual}'"
    )

playwright = packages["node_modules/playwright-core"]
if playwright.get("version") != pinned:
    actual = playwright.get("version", "")
    raise SystemExit(
        f"Unexpected playwright-core version: expected '{pinned}', got '{actual}'"
    )

if playwright.get("resolved") != resolved:
    raise SystemExit(f"Unexpected playwright-core registry source: {playwright.get('resolved', '')}")

if playwright.get("integrity") != integrity:
    actual = playwright.get("integrity", "")
    raise SystemExit(
        f"Unexpected playwright-core integrity: expected '{integrity}', got '{actual}'"
    )

if playwright.get("license") != "Apache-2.0":
    actual = playwright.get("license", "")
    raise SystemExit(
        f"Unexpected playwright-core license: expected 'Apache-2.0', got '{actual}'"
    )
PY
}

# ---------------------------------------------------------------------------
# assert_safe_tar_entries 对应 Assert-SafeTarEntries
# 用法：assert_safe_tar_entries <names_file> <verbose_file>
#   names_file    每行一个 tar 条目名
#   verbose_file  每行对应 tar -tvv 的条目，首字符为类型（- d l h 等）
# 两文件行数必须相等。
# ---------------------------------------------------------------------------
assert_safe_tar_entries() {
    local names_file="$1"
    local verbose_file="$2"
    local names_count verbose_count name vline entry_type

    names_count=$(wc -l <"$names_file" | tr -d ' ')
    verbose_count=$(wc -l <"$verbose_file" | tr -d ' ')
    if [[ "$names_count" -ne "$verbose_count" ]]; then
        echo "Tar name and verbose entry counts differ" >&2
        return 1
    fi

    # 用进程替换并行读取两个文件
    while IFS= read -r name && IFS= read -r vline <&3; do
        if [[ -z "$vline" ]]; then
            entry_type=$'\000'
        else
            entry_type="${vline:0:1}"
        fi
        if [[ "$entry_type" != "-" && "$entry_type" != "d" ]]; then
            echo "Unsafe tar entry type '$entry_type': $name" >&2
            return 1
        fi
        if [[ -z "${name//[[:space:]]/}" ]]; then
            echo "Unsafe tar path: $name" >&2
            return 1
        fi
        if [[ "$name" == *'\'* ]]; then
            echo "Unsafe tar path: $name" >&2
            return 1
        fi
        # 注意：bash 变量无法承载 NUL 字节，故 [char]0 检查在 sh 中无可实现的等价物。
        if [[ "$name" == /* ]]; then
            echo "Unsafe tar path: $name" >&2
            return 1
        fi
        if [[ "$name" =~ ^[A-Za-z]: ]]; then
            echo "Unsafe tar path: $name" >&2
            return 1
        fi
        if [[ "$name" != "package" && "$name" != "package/"* ]]; then
            echo "Unsafe tar path: $name" >&2
            return 1
        fi

        # 逐段校验：仅允许最后一段为空（目录条目 package/ 的尾随分隔符）
        local -a segments
        IFS='/' read -ra segments <<<"$name"
        local idx=0 last_idx=$(( ${#segments[@]} - 1 ))
        for segment in "${segments[@]}"; do
            local trailing_dir=0
            if [[ $idx -eq $last_idx && -z "$segment" && "$entry_type" == "d" ]]; then
                trailing_dir=1
            fi
            if [[ $trailing_dir -eq 0 ]]; then
                if [[ -z "$segment" || "$segment" == "." || "$segment" == ".." ]]; then
                    echo "Unsafe tar path: $name" >&2
                    return 1
                fi
            fi
            idx=$((idx + 1))
        done
    done <"$names_file" 3<"$verbose_file"
}

# ---------------------------------------------------------------------------
# assert_safe_extracted_tree 对应 Assert-SafeExtractedTree
# 用法：assert_safe_extracted_tree <root>
# ---------------------------------------------------------------------------
assert_safe_extracted_tree() {
    local root="$1"
    local full_root item

    full_root="$(normalize_path "$root")"
    assert_safe_temporary_path "$full_root"

    # find -L 默认会跟随符号链接；这里用不加 -L 的方式枚举，并显式拒绝符号链接
    while IFS= read -r -d '' item; do
        if [[ -L "$item" ]]; then
            echo "Extracted tree contains a reparse point: $item" >&2
            return 1
        fi
        assert_path_under_root "$item" "$full_root" \
            "Extracted item is outside its root" >/dev/null
    done < <(find "$full_root" -print0)
}
