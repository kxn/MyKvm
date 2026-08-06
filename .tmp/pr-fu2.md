## 关联 issue

Refs #35（FU-2：CLI encoding 开关）

## 改动摘要

headless CLI + config 加 RFB 编码开关，用户可强制选编码（排障/优化）：
- `--encoding raw|tight|auto`（默认 auto——客户端支持 Tight 则自动走 Tight+JPEG）
- `--jpeg-quality 1-100`（默认 85，仅 tight/auto 时生效）
- config 文件 `[video]` 段加 `encoding` / `jpeg_quality` 字段
- `parse_encoding()` 辅助函数（String → EncodingPreference）
- main.rs 的 RfbConnectionSettings 传入 preferred_encoding + jpeg_quality

## 测试

`cargo fmt --all --check` 干净；`cargo test --workspace --all-features` 42 套件 704 测试 0 失败。

Refs #35
