# GitHub 公开仓库迁移实施计划

> **面向自动化协作者：** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development（推荐）或 superpowers:executing-plans 执行本计划。每个步骤使用复选框跟踪。

**目标：** 将 GitHub 建立为 my_ipkvm 的公开开发与协作主仓库，将私有 Gitea 保留为代码备份和灾难恢复副本，并把仓库内的日常开发工作流从 Gitea/tea 平滑切换到 GitHub/gh。

**架构：** GitHub 是唯一的代码、Issue、PR、CI 和 Release 协作源；本地工作副本使用 origin 指向 GitHub、private 指向 Gitea。Gitea 只接收经过验证的主线、标签和必要的备份分支，不再接受独立的日常开发、Issue 或 PR 状态变更。公开仓库只推送经过审计的 main、发布标签和明确允许的开发分支，不把内部临时分支整体公开。

**技术栈：** Git、GitHub Web/CLI（gh）、Gitea CLI（tea，仅用于迁移前和备份确认）、GitHub Actions、PowerShell 7、Rust 1.89、现有 scripts/verify.ps1 与 scripts/verify.sh。

## 全局约束

- 所有新写仓库文档、Issue、PR 模板和迁移记录使用中文；命令、路径、标识符、协议字段和第三方专有名词保持原文。
- PowerShell 处理中文、调用 gh 或 tea 写入远端内容前，显式设置 UTF-8 输入、输出和 $OutputEncoding。
- 本计划对应 Gitea issue #169；本计划的实现提交使用英文 conventional commit，并在提交信息中包含 #169；收口 PR 使用 Closes #169。
- 不直接向 main 提交或推送。迁移计划文档从最新私有 origin/main 创建 issue 分支，并通过 Gitea PR 收口。
- GitHub 对外公开前必须完成历史敏感信息扫描、第三方资料许可审查和工作树内容审查。任何疑似密钥或无权再分发资料都必须先停在公开切换前，不得用删除当前文件代替历史清理或许可判断。
- 不为了修复历史提交中的 #编号 自动链接而重写整个提交历史；历史重写只允许在确认存在敏感信息、许可证风险或明确的法律要求时单独审批。
- GitHub 与 Gitea 不双向维护 Issue、PR、标签和开发分支。所有新工作从 GitHub Issue 开始，所有新 PR 在 GitHub 创建。
- 代码改动或构建配置改动按仓库默认要求运行 cargo fmt --all --check 和 cargo test --workspace --all-features；仅文档或平台模板变更运行与其范围匹配的静态检查，并在 PR 中说明未运行 Rust 业务测试的原因。
- 当前迁移计划分支从私有远端最新主线 f0e3892 开始。主工作树中的未跟踪 artifacts/ 不属于本计划，不得通过 git add . 纳入提交。

## 当前事实与迁移边界

迁移执行前需要以命令输出重新确认以下事实，计划中的路径是当前仓库已发现的处理边界：

- 当前私有远端名称为 origin，地址是私有 Gitea；迁移后将其重命名为 private，新增 GitHub 远端 origin。
- Cargo.toml 的 workspace repository 元数据仍是内网 Gitea 地址。
- crates/ipkvm-desktop-iced/src/app.rs 的 PROJECT_URL 和 crates/ipkvm-desktop-iced/src/platform/mod.rs 的项目链接仍指向内网 Gitea。
- AGENTS.md、CLAUDE.md、HANDOFF.md、docs/development-guidelines.md 和多个历史计划包含 Gitea/tea 工作流说明。
- .gitea/ISSUE_TEMPLATE/ 和 .gitea/PULL_REQUEST_TEMPLATE.md 是现有平台模板；GitHub 需要新增 .github/ISSUE_TEMPLATE/、.github/PULL_REQUEST_TEMPLATE.md 和 .github/workflows/。
- docs/references/ 包含 noVNC 资料、CH9329 资料、USB HID/UVC PDF/ZIP；third_party/ 包含 noVNC、iced_aw 和字体等第三方内容。它们不能因为已在私有仓库中就默认具有公开再分发权。
- artifacts/ 当前包含 Windows 构建包并且未跟踪；它应保持不进入 Git，是否发布二进制由单独的 GitHub Release 流程决定。
- 历史提交中的 Gitea Issue 编号与未来 GitHub Issue 编号没有一一对应关系。旧计划作为历史记录保留原语境，长期操作规则和新的协作入口改为 GitHub。

## 文件责任地图

本迁移工作的文件边界如下：

- 创建 docs/superpowers/plans/2026-08-04-github-public-migration.md：本次迁移的执行计划、验收标准、命令和回滚策略。
- 更新 Cargo.toml、crates/ipkvm-desktop-iced/src/app.rs、crates/ipkvm-desktop-iced/src/platform/mod.rs：将项目元数据和用户可见的项目主页指向 GitHub。
- 更新 README.md、AGENTS.md、CLAUDE.md、HANDOFF.md、docs/development-guidelines.md：将长期使用的开发入口、Issue/PR/验证和收口规则改成 GitHub/gh。
- 新增 .github/ISSUE_TEMPLATE/bug-fix.md、.github/ISSUE_TEMPLATE/development-task.md、.github/PULL_REQUEST_TEMPLATE.md：复制现有验收要求，但移除 Gitea 专属命令和内网地址。
- 新增 .github/workflows/verify.yml：在 GitHub Actions 上调用现有验证脚本或等价的分步命令，使用最小 contents: read 权限。
- 新增 .github/workflows/release.yml（在首次迁移稳定后）：只响应版本标签，构建经过验证的发布物，不把工作区 artifacts/ 自动上传为正式 Release。
- 新增 docs/migration/github-issue-map.md（需要迁移历史 Issue 时）：只记录公开可分享的旧 Gitea 编号、新 GitHub 编号、状态和迁移说明，不写入内网地址、账号或私有讨论。
- 新增 scripts/sync-private-mirror.ps1（需要自动备份时）：只同步 GitHub 的主线和标签到私有 Gitea，不包含凭据，不使用可能删除远端分支的无条件 --mirror。
- 删除或停止使用 .gitea/：GitHub 模板验证完成后，公开主线不再保留会诱导协作者使用 Gitea 的活动模板；历史内容通过 Git 历史保留，不重写历史。

---

## 阶段 A：公开前审计

### 任务 1：冻结迁移基线并建立清单

**文件：**

- 只读检查，不修改源代码。
- 记录到本 issue 和迁移 PR；不把本地构建输出加入计划提交。

**依赖：** Gitea #169 已创建；工作树从私有远端最新 main 创建。

- [ ] **步骤 1：确认当前工作树没有误改动。**

~~~~powershell
git status --short --branch
git branch --show-current
git log -1 --oneline --decorate
git worktree list
~~~~

预期：迁移分支工作树干净，只显示当前分支跟踪私有 main；主工作树的 artifacts/ 不出现在迁移工作树中。

- [ ] **步骤 2：抓取私有主线并记录对象完整性。**

~~~~powershell
git fetch origin --prune
git fsck --full
git show origin/main --no-patch --format='%H%n%P%n%s'
git branch --all
git tag --list
~~~~

预期：git fsck --full 没有 dangling secret-like 对象或损坏对象；主线提交哈希、分支和标签清单写入迁移 PR 的测试证据。

- [ ] **步骤 3：枚举公开范围。**

只允许以下内容进入首次 GitHub 推送：最新 main、经过审计的发布标签和明确选择的公开分支。带有内部实验、硬件现场、未完成设计或私有 Issue 依赖的分支先留在私有 Gitea。

验收：有一份“推送/不推送”分支清单，且首次推送命令使用显式分支名，不使用 git push --all origin。

### 任务 2：扫描密钥、内部地址和本地工件

**文件：**

- 检查 .gitignore、AGENTS.md、CLAUDE.md、HANDOFF.md、Cargo.toml、README.md、docs/、crates/ 和完整 Git 历史。
- 如确认构建包不应入库，修改 .gitignore 增加明确的 /artifacts/ 规则，并在计划 PR 中单独说明；不要删除用户未跟踪文件。

- [ ] **步骤 1：扫描当前工作树和 Git 历史中的凭据模式。**

~~~~powershell
rg -n --hidden -g '!target/**' -g '!target-*/**' -g '!.git/**' -g '!*.pdf' -g '!*.zip' '(?i)(password|passwd|secret|token|api[_-]?key|private[_-]?key|BEGIN [A-Z ]+ PRIVATE KEY|10\.10\.10\.5|\.env)' .
git log --all --name-only --format='' | Where-Object { $_ -match '(?i)(\.env|secret|token|credential|id_rsa|\.pem$|\.key$)' } | Sort-Object -Unique
~~~~

如果扫描发现真实凭据，立即停止公开推送；先撤销/轮换凭据，再用 git filter-repo 或经过批准的等价工具清理历史，最后重新运行全历史扫描。不要仅删除当前文件。

- [ ] **步骤 2：使用可用的历史扫描工具。**

优先运行：

~~~~powershell
gitleaks detect --source . --redact --no-banner
~~~~

如果本机没有 gitleaks，把“工具未安装”记录为验证缺口，不把 rg 结果当作完整密钥扫描替代品；在 GitHub 首次公开前使用 CI 或另一台干净机器完成一次正式扫描。

- [ ] **步骤 3：确认未跟踪工件不会被提交。**

~~~~powershell
git status --short
git check-ignore -v artifacts/ artifacts/my_ipkvm-windows-f0e3892.zip
~~~~

预期：构建包未被暂存；如果没有忽略规则，先决定是增加 /artifacts/ 忽略规则还是转入独立 Release 任务，再进行 git add。

验收：迁移 PR 记录扫描工具、扫描范围、结果、例外和处理动作；公开切换前没有未解释的内部地址、凭据或本地构建物。

### 任务 3：完成第三方资料和许可证审计

**文件：**

- LICENSE
- docs/references/README.md
- docs/references/USB-HID-Usage-Tables-1.7.pdf
- docs/references/USB-Video-Class-1.5-document-set.zip
- docs/references/uvc-1.5/
- docs/references/CH9329-*.pdf
- third_party/iced_aw/
- third_party/novnc/1.7.0/
- docs/dependency-license-policy.md

- [ ] **步骤 1：建立资料发布分类。**

逐项标记为“允许随项目分发”“仅用于本地研究”“需要保留许可证后才能分发”或“改为官方链接”。必须记录来源 URL、版本/提交、许可证文件路径、是否修改和公开分发义务。

- [ ] **步骤 2：处理无明确再分发权的规范资料。**

对 USB-IF/UVC/HID 和供应商数据手册，在没有明确再分发许可时，从公开主线移除原始 PDF/ZIP，只在 docs/references/README.md 保留中文用途、官方来源和获取说明。当前私有 Gitea 历史不重写，除非法律审计明确要求删除历史对象。

- [ ] **步骤 3：验证第三方代码声明。**

确保 noVNC、iced_aw、字体和嵌入资源的许可证文件、来源固定值和修改记录仍与 scripts/verify-web-assets.ps1、scripts/verify-licenses.ps1 和 docs/dependency-license-policy.md 一致；发现许可证缺口时先补充 NOTICE/说明或移除资源，不能关闭检查绕过问题。

验收：每个公开二进制和源码树中的第三方资产都有许可证依据；不公开的研究资料已移除或明确隔离，docs/references/README.md 没有声称未知资料可以再分发。

---

## 阶段 B：把仓库内容改成 GitHub 语境

### 任务 4：更新项目元数据和用户可见入口

**文件：**

- 修改 Cargo.toml 的 [workspace.package] repository。
- 修改 crates/ipkvm-desktop-iced/src/app.rs 的 PROJECT_URL。
- 修改 crates/ipkvm-desktop-iced/src/platform/mod.rs 的菜单链接。
- 更新 README.md 的仓库链接、开发入口、模板路径和验证说明。

- [ ] **步骤 1：统一仓库 URL。**

将所有运行时项目主页和 Cargo 元数据统一指向最终 GitHub canonical URL。不要在代码中保留内网备用地址；备份地址只出现在私有运维说明，不出现在公开用户界面。

- [ ] **步骤 2：保留 URL 回归覆盖。**

更新或新增现有 about/平台 URL 测试，使其断言最终公开 URL 经过同一个常量或同一配置入口；不要在测试中继续硬编码私有 Gitea URL。

- [ ] **步骤 3：运行代码范围验证。**

~~~~powershell
rg -n --hidden -g '!target/**' -g '!target-*/**' -g '!.git/**' '10\.10\.10\.5|http://10\.10\.10\.5|kxn/my_ipkvm' Cargo.toml README.md crates docs
cargo fmt --all --check
cargo test --workspace --all-features
~~~~

预期：运行时源代码、Cargo 元数据和长期用户文档不再命中内网地址；历史计划文件中的旧链接若保留，必须被标记为历史上下文并从新工作流说明中排除。

### 任务 5：把开发规则从 Gitea/tea 切换到 GitHub/gh

**文件：**

- 修改 AGENTS.md。
- 修改 CLAUDE.md。
- 修改 HANDOFF.md。
- 修改 docs/development-guidelines.md。
- 修改 README.md 的开发规范和模板路径。
- 新增 .github/ISSUE_TEMPLATE/bug-fix.md。
- 新增 .github/ISSUE_TEMPLATE/development-task.md。
- 新增 .github/PULL_REQUEST_TEMPLATE.md。
- 新增 .github/ISSUE_TEMPLATE/config.yml。

- [ ] **步骤 1：定义 GitHub 为唯一工作入口。**

将长期规则改成：新工作从 GitHub Issue 开始；分支从 GitHub 默认分支创建；PR 在 GitHub 创建；PR 描述使用 Closes #编号；合并后检查 GitHub Issue 状态。历史 Gitea Issue 只作为迁移映射和归档来源。

- [ ] **步骤 2：把常用命令改为 gh。**

公开文档中的命令至少覆盖以下操作；执行前将 GITHUB_REPOSITORY 环境变量设置为最终 GitHub 仓库全名，例如 owner/my_ipkvm：

~~~~powershell
$GitHubRepository = $env:GITHUB_REPOSITORY
$GitHubIssueNumber = 1
$GitHubPrNumber = 1
$FeatureBranch = "feature/example"
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)

gh issue list --repo $GitHubRepository
gh issue view $GitHubIssueNumber --repo $GitHubRepository
gh issue create --repo $GitHubRepository --title 'docs: record a migration decision' --body-file .github/issue-body.md
gh pr list --repo $GitHubRepository
gh pr create --repo $GitHubRepository --base main --head $FeatureBranch --title 'docs: record a migration decision' --body-file .github/pr-body.md
gh pr merge $GitHubPrNumber --repo $GitHubRepository --squash --delete-branch=false
~~~~

实际模板不应依赖临时 .github/issue-body.md 文件；上面的 --body-file 仅表示 PowerShell 传递中文正文时应使用 UTF-8 文件或 UTF-8 字节，不能把系统默认编码的管道文本直接写入 GitHub。

- [ ] **步骤 3：迁移模板内容。**

保留现有中文字段：背景、目标、不做范围、根因/设计依据、验收标准、自动化测试计划、人工验证例外、文档影响和相关资料。删除 tea、私有服务器地址和 Gitea 专属收口步骤。config.yml 明确是否允许空白 Issue；默认关闭空白 Issue，要求新工作至少包含背景和验收标准。

- [ ] **步骤 4：区分历史文档和操作文档。**

不要批量改写所有旧计划中的 Gitea #编号；这些文件是历史事实。只更新会指导未来协作者的入口文档，并在 README.md 和 HANDOFF.md 说明旧计划的历史性质。

验收：在仓库可搜索范围内，除历史记录、迁移计划和归档章节外，未来协作者不会再被引导使用 tea、内网 URL 或 .gitea 模板。

---

## 阶段 C：建立 GitHub 仓库与自动化

### 任务 6：创建 GitHub 仓库并配置治理

**文件：**

- GitHub 仓库设置，不涉及本地文件。
- .github/ 模板和 Actions 由后续任务提交。

- [ ] **步骤 1：创建空仓库。**

使用 GitHub Web 或已登录的 gh 创建仓库，第一次公开前推荐先设为 Private 完成审计，确认后再改为 Public。不要初始化 README、License、.gitignore，避免生成与现有历史无关的根提交。

- [ ] **步骤 2：配置基础仓库信息。**

设置默认分支为 main，检查仓库描述、Topics、License、Security policy、Code of Conduct 是否与当前项目实际状态一致。没有真实支持的功能不要写成项目保证；TLS 尚未实现等限制要保留在 README。

- [ ] **步骤 3：配置 main 保护。**

在第一次 GitHub Actions 验证成功后，为 main 设置：禁止直接推送、禁止强制推送、PR 合并前要求验证任务通过、要求解决审查意见、要求分支最新。单维护者的紧急绕过权限只作为 GitHub 管理员恢复手段，不写入日常流程。

- [ ] **步骤 4：配置权限和安全项。**

Actions 默认 permissions: contents: read；只有发布 workflow 为创建 Release 临时申请 contents: write。开启 Dependabot 或安全告警前先确认依赖策略和维护能力；任何硬件、私有 Gitea 或签名凭据只能放在 GitHub Secrets，不写入仓库和 workflow 日志。

验收：GitHub 仓库存在且默认分支正确，保护规则在验证 workflow 存在后可以选择对应的 required check，公开切换前 Security/Secret 扫描没有未处理结果。

### 任务 7：迁移验证流程到 GitHub Actions

**文件：**

- 新增 .github/workflows/verify.yml。
- 如有需要，最小修改 scripts/verify.ps1、scripts/verify.sh 以支持无交互 CI；修改时必须补充对应本地验证。

- [ ] **步骤 1：将现有本机门禁作为权威入口。**

GitHub Actions 首个 workflow 使用 actions/checkout、Rust 1.89 工具链、Node.js 20 和固定版本 cargo-deny 0.20.2，然后调用现有 scripts/verify.ps1。如果 Windows runner 上的真实浏览器门禁不稳定，先把稳定的静态、许可证、格式、测试、Clippy 和文档检查拆为必需 job，把浏览器闭环作为明确命名的非阻塞 job；不能静默删除检查。

- [ ] **步骤 2：定义触发和权限。**

workflow 在 pull_request 和推送到 main 时运行，使用 concurrency 取消同一 PR 的旧运行，权限只读。PR 模板要求填写实际 Actions run 链接或失败原因。

- [ ] **步骤 3：本地复现 CI。**

~~~~powershell
cargo install --locked --version 0.20.2 cargo-deny
.\scripts\verify.ps1
~~~~

预期：本地与 GitHub Actions 使用相同 Rust 版本约束、cargo-deny 版本和浏览器依赖入口；路径、编码和 shell 差异都有明确失败信息。

- [ ] **步骤 4：将 workflow 作为分支保护 required check。**

先在测试 PR 上确认 check 名称稳定，再把稳定 job 设置为 main 的 required status check；不要在 check 名称未验证时提前锁死分支保护。

### 任务 8：定义 Release 和发布物边界

**文件：**

- 新增 .github/workflows/release.yml，在首次手工 Release 验证后再启用自动发布。
- 修改 README.md 的安装/下载入口。
- 必要时新增 docs/release/github-release.md。

- [ ] **步骤 1：先手工验证发布物。**

使用版本标签触发或手工执行 Windows release 构建，运行：

~~~~powershell
cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced
.\scripts\verify-desktop-release.ps1
~~~~

记录构建提交、Rust 工具链、文件名、SHA-256 和启动冒烟结果。artifacts/ 是本地临时目录，不自动等同于 GitHub Release 内容。

- [ ] **步骤 2：确定标签和权限规则。**

只允许维护者创建 v* 版本标签；Release workflow 只对版本标签运行，上传最小发布包、许可证和校验文件，权限使用 contents: write，不把 Gitea Token 放进 GitHub Secrets。

- [ ] **步骤 3：将发布入口改到 GitHub。**

README 只链接 GitHub Releases 或构建说明，不链接私有 Gitea 下载地址；发布前明确当前平台支持、硬件要求、TLS 限制和已知风险。

验收：首次 GitHub Release 可以从干净目录下载、解压并通过 Windows 启动冒烟；发布物不包含私有地址、凭据或未许可资料。

---

## 阶段 D：迁移 Git 历史和建立双远端

### 任务 9：使用显式分支完成首次推送

**文件：**

- 修改本地 Git 远端配置，不把凭据写入仓库。
- 记录远端约定到 HANDOFF.md 和 docs/development-guidelines.md。

- [ ] **步骤 1：在干净迁移 clone 中重命名远端。**

以下命令执行前将 GITHUB_REPOSITORY 设置为最终 GitHub 仓库全名；如果使用 SSH，确认 ssh -T git@github.com 已通过；不要把 Personal Access Token 写进远端 URL。

~~~~powershell
git status --short
git remote -v
git remote rename origin private
$GitHubRepository = $env:GITHUB_REPOSITORY
$GitHubRemote = "git@github.com:$GitHubRepository.git"
git remote add origin $GitHubRemote
git fetch --all --prune
git switch main
git pull --ff-only private main
~~~~

如果 git status 有未提交内容、git pull --ff-only 不能快进或远端重命名后出现分支跟踪歧义，停止并处理原因，不使用 reset --hard 或强制推送绕过。

- [ ] **步骤 2：推送主线和标签前做 dry-run。**

~~~~powershell
git fsck --full
git push --dry-run origin main
git push --dry-run origin --tags
~~~~

预期：dry-run 只显示计划公开的 main 和标签，不显示内部 issue 分支或 artifacts/。

- [ ] **步骤 3：推送首次公开内容。**

~~~~powershell
git push -u origin main
git push origin --tags
~~~~

首次公开不执行 git push --all origin。每个要公开的开发分支都必须先过公开范围审计，然后执行显式的 git push -u origin <branch>。

- [ ] **步骤 4：同步私有备份并比对提交。**

~~~~powershell
git push private main
git push private --tags
git ls-remote origin refs/heads/main refs/tags/*
git ls-remote private refs/heads/main refs/tags/*
~~~~

预期：GitHub 和私有 Gitea 的 main 与发布标签对象一致；差异只能是明确保留在 Gitea 的内部分支和平台元数据。

### 任务 10：建立日常双远端和恢复流程

**文件：**

- 修改 HANDOFF.md、docs/development-guidelines.md。
- 新增 scripts/sync-private-mirror.ps1（如果实际备份需要脚本）。
- 新增或更新 docs/migration/github-mirror-recovery.md。

- [ ] **步骤 1：定义日常开发命令。**

~~~~powershell
git fetch origin --prune
git switch main
git pull --ff-only origin main
git switch -c feature/example
git push -u origin HEAD
~~~~

PR 合并后只从 GitHub 更新本地 main，再把同一个提交推到私有 Gitea：

~~~~powershell
git fetch origin main
git switch main
git pull --ff-only origin main
git push private origin/main:refs/heads/main
git push private --tags
~~~~

- [ ] **步骤 2：实现非破坏性备份脚本。**

脚本必须先执行 git fetch origin --prune，再把 refs/remotes/origin/main 显式推送到私有 refs/heads/main，最后同步标签；禁止使用未审查的 git push --mirror private，禁止在脚本中保存 Token、密码或内网登录信息。

- [ ] **步骤 3：安排备份频率和责任。**

每次主线合并后至少备份一次，每次发布标签后必须备份；如果没有常驻 runner，就由维护者在本地执行脚本并把命令输出记录到备份日志。备份失败不得阻塞 GitHub 开发，但必须在下一个发布前恢复备份健康状态。

- [ ] **步骤 4：执行恢复演练。**

从私有 Gitea 临时克隆到新目录，验证主线、标签、git fsck --full、README、许可证和 cargo test --workspace --all-features；再从 GitHub 做同样的干净克隆。两份克隆的主线提交哈希必须一致，恢复演练不修改生产仓库。

验收：新协作者只需要 GitHub；维护者能够在 GitHub 不可用时从私有 Gitea 恢复主线、标签和构建；双远端约定不会导致误推送到错误平台。

---

## 阶段 E：迁移 Issue、PR 和协作历史

### 任务 11：归档 Gitea Issue/PR 并建立 GitHub 映射

**文件：**

- 新增 docs/migration/github-issue-map.md，只有在确实迁移历史 Issue 时创建。
- 更新 README.md 或 HANDOFF.md 的迁移说明。
- GitHub 新建一个迁移说明 Issue，链接本计划和公开仓库。

- [ ] **步骤 1：导出 Gitea 当前状态。**

在私有 Gitea 仍可访问时，用 tea 获取开放 Issue、开放 PR、标签、里程碑和每个待迁移 Issue 的正文与状态。导出的文件只保存在受控本地目录，不直接提交；中文内容按仓库 PowerShell UTF-8 规则读回确认。

- [ ] **步骤 2：分类处理历史 Issue。**

将旧 Issue 分为“已完成历史”“仍有价值的公开需求”“只涉及私有硬件/内部讨论”“已被当前代码淘汰”。只有前两类进入 GitHub；私有内容不公开。新 GitHub Issue 使用新编号，在正文中写“来源：Gitea #旧编号”，不把内网 URL 或私有评论复制到公开正文。

- [ ] **步骤 3：处理开放 PR。**

已合入 PR 保留在 Gitea 作为历史；未合入但仍需要的工作，从对应提交或干净分支重新创建 GitHub PR，重新运行 GitHub Actions，不把旧 PR 的审查结论当成新 PR 的 CI 证据。无价值的开放 PR 在 Gitea 留下迁移说明后关闭。

- [ ] **步骤 4：处理旧编号自动链接风险。**

不重写历史提交。GitHub 可能把历史提交中的 #151 链接到新的 GitHub Issue 151；公开文档中提及旧工作时使用“Gitea #151”或完整上下文，避免把旧编号写成当前 GitHub Issue 引用。

- [ ] **步骤 5：完成平台切换通知。**

在 Gitea 仓库 README、置顶 Issue 和仍开放的相关 Issue 中说明：从指定切换时间起新 Issue/PR 只在 GitHub 创建；Gitea 仓库保留为只读代码备份和历史归档。公开 GitHub 仓库的迁移说明只引用 GitHub 可访问内容，不泄露私有 Gitea 地址。

验收：所有仍需继续的工作都有 GitHub Issue；公开协作者不需要访问私有 Gitea 才能开发；旧 Gitea Issue/PR 有明确归档策略，未发生两边同时推进同一工作的情况。

---

## 阶段 F：最终切换、验证与回滚

### 任务 12：执行切换窗口

**文件：**

- 最终修改长期文档和 GitHub 配置。
- Gitea 仓库设置为归档或只读（以平台权限能力为准）。

- [ ] **步骤 1：冻结私有 Gitea 写入。**

通知协作者在切换窗口内停止新 PR、Issue 和主线推送；保留必要的管理员备份权限。记录最后允许迁移的 Gitea 主线提交和标签。

- [ ] **步骤 2：完成最后一次审计和推送。**

重新运行密钥扫描、许可证检查、git fsck --full、git push --dry-run，确认 GitHub 仓库内容与迁移清单一致；然后推送 main、标签和明确允许的分支。

- [ ] **步骤 3：切换仓库可见性。**

在 GitHub Private 阶段完成页面、Actions、分支保护、模板、Release 草稿和安全项检查后，将仓库改为 Public。改为 Public 后立刻从未登录的干净浏览器或匿名 Git clone 检查 README、源码、Issue 模板、许可证和 Release 是否可访问。

- [ ] **步骤 4：设置 Gitea 为归档副本。**

保留代码、标签和恢复所需的备份分支；停止把 Gitea 作为新协作入口。若平台无法完全只读，至少通过 README、权限和维护规则阻止日常开发者误开 PR 或提交主线。

### 任务 13：执行最终验收

**自动化命令：**

~~~~powershell
git status --short --branch
git diff --check
git diff --cached --check
git fsck --full
git remote -v
git branch -vv
git ls-remote origin refs/heads/main
git ls-remote private refs/heads/main
cargo fmt --all --check
cargo test --workspace --all-features
.\scripts\verify.ps1
~~~~

**页面和权限检查：**

- [ ] GitHub 仓库为 Public，默认分支是 main，README、LICENSE 和项目主页链接正确。
- [ ] GitHub Issue/PR 模板可创建，正文不出现 Gitea、tea 或内网 URL。
- [ ] GitHub Actions 在 PR 和 main 推送上运行，required check 名称与分支保护一致。
- [ ] main 禁止直接推送和强制推送；PR 描述保留关联 Issue、测试证据、文档影响和人工验证例外。
- [ ] GitHub Secrets、Actions 权限、Release 权限最小化；仓库源码和历史没有凭据。
- [ ] GitHub 与私有 Gitea 的 main 和发布标签提交一致。
- [ ] 从 GitHub 和 Gitea 各做一次干净 clone，能读取源码、许可证和计划文档；恢复演练已记录。
- [ ] artifacts/、内部分支、私有资料和未许可 PDF/ZIP 没有误入公开仓库。
- [ ] Gitea #169 已通过包含 Closes #169 的 PR 合并并读回为 closed；若自动关闭未生效，使用 tea issues close --repo kxn/my_ipkvm 169 后读回确认。

**人工验证例外：** GitHub 可见性、分支保护、Actions 权限、Secrets、未登录页面和真实 Release 下载不能完全由本地测试替代，必须记录操作者、时间、步骤、预期结果和实际结果；代码、文档、Git 对象和双远端提交比对优先自动化完成。

### 任务 14：回滚条件和操作

满足任一条件时停止公开切换或回滚可见性，不继续向 Public 推送：

- 密钥扫描命中真实凭据，或无法确认历史是否含有凭据。
- 第三方资料再分发权无法确认，且尚未完成移除或许可隔离。
- GitHub 与私有 Gitea 的主线提交不一致，或发现误推送了内部分支/工件。
- GitHub Actions、分支保护或 Release 权限不能满足最小权限要求。
- 从干净 clone 无法构建、测试或恢复。

回滚顺序：暂停所有写入；将 GitHub 改回 Private；保留问题证据和最后安全提交；从私有 Gitea 恢复已审计主线到新的临时分支；修复公开范围或历史问题；重新执行全量审计。禁止用强制推送覆盖未确认的公开历史，除非安全事件处理已明确批准并完成凭据轮换。

---

## 计划自审清单

- [ ] 所有未来操作入口都从 Gitea/tea 替换为 GitHub/gh，且保留 Gitea 仅用于迁移前查询和备份确认。
- [ ] 当前已知内网 URL、Gitea 模板、运行时项目链接、Cargo repository、长期开发规则和 HANDOFF 入口均有处理任务。
- [ ] Git 历史、分支、标签、Issue、PR、CI、Release、Secrets、权限、备份和恢复各有独立任务。
- [ ] 公开前密钥扫描、许可证审查、未跟踪工件审查和公开范围审查都有停止条件。
- [ ] 计划没有要求整体 git push --all origin 或无条件 git push --mirror。
- [ ] 文档变更、代码变更、平台配置变更和人工验证的测试证据边界明确。
- [ ] 迁移计划提交使用英文 conventional commit 并包含 #169；Gitea PR 使用 Closes #169。
