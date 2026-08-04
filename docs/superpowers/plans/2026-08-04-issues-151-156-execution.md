# #151/#152/#153/#154/#156 联合执行计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans. 本计划按用户要求在一个连续工作流内执行，内部阶段不作为中间交付停点。

**Goal:** 先完成五单的依赖调研和逐单计划，再按依赖顺序实现、验证、推送、合并，并确认五个 issue 全部关闭。

**Architecture:** #151 建立共享 MouseProfile；#152 修复 profile 应用后的 control probe；#156 统一 Iced/Web 鼠标调度；#154 在稳定调度上分离本地捕获；#153 最后统一 iced 外观和布局。所有阶段使用同一分支和一份最终 PR，PR 描述使用 `Closes #151`、`Closes #152`、`Closes #153`、`Closes #154`、`Closes #156`。

## 顺序

1. #151：共享 profile、桌面/Web 配置/API、iced/Web 选择器。
2. #152：profile 加载后的控制设备自动探测。
3. #156：Iced/Web 移动合并、控制事件 flush、键盘独立路径。
4. #154：desktop 视频矩形 ClipCursor、Web Pointer Lock/软降级。
5. #153：最终视觉令牌、弹窗尺寸、两列表单和中英文/窄窗口 QA。

## 可并行工作包

- 调研阶段：五单 issue/文档/代码读取可并行；已在联合调研文档中合并结论。
- #151 内部：core profile 纯模型测试与 Web/iced UI 资源盘点可并行，接线按模型先后。
- #156 内部：Iced `DeltaSampler` 测试与 noVNC timer 设计可并行，合并接线必须顺序化。
- #154 内部：Windows rect/state 单测与 Web Pointer Lock state 单测可并行，接线等待 #156。
- 不能并行：#151/#153 UI 接线、#156/#154 输入接线、#152/#153 app/profile 接线。

## 每阶段出口

- 每个 issue 先补失败测试，再实现，再运行对应 crate/browser 测试。
- 任何行为或 API 变化同步长期文档；每阶段保留可回滚的提交边界。
- 阶段通过后继续下一阶段，不创建中间 PR，不暂停等待确认。

## 最终验证

```powershell
cargo fmt --all --check
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo doc --workspace --no-deps
cargo metadata --format-version 1 --no-deps
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-iced-m5-retirement.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/test-crate-boundaries.ps1
node browser-tests/novnc-browser.mjs
cargo build --release -p ipkvm-desktop-iced --bin ipkvm-desktop-iced
cargo build --release -p ipkvm-headless-app --bin ipkvm-headless
```

真实 Windows 光标裁剪、Pointer Lock、CH9329 和目标端 OS 输入栈写入 PR 人工验证例外，
不伪装成自动化通过。

