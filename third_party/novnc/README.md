# noVNC 第三方资源

本目录保存无头版网页前端使用的固定 noVNC 发布资源。

## 固定版本

- npm 包：`@novnc/novnc@1.7.0`
- 上游提交：`63107bd06d9e1f6136ff21aeda8cd62cbf0d433e`
- npm tarball：
  `https://registry.npmjs.org/@novnc/novnc/-/novnc-1.7.0.tgz`
- tarball SHA-256：
  `32689f18d6abe96bc6530828a6bd0b9ae33bda07c083a6575ed255b5a8f2e903`
- npm integrity：
  `sha512-ucEJOx4T2avIRCleodk7YobZj5O2Ga2AeLfQ69A/yjG9HHba2+PDgwSkN3FttrmG+70ZGx21sElNFouK13RzyA==`

`1.7.0/` 是 npm 发布包的完整、未修改内容。`manifest.sha256` 记录其中每个文件的
SHA-256。`npm-metadata.json` 保存固定版本元数据；`npm-attestations.json` 保存 npm
发布证明，后者只是人工审查参考，本项目当前不执行 Sigstore 签名验证。

## 许可证

- noVNC 核心 JavaScript：MPL-2.0。
- `vendor/pako`：MIT。
- `core/crypto/des.js`：文件内保留 BSD 风格声明。
- npm 包还原样附带上游列出的其他许可证文本。

项目不修改这些第三方文件，也不复制 noVNC 完整应用的图片、字体和主题资源。项目自有
HTML、CSS 和 JavaScript 放在 `crates/ipkvm-headless/web/`，不属于本目录。

## 更新和验证

显式更新：

```powershell
.\scripts\update-novnc.ps1
```

目标已经存在时，审查固定参数后使用：

```powershell
.\scripts\update-novnc.ps1 -Replace
```

离线验证：

```powershell
.\scripts\verify-web-assets.ps1
```

正常 Rust 构建不会下载 noVNC。升级版本必须先开 issue，重新审查运行依赖、许可证、
完整性和浏览器兼容性，再修改更新脚本中的固定参数。
