//! 端到端输入诊断日志。
//!
//! 本模块只提供轻量文件 logger，不向 console 输出。desktop/headless 负责
//! 根据 UI 或启动参数调用 [`configure`]，下层 crate 只按类别打结构化字段。

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::ops::{BitOr, BitOrAssign};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum DiagLevel {
    Error = 1,
    Warn = 2,
    Info = 3,
    Debug = 4,
    Trace = 5,
}

impl DiagLevel {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "error" => Some(Self::Error),
            "warn" | "warning" => Some(Self::Warn),
            "info" => Some(Self::Info),
            "debug" => Some(Self::Debug),
            "trace" => Some(Self::Trace),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagCategory(u32);

impl DiagCategory {
    pub const INPUT: Self = Self(1 << 0);
    pub const POINTER: Self = Self(1 << 1);
    pub const KEYBOARD: Self = Self(1 << 2);
    pub const QUEUE: Self = Self(1 << 3);
    pub const SERIAL: Self = Self(1 << 4);
    pub const LIFECYCLE: Self = Self(1 << 5);
    pub const ALL: Self = Self(
        Self::INPUT.0
            | Self::POINTER.0
            | Self::KEYBOARD.0
            | Self::QUEUE.0
            | Self::SERIAL.0
            | Self::LIFECYCLE.0,
    );

    pub fn contains(self, category: Self) -> bool {
        self.0 & category.0 != 0
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "input" => Some(Self::INPUT),
            "pointer" | "mouse" => Some(Self::POINTER),
            "keyboard" | "key" => Some(Self::KEYBOARD),
            "queue" => Some(Self::QUEUE),
            "serial" | "ch9329" => Some(Self::SERIAL),
            "lifecycle" => Some(Self::LIFECYCLE),
            "all" => Some(Self::ALL),
            _ => None,
        }
    }

    pub fn parse_list(value: &str) -> Result<Self, String> {
        let mut categories = Self(0);
        for part in value.split(',') {
            let trimmed = part.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some(category) = Self::parse(trimmed) else {
                return Err(trimmed.to_owned());
            };
            categories |= category;
        }
        Ok(categories)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::INPUT => "input",
            Self::POINTER => "pointer",
            Self::KEYBOARD => "keyboard",
            Self::QUEUE => "queue",
            Self::SERIAL => "serial",
            Self::LIFECYCLE => "lifecycle",
            _ => "mixed",
        }
    }
}

impl BitOr for DiagCategory {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for DiagCategory {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagConfig {
    path: PathBuf,
    level: DiagLevel,
    categories: DiagCategory,
}

impl DiagConfig {
    pub fn file(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            level: DiagLevel::Info,
            categories: DiagCategory::INPUT
                | DiagCategory::POINTER
                | DiagCategory::QUEUE
                | DiagCategory::SERIAL
                | DiagCategory::LIFECYCLE,
        }
    }

    pub fn level(mut self, level: DiagLevel) -> Self {
        self.level = level;
        self
    }

    pub fn categories(mut self, categories: DiagCategory) -> Self {
        self.categories = categories;
        self
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn configured_level(&self) -> DiagLevel {
        self.level
    }

    pub fn configured_categories(&self) -> DiagCategory {
        self.categories
    }
}

struct LoggerState {
    config: DiagConfig,
    file: File,
}

static LOGGER: OnceLock<Mutex<Option<LoggerState>>> = OnceLock::new();
static START: OnceLock<Instant> = OnceLock::new();

pub fn configure(config: DiagConfig) -> io::Result<()> {
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(config.path())?;
    let mut guard = logger()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = Some(LoggerState { config, file });
    Ok(())
}

pub fn disable() {
    let mut guard = logger()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *guard = None;
}

pub fn is_enabled(level: DiagLevel, category: DiagCategory) -> bool {
    let guard = logger()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    guard
        .as_ref()
        .is_some_and(|state| allows(&state.config, level, category))
}

pub fn log(
    level: DiagLevel,
    category: DiagCategory,
    component: &str,
    event: &str,
    fields: &[(&str, String)],
) {
    let mut guard = logger()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(state) = guard.as_mut() else {
        return;
    };
    if !allows(&state.config, level, category) {
        return;
    }

    let mut line = format!(
        "ts_ms={} mono_ms={} level={} category={} component={} event={}",
        epoch_ms(),
        START.get_or_init(Instant::now).elapsed().as_millis(),
        level.as_str(),
        category.as_str(),
        logfmt_value(component),
        logfmt_value(event)
    );
    for (key, value) in fields {
        line.push(' ');
        line.push_str(key);
        line.push('=');
        line.push_str(&logfmt_value(value));
    }
    let _ = writeln!(state.file, "{line}");
    let _ = state.file.flush();
}

fn allows(config: &DiagConfig, level: DiagLevel, category: DiagCategory) -> bool {
    level <= config.level && config.categories.contains(category)
}

fn logger() -> &'static Mutex<Option<LoggerState>> {
    LOGGER.get_or_init(|| Mutex::new(None))
}

fn epoch_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

fn logfmt_value(value: &str) -> String {
    if !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/' | b'#')
        })
    {
        return value.to_owned();
    }

    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for ch in value.chars() {
        match ch {
            '"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped.push('"');
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logfmt_quotes_values_with_spaces() {
        assert_eq!(logfmt_value("plain-value"), "plain-value");
        assert_eq!(logfmt_value("drag move"), "\"drag move\"");
        assert_eq!(logfmt_value("quote\""), "\"quote\\\"\"");
    }
}
