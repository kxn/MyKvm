# 依赖许可证白名单与本地自动审计实施计划

> **供自动化执行者使用：** 必须逐任务执行本计划，并在每个任务后自审。步骤使用复选框跟踪状态。

**目标：** 建立固定版本的 `cargo-deny` 本地门禁，使未批准许可证和依赖来源在进入主分支前自动失败。

**架构：** `deny.toml` 保存机器可执行规则，PowerShell 模块统一负责工具发现和版本校验，两个入口脚本分别验证策略本身和当前 workspace。统一验证脚本先运行许可证门禁，再运行现有 Rust 验收。

**技术栈：** PowerShell、Cargo、cargo-deny 0.20.2、TOML、临时 Cargo/Git 测试夹具。

## 全局约束

- 所有自写文档使用中文。
- 只检查 `licenses` 和 `sources`，不把在线 advisory 数据库接入本项阻塞门禁。
- `cargo-deny` 必须精确为 `0.20.2`，安装命令为 `cargo install --locked --version 0.20.2 cargo-deny`。
- 验证脚本不得自动安装工具或静默跳过检查。
- 当前没有 Gitea runner，全部验收在本机执行。
- 自动允许列表只包含设计文档列出的宽松许可证。
- MPL、LGPL 和其他条件许可证只能按具体依赖例外，不加入全局允许列表。
- 未知注册表和 Git 依赖默认拒绝。
- 临时测试数据只能在系统临时目录创建，并在删除前校验路径边界。
- 不修改主工作区中用户拥有的 `AGENTS.md` 变更。

---

## 文件结构

- 新建 `deny.toml`：Cargo 许可证和来源的机器可执行策略。
- 新建 `scripts/license-policy-tools.psm1`：固定工具版本、解析版本输出和发现可执行文件。
- 新建 `scripts/test-license-policy.ps1`：工具契约和许可证/来源负向夹具。
- 新建 `scripts/verify-licenses.ps1`：检查当前 workspace 的锁定依赖图。
- 修改 `scripts/verify.ps1`：接入策略测试和 workspace 审计。
- 新建 `docs/dependency-license-policy.md`：长期依赖准入和分发义务规则。
- 修改 `README.md`：记录本地许可证验证入口。
- 修改 `docs/development-guidelines.md`：新增依赖必须经过门禁和文档审查。
- 修改 `docs/ipkvm-coarse-design.md`：把阶段 0 许可证白名单标为完成。
- 修改设计文档和本计划：记录实施完成状态。

---

### 任务 1：固定 cargo-deny 工具版本契约

**文件：**

- 新建：`scripts/test-license-policy.ps1`
- 新建：`scripts/license-policy-tools.psm1`

**接口：**

- 产出：`Get-RequiredCargoDenyVersion -> string`
- 产出：`Assert-CargoDenyVersion -VersionOutput <string> -> string`
- 产出：`Get-CargoDenyExecutable -> string`
- 后续任务只通过该模块读取版本和定位工具，不复制版本常量。

- [ ] **步骤 1：先写版本契约测试**

`scripts/test-license-policy.ps1` 首先导入尚不存在的模块，并覆盖正确、错误和格式异常三种输出：

```powershell
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "license-policy-tools.psm1") -Force

function Assert-ThrowsLike {
    param(
        [Parameter(Mandatory)]
        [scriptblock]$Command,
        [Parameter(Mandatory)]
        [string]$Pattern
    )

    try {
        & $Command
    }
    catch {
        if ($_.Exception.Message -notmatch $Pattern) {
            throw "异常不包含预期内容 '$Pattern'：$($_.Exception.Message)"
        }
        return
    }

    throw "命令本应失败，但实际成功"
}

if ((Get-RequiredCargoDenyVersion) -ne "0.20.2") {
    throw "固定版本不是 0.20.2"
}

if ((Assert-CargoDenyVersion -VersionOutput "cargo-deny 0.20.2") -ne "0.20.2") {
    throw "正确版本没有通过"
}

Assert-ThrowsLike {
    Assert-CargoDenyVersion -VersionOutput "cargo-deny 0.20.1"
} "期望 0\.20\.2.*实际 0\.20\.1"

Assert-ThrowsLike {
    Assert-CargoDenyVersion -VersionOutput "无法解析"
} "无法解析 cargo-deny 版本"
```

- [ ] **步骤 2：运行测试并确认按预期失败**

运行：

```powershell
.\scripts\test-license-policy.ps1
```

预期：失败，原因是 `license-policy-tools.psm1` 不存在。

- [ ] **步骤 3：实现最小工具模块**

`scripts/license-policy-tools.psm1` 的核心实现：

```powershell
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

$script:RequiredCargoDenyVersion = "0.20.2"
$script:CargoDenyInstallCommand =
    "cargo install --locked --version 0.20.2 cargo-deny"

function Get-RequiredCargoDenyVersion {
    return $script:RequiredCargoDenyVersion
}

function Assert-CargoDenyVersion {
    param(
        [Parameter(Mandatory)]
        [string]$VersionOutput
    )

    $match = [regex]::Match(
        $VersionOutput.Trim(),
        "^cargo-deny\s+([0-9]+\.[0-9]+\.[0-9]+)(?:\s.*)?$"
    )
    if (-not $match.Success) {
        throw "无法解析 cargo-deny 版本。请执行：$script:CargoDenyInstallCommand"
    }

    $actual = $match.Groups[1].Value
    if ($actual -ne $script:RequiredCargoDenyVersion) {
        throw (
            "cargo-deny 版本不符：期望 $script:RequiredCargoDenyVersion，" +
            "实际 $actual。请执行：$script:CargoDenyInstallCommand"
        )
    }

    return $actual
}

function Get-CargoDenyExecutable {
    $command = Get-Command "cargo-deny" -CommandType Application `
        -ErrorAction SilentlyContinue
    if ($null -eq $command) {
        throw "未找到 cargo-deny。请执行：$script:CargoDenyInstallCommand"
    }

    $output = & $command.Source --version
    if ($LASTEXITCODE -ne 0) {
        throw "cargo-deny --version 执行失败，退出码：$LASTEXITCODE"
    }

    $null = Assert-CargoDenyVersion -VersionOutput ($output -join "`n")
    return $command.Source
}

Export-ModuleMember -Function @(
    "Get-RequiredCargoDenyVersion",
    "Assert-CargoDenyVersion",
    "Get-CargoDenyExecutable"
)
```

- [ ] **步骤 4：运行版本契约测试**

运行：

```powershell
.\scripts\test-license-policy.ps1
```

预期：版本解析测试通过；脚本退出码为 0。

- [ ] **步骤 5：检查 PowerShell 和 Git 差异**

运行：

```powershell
git diff --check
git status --short
```

预期：只有本任务两个脚本有改动，没有 `AGENTS.md`。

- [ ] **步骤 6：提交**

```powershell
git add scripts/test-license-policy.ps1 scripts/license-policy-tools.psm1
git commit -m "test: fix cargo-deny tool contract (#13)"
```

---

### 任务 2：用负向夹具固定许可证和来源策略

**文件：**

- 修改：`scripts/test-license-policy.ps1`
- 新建：`deny.toml`

**接口：**

- 消费：`Get-CargoDenyExecutable`
- 产出：可重复、无远端网络依赖的许可证和 Git 来源策略测试。
- 产出：后续 workspace 审计使用的根目录 `deny.toml`。

- [ ] **步骤 1：扩展临时夹具测试**

在版本契约测试之后增加：

1. `New-PolicyFixtureRoot`，使用 GUID 在系统临时目录创建目录，并以绝对路径前缀校验边界。
2. `Set-Utf8File`，使用无 BOM UTF-8 写入临时 `Cargo.toml` 和 `src/lib.rs`。
3. `Invoke-CargoDenyFixture`，同时捕获标准输出、标准错误和退出码。
4. 允许用例：MIT 根包加 BSD-3-Clause 路径依赖，`licenses sources` 返回 0。
5. 拒绝许可证用例：把依赖声明为 `GPL-3.0-only`，退出码包含 licenses 位 `4`，诊断包含 `rejected` 和 `GPL-3.0-only`。
6. 拒绝来源用例：在临时目录创建 MIT crate 的本地 Git 仓库，消费方使用 `git = "file:///..."`，`sources` 退出码为 `8`，诊断包含 `unknown-git`。
7. `finally` 中只删除已确认位于系统临时目录下的夹具根目录。

本地 Git 仓库固定提交身份：

```powershell
git -C $gitDependency config user.name "my_ipkvm policy test"
git -C $gitDependency config user.email "policy-test@invalid.local"
git -C $gitDependency add .
git -C $gitDependency commit -m "fixture"
```

所有 Cargo 夹具都包含独立 `[workspace]`，避免被父仓库 workspace 自动吸收。

- [ ] **步骤 2：运行测试并确认策略文件缺失**

运行：

```powershell
.\scripts\test-license-policy.ps1
```

预期：失败，明确指出根目录 `deny.toml` 不存在或无法读取。

- [ ] **步骤 3：新增最小 deny.toml**

创建：

```toml
[graph]
all-features = true

[licenses]
allow = [
    "MIT",
    "Apache-2.0",
    "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause",
    "BSD-3-Clause",
    "ISC",
    "Zlib",
    "Unicode-3.0",
]
confidence-threshold = 0.93
include-dev = true
unused-allowed-license = "allow"

[licenses.private]
ignore = false

[sources]
unknown-registry = "deny"
unknown-git = "deny"
required-git-spec = "rev"
allow-registry = [
    "https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
]
unused-allowed-source = "allow"
```

配置不增加 LGPL/MPL 例外；当前依赖图不需要例外。

- [ ] **步骤 4：安装固定工具**

若本机尚未安装，运行：

```powershell
cargo install --locked --version 0.20.2 cargo-deny
```

预期：

```text
Installed package `cargo-deny v0.20.2`
```

再次运行：

```powershell
cargo-deny --version
```

预期：

```text
cargo-deny 0.20.2
```

- [ ] **步骤 5：运行策略测试**

运行：

```powershell
.\scripts\test-license-policy.ps1
```

预期：允许夹具通过；GPL 和 Git 夹具均被目标规则拒绝；脚本最终退出码为 0。

- [ ] **步骤 6：确认测试没有污染仓库**

运行：

```powershell
git status --short
git diff --check
```

预期：没有临时 Cargo.lock、嵌套 Git 仓库或其他未跟踪夹具。

- [ ] **步骤 7：提交**

```powershell
git add deny.toml scripts/test-license-policy.ps1
git commit -m "test: enforce dependency license policy (#13)"
```

---

### 任务 3：审计当前 workspace 并接入统一验证

**文件：**

- 新建：`scripts/verify-licenses.ps1`
- 修改：`scripts/verify.ps1`

**接口：**

- 消费：`Get-CargoDenyExecutable`
- 产出：`scripts/verify-licenses.ps1` 作为当前 workspace 独立许可证验收入口。
- 产出：`scripts/verify.ps1` 在所有 Rust 验收前运行策略测试和 workspace 审计。

- [ ] **步骤 1：先在统一验证中调用尚不存在的脚本**

在文本编码检查之后、Rust 格式检查之前增加：

```powershell
Invoke-CheckedCommand "Test dependency license policy" {
    & (Join-Path $PSScriptRoot "test-license-policy.ps1")
}
Invoke-CheckedCommand "Check dependency licenses and sources" {
    & (Join-Path $PSScriptRoot "verify-licenses.ps1")
}
```

同时把编码扫描扩展名增加 `*.psm1`：

```powershell
& git ls-files -- "*.json" "*.md" "*.ps1" "*.psm1" "*.rs" "*.toml" ...
```

- [ ] **步骤 2：运行统一验证并确认按预期失败**

运行：

```powershell
.\scripts\verify.ps1
```

预期：策略测试通过，随后因为 `verify-licenses.ps1` 不存在而失败。

- [ ] **步骤 3：实现 workspace 审计脚本**

创建：

```powershell
[CmdletBinding()]
param()

Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"

Import-Module (Join-Path $PSScriptRoot "license-policy-tools.psm1") -Force

$repositoryRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$cargoDeny = Get-CargoDenyExecutable

Push-Location $repositoryRoot
try {
    & $cargoDeny --locked check licenses sources
    if ($LASTEXITCODE -ne 0) {
        throw "依赖许可证或来源检查失败，退出码：$LASTEXITCODE"
    }
}
finally {
    Pop-Location
}

Write-Host "依赖许可证和来源检查通过。"
```

- [ ] **步骤 4：单独运行当前依赖图审计**

运行：

```powershell
.\scripts\verify-licenses.ps1
```

预期：当前 Cargo.lock 全部通过，退出码为 0。

如果出现无效 SPDX 表达式，只能用 `licenses.clarify` 配合实际许可证文件哈希解决；禁止把未知许可证全局放行。

- [ ] **步骤 5：运行统一验证**

运行：

```powershell
.\scripts\verify.ps1
```

预期：新增两个门禁以及原有 Rust 验收全部通过。

- [ ] **步骤 6：提交**

```powershell
git add scripts/verify.ps1 scripts/verify-licenses.ps1
git commit -m "build: audit dependency licenses locally (#13)"
```

---

### 任务 4：写入长期中文规则和阶段状态

**文件：**

- 新建：`docs/dependency-license-policy.md`
- 修改：`README.md`
- 修改：`docs/development-guidelines.md`
- 修改：`docs/ipkvm-coarse-design.md`

**接口：**

- 产出：开发者新增依赖时可直接执行的稳定规则。
- 产出：阶段 0 状态与实际实现一致。

- [ ] **步骤 1：新增许可证策略文档**

文档必须明确：

- 自动允许、按包例外和默认拒绝三层。
- `cargo-deny 0.20.2` 安装与独立验证命令。
- 新增 Cargo 依赖必须先运行策略测试和 workspace 审计。
- LGPL Rust 静态链接不是绝对禁止，但必须为具体依赖设计重新链接和发布义务。
- noVNC、Qt、FFmpeg、GStreamer、系统 SDK 和资源文件不由 Cargo 门禁完整覆盖。
- 发布阶段仍需生成第三方清单并附带要求的许可证和源码说明。
- `cargo-deny` 是自动化证据，不是法律意见。

- [ ] **步骤 2：更新开发规范**

在依赖和 PR 规则中增加：

```text
新增或升级第三方依赖时，必须先通过许可证和来源门禁；条件许可证必须关联独立 issue 和中文合规记录。禁止用宽泛全局白名单代替具体依赖审查。
```

把统一验证说明更新为包含许可证策略测试和当前依赖图审计。

- [ ] **步骤 3：更新 README**

在本地验证部分加入固定工具安装命令，并说明 `scripts/verify.ps1` 已包含许可证和来源检查。

- [ ] **步骤 4：更新粗粒度阶段状态**

阶段 0 已完成列表增加：

```text
- 固定 cargo-deny 0.20.2，建立依赖许可证分级、来源限制和本地负向策略测试。
```

从待完成列表移除“确定依赖许可证白名单”。

把原许可证策略中的 MPL/LGPL 表述改成与本设计一致：可接受但按组件审查，不进入 Cargo 全局自动允许列表。

- [ ] **步骤 5：校验文档语言和格式**

运行：

```powershell
rg -n "TODO|TBD|待定|占位|<<<<<<<|=======|>>>>>>>" `
    README.md docs/dependency-license-policy.md `
    docs/development-guidelines.md docs/ipkvm-coarse-design.md
git diff --check
```

预期：没有占位符或冲突标记，差异格式通过。

- [ ] **步骤 6：提交**

```powershell
git add README.md docs/dependency-license-policy.md `
    docs/development-guidelines.md docs/ipkvm-coarse-design.md
git commit -m "docs: record dependency license rules (#13)"
```

---

### 任务 5：完整验收、自审和完成记录

**文件：**

- 修改：`docs/superpowers/specs/2026-07-31-dependency-license-policy-design.md`
- 修改：`docs/superpowers/plans/2026-07-31-dependency-license-policy.md`

**接口：**

- 产出：可审计的实施状态和完整本地验证证据。

- [ ] **步骤 1：运行精确工具版本检查**

运行：

```powershell
cargo-deny --version
```

预期：

```text
cargo-deny 0.20.2
```

- [ ] **步骤 2：独立运行策略负向测试**

运行：

```powershell
.\scripts\test-license-policy.ps1
```

预期：版本契约、宽松许可证、GPL 拒绝和 Git 来源拒绝全部通过。

- [ ] **步骤 3：独立运行当前依赖图审计**

运行：

```powershell
.\scripts\verify-licenses.ps1
```

预期：当前 workspace 的许可证和来源全部通过。

- [ ] **步骤 4：运行完整本地验收**

运行：

```powershell
.\scripts\verify.ps1
```

预期：许可证门禁、UTF-8、Rust 格式、全 workspace 测试、Clippy、Rust 文档和 Git 差异全部通过。

- [ ] **步骤 5：进行实施自审**

运行：

```powershell
git diff --check main...HEAD
git diff --stat main...HEAD
git status --short
rg -n "\[ \]" docs/superpowers/plans/2026-07-31-dependency-license-policy.md
```

核对：

- 没有全局允许 MPL、LGPL、GPL 或未知许可证。
- 没有允许任意 Git 组织或任意注册表。
- 负向测试校验目标诊断，不把任意失败误判为成功。
- 临时目录删除有边界检查。
- 工具缺失和版本错误不会静默通过。
- 文档没有声称 Cargo 审计覆盖非 Cargo 组件。
- 工作树没有临时夹具和无关文件。

- [ ] **步骤 6：回写完成状态**

设计文档状态改为：

```text
状态：已实施并通过本地自动化验证
```

勾选本计划全部步骤，并记录实际验证命令，不写无法复现的人工结论。

- [ ] **步骤 7：提交完成记录**

```powershell
git add docs/superpowers/specs/2026-07-31-dependency-license-policy-design.md `
    docs/superpowers/plans/2026-07-31-dependency-license-policy.md
git commit -m "docs: record license audit completion (#13)"
```

- [ ] **步骤 8：再次运行完整验收**

运行：

```powershell
.\scripts\verify.ps1
```

预期：提交完成记录后仍全部通过，`git status --short` 无输出。

- [ ] **步骤 9：创建 PR 并合并**

PR 标题：

```text
建立依赖许可证白名单与本地自动审计门禁
```

PR 描述包含：

- `Closes #13`
- 许可证分级和来源策略。
- 负向策略测试。
- 当前依赖图审计。
- 文档边界。
- 本机 `.\scripts\verify.ps1` 结果。
- 人工验证例外：无。

合并后确认 issue #13 关闭，清理功能工作树和远端分支，并将主分支快进到合并提交。

