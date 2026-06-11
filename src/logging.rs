use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error,
    Warn,
    Info,
    Debug,
}

impl Level {
    pub fn parse(s: &str) -> Option<Level> {
        match s.to_ascii_lowercase().as_str() {
            "error" => Some(Level::Error),
            "warn" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
        }
    }
}

fn format_line(timestamp: &str, level: Level, component: &str, msg: &str) -> String {
    format!("{timestamp} [{}] [{component}] {msg}", level.as_str())
}

fn rotate_if_oversized(path: &Path, max_bytes: u64) -> std::io::Result<bool> {
    match std::fs::metadata(path) {
        Ok(meta) if meta.len() > max_bytes => {
            std::fs::rename(path, path.with_extension("log.old"))?;
            Ok(true)
        }
        Ok(_) => Ok(false),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub fn log_path() -> PathBuf {
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("mplug")
        .join("mplug.log")
}

struct Logger {
    max_level: Level,
    file: Option<Mutex<File>>,
}

static LOGGER: OnceLock<Logger> = OnceLock::new();

/// Called once by the daemon. Reads MPLUG_LOG for the level, rotates an
/// oversized log, and opens the file sink. CLI subcommands never call this,
/// so their logging falls back to stderr only.
pub fn init() {
    let max_level = std::env::var("MPLUG_LOG")
        .ok()
        .and_then(|v| Level::parse(&v))
        .unwrap_or(Level::Info);

    let path = log_path();
    let file = (|| {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        rotate_if_oversized(&path, MAX_LOG_BYTES)?;
        OpenOptions::new().create(true).append(true).open(&path)
    })()
    .map_err(|e| eprintln!("mplug: cannot open log file {}: {e}", path.display()))
    .ok();

    let _ = LOGGER.set(Logger {
        max_level,
        file: file.map(Mutex::new),
    });
}

pub fn debug_enabled() -> bool {
    LOGGER.get().is_some_and(|l| l.max_level >= Level::Debug)
}

pub fn log(level: Level, component: &str, msg: &str) {
    let logger = LOGGER.get();
    let max_level = logger.map_or(Level::Info, |l| l.max_level);
    if level > max_level {
        return;
    }

    let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let line = format_line(&timestamp, level, component, msg);

    eprintln!("{line}");
    if let Some(Ok(mut f)) = logger.and_then(|l| l.file.as_ref()).map(|m| m.lock()) {
        let _ = writeln!(f, "{line}");
    }
}

/// `mplug log` subcommand: print the last `lines` lines, optionally keep
/// following the file like `tail -f`.
pub fn show_log(lines: usize, follow: bool) -> anyhow::Result<()> {
    use std::io::{BufRead, BufReader, Seek, SeekFrom};

    let path = log_path();
    if !path.exists() {
        println!(
            "no log file at {} (is the daemon running with this mplug version?)",
            path.display()
        );
        return Ok(());
    }

    let mut file = File::open(&path)?;
    let mut reader = BufReader::new(&mut file);
    let mut tail: std::collections::VecDeque<String> = std::collections::VecDeque::new();
    let mut line = String::new();
    loop {
        line.clear();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        if tail.len() == lines {
            tail.pop_front();
        }
        tail.push_back(line.trim_end().to_string());
    }
    for l in &tail {
        println!("{l}");
    }

    if follow {
        let mut pos = file.seek(SeekFrom::End(0))?;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let len = std::fs::metadata(&path)?.len();
            if len < pos {
                // rotated or truncated: start over from the beginning
                pos = 0;
            }
            if len > pos {
                let mut f = File::open(&path)?;
                f.seek(SeekFrom::Start(pos))?;
                let mut chunk = String::new();
                std::io::Read::read_to_string(&mut f, &mut chunk)?;
                pos = len;
                print!("{chunk}");
                use std::io::Write as IoWrite;
                let _ = std::io::stdout().flush();
            }
        }
    }
    Ok(())
}

#[macro_export]
macro_rules! log_error {
    ($component:expr, $($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Error, $component, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_warn {
    ($component:expr, $($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Warn, $component, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_info {
    ($component:expr, $($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Info, $component, &format!($($arg)*))
    };
}

#[macro_export]
macro_rules! log_debug {
    ($component:expr, $($arg:tt)*) => {
        $crate::logging::log($crate::logging::Level::Debug, $component, &format!($($arg)*))
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parse_accepts_known_names_case_insensitively() {
        assert_eq!(Level::parse("error"), Some(Level::Error));
        assert_eq!(Level::parse("WARN"), Some(Level::Warn));
        assert_eq!(Level::parse("Info"), Some(Level::Info));
        assert_eq!(Level::parse("debug"), Some(Level::Debug));
        assert_eq!(Level::parse("verbose"), None);
        assert_eq!(Level::parse(""), None);
    }

    #[test]
    fn level_ordering_lets_max_level_filter() {
        // A message is emitted when its level <= the configured max level.
        assert!(Level::Error <= Level::Info);
        assert!(Level::Info <= Level::Info);
        assert!(Level::Debug > Level::Info);
    }

    #[test]
    fn format_line_has_timestamp_level_component_message() {
        let line = format_line("2026-06-11 13:24:05", Level::Error, "clock-calendar", "boom");
        assert_eq!(line, "2026-06-11 13:24:05 [ERROR] [clock-calendar] boom");
    }

    #[test]
    fn rotate_renames_oversized_log_and_keeps_small_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mplug.log");

        std::fs::write(&path, vec![b'x'; 64]).unwrap();
        assert!(!rotate_if_oversized(&path, 1024).unwrap());
        assert!(path.exists());

        std::fs::write(&path, vec![b'x'; 2048]).unwrap();
        assert!(rotate_if_oversized(&path, 1024).unwrap());
        assert!(!path.exists());
        assert_eq!(
            std::fs::read(dir.path().join("mplug.log.old")).unwrap().len(),
            2048
        );
    }

    #[test]
    fn rotate_is_a_noop_when_log_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mplug.log");
        assert!(!rotate_if_oversized(&path, 1024).unwrap());
    }
}
