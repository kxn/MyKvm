# iced_aw 0.14.1 vendored 补丁说明

本目录是 crates.io `iced_aw 0.14.1`（MIT）的本地副本，通过根工作区
`[patch.crates-io]` 引入，只启用 `menu` feature。目的：在 iced 0.14 上使用
iced_aw 的现成菜单实现，同时规避上游未发版的已知缺陷。

## 补丁 1：嵌套子菜单树状态 bug（核心修复）

文件：`src/widget/menu/menu_tree.rs`

上游问题（iced-rs/iced_aw#408，本仓库 spike 2026-08-03 复现）：

- `Item::children()` 把菜单子树注释掉，只返回 widget 子树；
- v0.14.1 的 `Item::diff` 在子菜单树缺失时执行 `*tree = self.tree()`，
  把已推入的菜单子树整个丢掉；
- `MenuBarOverlay::operate` 对「带子菜单但未展开」的项无条件访问
  `tree.children[1]` → `len is 1 but the index is 1` 越界 panic。

触发条件：打开含「带子菜单但未展开项」的菜单后，任何 operate 遍历
（iced_test `find`/焦点/滚动）都会 panic。

修复（backport 自上游 main，0.15-dev 未发版）：

1. `MenuState::open_new_menu`：新建菜单树后先 `menu.diff(&mut menu_tree)`，
   让每个 item 子树初始化；
2. `Item::diff`：缺子树就补 `Tree::empty()`，有菜单就 diff `children[1]`，
   无菜单就截断到 1 个孩子，不再用 `*tree = self.tree()` 丢子树。

## 补丁 2：vendored 路径依赖下的图标宏适配

文件：`src/lib.rs`

`generate_icon_functions!` 宏在编译期用相对路径 `std::fs::read("font.ttf")`
读字体文件（相对 rustc 当前工作目录）。crates.io 的 registry 依赖编译时
工作目录恰好是包目录，能读到；**vendored 路径依赖编译时工作目录不是包目录**，
宏会 panic。

处理：把 `use iced_fonts::generate_icon_functions;` 和
`generate_icon_functions!(...)` 调用 gate 到真正使用图标字体的 feature
（`tab_bar`/`sidebar`/`card`/`number_input`/`date_picker`/`color_picker`/
`time_picker`）。本仓库只启用 `menu`，不编译该宏；`menu` 模块不引用
`iced_aw_font`。若未来启用上述任一 feature，需要恢复宏调用并保证编译
工作目录问题已解决。

## 补丁 3：独立 workspace 标记

文件：`Cargo.toml`

在末尾增加 `[workspace]`，配合根工作区 `exclude = ["third_party/iced_aw"]`，
避免嵌套 workspace 冲突（vendored 目录位于根工作区目录树内）。

## 升级与替换

- 升级到 iced_aw 0.15 正式版时：直接删除本目录，改回 crates.io 依赖，
  并重新评估补丁 1/2 是否已上游修复。
- 上游 main 分支当前依赖 iced 0.15-dev + rust-version 1.92，与本仓库
  （iced 0.14 + rust 1.89）不兼容，故不做 git 依赖。
