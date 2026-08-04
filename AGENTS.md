# 自动化协作者规范

本文件适用于整个仓库。所有自动化协作者、代码生成工具和辅助开发会话都必须遵守。

## 语言

- 仓库内自写文档必须使用中文，包括设计文档、计划、审查记录、issue 模板、PR 模板和用户说明。
- 外部资料可以保留原文，但索引、摘要、采纳结论和使用说明必须使用中文。
- 代码标识符、协议字段、命令、文件路径和第三方专有名词按原文保留。

## Windows PowerShell 编码

在 Windows PowerShell 中处理中文文档、向 Python/其他子进程传递中文、或调用 Gitea API 写入中文内容前，必须显式设置 UTF-8 编码：

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
- 使用 `Invoke-RestMethod` 或脚本调用 Gitea API 写中文标题/正文时，必须确保 JSON 按 UTF-8 字节发送，并带 `Content-Type: application/json; charset=utf-8`。
- 写入外部系统后要读回确认中文内容，不要只相信本地命令的显示结果。

## Gitea 客户端（tea）

本仓库通过 Gitea 管理，命令行交互统一使用 `tea` 客户端，并且已在本机登录，直接使用即可：

- 已保存登录：登录名 `srpg`，服务器 `http://10.10.10.5:3000`，用户 `kxn`，仓库 `kxn/my_ipkvm`。
- 非交互模式提示 `no gitea login detected, falling back to login 'srpg'` 是回退到已保存登录的正常提示，不是错误。
- 常用命令（推荐显式传 `--repo kxn/my_ipkvm`）：
  - 列出 issue：`tea issues list --repo kxn/my_ipkvm`
  - 查看 issue 详情：`tea issues 18 --repo kxn/my_ipkvm`
  - 创建 issue：`tea issues create --repo kxn/my_ipkvm --title "..." --description "..."`
  - 列出 PR：`tea pulls list --repo kxn/my_ipkvm`
  - 创建 PR：`tea pulls create --repo kxn/my_ipkvm --base main --head <branch> --title "..." --description "..."`
  - 合并 PR：`tea pulls merge --repo kxn/my_ipkvm <PR编号>`
- 通过 `tea` 写入中文标题或正文前，先按上文设置 UTF-8 编码，并在写入后读回确认中文内容。

## 工作入口

- 非平凡改动必须围绕 Gitea issue 开发。issue 是工作单元，记录背景、目标、范围、验收标准、测试计划和讨论。
- 架构、协议、用户行为、开发流程、测试策略发生变化时，必须同步更新长期文档。
- 文档是长期事实来源；issue 是一次工作的过程记录。PR 负责把两者收口并链接起来。
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

## 修复原则

- 修改代码应从根因修复，禁止用绕过、吞错、固定延时、只改测试适配坏实现等补丁式修复代替真实修复。
- 修复 bug 时必须说明根因、失败路径、修复点和回归测试覆盖。
- 如果根因暂时无法完全修复，必须把临时限制写入 issue 或文档，并明确后续收敛条件。

## Git 与评审

- 不要回滚用户或其他协作者的未提交改动，除非用户明确要求。
- 提交信息使用简洁英文 conventional commit 风格，例如 `feat: ...`、`fix: ...`、`docs: ...`、`chore: ...`。
- PR 描述必须包含关联 issue、改动摘要、测试证据、文档影响和人工验证例外。
- Gitea issue、PR 和提交信息中使用 `#编号` 关联工作；合并 PR 时可使用 `Closes #编号` 收口。
