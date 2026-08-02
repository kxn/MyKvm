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

/// 安装系统字体到 egui 上下文；找不到字体时只告警，不阻止窗口启动。
pub fn install(ctx: &eframe::egui::Context) {
    let Some((_path, bytes)) = first_existing_font() else {
        eprintln!("warning: 未找到可用的系统字体，界面文字将无法渲染");
        return;
    };

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
}
