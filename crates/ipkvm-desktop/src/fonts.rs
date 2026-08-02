use std::path::PathBuf;

/// 返回当前平台的系统字体候选路径，优先选择覆盖中文的字体。
///
/// 字体文件来自操作系统安装目录，应用不随二进制再分发字体文件，
/// 因此不引入 OFL/Ubuntu Font Licence 等捆绑字体许可证义务。
pub fn system_font_candidates() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    let candidates = {
        let windows_dir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".to_owned());
        vec![
            PathBuf::from(&windows_dir).join("Fonts\\msyh.ttc"),
            PathBuf::from(&windows_dir).join("Fonts\\msyhbd.ttc"),
            PathBuf::from(&windows_dir).join("Fonts\\simhei.ttf"),
            PathBuf::from(&windows_dir).join("Fonts\\simsun.ttc"),
            PathBuf::from(&windows_dir).join("Fonts\\arial.ttf"),
        ]
    };

    #[cfg(target_os = "linux")]
    let candidates = vec![
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc"),
        PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"),
    ];

    #[cfg(target_os = "macos")]
    let candidates = vec![
        PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
        PathBuf::from("/System/Library/Fonts/Supplemental/Arial.ttf"),
        PathBuf::from("/System/Library/Fonts/Helvetica.ttc"),
    ];

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    let candidates: Vec<PathBuf> = Vec::new();

    candidates
}

/// 返回第一个可读取的系统字体（路径 + 字节）。
pub fn first_existing_font() -> Option<(PathBuf, Vec<u8>)> {
    system_font_candidates()
        .into_iter()
        .find_map(|path| std::fs::read(&path).ok().map(|bytes| (path, bytes)))
}

/// 内置兜底字体（Roboto-Regular，Apache-2.0，许可证文本见 assets/ROBOTO-LICENSE.txt）。
///
/// 任何环境都保证至少一个字体可用，避免 egui 空字体集渲染文本时 panic。
pub fn fallback_font_bytes() -> &'static [u8] {
    include_bytes!("../assets/Roboto-Regular.ttf")
}

/// 按候选顺序解析第一个可读取的字体字节。
pub fn resolve_font_bytes(candidates: Vec<PathBuf>) -> Option<Vec<u8>> {
    candidates
        .into_iter()
        .find_map(|path| std::fs::read(&path).ok())
}

/// 安装字体到 egui 上下文：系统字体优先，找不到时用内置兜底字体。
pub fn install(ctx: &eframe::egui::Context) {
    let bytes = resolve_font_bytes(system_font_candidates())
        .unwrap_or_else(|| fallback_font_bytes().to_vec());

    let mut fonts = eframe::egui::FontDefinitions::empty();
    fonts.font_data.insert(
        "system".to_owned(),
        eframe::egui::FontData::from_owned(bytes).into(),
    );
    for family in [
        eframe::egui::FontFamily::Proportional,
        eframe::egui::FontFamily::Monospace,
    ] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "system".to_owned());
    }
    ctx.set_fonts(fonts);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_are_non_empty_on_supported_platforms() {
        assert!(!system_font_candidates().is_empty());
    }

    #[test]
    fn first_existing_font_reads_non_empty_bytes_when_available() {
        if let Some((path, bytes)) = first_existing_font() {
            assert!(system_font_candidates().contains(&path));
            assert!(!bytes.is_empty());
        }
    }

    #[test]
    fn fallback_font_is_embedded_and_parseable() {
        let bytes = fallback_font_bytes();
        assert!(bytes.len() > 100_000);
        assert_eq!(&bytes[..4], &[0x00, 0x01, 0x00, 0x00]);
    }

    #[test]
    fn resolve_font_bytes_prefers_first_readable_candidate() {
        let bytes = fallback_font_bytes().to_vec();
        let dir = std::env::temp_dir().join(format!("my-ipkvm-font-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("first.ttf");
        let second = dir.join("second.ttf");
        std::fs::write(&first, &bytes).unwrap();
        std::fs::write(&second, b"not a font").unwrap();

        let resolved = resolve_font_bytes(vec![first.clone(), second]).unwrap();

        assert_eq!(resolved, bytes);
        let _ = std::fs::remove_dir_all(dir);
    }
}
