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

/// VNC 密码派生 DES 密钥：密码截断/补零到 8 字节后每字节位反转
/// （RFC 6143 §7.2.2 + Erratum 4951）。
///
/// 经典实现（d3des / libvncclient / noVNC / TigerVNC）的位反转 DES 变体
/// 等价于标准 DES 使用逐字节位反转的密钥；不做字节序反转、无需掩码
/// （DES 密钥调度丢弃每字节最高位，掩码无效果）。按位反转实现才能与
/// 真实 VNC 客户端互操作。
pub(crate) fn vnc_key(password: [u8; 8]) -> [u8; 8] {
    let mut key = [0_u8; 8];
    for (index, byte) in password.into_iter().enumerate() {
        key[index] = byte.reverse_bits();
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
    fn vnc_key_reverses_bits_of_each_byte() {
        // RFC 6143 §7.2.2（勘误 4951）：截断/补零后每字节位反转，不做字节序反转。
        // "1234" 补零到 8 → 每字节位反转：0x31→0x8c、0x32→0x4c、0x33→0xcc、0x34→0x2c。
        assert_eq!(
            vnc_key(*b"1234\0\0\0\0"),
            [0x8c, 0x4c, 0xcc, 0x2c, 0, 0, 0, 0]
        );
        // 位反转而非掩码：0x80 → 0x01、0x41 → 0x82。
        assert_eq!(
            vnc_key([0x80, 0x41, 0, 0, 0, 0, 0, 0]),
            [0x01, 0x82, 0, 0, 0, 0, 0, 0]
        );
    }

    #[test]
    fn vnc_expected_response_matches_des_ecb_construction() {
        // 已知密钥/明文的 DES-ECB 标准向量（NIST FIPS 46-3 附录），验证
        // 调用方式正确：key=133457799BBCDFF1 加密 0123456789ABCDEF。
        let cipher =
            Des::new_from_slice(&[0x13, 0x34, 0x57, 0x79, 0x9b, 0xbc, 0xdf, 0xf1]).unwrap();
        // 明文 0123456789ABCDEF 是十六进制字节（DES 块 8 字节，ASCII 形式是 16 字节）。
        let mut block = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
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
