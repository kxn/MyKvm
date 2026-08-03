//! 剪贴板读取薄层（粘贴文本用；系统实现走 arboard）。

/// 剪贴板读取接口：生产用系统剪贴板，测试注入 fake。
pub trait ClipboardReader: Send + Sync {
    fn read_text(&self) -> Result<String, String>;
}

/// 系统剪贴板（arboard）。
pub struct SystemClipboard;

impl ClipboardReader for SystemClipboard {
    fn read_text(&self) -> Result<String, String> {
        arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|error| error.to_string())
    }
}

/// 读取剪贴板文本（空文本返回 Ok("")，调用方决定提示文案）。
pub fn read_clipboard_text(reader: &dyn ClipboardReader) -> Result<String, String> {
    reader.read_text()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeClipboard;

    impl ClipboardReader for FakeClipboard {
        fn read_text(&self) -> Result<String, String> {
            Ok("hello".into())
        }
    }

    struct FailingClipboard;

    impl ClipboardReader for FailingClipboard {
        fn read_text(&self) -> Result<String, String> {
            Err("clipboard locked".into())
        }
    }

    #[test]
    fn reader_trait_returns_text() {
        assert_eq!(read_clipboard_text(&FakeClipboard), Ok("hello".into()));
    }

    #[test]
    fn reader_error_propagates() {
        assert_eq!(
            read_clipboard_text(&FailingClipboard),
            Err("clipboard locked".into())
        );
    }
}
