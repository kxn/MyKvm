//! headless 运行配置：`--config` TOML 文件与 CLI 参数合并。
//!
//! 合并优先级：CLI 参数 > 配置文件字段 > 运行时设置（`/api/settings`）>
//! 默认值。所有配置错误都返回确定性中文报错（含文件路径），供 `main` 打印后
//! 以非零码退出。

use std::path::PathBuf;

use ipkvm_rfb::RfbSecurity;
use serde::Deserialize;

/// 命令行用法帮助文本（`--help`/`-h` 时打印后退出）。
pub const USAGE: &str = "\
用法：ipkvm-headless [--list-cameras] [--camera <名称>|--assets <目录>] \
[--serial <串口> [--baud <波特率>]] [--bind <地址>] [--tcp <端口>] [--http <端口>] [--fps <帧率>]

  --list-cameras   枚举视频采集设备并退出
  --camera <名称>  按名称（id 或显示名）打开相机；与 --assets 互斥
  --assets <目录>  存放 *.y4m 素材的目录（文件伪设备，按文件名排序循环）
  --serial <串口>  CH9329 串口路径（如 COM9 / /dev/ttyUSB0），启用真实键鼠注入；
                   不指定时键鼠事件进入模拟队列被丢弃
  --baud <波特率>  CH9329 串口波特率；未指定时取运行时设置（初始 115200）
  --bind <地址>    监听地址，默认 127.0.0.1
  --tcp <端口>     RFB TCP 监听端口，默认 5900
  --http <端口>    HTTP/noVNC 监听端口，默认 6080
  --fps <帧率>     播放帧率；未指定时取运行时设置（初始 30）
  --config <路径>  读取 TOML 配置文件；CLI 参数覆盖文件字段（CLI > 文件 > 默认）
  --token <token>  [auth] HTTP/WS 鉴权 token（非空，仅含字母数字与 - _ . ~）；未配置时仅允许本机访问
  --vnc-password <密码>
                   [auth] RFB VNC 密码（1-8 个 ASCII 字符）；未配置时 TCP 仅允许本机连接
";

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

/// 合并后的最终配置（CLI > 文件 > 运行时设置 > 默认）。`assets_dir`/
/// `camera_name` 均为 `None` 表示默认相机选择（build_source 的既有语义）；
/// `serial_baud`/`frames_per_second` 为 `None` 表示 CLI 与配置文件均未指定，
/// 由组装层回退到运行时设置。
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Options {
    pub assets_dir: Option<PathBuf>,
    pub camera_name: Option<String>,
    pub list_cameras: bool,
    pub serial_path: Option<String>,
    pub serial_baud: Option<u32>,
    pub bind_address: String,
    pub tcp_port: u16,
    pub http_port: u16,
    pub frames_per_second: Option<u64>,
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
    toml::from_str(&text).map_err(|error| format!("解析配置文件 {} 失败：{error}", path.display()))
}

/// 合并 CLI 与文件配置（CLI > 文件 > 运行时设置 > 默认），并做互斥与 token 校验。
pub fn resolve(cli: CliOptions, file: Option<FileConfig>) -> Result<Options, String> {
    let file = file.unwrap_or_default();
    let server = file.server.as_ref();
    let video = file.video.as_ref();
    let input = file.input.as_ref();
    let auth = file.auth.as_ref();

    let camera_name = cli
        .camera_name
        .clone()
        .or_else(|| video.and_then(|v| v.camera.clone()));
    let assets_dir = cli
        .assets_dir
        .clone()
        .or_else(|| video.and_then(|v| v.assets.clone()));
    if assets_dir.is_some() && camera_name.is_some() {
        return Err("--assets 与 --camera 互斥，只能指定其中一个".to_string());
    }

    let token = cli
        .token
        .clone()
        .or_else(|| auth.and_then(|a| a.token.clone()));
    if token.as_ref().is_some_and(|token| {
        token.is_empty()
            || !token
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~'))
    }) {
        return Err(
            "[auth] token 只能包含字母、数字和 - _ . ~（RFC 3986 无保留字符），当前包含非法字符"
                .to_string(),
        );
    }
    let vnc_password = cli
        .vnc_password
        .clone()
        .or_else(|| auth.and_then(|a| a.vnc_password.clone()));
    if let Some(password) = &vnc_password
        && (password.is_empty() || password.len() > 8 || !password.is_ascii())
    {
        return Err(format!(
            "[auth] vnc_password 长度必须为 1-8 个 ASCII 字符（RFC 6143 密码上限 8 字节），当前 {} 字节",
            password.len()
        ));
    }

    Ok(Options {
        assets_dir,
        camera_name,
        list_cameras: cli.list_cameras,
        serial_path: cli
            .serial_path
            .clone()
            .or_else(|| input.and_then(|i| i.serial.clone())),
        serial_baud: cli.serial_baud.or_else(|| input.and_then(|i| i.baud)),
        bind_address: cli.bind_address.clone().unwrap_or_else(|| {
            server
                .and_then(|s| s.bind.clone())
                .unwrap_or_else(|| "127.0.0.1".to_string())
        }),
        tcp_port: cli
            .tcp_port
            .unwrap_or_else(|| server.and_then(|s| s.tcp_port).unwrap_or(5900)),
        http_port: cli
            .http_port
            .unwrap_or_else(|| server.and_then(|s| s.http_port).unwrap_or(6080)),
        frames_per_second: cli.frames_per_second.or_else(|| video.and_then(|v| v.fps)),
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

/// 解析 `std::env::args()`（跳过程序名）为 `CliOptions`。
/// 解析错误返回确定性中文报错，供 `main` 打印后退出。
pub fn parse_cli() -> Result<CliOptions, String> {
    parse_cli_from(&std::env::args().skip(1).collect::<Vec<_>>())
}

/// `parse_cli` 的可注入实现：测试直接传参数切片，避免依赖进程环境。
fn parse_cli_from<S: AsRef<str>>(args: &[S]) -> Result<CliOptions, String> {
    let mut options = CliOptions::default();
    let mut args = args.iter();
    while let Some(argument) = args.next() {
        match argument.as_ref() {
            "--list-cameras" => options.list_cameras = true,
            "--camera" => {
                options.camera_name = Some(
                    args.next()
                        .ok_or_else(|| "--camera 需要一个名称参数".to_string())?
                        .as_ref()
                        .to_string(),
                );
            }
            "--assets" => {
                options.assets_dir = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--assets 需要一个目录参数".to_string())?
                        .as_ref(),
                ));
            }
            "--bind" => {
                options.bind_address = Some(
                    args.next()
                        .ok_or_else(|| "--bind 需要一个地址参数".to_string())?
                        .as_ref()
                        .to_string(),
                );
            }
            "--tcp" => {
                options.tcp_port = Some(
                    args.next()
                        .ok_or_else(|| "--tcp 需要一个端口参数".to_string())?
                        .as_ref()
                        .parse()
                        .map_err(|error| format!("无效端口：{error}"))?,
                );
            }
            "--http" => {
                options.http_port = Some(
                    args.next()
                        .ok_or_else(|| "--http 需要一个端口参数".to_string())?
                        .as_ref()
                        .parse()
                        .map_err(|error| format!("无效端口：{error}"))?,
                );
            }
            "--fps" => {
                options.frames_per_second = Some(
                    args.next()
                        .ok_or_else(|| "--fps 需要一个帧率参数".to_string())?
                        .as_ref()
                        .parse()
                        .map_err(|error| format!("无效帧率：{error}"))?,
                );
            }
            "--serial" => {
                options.serial_path = Some(
                    args.next()
                        .ok_or_else(|| "--serial 需要一个串口路径参数".to_string())?
                        .as_ref()
                        .to_string(),
                );
            }
            "--baud" => {
                options.serial_baud = Some(
                    args.next()
                        .ok_or_else(|| "--baud 需要一个波特率参数".to_string())?
                        .as_ref()
                        .parse()
                        .map_err(|error| format!("无效波特率：{error}"))?,
                );
            }
            "--config" => {
                options.config_path = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| "--config 需要一个路径参数".to_string())?
                        .as_ref(),
                ));
            }
            "--token" => {
                options.token = Some(
                    args.next()
                        .ok_or_else(|| "--token 需要一个 token 参数".to_string())?
                        .as_ref()
                        .to_string(),
                );
            }
            "--vnc-password" => {
                options.vnc_password = Some(
                    args.next()
                        .ok_or_else(|| "--vnc-password 需要一个密码参数".to_string())?
                        .as_ref()
                        .to_string(),
                );
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("未知参数：{other}")),
        }
    }
    Ok(options)
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
        // 未指定时保留 None：组装层回退到运行时设置（CLI > 文件 > 运行时 > 默认）。
        assert_eq!(options.frames_per_second, None);
        assert_eq!(options.serial_baud, None);
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
        assert_eq!(options.frames_per_second, Some(30));
        assert_eq!(options.assets_dir, Some(PathBuf::from(r"C:\assets")));
        assert_eq!(options.camera_name, None);
        assert_eq!(options.serial_path, Some("COM9".to_string()));
        assert_eq!(options.serial_baud, Some(115200));
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
        assert_eq!(options.frames_per_second, Some(15));
        assert_eq!(options.camera_name, Some("OBS Virtual Camera".to_string()));
        assert_eq!(options.serial_path, Some("COM9".to_string()));
        assert_eq!(options.serial_baud, Some(115200));
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
    fn token_must_be_non_empty_unreserved_ascii() {
        // 空串、非 ASCII 与 URL 保留字符（空格/&/#/%= 等）均拒绝。
        for token in ["", "密abc", "a b", "a&b", "a#b", "a%b", "a=b"] {
            let file = file_config(&format!("[auth]\ntoken = \"{token}\"\n"));
            let error = resolve(CliOptions::default(), Some(file)).unwrap_err();
            assert!(
                error.contains("无保留字符"),
                "token {token:?} 报错：{error}"
            );
        }
        // token 不设长度上限（HTTP 凭证），且只允许无保留字符（ALPHA/DIGIT/-_.~）。
        for token in ["a".repeat(32), "abc-_.~123".to_string()] {
            let cli = CliOptions {
                token: Some(token.clone()),
                ..CliOptions::default()
            };
            assert_eq!(resolve(cli, None).unwrap().token, Some(token));
        }
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
        let error = load_config(std::path::Path::new("missing.toml")).unwrap_err();
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
            RfbSecurity::Vnc {
                password: *b"secret\0\0"
            }
        );
    }

    #[test]
    fn parse_cli_reads_all_flags_including_config_and_token() {
        let cli = parse_cli_from(&[
            "--camera",
            "OBS",
            "--tcp",
            "6000",
            "--http",
            "7000",
            "--fps",
            "15",
            "--serial",
            "COM9",
            "--baud",
            "115200",
            "--bind",
            "0.0.0.0",
            "--assets",
            "assets",
            "--config",
            "my.toml",
            "--token",
            "secret",
            "--vnc-password",
            "abc12345",
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
}
