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
  - 合并 PR 并删除分支：`gh pr merge <PR编号> --repo kxn/MyKvm --squash --delete-branch`
  - 关闭 issue：`gh issue close <编号> --repo kxn/MyKvm`
  - 删除远端分支：`git push origin --delete <branch>`
  - 确认远端分支不存在：`git ls-remote --heads origin <branch>`
- 通过 `gh` 写入中文标题或正文时，用 `--body-file` 传 UTF-8 文件（不要用管道直接传中文，避免编码问题），并在写入后用 `gh issue view` / `gh pr view` 读回确认中文内容。

## 工作入口

- 非平凡改动必须围绕 GitHub issue 开发。issue 是工作单元，记录背景、目标、范围、验收标准、测试计划和讨论。
- 架构、协议、用户行为、开发流程、测试策略发生变化时，必须同步更新长期文档。
- 文档是长期事实来源；issue 是一次工作的过程记录。PR 负责把两者收口并链接起来。
- 非平凡改动默认从 `main` 创建带 issue 编号的分支，推送分支并通过 PR 合入；只有用户明确授权时才允许直接提交或推送 `main`。
- 实现类 PR 必须使用 `Closes #编号` 或等价关键字收口；只有不完成该工作的引用型 PR 才使用 `Refs #编号`。
- **大设计先调研、后开单**：对大型设计（新 UI/新子系统/协议扩展等），必须先完成深入
  技术调研并把调研结论写入设计文档（`docs/superpowers/specs/`），确认后再决定如何拆
  单。禁止在调研完成前按子 feature 预开一批 issue——按子 feature 预开容易丧失关联性，
  导致调研漏项或单子之间互相打架。调研结论至少覆盖：现状代码事实、约束与风险、方案
  取舍、依赖关系；据此拆出的前置单与主实施单必须在文档中列明关联。

## 测试要求

- 默认优先自动化测试。只有无法稳定自动化的场景才允许人工验证。
- 新增或修改核心逻辑时，先补能失败的测试，再实现，再确认测试通过。
- 手工验证必须写明无法自动化的原因、验证步骤、预期结果和后续是否可以自动化。
- 提交或声称完成前，至少运行与改动范围匹配的验证命令。Rust 代码改动默认运行：

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
```

## 工作完成与收口

非平凡改动只有完成以下收口步骤后才能声称完成：

1. 已有对应 GitHub issue，且 issue 记录了背景、目标、范围、验收标准、测试计划和文档影响。
2. 已按 TDD 要求补充失败测试（适用时），完成实现、回归测试和必要的人工验证。
3. 已创建英文 conventional commit，提交信息包含 `#编号`；不能只修改工作区而不提交。
4. 默认分支开发流程必须推送 issue 分支、创建 PR，并在 PR 描述中填写 `Closes #编号`、改动摘要、根因或设计依据、测试证据、文档影响和人工验证例外。
5. PR 合并必须默认使用 `gh pr merge <PR编号> --repo kxn/MyKvm --squash --delete-branch`；如果 GitHub 没有自动删除 head 分支，必须立即使用 `git push origin --delete <branch>` 删除。
6. PR 合并后必须确认 issue 已自动关闭；如果 issue 仍为 open，必须使用 `gh issue close <issue编号> --repo kxn/MyKvm` 关闭，并读回确认状态为 `closed`。
7. 工作分支删除后必须读回确认：使用 `git ls-remote --heads origin <branch>`、`gh api repos/kxn/MyKvm/branches` 或等价 GitHub 读回方式确认该分支不再存在。远端分支仍存在时，不能声称任务完成或收口完成。
8. 如果用户明确授权直接推送 `main`，不能创建 PR 的 `Closes` 收口不会自动生效；推送成功后必须手动关闭 issue，并读回确认 issue 状态；如果为该 issue 推送过远端工作分支，也必须按第 7 条删除并确认。
9. 关闭未合并 PR、放弃工作或清理旧单时，除非存在 open PR、open issue 或用户明确要求保留 WIP 分支，否则必须删除对应远端工作分支并读回确认；私有备份远端不纳入默认删除范围，除非用户明确要求。
10. 收口前必须核对本地 commit 与远端目标分支一致、PR/issue/分支状态正确、工作区没有误纳入的文件；必要时同步 `HANDOFF.md`、台账和长期文档。

没有完成上述步骤时，只能报告为“实现完成但尚未收口”，不能报告为任务完成。

## 修复原则

- 修改代码应从根因修复，禁止用绕过、吞错、固定延时、只改测试适配坏实现等补丁式修复代替真实修复。
- 修复 bug 时必须说明根因、失败路径、修复点和回归测试覆盖。
- 如果根因暂时无法完全修复，必须把临时限制写入 issue 或文档，并明确后续收敛条件。

## Git 与评审

- 不要回滚用户或其他协作者的未提交改动，除非用户明确要求。
- 提交信息使用简洁英文 conventional commit 风格，例如 `feat: ...`、`fix: ...`、`docs: ...`、`chore: ...`。
- PR 描述必须包含关联 issue、改动摘要、测试证据、文档影响和人工验证例外。
- GitHub issue、PR 和提交信息中使用 `#编号` 关联工作；实现类 PR 必须使用 `Closes #编号` 或等价关键字收口。
