# headless 配置与最小鉴权 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** ipkvm-headless 支持 `--config <路径>` 读取 TOML 配置（CLI 覆盖文件字段），并落地最小鉴权：`[auth] token` 配置时 HTTP/WS 走 Bearer/cookie/query token、`[auth] vnc_password` 配置时 RFB TCP/WS 走 VNC 密码挑战；对应凭证未配置时该通道拒绝非 127.0.0.1 来源。

**Architecture:** 配置层（headless 新 `config` 模块）产出最终 `Options`（CLI > 文件 > 默认）；凭证按通道注入——`token` 进 HTTP 的 axum 统一中间件（Bearer/cookie/query），`vnc_password` 进 RFB 传输配置，在协议握手层用 VNC 密码挑战（security type 2，扩展 ipkvm-rfb 的 `RfbConnectionCore` 状态机），TCP 入口在 accept 后做本机来源检查。协议核心（`RfbConnectionCore`）是 TCP/WS 共用路径，VNC 密码只需扩展一处。

**Tech Stack:** Rust 2024、axum 0.8 middleware、toml（serde 解析）、des 0.8 + cipher 0.4（RustCrypto，VNC 密码 DES-ECB）、rand 0.9（challenge 随机源）。新增依赖全部在许可证全局允许列表（MIT/Apache-2.0），无需例外流程。

## Global Constraints

- **合并优先级**：CLI 参数 > 配置文件字段 > 默认值（bind=127.0.0.1、tcp=5900、http=6080、fps=10、baud=9600）。
- **token 限制**：`[auth] token`（HTTP/WS 凭证）必须为非空 ASCII 字符串；违反时给出确定性中文报错并说明原因。
- **VNC 密码限制**：`[auth] vnc_password`（RFB 凭证）必须为 1-8 个 ASCII 字符（RFC 6143 密码上限 8 字节）；违反时给出确定性中文报错并说明原因。
- **配置错误**：文件不存在、TOML 语法/类型/未知字段错误、camera+assets 互斥冲突，全部给出确定性中文报错（含文件路径）。
- **鉴权统一入口**：HTTP 全部路由一个中间件；RFB TCP/WS 在握手层（`RfbConnectionCore`）统一；TCP 本机检查在 `RfbTcpServer::run` 的 accept 循环。禁止在每个路由散落校验。
- **鉴权语义**：每通道独立凭证——HTTP/WS 用 `token`（Bearer/cookie/query），RFB TCP/WS 握手用 `vnc_password`（VNC 密码挑战）。某凭证未配置时该通道回退仅 127.0.0.1 来源放行（HTTP 403 / TCP 直接关闭）；配置后来源不再限制，凭证无效时 401（HTTP）/握手失败（RFB）。
- **不做**（后续 issue）：HTTP 管理 API（#32）、TLS/证书/RFB 加密、多用户、动态 token 刷新、配置文件热加载、环境变量覆盖。
- **安全实现**：DES 必须用 `des` crate（禁止自研加密原语）；challenge 必须用 CSPRNG（`rand::rng()`）；响应比较必须恒定时间（禁止普通 `==`）。
- **依赖纪律**：新增依赖（toml、des、cipher、rand）只加在 `Cargo.toml` 的 `[workspace.dependencies]`，各 crate 用 `X.workspace = true` 引用；许可证必须落在全局允许列表。
- **验证门禁**：`cargo fmt --all --check`、`cargo test --workspace --all-features`、`cargo clippy --workspace --all-targets --all-features -- -D warnings` 全部通过；提交用英文 conventional commit。
- **兼容性**：未配置 token 时现有行为完全不变（现有 347 个测试全部保持通过）；`HeadlessConfig`（lib.rs）本轮不动。
- **文档语言**：仓库内自写文档（README、设计文档、PR 描述）用中文；代码标识符、协议字段、命令按原文。

---

### Task 1: ipkvm-rfb VNC 安全类型（密码挑战握手）

**Files:**
- Modify: `Cargo.toml`（workspace.dependencies 加 `des`、`cipher`、`rand`）
- Modify: `crates/ipkvm-rfb/Cargo.toml`（dependencies 加 des/cipher/rand）
- Create: `crates/ipkvm-rfb/src/security.rs`
- Modify: `crates/ipkvm-rfb/src/lib.rs`（`mod security` + `pub use`）
- Modify: `crates/ipkvm-rfb/src/protocol/client.rs`（`RfbProtocolError::AuthenticationFailed`）
- Modify: `crates/ipkvm-rfb/src/protocol/server.rs`（`VNC_SECURITY_TYPES`、`SECURITY_RESULT_FAILED` 常量）
- Modify: `crates/ipkvm-rfb/src/connection.rs`（`RfbConnectionConfig.security`、状态机扩展）

**Interfaces:**
- Produces: `RfbSecurity`（`ipkvm_rfb::RfbSecurity`）：`enum RfbSecurity { None, Vnc { password: [u8; 8] } }`（Clone/Debug/Eq/PartialEq）——T2 的 `RfbConnectionSettings` 与 T4/T6 的 headless 配置层使用。
- Produces: `RfbConnectionConfig.security: RfbSecurity`——`RfbConnectionCore::new` 按它选择安全类型。
- Produces: `RfbProtocolError::AuthenticationFailed`——T2 映射断连原因。

- [ ] **Step 1: workspace 依赖与 rfb 依赖**

修改根 `Cargo.toml` 的 `[workspace.dependencies]`，追加：

```toml
cipher = "0.4"
des = "0.8"
rand = "0.9"
toml = "0.9"
```

（`toml` 本轮最后才用——T3；des/cipher/rand 本任务即用。许可证：`des`/`cipher`/`rand`/`toml` 均为 MIT OR Apache-2.0，在 `deny.toml` 全局允许列表内。）

修改 `crates/ipkvm-rfb/Cargo.toml` 的 `[dependencies]`：

```toml
cipher.workspace = true
des.workspace = true
rand.workspace = true
thiserror.workspace = true
```

- [ ] **Step 2: 写失败测试（security.rs 单元测试）**

创建 `crates/ipkvm-rfb/src/security.rs`（先写测试与函数骨架，`RfbSecurity` 为暂未引用的定义）：

```rust
//! RFB 安全类型与 VNC 密码挑战（RFC 6143 §7.2）。
//!
//! security type 2（VNC Authentication）：服务器发 16 字节随机 challenge，
//! 客户端用密码派生的 DES 密钥做 ECB 加密（8 字节一块）返回 16 字节响应，
//! 服务器校验后发 SecurityResult。DES 必须用 `des` crate，禁止自研加密原语。

use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
use des::Des;

/// RFB 安全类型配置。`None` 表示匿名连接（security type 1），`Vnc` 表示
/// VNC 密码挑战（security type 2）。密码固定为 8 字节（RFC 6143 上限）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RfbSecurity {
    None,
    Vnc { password: [u8; 8] },
}

/// VNC 密码派生 DES 密钥：密码字节反转、截断/补零到 8 字节、每字节保留
/// 低 7 位（RFC 6143 §7.2.2，客户端与服务器对称派生）。
pub(crate) fn vnc_key(password: [u8; 8]) -> [u8; 8] {
    let mut key = [0_u8; 8];
    for (index, byte) in password.into_iter().rev().enumerate().take(8) {
        key[index] = byte & 0x7F;
    }
    key
}

/// 用 VNC 密码对 16 字节 challenge 做 DES-ECB 加密（两块），返回期望的
/// 客户端响应。服务器侧校验用：与收到的响应做恒定时间比较。
pub(crate) fn vnc_expected_response(password: [u8; 8], challenge: [u8; 16]) -> [u8; 16] {
    let cipher = Des::new_from_slice(&vnc_key(password)).expect("8 字节 DES 密钥恒有效");
    let mut encrypted = challenge;
    for chunk in encrypted.chunks_exact_mut(8) {
        cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
    }
    encrypted
}

/// 恒定时间比较两个 16 字节数组（VNC 响应校验，防时序侧信道）。
/// 长度固定所以没有长度泄漏；逐字节异或累积，不提前返回。
pub(crate) fn constant_time_eq(left: &[u8; 16], right: &[u8; 16]) -> bool {
    let mut diff = 0_u8;
    for (a, b) in left.iter().zip(right) {
        diff |= a ^ b;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vnc_key_reverses_pads_and_masks_seven_bits() {
        // "1234" → 反转 "4321" → 补零到 8 → 每字节 & 0x7F（ASCII 本来低 7 位）。
        assert_eq!(vnc_key(*b"1234\0\0\0\0"), [0x34, 0x33, 0x32, 0x31, 0, 0, 0, 0]);
        // 高位字节被掩码清掉：0x80 → 0。
        assert_eq!(vnc_key([0x80, 0x41, 0, 0, 0, 0, 0, 0]), [0, 0x41, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn vnc_expected_response_matches_des_ecb_construction() {
        // 已知密钥/明文的 DES-ECB 标准向量（NIST FIPS 46-3 附录），验证
        // 调用方式正确：key=133457799BBCDFF1 加密 0123456789ABCDEF。
        let cipher = Des::new_from_slice(&[0x13, 0x34, 0x57, 0x79, 0x9b, 0xbc, 0xdf, 0xf1])
            .unwrap();
        let mut block = *b"0123456789ABCDEF";
        cipher.encrypt_block(GenericArray::from_mut_slice(&mut block));
        assert_eq!(block, [0x85, 0xe8, 0x13, 0x54, 0x0f, 0x0a, 0xb4, 0x05]);
    }

    #[test]
    fn constant_time_eq_detects_any_difference() {
        let left = [1_u8; 16];
        assert!(constant_time_eq(&left, &[1_u8; 16]));
        for position in 0..16 {
            let mut right = [1_u8; 16];
            right[position] = 2;
            assert!(!constant_time_eq(&left, &right));
        }
    }

    #[test]
    fn vnc_expected_response_is_deterministic() {
        let password = *b"abc12345";
        let challenge = [7_u8; 16];
        assert_eq!(
            vnc_expected_response(password, challenge),
            vnc_expected_response(password, challenge)
        );
    }
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p ipkvm-rfb security`
Expected: 编译错误（`RfbSecurity` 未导出或模块未声明）——测试尚未接入。

- [ ] **Step 4: lib.rs 声明模块并导出**

修改 `crates/ipkvm-rfb/src/lib.rs`：

```rust
mod connection;
mod framebuffer;
mod protocol;
mod security;

pub use connection::{
    FramebufferUpdateOutcome, RfbConfigError, RfbConnectionConfig, RfbConnectionCore,
    RfbConnectionState, RfbEncodeError, RfbEvent,
};
pub use framebuffer::{BgraFrameView, RfbFramebufferError, RfbRectangle, RfbSize};
pub use protocol::client::{FramebufferUpdateRequest, RfbProtocolError};
pub use protocol::pixel_format::{RfbColorChannel, RfbPixelFormat, RfbPixelFormatError};
pub use security::RfbSecurity;
```

- [ ] **Step 5: 写失败测试（连接状态机：VNC 握手）**

在 `crates/ipkvm-rfb/src/connection.rs` 的 `mod tests` 内追加（config 构造辅助改为带 security 参数）：

```rust
    fn config_with_security(security: RfbSecurity) -> RfbConnectionConfig {
        RfbConnectionConfig {
            desktop_name: "my_ipkvm".to_owned(),
            initial_size: RfbSize::new(640, 480).unwrap(),
            limits: RfbProtocolLimits::default(),
            security,
        }
    }

    fn vnc_config() -> RfbConnectionConfig {
        config_with_security(RfbSecurity::Vnc { password: *b"12345678" })
    }

    /// 完整 VNC 密码握手：读 challenge 并生成响应（协议测试里直接用产品
    /// 实现生成响应是自证——这里特意用 des crate + 内联派生交叉验证）。
    fn vnc_response(password: &[u8; 8], challenge: &[u8; 16]) -> [u8; 16] {
        use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
        let mut key = [0_u8; 8];
        for (index, byte) in password.into_iter().rev().copied().enumerate() {
            key[index] = byte & 0x7F;
        }
        let cipher = des::Des::new_from_slice(&key).unwrap();
        let mut response = *challenge;
        for chunk in response.chunks_exact_mut(8) {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        response
    }

    #[test]
    fn vnc_handshake_succeeds_with_correct_password() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        assert_eq!(connection.take_output(), b"RFB 003.008\n");

        assert!(connection.push_input(b"RFB 003.008\n").is_empty());
        assert_eq!(connection.state(), RfbConnectionState::AwaitingSecuritySelection);
        // 安全类型列表只提供 VNC（一个类型：2）。
        assert_eq!(connection.take_output(), [1, 2]);

        assert!(connection.push_input(&[2]).is_empty());
        assert_eq!(connection.state(), RfbConnectionState::AwaitingChallengeResponse);
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();

        let response = vnc_response(b"12345678", &challenge);
        assert!(connection.push_input(&response).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);

        assert_eq!(
            connection.push_input(&[1]),
            vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
        );
        assert_eq!(connection.state(), RfbConnectionState::Normal);
        assert!(!connection.take_output().is_empty());
    }

    #[test]
    fn vnc_handshake_rejects_wrong_password() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        connection.take_output();
        connection.push_input(b"RFB 003.008\n");
        connection.take_output();
        connection.push_input(&[2]);
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();

        let wrong = vnc_response(b"wrongpas", &challenge);
        assert_eq!(
            connection.push_input(&wrong),
            vec![Err(RfbProtocolError::AuthenticationFailed)]
        );
        assert_eq!(connection.state(), RfbConnectionState::Failed);
        // 失败响应：SecurityResultFailed + 原因字符串（RFC 6143 §7.2.3）。
        let output = connection.take_output();
        let mut expected = vec![0, 0, 0, 1];
        expected.extend_from_slice(&(18_u32).to_be_bytes());
        expected.extend_from_slice(b"authentication failed");
        assert_eq!(output, expected);
        assert_eq!(
            connection.push_input(&[1]),
            vec![Err(RfbProtocolError::ConnectionFailed)]
        );
    }

    #[test]
    fn vnc_response_accepted_across_arbitrary_chunks() {
        let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
        connection.take_output();
        connection.push_input(b"RFB 003.008\n");
        connection.take_output();
        connection.push_input(&[2]);
        let challenge: [u8; 16] = connection.take_output().try_into().unwrap();
        let response = vnc_response(b"12345678", &challenge);

        // 7 + 9 字节分块到达。
        assert!(connection.push_input(&response[..7]).is_empty());
        assert!(connection.push_input(&response[7..]).is_empty());
        assert_eq!(connection.take_output(), [0, 0, 0, 0]);
        assert_eq!(connection.state(), RfbConnectionState::AwaitingClientInit);
    }

    #[test]
    fn vnc_challenge_is_random_across_connections() {
        // 两个 core 的 challenge 必须不同（CSPRNG）。相同概率 2^-128，
        // 可忽略；该断言防「challenge 硬编码」回归。
        let first: [u8; 16] = {
            let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
            connection.take_output();
            connection.push_input(b"RFB 003.008\n");
            connection.take_output();
            connection.push_input(&[2]);
            connection.take_output().try_into().unwrap()
        };
        let second: [u8; 16] = {
            let mut connection = RfbConnectionCore::new(vnc_config()).unwrap();
            connection.take_output();
            connection.push_input(b"RFB 003.008\n");
            connection.take_output();
            connection.push_input(&[2]);
            connection.take_output().try_into().unwrap()
        };
        assert_ne!(first, second);
    }

    #[test]
    fn vnc_mode_rejects_none_selection_and_none_mode_rejects_vnc() {
        let mut vnc = RfbConnectionCore::new(vnc_config()).unwrap();
        vnc.take_output();
        vnc.push_input(b"RFB 003.008\n");
        vnc.take_output();
        assert_eq!(
            vnc.push_input(&[1]),
            vec![Err(RfbProtocolError::UnsupportedSecurityType(1))]
        );

        let mut none = RfbConnectionCore::new(config_with_security(RfbSecurity::None)).unwrap();
        none.take_output();
        none.push_input(b"RFB 003.008\n");
        none.take_output();
        assert_eq!(
            none.push_input(&[2]),
            vec![Err(RfbProtocolError::UnsupportedSecurityType(2))]
        );
    }
```

同时把现有 `config()` 辅助改为 `config_with_security(RfbSecurity::None)`（返回类型不变），并让所有现有测试改用 `config()`（它们不受影响——None 行为完全不变）。

- [ ] **Step 6: 运行测试确认失败**

Run: `cargo test -p ipkvm-rfb`
Expected: 编译错误（`security` 字段不存在、`AwaitingChallengeResponse` 不存在、`AuthenticationFailed` 不存在）。

- [ ] **Step 7: 协议常量**

在 `crates/ipkvm-rfb/src/protocol/server.rs` 顶部追加：

```rust
pub(crate) const VNC_SECURITY_TYPES: [u8; 2] = [1, 2];
pub(crate) const SECURITY_RESULT_FAILED: [u8; 4] = [0, 0, 0, 1];
pub(crate) const AUTH_FAILED_REASON: &[u8] = b"authentication failed";
```

在 `handshake_constants_match_rfb_38_none_security` 测试中追加断言：

```rust
        assert_eq!(VNC_SECURITY_TYPES, [1, 2]);
        assert_eq!(SECURITY_RESULT_FAILED, [0, 0, 0, 1]);
```

- [ ] **Step 8: RfbProtocolError 加变体**

在 `crates/ipkvm-rfb/src/protocol/client.rs` 的 `RfbProtocolError` enum 中追加变体（与现有 `#[error(...)]` 风格一致）：

```rust
    #[error("VNC password authentication failed")]
    AuthenticationFailed,
```

- [ ] **Step 9: 连接状态机扩展**

修改 `crates/ipkvm-rfb/src/connection.rs`：

1. `RfbConnectionConfig` 加字段：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionConfig {
    pub desktop_name: String,
    pub initial_size: RfbSize,
    pub limits: RfbProtocolLimits,
    pub security: RfbSecurity,
}
```

2. `RfbConnectionState` 加变体：

```rust
pub enum RfbConnectionState {
    AwaitingVersion,
    AwaitingSecuritySelection,
    AwaitingChallengeResponse,
    AwaitingClientInit,
    Normal,
    Failed,
}
```

3. `RfbConnectionCore` 加字段 `vnc_challenge: [u8; 16]`，`new()` 里生成：

```rust
        let mut vnc_challenge = [0_u8; 16];
        rand::RngCore::fill_bytes(&mut rand::rng(), &mut vnc_challenge);
```

（`rand::rng()` 返回 CSPRNG 线程随机源；`RngCore` 全限定调用无需 use。无失败路径。）

4. `new()` 的 struct 初始化加 `vnc_challenge`。

5. `AwaitingVersion` 分支的安全类型列表按配置选择：

```rust
                    self.output.extend_from_slice(match &self.config.security {
                        RfbSecurity::None => &NONE_SECURITY_TYPES,
                        RfbSecurity::Vnc { .. } => &VNC_SECURITY_TYPES,
                    });
```

6. `AwaitingSecuritySelection` 分支按配置接受 1（None）/2（Vnc）：

```rust
                RfbConnectionState::AwaitingSecuritySelection => {
                    let Some(selection) = self.handshake_input.first().copied() else {
                        break;
                    };
                    self.handshake_input.drain(..1);
                    match (selection, &self.config.security) {
                        (1, RfbSecurity::None) => {
                            self.output.extend_from_slice(&SECURITY_RESULT_OK);
                            self.state = RfbConnectionState::AwaitingClientInit;
                        }
                        (2, RfbSecurity::Vnc { .. }) => {
                            self.output.extend_from_slice(&self.vnc_challenge);
                            self.state = RfbConnectionState::AwaitingChallengeResponse;
                        }
                        _ => {
                            results.extend(
                                self.fail(RfbProtocolError::UnsupportedSecurityType(selection)),
                            );
                            break;
                        }
                    }
                }
                RfbConnectionState::AwaitingChallengeResponse => {
                    if self.handshake_input.len() < 16 {
                        break;
                    }
                    let mut response = [0_u8; 16];
                    response.copy_from_slice(&self.handshake_input[..16]);
                    self.handshake_input.drain(..16);
                    let RfbSecurity::Vnc { password } = self.config.security else {
                        unreachable!("AwaitingChallengeResponse 只在 Vnc 配置下进入");
                    };
                    let expected = vnc_expected_response(password, self.vnc_challenge);
                    if constant_time_eq(&expected, &response) {
                        self.output.extend_from_slice(&SECURITY_RESULT_OK);
                        self.state = RfbConnectionState::AwaitingClientInit;
                    } else {
                        // RFC 6143 §7.2.3：失败发 SecurityResultFailed + 原因，
                        // 随后连接关闭（状态置 Failed）。
                        let reason = AUTH_FAILED_REASON;
                        self.output.extend_from_slice(&SECURITY_RESULT_FAILED);
                        self.output.extend_from_slice(&(reason.len() as u32).to_be_bytes());
                        self.output.extend_from_slice(reason);
                        results.extend(self.fail(RfbProtocolError::AuthenticationFailed));
                        break;
                    }
                }
```

7. 模块顶部 `use` 追加：

```rust
use crate::security::{RfbSecurity, constant_time_eq, vnc_expected_response};
use crate::protocol::server::{
    AUTH_FAILED_REASON, NONE_SECURITY_TYPES, PROTOCOL_VERSION, SECURITY_RESULT_FAILED,
    SECURITY_RESULT_OK, VNC_SECURITY_TYPES, checked_output_len, encode_desktop_size_update,
    encode_empty_update, encode_raw_update, encode_server_init,
};
```

8. 更新现有测试构造点：`config()` 改为 `config_with_security(RfbSecurity::None)`；`config_with_size` 同样加 `security: RfbSecurity::None`；`limited`/`no_timeout` 等构造点（`RfbConnectionConfig { ... }` 字面量）加 `security` 字段。运行 `cargo test -p ipkvm-rfb` 修正所有编译错误（约 6 处）。

- [ ] **Step 10: 运行全部测试**

Run: `cargo test -p ipkvm-rfb`
Expected: 全部通过（现有 None 握手测试不变 + 新增 7 个 VNC 测试）。

- [ ] **Step 11: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy -p ipkvm-rfb --all-targets -- -D warnings
cargo test --workspace --all-features
git add Cargo.toml Cargo.lock crates/ipkvm-rfb/
git commit -m "feat(rfb): VNC password challenge handshake (security type 2)"
```

Expected: 全绿（现有 347 个测试 + 新增）。

---

### Task 2: session 传递安全配置与断连原因

**Files:**
- Modify: `Cargo.toml`（workspace.dependencies 已含 des——本任务不需要新增）
- Modify: `crates/ipkvm-session/Cargo.toml`（dev-dependencies 加 des）
- Modify: `crates/ipkvm-session/src/rfb_connection/mod.rs`（`RfbConnectionSettings.security`、`RfbDisconnectReason::AuthenticationFailed`）
- Modify: `crates/ipkvm-session/src/rfb_connection/driver.rs`（core config 传 security、reason 映射、VNC 流程测试）

**Interfaces:**
- Consumes: `ipkvm_rfb::RfbSecurity`（T1）、`RfbProtocolError::AuthenticationFailed`（T1）。
- Produces: `RfbConnectionSettings.security: RfbSecurity`（默认 `RfbSecurity::None`）——T4/T6 的 headless 两个传输 config（`RfbTcpConfig.connection`、`RfbWebSocketConfig.connection`）通过它注入密码。
- Produces: `RfbDisconnectReason::AuthenticationFailed`——事件流的 `Disconnected { reason }` 携带。

- [ ] **Step 1: 写失败测试**

在 `crates/ipkvm-session/Cargo.toml` 的 `[dev-dependencies]` 追加：

```toml
des.workspace = true
```

在 `crates/ipkvm-session/src/rfb_connection/mod.rs` 的测试模块追加：

```rust
    #[test]
    fn default_connection_settings_use_none_security() {
        let settings = RfbConnectionSettings::default();
        assert_eq!(settings.security, ipkvm_rfb::RfbSecurity::None);
    }

    #[test]
    fn vnc_security_is_derivable_from_connection_settings() {
        let settings = RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc { password: *b"secret12" },
            ..RfbConnectionSettings::default()
        };
        assert_eq!(
            settings.security,
            ipkvm_rfb::RfbSecurity::Vnc { password: *b"secret12" }
        );
    }
```

在 `crates/ipkvm-session/src/rfb_connection/driver.rs` 的测试模块追加（真实 TCP 的 VNC 握手流程；测试夹具内联独立实现密钥派生与加密，交叉验证产品代码）：

```rust
    /// 独立于产品实现的 VNC 响应复刻（RFC 6143 §7.2）——用测试自己的
    /// 派生与 des 加密，避免产品函数自证。
    fn vnc_response(password: &[u8; 8], challenge: &[u8; 16]) -> [u8; 16] {
        use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
        let mut key = [0_u8; 8];
        for (index, byte) in password.into_iter().rev().copied().enumerate() {
            key[index] = byte & 0x7F;
        }
        let cipher = des::Des::new_from_slice(&key).unwrap();
        let mut response = *challenge;
        for chunk in response.chunks_exact_mut(8) {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        response
    }

    /// 在真实 TCP 对上完成 VNC 密码握手（`security` 配置的 settings）。
    async fn finish_vnc_handshake(
        stream: &mut TcpStream,
        password: &[u8; 8],
    ) -> (u16, u16, String) {
        assert_eq!(read_exact_vec(stream, 12).await, b"RFB 003.008\n");
        write_fragmented(stream, b"RFB 003.008\n").await;
        assert_eq!(read_exact_vec(stream, 2).await, [1, 2]);
        stream.write_all(&[2]).await.unwrap();
        let challenge: [u8; 16] = read_exact_vec(stream, 16).await.try_into().unwrap();
        stream.write_all(&vnc_response(password, &challenge)).await.unwrap();
        assert_eq!(read_exact_vec(stream, 4).await, [0, 0, 0, 0]);
        stream.write_all(&[1]).await.unwrap();

        let header = read_exact_vec(stream, 24).await;
        let width = u16::from_be_bytes([header[0], header[1]]);
        let height = u16::from_be_bytes([header[2], header[3]]);
        let name_length =
            u32::from_be_bytes([header[20], header[21], header[22], header[23]]) as usize;
        let name = String::from_utf8(read_exact_vec(stream, name_length).await).unwrap();
        (width, height, name)
    }

    #[tokio::test]
    async fn vnc_password_handshake_emits_connected_event() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 2, 1, &[1, 2, 3, 0, 4, 5, 6, 0]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, mut event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let settings = RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc { password: *b"12345678" },
            ..RfbConnectionSettings::default()
        };
        let mut task = spawn_connection(
            RfbClientId(1),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            settings,
            shutdown_rx,
        );

        let handshake = tokio::select! {
            handshake = finish_vnc_handshake(&mut client_stream, b"12345678") => handshake,
            end = &mut task => panic!("connection ended before handshake: {end:?}"),
        };
        assert_eq!(handshake, (2, 1, "my_ipkvm".to_string()));
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected {
                client_id: RfbClientId(1),
                shared: true,
                ..
            })
        ));

        drop(client_stream);
        assert!(matches!(task.await.unwrap(), ConnectionEnd::ClientClosed));
    }

    #[tokio::test]
    async fn wrong_vnc_password_ends_with_authentication_failed() {
        let frame_source = MockFrameSource::new();
        frame_source.publish_frame(shared_bgra_frame(1, 1, 1, &[0; 4]));
        let (server_stream, mut client_stream, peer_addr) = tcp_pair().await;
        let (event_tx, _event_rx) = mpsc::channel(8);
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let settings = RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc { password: *b"12345678" },
            ..RfbConnectionSettings::default()
        };
        let task = spawn_connection(
            RfbClientId(2),
            peer_addr,
            server_stream,
            &frame_source,
            event_tx,
            settings,
            shutdown_rx,
        );

        assert_eq!(read_exact_vec(&mut client_stream, 12).await, b"RFB 003.008\n");
        client_stream.write_all(b"RFB 003.008\n").await.unwrap();
        assert_eq!(read_exact_vec(&mut client_stream, 2).await, [1, 2]);
        client_stream.write_all(&[2]).await.unwrap();
        let challenge: [u8; 16] = read_exact_vec(&mut client_stream, 16).await.try_into().unwrap();
        let wrong = vnc_response(b"wrongpas", &challenge);
        client_stream.write_all(&wrong).await.unwrap();
        let failed = read_exact_vec(&mut client_stream, 4).await;
        assert_eq!(failed, [0, 0, 0, 1]);

        assert!(matches!(
            task.await.unwrap(),
            ConnectionEnd::Failed(RfbConnectionError::Protocol(
                ipkvm_rfb::RfbProtocolError::AuthenticationFailed
            ))
        ));
        assert_eq!(
            ConnectionEnd::Failed(RfbConnectionError::Protocol(
                ipkvm_rfb::RfbProtocolError::AuthenticationFailed
            ))
            .reason(),
            Some(RfbDisconnectReason::AuthenticationFailed)
        );
    }
```

（`read_exact_vec`/`write_fragmented`/`spawn_connection`/`shared_bgra_frame`/`tcp_pair` 均为 driver.rs 测试已有辅助，直接复用。）

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-session rfb_connection`
Expected: 编译错误（`security` 字段不存在、`AuthenticationFailed` reason 变体不存在）。

- [ ] **Step 3: mod.rs 加字段与断连原因**

修改 `crates/ipkvm-session/src/rfb_connection/mod.rs`：

1. `RfbConnectionSettings` 加字段与默认值：

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RfbConnectionSettings {
    pub desktop_name: String,
    pub handshake_timeout: Duration,
    pub protocol_limits: RfbProtocolLimits,
    pub security: RfbSecurity,
}
```

`impl Default` 加 `security: RfbSecurity::None`。

2. `RfbDisconnectReason` 加变体：

```rust
    AuthenticationFailed,
```

3. 顶部 `use ipkvm_rfb::{...}` 列表加 `RfbSecurity`。

- [ ] **Step 4: driver.rs 传递与映射**

修改 `crates/ipkvm-session/src/rfb_connection/driver.rs`：

1. `drive_connection` 里构造 `RfbConnectionConfig` 加字段：

```rust
    let mut core = RfbConnectionCore::new(RfbConnectionConfig {
        desktop_name: settings.desktop_name.clone(),
        initial_size: initial_view.size(),
        limits: settings.protocol_limits,
        security: settings.security.clone(),
    })?;
```

2. `ConnectionEnd::reason()` 的 `Self::Failed(...)` 匹配加分支（放在 `Protocol` 分支旁）：

```rust
            Self::Failed(RfbConnectionError::Protocol(RfbProtocolError::AuthenticationFailed)) => {
                RfbDisconnectReason::AuthenticationFailed
            }
```

注意：`Self::Failed(RfbConnectionError::Protocol(error)) => RfbDisconnectReason::Protocol(error.clone())` 通配分支在特定分支之后匹配，顺序必须保证特定分支在前。

- [ ] **Step 5: 运行测试**

Run: `cargo test -p ipkvm-session`
Expected: 全部通过（新增 4 个测试；现有 None 行为不变）。

- [ ] **Step 6: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy -p ipkvm-session --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git add Cargo.toml Cargo.lock crates/ipkvm-session/
git commit -m "feat(session): thread RFB security through connection settings"
```

---

### Task 3: headless 配置模型（TOML 文件 + CLI 覆盖 + 确定性中文错误）

**Files:**
- Create: `crates/ipkvm-headless/src/config.rs`
- Modify: `crates/ipkvm-headless/src/lib.rs`（`pub mod config;`）
- Modify: `crates/ipkvm-headless/Cargo.toml`（dependencies 加 toml）

**Interfaces:**
- Produces（全部 `ipkvm_headless::config`）：
  - `CliOptions`（全 Option，parse_cli 产物）——T4 main.rs 调用 `parse_cli()`。
  - `Options`（合并后落定值，含 `token: Option<String>`、`vnc_password: Option<String>`）——T4 传给 `build_source`/`run`。
  - `FileConfig`——`load_config` 产物。
  - `parse_cli() -> Result<CliOptions, String>`——CLI 解析（从 main.rs 迁入，便于单测）。
  - `load_config(path: &std::path::Path) -> Result<FileConfig, String>`。
  - `resolve(cli: CliOptions, file: Option<FileConfig>) -> Result<Options, String>`。
  - `vnc_security(vnc_password: Option<&str>) -> RfbSecurity`——T4 注入两个传输 config。
- Consumes: `ipkvm_rfb::RfbSecurity`（T1）。

- [ ] **Step 1: 加 toml 依赖**

修改 `crates/ipkvm-headless/Cargo.toml` 的 `[dependencies]` 追加：

```toml
toml.workspace = true
```

- [ ] **Step 2: 写失败测试**

创建 `crates/ipkvm-headless/src/config.rs`，先写模块骨架（类型定义 + 空实现）与完整测试：

```rust
//! headless 运行配置：`--config` TOML 文件与 CLI 参数合并。
//!
//! 合并优先级：CLI 参数 > 配置文件字段 > 默认值。所有配置错误都返回
//! 确定性中文报错（含文件路径），供 `main` 打印后以非零码退出。

use std::path::PathBuf;

use ipkvm_rfb::RfbSecurity;
use serde::Deserialize;

/// CLI 参数（全部可选：`None`/`false` 表示未显式指定，不覆盖文件字段）。
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CliOptions {
    pub assets_dir: Option<PathBuf>,
    pub camera_name: Option<String>,
    pub list_cameras: bool,
    pub serial_path: Option<String>,
    pub serial_baud: Option<u32>,
    pub bind_address: Option<String>,
    pub tcp_port: Option<u16>,
    pub http_port: Option<u16>,
    pub frames_per_second: Option<u64>,
    pub config_path: Option<PathBuf>,
    pub token: Option<String>,
    pub vnc_password: Option<String>,
}

/// 合并后的最终配置（CLI > 文件 > 默认）。`assets_dir`/`camera_name` 均为
/// `None` 表示默认相机选择（build_source 的既有语义）。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    pub assets_dir: Option<PathBuf>,
    pub camera_name: Option<String>,
    pub list_cameras: bool,
    pub serial_path: Option<String>,
    pub serial_baud: u32,
    pub bind_address: String,
    pub tcp_port: u16,
    pub http_port: u16,
    pub frames_per_second: u64,
    pub token: Option<String>,
    pub vnc_password: Option<String>,
}

/// 配置文件顶层。`deny_unknown_fields` 保证未知字段确定性报错。
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileConfig {
    pub server: Option<ServerSection>,
    pub video: Option<VideoSection>,
    pub input: Option<InputSection>,
    pub auth: Option<AuthSection>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSection {
    pub bind: Option<String>,
    pub tcp_port: Option<u16>,
    pub http_port: Option<u16>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VideoSection {
    pub camera: Option<String>,
    pub assets: Option<PathBuf>,
    pub fps: Option<u64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputSection {
    pub serial: Option<String>,
    pub baud: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthSection {
    pub token: Option<String>,
    pub vnc_password: Option<String>,
}

/// 读取并解析配置文件。错误信息含文件路径（TOML 解析错误自带行列号）。
pub fn load_config(path: &std::path::Path) -> Result<FileConfig, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| format!("无法读取配置文件 {}：{error}", path.display()))?;
    toml::from_str(&text)
        .map_err(|error| format!("解析配置文件 {} 失败：{error}", path.display()))
}

/// 合并 CLI 与文件配置（CLI > 文件 > 默认），并做互斥与 token 校验。
pub fn resolve(cli: CliOptions, file: Option<FileConfig>) -> Result<Options, String> {
    let file = file.unwrap_or_default();
    let server = file.server.as_ref();
    let video = file.video.as_ref();
    let input = file.input.as_ref();
    let auth = file.auth.as_ref();

    let camera_name = cli.camera_name.clone().or_else(|| video.and_then(|v| v.camera.clone()));
    let assets_dir = cli.assets_dir.clone().or_else(|| video.and_then(|v| v.assets.clone()));
    if assets_dir.is_some() && camera_name.is_some() {
        return Err("--assets 与 --camera 互斥，只能指定其中一个".to_string());
    }

    let token = cli.token.clone().or_else(|| auth.and_then(|a| a.token.clone()));
    if let Some(token) = &token {
        if token.is_empty() || !token.is_ascii() {
            return Err("[auth] token 必须为非空 ASCII 字符串".to_string());
        }
    }
    let vnc_password = cli
        .vnc_password
        .clone()
        .or_else(|| auth.and_then(|a| a.vnc_password.clone()));
    if let Some(password) = &vnc_password {
        if password.is_empty() || password.len() > 8 || !password.is_ascii() {
            return Err(format!(
                "[auth] vnc_password 长度必须为 1-8 个 ASCII 字符（RFC 6143 密码上限 8 字节），当前 {} 字节",
                password.len()
            ));
        }
    }

    Ok(Options {
        assets_dir,
        camera_name,
        list_cameras: cli.list_cameras,
        serial_path: cli.serial_path.clone().or_else(|| input.and_then(|i| i.serial.clone())),
        serial_baud: cli.serial_baud.unwrap_or_else(|| input.and_then(|i| i.baud).unwrap_or(9600)),
        bind_address: cli
            .bind_address
            .clone()
            .unwrap_or_else(|| server.and_then(|s| s.bind.clone()).unwrap_or_else(|| "127.0.0.1".to_string())),
        tcp_port: cli.tcp_port.unwrap_or_else(|| server.and_then(|s| s.tcp_port).unwrap_or(5900)),
        http_port: cli
            .http_port
            .unwrap_or_else(|| server.and_then(|s| s.http_port).unwrap_or(6080)),
        frames_per_second: cli
            .frames_per_second
            .unwrap_or_else(|| video.and_then(|v| v.fps).unwrap_or(10)),
        token,
        vnc_password,
    })
}

/// 把 `[auth] vnc_password` 转成 RFB 安全配置（VNC 密码挑战）。
/// 调用方保证密码已通过 `resolve` 校验（1-8 个 ASCII 字符）。
pub fn vnc_security(vnc_password: Option<&str>) -> RfbSecurity {
    match vnc_password {
        Some(password) => {
            let mut derived = [0_u8; 8];
            derived[..password.len()].copy_from_slice(password.as_bytes());
            RfbSecurity::Vnc { password: derived }
        }
        None => RfbSecurity::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_config(toml_text: &str) -> FileConfig {
        toml::from_str(toml_text).unwrap()
    }

    #[test]
    fn defaults_apply_when_neither_cli_nor_file_specified() {
        let options = resolve(CliOptions::default(), None).unwrap();
        assert_eq!(options.bind_address, "127.0.0.1");
        assert_eq!(options.tcp_port, 5900);
        assert_eq!(options.http_port, 6080);
        assert_eq!(options.frames_per_second, 10);
        assert_eq!(options.serial_baud, 9600);
        assert_eq!(options.token, None);
        assert_eq!(options.vnc_password, None);
        assert_eq!(options.assets_dir, None);
        assert_eq!(options.camera_name, None);
        assert_eq!(options.serial_path, None);
    }

    #[test]
    fn file_fields_override_defaults() {
        let file = file_config(
            r#"
[server]
bind = "0.0.0.0"
tcp_port = 6000
http_port = 7000

[video]
assets = "C:\\assets"
fps = 30

[input]
serial = "COM9"
baud = 115200

[auth]
token = "secret"
vnc_password = "abc12345"
"#,
        );
        let options = resolve(CliOptions::default(), Some(file)).unwrap();
        assert_eq!(options.bind_address, "0.0.0.0");
        assert_eq!(options.tcp_port, 6000);
        assert_eq!(options.http_port, 7000);
        assert_eq!(options.frames_per_second, 30);
        assert_eq!(options.assets_dir, Some(PathBuf::from(r"C:\assets")));
        assert_eq!(options.camera_name, None);
        assert_eq!(options.serial_path, Some("COM9".to_string()));
        assert_eq!(options.serial_baud, 115200);
        assert_eq!(options.token, Some("secret".to_string()));
        assert_eq!(options.vnc_password, Some("abc12345".to_string()));
    }

    #[test]
    fn cli_fields_override_file_fields() {
        let file = file_config(
            r#"
[server]
bind = "0.0.0.0"
tcp_port = 6000
http_port = 7000

[video]
camera = "A"
fps = 30

[input]
serial = "COM9"
baud = 115200

[auth]
token = "secret"
vnc_password = "filepass"
"#,
        );
        let cli = CliOptions {
            assets_dir: None,
            camera_name: Some("OBS Virtual Camera".to_string()),
            list_cameras: false,
            serial_path: None,
            serial_baud: None,
            bind_address: Some("10.0.0.1".to_string()),
            tcp_port: Some(5901),
            http_port: None,
            frames_per_second: Some(15),
            config_path: None,
            token: None,
            vnc_password: Some("clipass".to_string()),
        };
        let options = resolve(cli, Some(file)).unwrap();
        assert_eq!(options.bind_address, "10.0.0.1");
        assert_eq!(options.tcp_port, 5901);
        assert_eq!(options.http_port, 7000); // CLI 未指定，文件生效
        assert_eq!(options.frames_per_second, 15);
        assert_eq!(options.camera_name, Some("OBS Virtual Camera".to_string()));
        assert_eq!(options.serial_path, Some("COM9".to_string()));
        assert_eq!(options.serial_baud, 115200);
        assert_eq!(options.token, Some("secret".to_string()));
        assert_eq!(options.vnc_password, Some("clipass".to_string())); // CLI 覆盖文件
    }

    #[test]
    fn camera_and_assets_conflict_is_rejected_across_layers() {
        let file = file_config("[video]\ncamera = \"A\"\n");
        let cli = CliOptions {
            assets_dir: Some(PathBuf::from("assets")),
            ..CliOptions::default()
        };
        let error = resolve(cli, Some(file)).unwrap_err();
        assert_eq!(error, "--assets 与 --camera 互斥，只能指定其中一个");

        let file = file_config("[video]\ncamera = \"A\"\nassets = \"B\"\n");
        let error = resolve(CliOptions::default(), Some(file)).unwrap_err();
        assert_eq!(error, "--assets 与 --camera 互斥，只能指定其中一个");
    }

    #[test]
    fn token_must_be_non_empty_ascii() {
        for token in ["", "密abc"] {
            let file = file_config(&format!("[auth]\ntoken = \"{token}\"\n"));
            let error = resolve(CliOptions::default(), Some(file)).unwrap_err();
            assert!(error.contains("非空 ASCII"), "token {token:?} 报错：{error}");
        }
        // token 不设长度上限（HTTP 凭证）。
        let cli = CliOptions {
            token: Some("a".repeat(32)),
            ..CliOptions::default()
        };
        assert_eq!(resolve(cli, None).unwrap().token, Some("a".repeat(32)));
    }

    #[test]
    fn vnc_password_must_be_one_to_eight_ascii_bytes() {
        for password in ["", "123456789", "密abc"] {
            let file = file_config(&format!("[auth]\nvnc_password = \"{password}\"\n"));
            let error = resolve(CliOptions::default(), Some(file)).unwrap_err();
            assert!(
                error.contains("1-8 个 ASCII 字符"),
                "vnc_password {password:?} 报错：{error}"
            );
        }
        let cli = CliOptions {
            vnc_password: Some("abc12345".to_string()),
            ..CliOptions::default()
        };
        assert_eq!(
            resolve(cli, None).unwrap().vnc_password,
            Some("abc12345".to_string())
        );
    }

    #[test]
    fn unknown_fields_are_rejected_with_path_in_error() {
        let text = "[server]\nport = 1\n";
        let error = load_config(&std::path::Path::new("missing.toml")).unwrap_err();
        assert!(error.contains("missing.toml"), "报错应含文件路径：{error}");

        // 未知字段走 toml::from_str 的错误路径（deny_unknown_fields）。
        let error = toml::from_str::<FileConfig>(text).unwrap_err();
        assert!(error.to_string().contains("unknown field `port`"));
    }

    #[test]
    fn vnc_security_maps_password_to_rfb_security() {
        assert_eq!(vnc_security(None), RfbSecurity::None);
        assert_eq!(
            vnc_security(Some("secret")),
            RfbSecurity::Vnc { password: *b"secret\0\0" }
        );
    }
}
```

**注意**：`file_config` 用 `toml::from_str` 直接解析（测试不需要走文件系统）；`unknown_fields_are_rejected_with_path_in_error` 里 `load_config` 只测「文件不存在」分支。

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test -p ipkvm-headless config`
Expected: 编译错误（`resolve`/`auth_security`/`load_config` 未实现——Step 2 里已给实现，测试与实现同文件，实际是确认模块接线后测试通过；若先写空实现则确认测试红，再填实现）。

- [ ] **Step 4: lib.rs 声明模块**

修改 `crates/ipkvm-headless/src/lib.rs`：

```rust
use ipkvm_rfb::RfbServerConfig;

pub use ipkvm_session::{rfb_connection, rfb_input};
pub mod config;
pub mod rfb_tcp;
pub mod rfb_ws;
pub mod web;
```

- [ ] **Step 5: 运行测试**

Run: `cargo test -p ipkvm-headless config`
Expected: 全部通过。

- [ ] **Step 6: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git add crates/ipkvm-headless/ Cargo.lock
git commit -m "feat(headless): TOML config model with CLI override and Chinese errors"
```

---

### Task 4: main.rs 接线（--config/--token + 注入安全配置）

**Files:**
- Modify: `crates/ipkvm-headless/src/main.rs`
- Modify: `crates/ipkvm-headless/src/config.rs`（`parse_cli` + `USAGE`——Step 1 定义，Step 3 实现）
- Modify: `crates/ipkvm-headless/tests/headless_process.rs`（若需要，见 Step 6）

**Interfaces:**
- Consumes: `config::{parse_cli, load_config, resolve, vnc_security, CliOptions}`（T3）、`HeadlessWebService::new(..., auth: Option<String>)`（T5——**本任务排在 T5 之后执行**）、`RfbConnectionSettings.security`（T2）。
- Produces: `config::USAGE` 常量（含 --config/--token 帮助文本）。

- [ ] **Step 1: parse_cli 迁入 config.rs**

把 `main.rs` 的 `Options` 结构体、`USAGE` 常量、`parse_args` 函数整体迁入 `config.rs` 并改造：

1. `Options` 结构体删除（config.rs 已有新 `Options`）。
2. `USAGE` 改为 `pub const USAGE: &str`，追加两行帮助文本：

```rust
  --config <路径>  读取 TOML 配置文件；CLI 参数覆盖文件字段（CLI > 文件 > 默认）
  --token <token>  [auth] HTTP/WS 鉴权 token（非空 ASCII）；未配置时仅允许本机访问
  --vnc-password <密码>
                   [auth] RFB VNC 密码（1-8 个 ASCII 字符）；未配置时 TCP 仅允许本机连接
```

3. `parse_args` 改为 `pub fn parse_cli() -> Result<CliOptions, String>`：所有参数改为 `Option` 赋值（`--list-cameras` 仍为 bool），新增两个分支：

```rust
            "--config" => {
                options.config_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--config 需要一个路径参数".to_string())?,
                ));
            }
            "--token" => {
                options.token = Some(
                    args.next()
                        .ok_or_else(|| "--token 需要一个 token 参数".to_string())?,
                );
            }
            "--vnc-password" => {
                options.vnc_password = Some(
                    args.next()
                        .ok_or_else(|| "--vnc-password 需要一个密码参数".to_string())?,
                );
            }
```

`--help`/`-h` 分支照旧打印 `USAGE`。删除末尾的 `--assets 与 --camera 互斥` 检查（互斥校验已由 `resolve` 统一负责）。

- [ ] **Step 2: 写失败测试（config.rs 测试模块追加）**

```rust
    #[test]
    fn parse_cli_reads_all_flags_including_config_and_token() {
        let cli = parse_cli_from(&[
            "--camera", "OBS", "--tcp", "6000", "--http", "7000", "--fps", "15",
            "--serial", "COM9", "--baud", "115200", "--bind", "0.0.0.0",
            "--assets", "assets", "--config", "my.toml", "--token", "secret",
            "--vnc-password", "abc12345",
        ])
        .unwrap();
        assert_eq!(cli.camera_name, Some("OBS".to_string()));
        assert_eq!(cli.assets_dir, Some(PathBuf::from("assets")));
        assert_eq!(cli.tcp_port, Some(6000));
        assert_eq!(cli.http_port, Some(7000));
        assert_eq!(cli.frames_per_second, Some(15));
        assert_eq!(cli.serial_path, Some("COM9".to_string()));
        assert_eq!(cli.serial_baud, Some(115200));
        assert_eq!(cli.bind_address, Some("0.0.0.0".to_string()));
        assert_eq!(cli.config_path, Some(PathBuf::from("my.toml")));
        assert_eq!(cli.token, Some("secret".to_string()));
        assert_eq!(cli.vnc_password, Some("abc12345".to_string()));
        assert!(!cli.list_cameras);
    }

    #[test]
    fn parse_cli_errors_are_deterministic_chinese() {
        let unknown = parse_cli_from(&["--nope"]).unwrap_err();
        assert_eq!(unknown, "未知参数：--nope");

        let missing_value = parse_cli_from(&["--camera"]).unwrap_err();
        assert_eq!(missing_value, "--camera 需要一个名称参数");

        let bad_port = parse_cli_from(&["--tcp", "not-a-port"]).unwrap_err();
        assert!(bad_port.contains("无效端口"));
    }

    #[test]
    fn list_cameras_flag_is_kept() {
        let cli = parse_cli_from(&["--list-cameras"]).unwrap();
        assert!(cli.list_cameras);
    }
```

（`parse_cli` 读 `std::env::args`——测试需要注入参数。实现时把参数读取抽成内部函数 `parse_cli_from(&[String])`，`parse_cli()` 包装 `std::env::args().skip(1)` 调用它；测试直接用 `parse_cli_from`。）

- [ ] **Step 3: 实现参数注入**

`config.rs` 中实现：

```rust
pub fn parse_cli() -> Result<CliOptions, String> {
    parse_cli_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

fn parse_cli_from(args: &[String]) -> Result<CliOptions, String> {
    // 逻辑与 Step 1 描述的相同，遍历 args 而非 std::env::args()。
}
```

- [ ] **Step 4: 运行测试**

Run: `cargo test -p ipkvm-headless config`
Expected: 通过。

- [ ] **Step 5: main.rs 改造**

修改 `crates/ipkvm-headless/src/main.rs`：

1. 删除 `Options` 结构体与 `USAGE` 常量与 `parse_args` 函数；顶部 import 改为：

```rust
use ipkvm_headless::config::{self, CliOptions};
use ipkvm_headless::rfb_connection::{RfbConnectionGate, RfbConnectionSettings};
```

2. `main()` 开头：

```rust
    let cli = match config::parse_cli() {
        Ok(cli) => cli,
        Err(error) => {
            eprintln!("参数错误：{error}");
            eprint!("{}", config::USAGE);
            std::process::exit(2);
        }
    };
    let file = match cli.config_path.as_deref().map(config::load_config).transpose() {
        Ok(file) => file,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let options = match config::resolve(cli, file) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("配置错误：{error}");
            eprint!("{}", config::USAGE);
            std::process::exit(2);
        }
    };
```

（后续 `if options.list_cameras`、`build_source(&options)`、`run(source, options)` 均不变——`Options` 形状兼容。）

3. `run()` 里构造两个传输配置（替换 `RfbTcpConfig::default()` 与 `RfbWebSocketConfig::default()`）：

```rust
    let security = config::vnc_security(options.vnc_password.as_deref());
    let tcp_config = RfbTcpConfig {
        connection: RfbConnectionSettings {
            security: security.clone(),
            ..RfbConnectionSettings::default()
        },
        ..RfbTcpConfig::default()
    };
    let ws_config = RfbWebSocketConfig {
        connection: RfbConnectionSettings {
            security,
            ..RfbConnectionSettings::default()
        },
    };
```

4. `RfbTcpServer::new` 用 `tcp_config`；`HeadlessWebService::new` 用 `ws_config` 并追加 `options.token.clone()` 参数（T5 已改签名）。

5. 启动信息追加鉴权状态（在现有 `println!` 的 `ipkvm-headless 已启动` 之前）：

```rust
    if options.token.is_some() {
        println!("HTTP 鉴权：已启用（Bearer/cookie/query token）");
    } else {
        println!("HTTP 鉴权：未配置 token，仅允许本机来源访问");
    }
    if options.vnc_password.is_some() {
        println!("RFB 鉴权：已启用（VNC 密码挑战）");
    } else {
        println!("RFB 鉴权：未配置 VNC 密码，TCP 仅允许本机来源连接");
    }
```

- [ ] **Step 6: 进程级测试**

读 `crates/ipkvm-headless/tests/headless_process.rs`：`HeadlessAssembly::start()` 不经过 main.rs CLI，无需改动。**验证**：现有进程测试不受影响（`HeadlessWebService::new` 签名变化已在 T5 更新该文件调用点）。若该文件还有其它 `HeadlessWebService::new` 调用点，按 T5 签名补 `None` 参数。

- [ ] **Step 7: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git add crates/ipkvm-headless/
git commit -m "feat(headless): wire config file and auth into main assembly"
```

---

### Task 5: HTTP 鉴权中间件

**Files:**
- Create: `crates/ipkvm-headless/src/web/auth.rs`
- Modify: `crates/ipkvm-headless/src/web/mod.rs`（`pub mod auth;`）
- Modify: `crates/ipkvm-headless/src/web/service.rs`（`HeadlessWebService` 加 `auth` 字段与参数、`serve` 挂中间件）
- Modify: `crates/ipkvm-headless/tests/web_http.rs`（TestWebServer 支持 auth、新增鉴权用例）

**Interfaces:**
- Consumes: `AuthState`/`authorize`（本任务）、T4 的 main.rs 调用 `HeadlessWebService::new(..., token: Option<String>)`。
- Produces:
  - `web::auth::{AuthState, authorize, require_auth}`。
  - `HeadlessWebService::new(frame_source, event_tx, config, shutdown, gate, auth: Option<String>)`——**签名新增第 6 个参数**，现有调用点（main.rs、web_http.rs）全部更新。
  - Cookie 名 `ipkvm_token`（常量 `AUTH_COOKIE`，pub 供测试断言）。

- [ ] **Step 1: 写失败测试（auth.rs 单元测试）**

创建 `crates/ipkvm-headless/src/web/auth.rs`（完整实现 + 测试，先写测试逻辑骨架再实现）：

```rust
//! HTTP 鉴权中间件：统一作用于全部路由。
//!
//! - 配置了 `[auth] token`：请求必须带 `Authorization: Bearer <token>`、
//!   `Cookie: ipkvm_token=<token>` 或 query 参数 `?token=<token>` 之一；
//!   后两者放行时附带 Set-Cookie（query token 只在静态页首访时用，换来
//!   cookie 后后续请求自动带）。
//! - 未配置 token：仅放行回环来源（防默认暴露），其余 403。
//!
//! 判定逻辑抽成纯函数 `authorize`，不依赖 HTTP 运行时，便于单元测试。

use std::net::IpAddr;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::{HeaderMap, StatusCode, Uri, header},
    middleware::Next,
    response::{IntoResponse, Response},
};

/// 鉴权 cookie 名。值即 token。
pub const AUTH_COOKIE: &str = "ipkvm_token";

/// 中间件共享的鉴权状态。
#[derive(Clone, Debug)]
pub struct AuthState {
    pub token: Option<String>,
}

/// 纯判定函数（不依赖运行时）：`None` 表示未配置 token。
///
/// 返回 `true` 放行；`false` 拒绝（401/403 由调用方按配置与否选择）。
pub fn authorize(
    peer_ip: IpAddr,
    bearer: Option<&str>,
    cookie: Option<&str>,
    query_token: Option<&str>,
    configured: Option<&str>,
) -> bool {
    let Some(configured) = configured else {
        return peer_ip.is_loopback();
    };
    bearer == Some(configured) || cookie == Some(configured) || query_token == Some(configured)
}

/// 从 `Authorization` 头提取 Bearer token。
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// 从 `Cookie` 头提取 `ipkvm_token` 值。
fn cookie_token(headers: &HeaderMap) -> Option<&str> {
    let value = headers.get(header::COOKIE)?.to_str().ok()?;
    value.split(';').find_map(|pair| {
        let (name, value) = pair.trim().split_once('=')?;
        (name == AUTH_COOKIE).then_some(value)
    })
}

/// 从 URI query 提取 `token` 参数（token 为 ASCII，不做 URL 解码）。
fn query_token(uri: &Uri) -> Option<&str> {
    uri.query()?.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == "token").then_some(value)
    })
}

pub async fn require_auth(
    State(state): State<AuthState>,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    request: Request,
    next: Next,
) -> Response {
    let bearer = bearer_token(request.headers());
    let cookie = cookie_token(request.headers());
    let query = query_token(request.uri());
    let configured = state.token.as_deref();
    let via_query = query.is_some_and(|value| Some(value) == configured);

    if !authorize(peer.ip(), bearer, cookie, query, configured) {
        return if configured.is_some() {
            StatusCode::UNAUTHORIZED
                .into_response()
                .with_header(header::WWW_AUTHENTICATE, "Bearer")
        } else {
            StatusCode::FORBIDDEN.into_response()
        };
    }

    let mut response = next.run(request).await;
    // query token 放行时种下 cookie：后续同源请求（含 /rfb 升级）自动带。
    if via_query && let Some(token) = configured {
        let value = format!("{AUTH_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax");
        if let Ok(value) = HeaderMap::from_iter([(header::SET_COOKIE, value)]) {
            response.headers_mut().extend(value);
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_token_only_loopback_is_allowed() {
        let loopback = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);
        let remote = std::net::IpAddr::V4([192, 168, 1, 5].into());
        assert!(authorize(loopback, None, None, None, None));
        assert!(!authorize(remote, None, None, None, None));
    }

    #[test]
    fn configured_token_accepts_bearer_cookie_or_query() {
        let remote = std::net::IpAddr::V4([192, 168, 1, 5].into());
        assert!(authorize(remote, Some("secret"), None, None, Some("secret")));
        assert!(authorize(remote, None, Some("secret"), None, Some("secret")));
        assert!(authorize(remote, None, None, Some("secret"), Some("secret")));
        assert!(!authorize(remote, Some("wrong"), None, None, Some("secret")));
        assert!(!authorize(remote, None, None, None, Some("secret")));
    }

    #[test]
    fn cookie_parser_handles_other_cookies_and_whitespace() {
        let headers = HeaderMap::from_iter([
            (
                header::COOKIE,
                "session=abc; ipkvm_token=secret; other=1".parse().unwrap(),
            ),
        ]);
        assert_eq!(cookie_token(&headers), Some("secret"));
    }

    #[test]
    fn query_parser_extracts_token_parameter() {
        let uri: Uri = "/?token=secret&x=1".parse().unwrap();
        assert_eq!(query_token(&uri), Some("secret"));
        let no_token: Uri = "/".parse().unwrap();
        assert_eq!(query_token(&no_token), None);
    }

    #[test]
    fn bearer_parser_requires_prefix_and_valid_header() {
        let headers = HeaderMap::from_iter([
            (header::AUTHORIZATION, "Bearer secret".parse().unwrap()),
        ]);
        assert_eq!(bearer_token(&headers), Some("secret"));
        let wrong_prefix = HeaderMap::from_iter([
            (header::AUTHORIZATION, "Basic secret".parse().unwrap()),
        ]);
        assert_eq!(bearer_token(&wrong_prefix), None);
    }
}
```

**注意**：`StatusCode::into_response().with_header(...)` 是示意写法；实际用：

```rust
let mut response = StatusCode::UNAUTHORIZED.into_response();
response
    .headers_mut()
    .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
response
```

`HeaderMap::from_iter([(header::SET_COOKIE, value)])` 的类型是 `HeaderMap<HeaderValue>`（`value` 为 String 需 `try_from` 失败忽略——见实现里 `if let Ok`）。实现时按 axum 0.8 实际 API 调整，保持语义一致：401 带 `WWW-Authenticate: Bearer`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test -p ipkvm-headless web::auth`
Expected: 编译/断言失败（模块未接线时测试不编译——先建模块，测试用真实实现跑通）。

- [ ] **Step 3: web/mod.rs 声明**

修改 `crates/ipkvm-headless/src/web/mod.rs`：

```rust
pub mod assets;
pub mod auth;
mod service;
```

- [ ] **Step 4: service.rs 挂中间件**

修改 `crates/ipkvm-headless/src/web/service.rs`：

1. `HeadlessWebService` 加字段：

```rust
pub struct HeadlessWebService<S: ?Sized> {
    rfb: RfbWebSocketService<S>,
    api: Arc<ApiState<S>>,
    shutdown: watch::Receiver<bool>,
    auth: Option<String>,
}
```

2. `new` 签名加参数并赋值：

```rust
    pub fn new(
        frame_source: Arc<S>,
        event_tx: mpsc::Sender<RfbServerEvent>,
        config: RfbWebSocketConfig,
        shutdown: watch::Receiver<bool>,
        gate: RfbConnectionGate,
        auth: Option<String>,
    ) -> Result<Self, HeadlessWebServiceError> {
        ...
        Ok(Self { rfb, api, shutdown, auth })
    }
```

3. `serve` 挂中间件：

```rust
    pub async fn serve(self, listener: TcpListener) -> Result<(), HeadlessWebServiceError> {
        let shutdown = self.shutdown;
        let router = static_router()
            .merge(self.rfb.router())
            .merge(api_router(self.api))
            .layer(axum::middleware::from_fn_with_state(
                auth::AuthState { token: self.auth },
                auth::require_auth,
            ));
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        ...
    }
```

4. 顶部 `use` 加 `super::auth;`。

- [ ] **Step 5: 更新调用点**

`grep -rn "HeadlessWebService::new" crates/` 找出全部调用点（main.rs、web_http.rs），全部在末尾加 `None`（main.rs 的改法在 T4；本任务先加 `None` 保持编译，T4 再注入真实 token）。

- [ ] **Step 6: 集成测试（web_http.rs 扩展）**

修改 `crates/ipkvm-headless/tests/web_http.rs`：

1. `TestWebServer::start_with_frame` 改签名 `start_with_frame(frame, auth: Option<String>)`（`start()` 传 `None`），`HeadlessWebService::new` 末尾加 `auth`。加辅助：

```rust
    async fn request_with_headers(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
    ) -> HttpResponse {
        let mut stream = TcpStream::connect(self.address).await.unwrap();
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Length: 0\r\n",
            self.address
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        parse_http_response(&response)
    }
```

2. 追加用例：

```rust
#[tokio::test]
async fn configured_token_requires_credentials_everywhere() {
    let server = TestWebServer::start_with_frame(Some(test_frame()), Some("secret".to_string()))
        .await;

    let anonymous = server.request("GET", "/api/status").await;
    assert_eq!(anonymous.status, 401);
    assert_eq!(
        anonymous.headers.get("www-authenticate").map(String::as_str),
        Some("Bearer")
    );

    let wrong = server
        .request_with_headers("GET", "/api/status", &[("Authorization", "Bearer wrong")])
        .await;
    assert_eq!(wrong.status, 401);

    let correct = server
        .request_with_headers("GET", "/api/status", &[("Authorization", "Bearer secret")])
        .await;
    assert_eq!(correct.status, 200);

    server.stop().await;
}

#[tokio::test]
async fn configured_token_accepts_cookie_and_query_and_sets_cookie() {
    let server = TestWebServer::start_with_frame(Some(test_frame()), Some("secret".to_string()))
        .await;

    let cookie = server
        .request_with_headers("GET", "/api/status", &[("Cookie", "ipkvm_token=secret")])
        .await;
    assert_eq!(cookie.status, 200);

    let query = server.request("GET", "/?token=secret").await;
    assert_eq!(query.status, 200);
    let set_cookie = query
        .headers
        .get("set-cookie")
        .cloned()
        .unwrap_or_default();
    assert!(
        set_cookie.contains("ipkvm_token=secret"),
        "应种下 ipkvm_token cookie：{set_cookie}"
    );

    // 静态页与 /rfb 升级同样被中间件覆盖。
    let page_without_credentials = server.request("GET", "/").await;
    assert_eq!(page_without_credentials.status, 401);
    assert!(server
        .request_with_headers(
            "GET",
            "/vendor/novnc/core/rfb.js",
            &[("Authorization", "Bearer secret")],
        )
        .await
        .status == 200);

    server.stop().await;
}
```

3. 确认现有全部用例在 `auth: None` 下保持 200（无 token 时中间件只拦非本机——测试连 127.0.0.1，全放行）。

- [ ] **Step 7: 运行测试**

Run: `cargo test -p ipkvm-headless --test web_http`
Expected: 全部通过（现有 + 新增 2 个鉴权用例）。

- [ ] **Step 8: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy -p ipkvm-headless --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git add crates/ipkvm-headless/
git commit -m "feat(headless): unified HTTP auth middleware (bearer/cookie/query token)"
```

---

### Task 6: RFB TCP/WS 密码集成测试 + TCP 本机来源检查

**Files:**
- Modify: `crates/ipkvm-headless/src/rfb_tcp/server.rs`（`tcp_peer_allowed` + accept 循环检查 + 单元测试）
- Modify: `crates/ipkvm-headless/Cargo.toml`（dev-dependencies 加 des）
- Modify: `crates/ipkvm-headless/tests/rfb_tcp.rs`（VNC 密码集成测试）
- Modify: `crates/ipkvm-headless/tests/rfb_websocket.rs`（WS VNC 密码集成测试）

**Interfaces:**
- Consumes: `RfbConnectionSettings.security`（T2）、`RfbSecurity`（T1）。
- Produces: `RfbTcpServer` 内部 `tcp_peer_allowed(peer: SocketAddr, security: &RfbSecurity) -> bool`（pub(crate) 或私有 + 同文件单测）。

- [ ] **Step 1: 写失败测试（tcp_peer_allowed 单元）**

在 `crates/ipkvm-headless/src/rfb_tcp/server.rs` 的测试模块追加：

```rust
    #[test]
    fn tcp_peer_allowed_rejects_remote_without_password() {
        let loopback: SocketAddr = "127.0.0.1:5900".parse().unwrap();
        let remote: SocketAddr = "192.168.1.5:5900".parse().unwrap();
        assert!(tcp_peer_allowed(loopback, &RfbSecurity::None));
        assert!(!tcp_peer_allowed(remote, &RfbSecurity::None));
        // 配置密码后来源不再限制，完全交给密码校验。
        assert!(tcp_peer_allowed(remote, &RfbSecurity::Vnc { password: [0; 8] }));
    }
```

- [ ] **Step 2: 实现 tcp_peer_allowed**

在 `crates/ipkvm-headless/src/rfb_tcp/server.rs` 加函数（`impl` 块外、`shutdown_is_requested` 旁）：

```rust
/// 未配置密码时，TCP 入口只允许回环来源（防默认暴露）；配置了 VNC 密码
/// 则来源不再限制，完全交给密码挑战校验。
fn tcp_peer_allowed(peer: SocketAddr, security: &RfbSecurity) -> bool {
    match security {
        RfbSecurity::None => peer.ip().is_loopback(),
        RfbSecurity::Vnc { .. } => true,
    }
}
```

`run()` 的 accept 分支拿到 `peer_addr` 后、`gate.acquire` 之前加：

```rust
            if !tcp_peer_allowed(peer_addr, &self.config.connection.security) {
                // 未配置密码：非回环来源直接关闭，不进入握手。
                continue;
            }
```

顶部 `use` 加 `ipkvm_rfb::RfbSecurity`（或从 `ipkvm_session` 重导出路径——按现文件 `use ipkvm_session::rfb_connection::{...}` 风格，`RfbSecurity` 直接从 `ipkvm_rfb` 导入）。

- [ ] **Step 3: dev-dependencies 加 des**

修改 `crates/ipkvm-headless/Cargo.toml` 的 `[dev-dependencies]` 追加：

```toml
des.workspace = true
```

- [ ] **Step 4: TCP VNC 密码集成测试**

在 `crates/ipkvm-headless/tests/rfb_tcp.rs` 追加（先读该文件确认现有 server 构造辅助并复用；以下为测试代码）：

```rust
    /// 独立实现 VNC 密码响应（RFC 6143 §7.2），交叉验证服务器校验逻辑。
    fn vnc_response(password: &[u8; 8], challenge: &[u8; 16]) -> [u8; 16] {
        use cipher::{BlockEncrypt, KeyInit, generic_array::GenericArray};
        let mut key = [0_u8; 8];
        for (index, byte) in password.into_iter().rev().copied().enumerate() {
            key[index] = byte & 0x7F;
        }
        let cipher = des::Des::new_from_slice(&key).unwrap();
        let mut response = *challenge;
        for chunk in response.chunks_exact_mut(8) {
            cipher.encrypt_block(GenericArray::from_mut_slice(chunk));
        }
        response
    }

    async fn complete_vnc_handshake(
        stream: &mut TcpStream,
        password: &[u8; 8],
        expect_success: bool,
    ) {
        let mut banner = [0_u8; 12];
        stream.read_exact(&mut banner).await.unwrap();
        assert_eq!(&banner, b"RFB 003.008\n");
        stream.write_all(b"RFB 003.008\n").await.unwrap();
        let mut security_types = [0_u8; 2];
        stream.read_exact(&mut security_types).await.unwrap();
        assert_eq!(security_types, [1, 2]);
        stream.write_all(&[2]).await.unwrap();
        let mut challenge = [0_u8; 16];
        stream.read_exact(&mut challenge).await.unwrap();
        stream.write_all(&vnc_response(password, &challenge)).await.unwrap();
        let mut result = [0_u8; 4];
        stream.read_exact(&mut result).await.unwrap();
        assert_eq!(result, if expect_success { [0; 4] } else { [0, 0, 0, 1] });
    }

    #[tokio::test]
    async fn vnc_password_handshake_succeeds_over_tcp() {
        let (server, mut event_rx) = make_server_with_security(
            RfbSecurity::Vnc { password: *b"12345678" },
        )
        .await;
        let address = server.listener.local_addr().unwrap();
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let owner = tokio::spawn(server.run(shutdown_rx));
        let mut stream = TcpStream::connect(address).await.unwrap();

        complete_vnc_handshake(&mut stream, b"12345678", true).await;
        stream.write_all(&[1]).await.unwrap();
        let mut server_init = [0_u8; 24];
        stream.read_exact(&mut server_init).await.unwrap();
        assert!(matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected { .. })
        ));

        drop(stream);
        owner.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn vnc_password_handshake_rejects_wrong_password_over_tcp() {
        let (server, _event_rx) = make_server_with_security(
            RfbSecurity::Vnc { password: *b"12345678" },
        )
        .await;
        let address = server.listener.local_addr().unwrap();
        let (_shutdown_tx, shutdown_rx) = watch::channel(false);
        let owner = tokio::spawn(server.run(shutdown_rx));
        let mut stream = TcpStream::connect(address).await.unwrap();

        complete_vnc_handshake(&mut stream, b"wrongpas", false).await;
        // 失败后服务器关闭连接：读返回 0。
        let mut tail = Vec::new();
        stream.read_to_end(&mut tail).await.unwrap();
        assert!(!matches!(
            event_rx.recv().await,
            Some(RfbServerEvent::Connected { .. })
        ));

        drop(stream);
        owner.await.unwrap().unwrap();
    }
```

辅助 `make_server_with_security(security) -> (RfbTcpServer<MockFrameSource>, mpsc::Receiver<RfbServerEvent>)`：参照现有 `make_server()`，`RfbTcpConfig { connection: RfbConnectionSettings { security, ..RfbConnectionSettings::default() }, ..RfbTcpConfig::default() }`。需要 `use ipkvm_session::rfb_connection::RfbConnectionSettings;`（rfb_tcp.rs 若已通过 `ipkvm_headless::rfb_connection` 重导出则用重导出路径）。

- [ ] **Step 5: WS VNC 密码集成测试**

读 `crates/ipkvm-headless/tests/rfb_websocket.rs`：该文件已有 `TestRfbServer::start_with_config(config)` 与支持层 `TestRfbClient`。追加：

```rust
#[tokio::test]
async fn vnc_password_handshake_succeeds_over_websocket() {
    let config = RfbWebSocketConfig {
        connection: ipkvm_session::rfb_connection::RfbConnectionSettings {
            security: ipkvm_rfb::RfbSecurity::Vnc { password: *b"12345678" },
            ..ipkvm_session::rfb_connection::RfbConnectionSettings::default()
        },
    };
    let server = TestRfbServer::start_with_config(config).await;

    // 升级成功后，RFB 握手跑在 WS 二进制消息上（与 TCP 同一路径）：
    // banner → [1,2] → 选 2 → challenge → 响应 → OK → ServerInit。
    let (mut socket, _) = server.connect().await;  // 参照现有用例的连接方式
    // ... 用现有接收/发送二进制消息辅助完成挑战流程（见 Step 4 的
    // complete_vnc_handshake 逻辑的 WS 变体，消息边界按现有辅助）。
    let mut banner = receive_binary(&mut socket).await;
    assert_eq!(banner, b"RFB 003.008\n");
    socket.send(Message::Binary(b"RFB 003.008\n".to_vec().into())).await.unwrap();
    assert_eq!(receive_binary(&mut socket).await, [1, 2]);
    socket.send(Message::Binary(vec![2].into())).await.unwrap();
    let challenge: [u8; 16] = receive_binary(&mut socket).await.try_into().unwrap();
    socket
        .send(Message::Binary(vnc_response(b"12345678", &challenge).to_vec().into()))
        .await
        .unwrap();
    assert_eq!(receive_binary(&mut socket).await, [0, 0, 0, 0]);
    socket.send(Message::Binary(vec![1].into())).await.unwrap();
    assert!(receive_binary(&mut socket).await.len() >= 24);
    assert!(matches!(
        server.expect_connected().await,
        RfbClientId(_)
    ));

    server.stop().await;
}
```

（`receive_binary`/`Message`/`connect` 等按该文件现有辅助命名调整；`vnc_response` 复制到该文件。若文件内已有 `receive_binary` 等价辅助则复用。错误密码用例可省略——TCP 已覆盖失败路径，WS 与 TCP 同一驱动，避免重复。）

- [ ] **Step 6: 运行测试**

Run: `cargo test -p ipkvm-headless --test rfb_tcp --test rfb_websocket`
Expected: 全部通过（现有 None 测试不受影响 + 新增 3 个 VNC 用例）。

- [ ] **Step 7: 全量验证并提交**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
git add crates/ipkvm-headless/
git commit -m "test(headless): VNC password over TCP/WS, loopback-only TCP gate"
```

---

### Task 7: app.js token + 文档 + 全量验证 + PR

**Files:**
- Modify: `crates/ipkvm-headless/web/app.js`
- Modify: `README.md`
- Modify: `docs/superpowers/specs/2026-08-02-product-apps-wiring-design.md`
- Create: `scripts/vnc-auth-check.py`（vncdotool 互操作验证，若 verify.ps1 结构允许则接入，否则记录人工验证）
- Modify: `scripts/verify.ps1`（条件性接入，见 Step 3）

- [ ] **Step 1: app.js 支持 query token**

修改 `crates/ipkvm-headless/web/app.js` 的 `websocketUrl`：

```js
function websocketUrl() {
  const scheme = location.protocol === "https:" ? "wss" : "ws";
  const token = new URLSearchParams(location.search).get("token");
  const query = token ? `?token=${encodeURIComponent(token)}` : "";
  return `${scheme}://${location.host}/rfb${query}`;
}
```

（服务端中间件对 `/rfb?token=...` 升级请求校验 query token。无 token 时行为与现在完全一致。）

- [ ] **Step 2: 文档更新**

1. `README.md`：
   - 「运行无头后台进程」节：加 `--config`、`--token`、`--vnc-password` 用法示例与说明；说明默认值（bind/tcp/http/fps/baud）、优先级（CLI > 文件 > 默认）、token 限制（非空 ASCII，HTTP/WS 凭证）、vnc_password 限制（1-8 个 ASCII，RFB 凭证）、未配置对应凭证时仅本机可访问、HTTP 用 Bearer/cookie、浏览器首访用 `http://host:6080/?token=xxx`、VNC 客户端用 vnc_password 对应的密码。
   - 「当前模块」的 `ipkvm-headless` 条目：去掉「鉴权和 TLS 尚未实现」中的鉴权部分（改为「TLS 尚未实现」）。
   - 配置示例 TOML 块（与设计文档一致）。
2. `docs/superpowers/specs/2026-08-02-product-apps-wiring-design.md`：
   - 阶段 2 标题下「配置：TOML 文件 + CLI 覆盖」与「鉴权（最小）」两节标记已实现（注明 issue #31）。
   - 阶段 2 的 TOML 示例 `[auth]` 节补 `vnc_password` 字段：`token` 管 HTTP/WS 凭证，`vnc_password`（1-8 个 ASCII）管 RFB VNC 密码挑战，两者独立。
   - 「HTTP 管理 API」节保持待办（#32）。
   - 「后续工作」节：注明最小鉴权已落地（#31），完整安全子系统（TLS/RFB 加密）仍留后续。
3. 检查 `AGENTS.md`/`docs/development-guidelines.md` 是否需要补充鉴权说明（如无相关章节则跳过，不硬加）。

- [ ] **Step 3: vncdotool 互操作验证**

读 `scripts/vnc-dynamic-resolution-check.py`（已有 vncdotool 使用先例）与 `scripts/verify.ps1` 的浏览器闭环部分：

1. 新增 `scripts/vnc-auth-check.py`：以 vncdotool 客户端连接启用了 VNC 密码的服务，验证标准客户端能完成密码挑战。关键逻辑（参照 dynamic-resolution 脚本的客户端用法）：

```python
#!/usr/bin/env python3
"""用 vncdotool 验证 VNC 密码挑战互操作（issue #31 最小鉴权）。"""
import argparse
import vncdotool.api

def main() -> int:
    parser = argparse.ArgumentParser(description="验证 VNC 密码挑战互操作")
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--password", required=True)
    args = parser.parse_args()

    client = vncdotool.api.connect(
        f"127.0.0.1::{args.port}", password=args.password
    )
    client.mouseMove(0, 0)
    client.disconnect()
    print("VNC 密码挑战互操作验证通过")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
```

（以 vncdotool API 实际签名为准调整——`vncdotool.api.connect(host, password=...)` 是标准签名。）

2. 读 `verify.ps1`：若其浏览器闭环部分有「以自定义参数启动 headless 进程」的既有模式（例如 demo 启动 + 脚本检查），则加一步「带 `--vnc-password` 启动 + `vnc-auth-check.py` 验证 + 错误密码拒绝验证」；若结构不允许低成本接入，则把互操作验证步骤记录到 PR 描述的人工验证部分（明确步骤与预期：`cargo run ... --vnc-password abc12345` + `.venv/bin/python scripts/vnc-auth-check.py --port 5900 --password abc12345`），并注明「后续可自动化」的收敛条件。

- [ ] **Step 4: 全量验证**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
.\scripts\verify.ps1
```

Expected: 全绿（verify.ps1 含真实浏览器闭环与 license/资源门禁；默认无 token 路径行为不变）。

- [ ] **Step 5: 手动冒烟（可选，验证脚本覆盖不了的部分）**

若 Step 3 未接入 verify.ps1，手工执行（记录到 PR 人工验证）：
1. 带凭证启动：`cargo run -p ipkvm-headless --features demo --bin ipkvm-headless --assets .cache/demo-assets --token abc12345 --vnc-password abc12345`。
2. 无凭证访问 `http://127.0.0.1:6080/api/status` → 401；带 `Authorization: Bearer abc12345` → 200。
3. `vncdotool` 用 `abc12345` 密码连接 → 成功；错误密码 → 失败。

- [ ] **Step 6: 提交并创建 PR**

```bash
git add -A
git commit -m "docs(headless): auth docs, app.js query token, vnc interop check"
git push -u origin feat/headless-config-auth
```

创建 PR（PowerShell UTF-8 前置，参照上次 create-pr-30.ps1 模式）：

```powershell
$OutputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::InputEncoding = [System.Text.UTF8Encoding]::new($false)
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$desc = Get-Content -Raw -Encoding UTF8 pr-31-description.md
& tea pulls create --repo kxn/my_ipkvm --base main --head feat/headless-config-auth --title "feat(headless): TOML 配置与最小鉴权（token/VNC 密码/本机限制）" --description $desc
```

PR 描述按 `.gitea/PULL_REQUEST_TEMPLATE.md` 结构：关联 issue（Closes #31）、改动摘要、根因或设计依据、测试结果（完整数字）、人工验证例外（非本机来源拒绝需真机验证——测试环境无法伪造 peer 源地址，说明步骤与后续自动化收敛条件）、文档影响、检查清单。

- [ ] **Step 7: 收口 ledger**

更新 `.superpowers/sdd/2026-08-02-headless-config-auth/progress.md`：全部任务 complete、PR 号、合并状态。

---

## Self-Review

### 1. Spec coverage（对照 issue #31 与设计文档阶段 2 配置/鉴权部分）

| 需求 | 任务 |
|---|---|
| `--config <路径>` 读 TOML | T3/T4 |
| CLI 覆盖文件字段（CLI > 文件 > 默认） | T3（resolve 单测） |
| 默认值 bind/tcp/http/fps/baud | T3（defaults 测试断言 127.0.0.1/5900/6080/10/9600） |
| 配置错误确定性中文报错（含路径/行列号） | T3（load_config/resolve 单测） |
| `[auth] token` 存在则启用（HTTP/WS 凭证） | T3（token 校验）+ T4（注入 HeadlessWebService） |
| `[auth] vnc_password` 管 RFB（VNC 密码挑战） | T3（vnc_security）+ T4（注入两传输 config）+ T1/T2/T6 |
| 未配置时拒绝非 127.0.0.1 来源 | T5（authorize 单测 + 中间件 403）、T6（tcp_peer_allowed） |
| HTTP Bearer / cookie | T5（中间件 + 集成测试） |
| RFB TCP VNC 密码 | T1（协议）/T2（传递）/T6（集成） |
| WS 同样校验（升级请求 token） | T5（中间件覆盖升级请求）+ T7（app.js query token） |
| 统一中间件/包装点 | T5（一个 layer）、T6（accept 循环一处）、T1（core 一处） |
| 验收：merge 优先级单元测试 | T3 |
| 验收：确定性中文错误 | T3 |
| 验收：正确放行/错误拒绝 | T5（HTTP 集成）、T6（TCP 集成） |
| 验收：verify.ps1 通过 | T7 |
| 不做：API/TLS/多用户/token 刷新/热加载/env 覆盖 | 全部任务明确不做 |

### 2. Placeholder scan

- T5 `require_auth` 中 `with_header` 示意写法已在任务内标注真实实现方式。
- T6 WS 测试标注「参照现有辅助命名调整」——给出完整流程代码，仅辅助名按文件现状对齐（实现者读文件后机械对齐，非占位）。
- 无 TBD/TODO；每个步骤有代码或明确指令。

### 3. Type consistency

- `RfbSecurity::{None, Vnc { password: [u8; 8] }}` 在 T1 定义，T2（settings.security）/T3（auth_security 返回）/T6（tcp_peer_allowed 匹配）一致。
- `RfbConnectionSettings.security` 在 T2 定义，T4（main.rs 构造）/T6（make_server_with_security）一致。
- `HeadlessWebService::new` 第 6 参数 `auth: Option<String>` 在 T5 定义，T4/T5 更新调用点；T4 排在 T5 之后执行（计划执行顺序 T1→T2→T3→T5→T4→T6→T7）。
- `config::{CliOptions, Options, load_config, resolve, parse_cli, vnc_security, USAGE}` 在 T3 定义，T4 使用；`parse_cli` 内部 `parse_cli_from` 供 T4 测试。
- `CliOptions.vnc_password`/`Options.vnc_password`/`FileConfig.AuthSection.vnc_password` 三处命名一致（T3），T4 的 `--vnc-password` 与 `vnc_security` 输入一致。
- `RfbProtocolError::AuthenticationFailed` 在 T1 定义，T2 的 reason 映射使用。
- Cookie 名 `ipkvm_token` 在 T5 定义，T5 集成测试断言使用。
