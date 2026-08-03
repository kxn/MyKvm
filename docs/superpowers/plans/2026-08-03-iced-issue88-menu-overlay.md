# iced 菜单弹出层文字不渲染修复记录（#88）

- **日期**：2026-08-03
- **关联**：Gitea `kxn/my_ipkvm#88`；分支 `codex/issue88-menu-overlay-pixels`

## 根因

iced runtime 在 update 阶段用 overlay 实例 A 计算布局并只保存 layout；draw 阶段
新建实例 B 用 A 的 layout 重画。`MenuPopup` 的 `tree: Tree` 是每次新建的临时树，
B 的 text 段落从未 layout，draw 时拿到空段落 → 面板/悬停/分隔线正常、文字一个
像素都没有（离屏像素测试精确复现）。

## 修复点

`MenuPopup::draw` 不再委托 text widget 绘制，改为 `renderer.fill_text` 直接绘制
（iced 官方菜单同款）：

- 普通项：标签文本左对齐、垂直居中；
- 子菜单项：右侧补 `›` 箭头（右对齐、12px 右边距）；
- 分隔线绘制保持不变。

## 测试证据

- 离屏渲染回归测试 `crates/ipkvm-desktop-iced/tests/menu_overlay_pixels.rs`：
  - 修复前：弹出区域文字像素 = 0（FAIL）；
  - 修复后：弹出区域文字像素 > 0（PASS）。
- M2 菜单回归：menu_interact 8 项、corridor_hover 1 项、i18n_switch 4 项、
  modal_blocking 6 项全绿。
- 门禁：`cargo fmt --all --check` 通过；`cargo test --workspace --all-features`
  全绿（含新测试）。

## 附加构建修复（workspace 门禁前提）

`ipkvm-desktop-iced` 与 `ipkvm-desktop-iced-spike` 的 example 同名 `video_1080p`，
重编译时链接器竞争同一输出文件导致 LNK1104。将 spike example 改名为
`video_1080p_spike`（spike 为临时 crate，M5 将删除），并同步其 perf 脚本。

## 人工验证（待用户）

- 真机截图确认菜单弹出层文字可见、箭头/分隔线/悬停高亮正常。
