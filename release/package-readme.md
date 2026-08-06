# MyKvm {version}

本包由 GitHub Actions 自动构建（commit `{commit}`，{date}），包含两个用户可用程序：

| 程序 | 说明 |
|---|---|
| `bin/ipkvm-headless` | 正式无头后台：RFB TCP（标准 VNC 客户端）+ 嵌入式 noVNC 网页与 RFB WebSocket（浏览器），单活动控制者连接闸门 |
| `bin/ipkvm-desktop-iced` | 桌面图形界面：设备选择、视频控制台、本地键鼠直通、特殊键/粘贴/截图 |

## 快速开始（headless）

```bash
# 首次运行先下载 Y4M 演示素材（需网络）
./scripts/fetch-demo-assets.sh

# 演示模式（无摄像头）
ipkvm-headless --assets .cache/demo-assets --tcp 5900 --http 6080 --fps 10

# Windows 相机模式（如 OBS 虚拟摄像头）
ipkvm-headless --camera "OBS Virtual Camera" --tcp 5900 --http 6080

# 相机 + 真实 CH9329 串口
ipkvm-headless --camera "OBS Virtual Camera" --tcp 5900 --http 6080 --serial COM9

# 只枚举设备
ipkvm-headless --list-cameras
```

启动后浏览器打开 `http://127.0.0.1:6080`，或 VNC 客户端连接 `127.0.0.1:5900`。

## 配置与安全

- 默认只监听 `127.0.0.1`（`--bind` 可改，暴露到公网前必须自行评估鉴权）。
- `--token` 设置管理 API/页面/WebSocket 的访问令牌，`--vnc-password` 设置 VNC 密码，`--config` 读取 TOML 配置文件（CLI 参数 > 文件 > 默认）。
- 键鼠输入与视频画面仅在本地网络链路传输；部署到不可信网络前请自行加 TLS 与鉴权。

## 第三方组件

依赖清单见同目录 `THIRD_PARTY_LICENSES.txt`（由 `cargo metadata` 自动生成）。
许可证与准入规则见 https://github.com/kxn/MyKvm/blob/main/docs/dependency-license-policy.md
嵌入式 noVNC 固定为 1.7.0（MPL-2.0，完整 npm 发布包与逐文件 SHA-256 清单见仓库
`third_party/novnc/`）。仓库根 LICENSE 为 MIT。

## dev 版说明

本包为 **dev 预发布**（固定 `dev` release，随 main 每次 CI 绿后覆盖更新），
仅用于开发测试，不代表正式版本质量。正式版本请使用仓库 Release 页面的
`v*` 版本。
