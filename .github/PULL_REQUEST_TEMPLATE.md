# PR 说明

## 关联 issue

实现类 PR 请填写 `Closes #编号` 或等价关键字；仅引用、不完成对应工作的 PR 才填写 `Refs #编号`。

## 改动摘要

- 请填写主要改动。

## 根因或设计依据

请填写这次改动解决的根因、采用的设计判断，以及为什么不是补丁式绕过。

## 测试结果

请填写实际运行过的命令和结果。合并前请运行全量门禁：

```powershell
.\scripts\verify-full.ps1
```

开发迭代期间可先运行快速门禁 `.\scripts\verify.ps1`（Linux/macOS 对应 `./scripts/verify.sh`、`./scripts/verify-full.sh`）。涉及 noVNC/前端改动时另运行 `.\scripts\verify-browser.ps1`；涉及发布/桌面 UI 时另运行 `.\scripts\verify-desktop-release.ps1`。

## 人工验证例外

请填写无法自动化验证的内容、原因、步骤和结果。没有则写“无”。

## 文档影响

请填写已更新的文档，或写“无，原因：...”。

## 检查清单

- [ ] 自写文档为中文。
- [ ] 自动化测试已覆盖可自动化部分。
- [ ] 无法自动化的验证已说明原因和步骤。
- [ ] 修复从根因处理，没有用绕过代替真实修复。
- [ ] PR 已关联 issue。
