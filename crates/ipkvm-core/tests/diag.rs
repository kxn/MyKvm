use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use ipkvm_core::diag::{self, DiagCategory, DiagConfig, DiagLevel};

static TEST_LOCK: Mutex<()> = Mutex::new(());

fn temp_log_path(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "ipkvm-diag-{name}-{}-{suffix}.log",
        std::process::id()
    ))
}

#[test]
fn file_logger_writes_stable_logfmt_fields() {
    let _guard = TEST_LOCK.lock().unwrap();
    let path = temp_log_path("logfmt");
    diag::configure(
        DiagConfig::file(path.clone())
            .level(DiagLevel::Trace)
            .categories(DiagCategory::ALL),
    )
    .unwrap();

    diag::log(
        DiagLevel::Trace,
        DiagCategory::POINTER,
        "desktop.input",
        "pointer_abs_send",
        &[
            ("trace", "desk-42".into()),
            ("seq", "187".into()),
            ("mask", "0x01".into()),
            ("note", "drag move".into()),
        ],
    );
    diag::disable();

    let body = fs::read_to_string(&path).unwrap();
    assert!(body.contains("level=trace"));
    assert!(body.contains("category=pointer"));
    assert!(body.contains("component=desktop.input"));
    assert!(body.contains("event=pointer_abs_send"));
    assert!(body.contains("trace=desk-42"));
    assert!(body.contains("seq=187"));
    assert!(body.contains("mask=0x01"));
    assert!(body.contains("note=\"drag move\""));
}

#[test]
fn logger_filters_by_level_and_category() {
    let _guard = TEST_LOCK.lock().unwrap();
    let path = temp_log_path("filter");
    diag::configure(
        DiagConfig::file(path.clone())
            .level(DiagLevel::Info)
            .categories(DiagCategory::POINTER),
    )
    .unwrap();

    diag::log(
        DiagLevel::Debug,
        DiagCategory::POINTER,
        "desktop.input",
        "debug_pointer",
        &[("seq", "1".into())],
    );
    diag::log(
        DiagLevel::Info,
        DiagCategory::SERIAL,
        "core.serial",
        "serial_enqueue",
        &[("seq", "2".into())],
    );
    diag::log(
        DiagLevel::Info,
        DiagCategory::POINTER,
        "desktop.input",
        "info_pointer",
        &[("seq", "3".into())],
    );
    diag::disable();

    let body = fs::read_to_string(&path).unwrap();
    assert!(!body.contains("debug_pointer"));
    assert!(!body.contains("serial_enqueue"));
    assert!(body.contains("info_pointer"));
    assert!(body.contains("seq=3"));
}

#[test]
fn parses_levels_and_category_lists_for_frontends() {
    assert_eq!(DiagLevel::parse("trace"), Some(DiagLevel::Trace));
    assert_eq!(DiagLevel::parse("WARN"), Some(DiagLevel::Warn));
    assert_eq!(DiagLevel::parse("verbose"), None);

    let categories = DiagCategory::parse_list("pointer, queue, lifecycle").unwrap();
    assert!(categories.contains(DiagCategory::POINTER));
    assert!(categories.contains(DiagCategory::QUEUE));
    assert!(categories.contains(DiagCategory::LIFECYCLE));
    assert!(!categories.contains(DiagCategory::SERIAL));
    assert!(DiagCategory::parse_list("pointer,nope").is_err());
}
