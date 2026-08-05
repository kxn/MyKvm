# 自动化协作者规范

本文件适用于整个仓库。所有自动化协作者、代码生成工具和辅助开发会话都必须遵守。

## 语言

- 仓库内自写文档必须使用中文，包括设计文档、计划、审查记录、issue 模板、PR 模板和用户说明。
- 外部资料可以保留原文，但索引、摘要、采纳结论和使用说明必须使用中文。
- 代码标识符、协议字段、命令、文件路径和第三方专有名词按原文保留。

## Windows PowerShell 编码

在 Windows PowerShell 中处理中文文档、向 Python/其他子进程传递中文、或调用 GitHub API 写入中文内容前，必须显式设置 UTF-8 编码：

```powershell
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
```

读取仓库内 UTF-8 文档时，使用显式编码：

```powershell
Get-Content -Raw -Encoding UTF8 AGENTS.md
```

问题现象和根因：

- `Get-Content -Raw AGENTS.md` 可能把中文显示成乱码，但文件本身仍可能是正确的 UTF-8。
- PowerShell here-string 通过管道传给 Python 或其他子进程时，如果没有先设置 `InputEncoding`、`OutputEncoding` 和 `$OutputEncoding`，中文可能在进入子进程前被转成 `?`。
- 使用 `Invoke-RestMethod` 或脚本调用 GitHub API 写中文标题/正文时，必须确保 JSON 按 UTF-8 字节发送，并带 `Content-Type: application/json; charset=utf-8`。
- 写入外部系统后要读回确认中文内容，不要只相信本地命令的显示结果。

## GitHub 客户端（gh）

本仓库日常开发在 GitHub 公开仓库 `kxn/MyKvm`（https://github.com/kxn/MyKvm）进行，命令行交互统一使用 `gh` 客户端，并且已在本机登录，直接使用即可。私有 Gitea 仅保留为代码备份和灾难恢复副本，不再用于日常 Issue/PR。

- 已保存登录：GitHub 账号 `kxn`（通过 `gh auth status` 确认）。
- 远端约定：`origin` 指向 GitHub（`https://github.com/kxn/MyKvm.git`），`private` 指向私有 Gitea 备份。
- 常用命令：
  - 列出 issue：`gh issue list --repo kxn/MyKvm`
  - 查看 issue 详情：`gh issue view <编号> --repo kxn/MyKvm`
  - 创建 issue：`gh issue create --repo kxn/MyKvm --title "..." --body-file <UTF-8 文件>`
  - 列出 PR：`gh pr list --repo kxn/MyKvm`
  - 创建 PR：`gh pr create --repo kxn/MyKvm --base main --head <branch> --title "..." --body-file <UTF-8 文件>`
  - 合并 PR：`gh pr merge <PR编号> --repo kxn/MyKvm --squash`
- 通过 `gh` 写入中文标题或正文时，用 `--body-file` 传 UTF-8 文件，并在写入后读回确认中文内容。

## 工作入口

- 非平凡改动必须围绕 GitHub issue 开发。issue 是工作单元，记录背景、目标、范围、验收标准、测试计划和讨论。
- 架构、协议、用户行为、开发流程、测试策略发生变化时，必须同步更新长期文档。
- 文档是长期事实来源；issue 是一次工作的过程记录。PR 负责把两者收口并链接起来。

## 测试要求

- 默认优先自动化测试。只有无法稳定自动化的场景才允许人工验证。
- 新增或修改核心逻辑时，先补能失败的测试，再实现，再确认测试通过。
- 手工验证必须写明无法自动化的原因、验证步骤、预期结果和后续是否可以自动化。
- 提交或声称完成前，至少运行与改动范围匹配的验证命令。Rust 代码改动默认运行：

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
```

## 修复原则

- 修改代码应从根因修复，禁止用绕过、吞错、固定延时、只改测试适配坏实现等补丁式修复代替真实修复。
- 修复 bug 时必须说明根因、失败路径、修复点和回归测试覆盖。
- 如果根因暂时无法完全修复，必须把临时限制写入 issue 或文档，并明确后续收敛条件。

## Git 与评审

- 不要回滚用户或其他协作者的未提交改动，除非用户明确要求。
- 提交信息使用简洁英文 conventional commit 风格，例如 `feat: ...`、`fix: ...`、`docs: ...`、`chore: ...`。
- PR 描述必须包含关联 issue、改动摘要、测试证据、文档影响和人工验证例外。
- GitHub issue、PR 和提交信息中使用 `#编号` 关联工作；合并 PR 时可使用 `Closes #编号` 收口。
