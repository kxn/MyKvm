use include_dir::{Dir, include_dir};

static PROJECT_ASSETS: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web");
static NOVNC_ASSETS: Dir<'static> =
    include_dir!("$CARGO_MANIFEST_DIR/../../third_party/novnc/1.7.0");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct WebAsset {
    bytes: &'static [u8],
    content_type: &'static str,
}

impl WebAsset {
    pub(crate) fn bytes(self) -> &'static [u8] {
        self.bytes
    }

    pub(crate) fn content_type(self) -> &'static str {
        self.content_type
    }
}

pub(crate) fn find_asset(request_path: &str) -> Option<WebAsset> {
    let path = canonical_path(request_path)?;
    let (directory, asset_path) = match path {
        "" | "index.html" => (&PROJECT_ASSETS, "index.html"),
        "assets/app.css" => (&PROJECT_ASSETS, "app.css"),
        "assets/app.js" => (&PROJECT_ASSETS, "app.js"),
        "licenses" | "licenses/" => (&PROJECT_ASSETS, "licenses.html"),
        path => (&NOVNC_ASSETS, path.strip_prefix("vendor/novnc/")?),
    };
    if asset_path.is_empty() {
        return None;
    }
    let file = directory.get_file(asset_path)?;
    Some(WebAsset {
        bytes: file.contents(),
        content_type: content_type_for(asset_path),
    })
}

fn canonical_path(path: &str) -> Option<&str> {
    if !path.starts_with('/') || path.contains('\\') || path.contains('\0') {
        return None;
    }
    let path = path.strip_prefix('/')?;
    if path.is_empty() {
        return Some(path);
    }
    let segments: Vec<_> = path.split('/').collect();
    for (index, segment) in segments.iter().enumerate() {
        let is_allowed_trailing_separator =
            index == segments.len() - 1 && segment.is_empty() && path == "licenses/";
        if !is_allowed_trailing_separator
            && (segment.is_empty() || *segment == "." || *segment == "..")
        {
            return None;
        }
    }
    Some(path)
}

fn content_type_for(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension);
    match extension {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("md" | "txt") | None => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serves_the_fixed_novnc_module_with_javascript_mime() {
        let asset = find_asset("/vendor/novnc/core/rfb.js").unwrap();

        assert_eq!(asset.content_type(), "text/javascript; charset=utf-8");
        assert!(
            std::str::from_utf8(asset.bytes())
                .unwrap()
                .contains("export default class RFB")
        );
    }

    #[test]
    fn embeds_the_expected_novnc_package_version() {
        let asset = find_asset("/vendor/novnc/package.json").unwrap();
        let package = std::str::from_utf8(asset.bytes()).unwrap();

        assert!(package.contains("\"name\": \"@novnc/novnc\""));
        assert!(package.contains("\"version\": \"1.7.0\""));
        assert!(package.contains("\"dependencies\": {}"));
    }

    #[test]
    fn maps_supported_content_types_without_sniffing() {
        assert_eq!(content_type_for("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type_for("app.css"), "text/css; charset=utf-8");
        assert_eq!(content_type_for("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(
            content_type_for("module.mjs"),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(content_type_for("data.json"), "application/json");
        assert_eq!(content_type_for("LICENSE.txt"), "text/plain; charset=utf-8");
        assert_eq!(content_type_for("AUTHORS"), "text/plain; charset=utf-8");
        assert_eq!(content_type_for("frame.bin"), "application/octet-stream");
    }

    #[test]
    fn rejects_non_canonical_or_unsafe_paths() {
        for path in [
            "",
            "vendor/novnc/core/rfb.js",
            "//vendor/novnc/core/rfb.js",
            "/vendor//novnc/core/rfb.js",
            "/vendor/novnc/./core/rfb.js",
            "/vendor/novnc/../package.json",
            "/vendor\\novnc\\core\\rfb.js",
            "/vendor/novnc/core/rfb.js\0",
        ] {
            assert!(find_asset(path).is_none(), "{path:?} should be rejected");
        }
    }

    #[test]
    fn does_not_serve_unknown_or_internal_project_files() {
        assert!(find_asset("/vendor/novnc/missing.js").is_none());
        assert!(find_asset("/assets/README.md").is_none());
    }

    #[test]
    fn serves_the_project_console_and_license_page() {
        let index = find_asset("/").unwrap();
        let index_text = std::str::from_utf8(index.bytes()).unwrap();
        assert_eq!(index.content_type(), "text/html; charset=utf-8");
        assert!(index_text.contains("lang=\"zh-CN\""));
        assert!(index_text.contains("data-connection-state=\"connecting\""));

        let script = std::str::from_utf8(find_asset("/assets/app.js").unwrap().bytes()).unwrap();
        assert!(script.contains("from \"/vendor/novnc/core/rfb.js\""));
        assert!(script.contains("next.scaleViewport = true"));
        assert!(script.contains("next.resizeSession = false"));

        let licenses = std::str::from_utf8(find_asset("/licenses/").unwrap().bytes()).unwrap();
        assert!(licenses.contains("第三方组件与许可证"));
    }
}
