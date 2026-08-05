# noVNC 1.7.0 vendored 补丁说明

本目录的 `1.7.0/` 是 noVNC 1.7.0（MPL-2.0）官方 npm 发布包的 vendored 副本，
固定提交 `63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`，来源、逐文件 SHA-256 和
npm 元数据由 `manifest.sha256`、`npm-metadata.json` 和 `npm-attestations.json`
锁定。`scripts/verify-web-assets.ps1` 在每次验证时重新计算 `1.7.0/` 下每个文件
的 SHA-256 并与 `manifest.sha256` 比对，确保资源完整性。

上游 noVNC 1.7.0 不支持 RFB 相对指针消息（消息类型 0x08）和 pointer lock 下的
相对移动批处理，因此本仓库对 `1.7.0/core/rfb.js` 应用了本地补丁。补丁在源码中
以 `my_ipkvm local patch:` 注释逐处标注。

## 补丁内容（`core/rfb.js`）

1. **RFB 相对指针消息（0x08）发送路径**：新增相对位移累加器
   (`_relativeDeltaX`/`_relativeDeltaY`)、限流常量 `RELATIVE_MOVE_DELAY`
   和定时器，把 pointer lock 下的 `movementX`/`movementY` 增量按 RFB 0x08
   相对指针消息批处理发送，避免高频小位移淹没 CH9329 串口。
2. **pointer lock 下改走相对路径**：在指针锁定激活期间，把鼠标移动改为
   发送相对位移而非绝对坐标。
3. **teardown 清理**：`_disconnect()` 中调用 `_clearRelativeMoveState()`，
   保证挂起的相对移动定时器不会在 RFB 拆除后仍触发。
4. **暴露 canvas 与 screen 容器**：暴露 noVNC canvas 和 screen 容器引用，
   供上层应用做 pointer lock 进入/退出和光标隐藏处理。

这些改动使 `core/rfb.js` 的 SHA-256 偏离上游原始值；`manifest.sha256` 中
`core/rfb.js` 一行记录的是**打过本地补丁后**的哈希值（与本文件一同维护），
其余文件仍与上游 1.7.0 完全一致。

## 升级与替换

- 升级 noVNC 时：重新 vendor 指定版本，重算 `manifest.sha256`，并重新评估
  上述补丁是否已被上游采纳（上游若支持 0x08 相对指针则可移除本补丁）。
- 若修改了 `core/rfb.js`，必须同步更新 `manifest.sha256` 中对应行的哈希值，
  否则 `verify-web-assets.ps1` 会报完整性失配。任何 vendored 文件改动都
  必须在此文件记录原因。
