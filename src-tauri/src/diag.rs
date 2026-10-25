//! Diagnostics: log setup, redaction of secrets and configuration values, the logging
//! transport wrapper, and the commands behind the Diagnostics screen.
//!
//! Redaction rules (mandatory):
//! - Keyword / PIN: `:sec_*` commands (except `:sec_status`) are never shown, nor their
//!   replies. The pairing PIN is never passed to the log.
//! - Configuration values: `:cfg_getcfg` replies and `:cfg_setparam:` payloads are
//!   summarised as length plus the first 32 bytes with every value masked (`id:*`).
//! - The device debug log stream (`.log:1`) is logged by size only.

use std::io;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

use crate::transport::Transport;

/// What the bytes currently on the wire are, for redaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ctx {
    Normal = 0,
    Secret = 1,
    Config = 2,
    Stream = 3,
}

static CTX: AtomicU8 = AtomicU8::new(0);

fn current() -> Ctx {
    match CTX.load(Ordering::Relaxed) {
        1 => Ctx::Secret,
        2 => Ctx::Config,
        3 => Ctx::Stream,
        _ => Ctx::Normal,
    }
}

/// How a command and its reply must be logged.
pub fn classify_command(bytes: &[u8]) -> Ctx {
    let text = String::from_utf8_lossy(bytes);
    let t = text.trim_start_matches(['\r', '\n', ' ']).to_ascii_lowercase();
    if t.starts_with(":sec_") && !t.starts_with(":sec_status") {
        Ctx::Secret
    } else if t.starts_with(":cfg_getcfg") || t.starts_with(":cfg_setparam") {
        Ctx::Config
    } else if t.starts_with(".log:1") {
        Ctx::Stream
    } else {
        Ctx::Normal
    }
}

pub const PREVIEW_BYTES: usize = 256;
pub const MASKED_PREVIEW_BYTES: usize = 32;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn ascii_preview(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| match b {
            b'\r' => "\\r".to_string(),
            b'\n' => "\\n".to_string(),
            0x20..=0x7e => (b as char).to_string(),
            _ => ".".to_string(),
        })
        .collect()
}

/// Keeps parameter IDs and tags, replaces every value with `*`.
pub fn mask_values(text: &str) -> String {
    let text = text
        .trim_start_matches(|c| c == '\r' || c == '\n')
        .strip_prefix(":cfg_setparam:")
        .map(|rest| rest.to_string())
        .unwrap_or_else(|| text.to_string());
    let mut out = String::new();
    for piece in text.split_inclusive(['\r', '\n', ';']) {
        let (body, sep) = match piece.char_indices().last() {
            Some((i, c)) if matches!(c, '\r' | '\n' | ';') => (&piece[..i], &piece[i..]),
            _ => (piece, ""),
        };
        let sep = sep.replace('\r', "\\r").replace('\n', "\\n");
        match body.split_once(':') {
            Some((id, _)) if !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) => {
                out.push_str(&format!("{id}:*{sep}"))
            }
            _ if body.starts_with('<') && body.ends_with('>') => out.push_str(&format!("{body}{sep}")),
            _ if body.chars().all(|c| c.is_ascii_digit()) => out.push_str(&format!("{body}{sep}")),
            _ => out.push_str(&format!("*{sep}")),
        }
    }
    out
}

/// One log line for `len` bytes travelling in `dir` ("TX"/"RX").
pub fn describe_io(dir: &str, bytes: &[u8], ctx: Ctx) -> String {
    let total = bytes.len();
    match ctx {
        Ctx::Secret => format!("{dir} secret exchange redacted"),
        Ctx::Stream => format!("{dir} {total} bytes of device log (content not shown)"),
        Ctx::Config => {
            let head = &bytes[..total.min(MASKED_PREVIEW_BYTES)];
            let more = if total > head.len() { " ..." } else { "" };
            format!(
                "{dir} {total} bytes of configuration data, first {} bytes masked: {}{more}",
                head.len(),
                mask_values(&String::from_utf8_lossy(head))
            )
        }
        Ctx::Normal => {
            let head = &bytes[..total.min(PREVIEW_BYTES)];
            let more = if total > head.len() { " ..." } else { "" };
            format!("{dir} {total} bytes: {} | \"{}\"{more}", hex(head), ascii_preview(head))
        }
    }
}

/// Removes anything that looks like a keyword from free text (frontend messages).
pub fn redact_text(text: &str) -> String {
    let lower = text.to_ascii_lowercase();
    if let Some(i) = lower.find(":sec_") {
        return format!("{}[redacted]", &text[..i]);
    }
    text.to_string()
}

// ---------------------------------------------------------------- logging transport

/// Wraps a transport: logs every write and read, and aborts blocked reads on cancel.
pub struct LoggedTransport {
    inner: Box<dyn Transport>,
    link: String,
    cancel: Arc<AtomicBool>,
}

impl LoggedTransport {
    pub fn new(inner: Box<dyn Transport>, link: &str, cancel: Arc<AtomicBool>) -> Self {
        log::debug!("transport[{link}]: attached");
        Self { inner, link: link.to_string(), cancel }
    }
}

impl Drop for LoggedTransport {
    fn drop(&mut self) {
        log::info!("transport[{}]: closed", self.link);
    }
}

/// Longest a read blocks before the cancel flag is checked.
const CANCEL_SLICE: Duration = Duration::from_millis(100);

impl Transport for LoggedTransport {
    fn write_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let ctx = classify_command(bytes);
        CTX.store(ctx as u8, Ordering::Relaxed);
        log::debug!("transport[{}]: {}", self.link, describe_io("TX", bytes, ctx));
        let r = self.inner.write_all(bytes);
        if let Err(e) = &r {
            log::error!("transport[{}]: write failed: {e}", self.link);
        }
        r
    }

    fn read(&mut self, buf: &mut [u8], deadline: Instant) -> io::Result<usize> {
        loop {
            if self.cancel.load(Ordering::SeqCst) {
                log::info!("transport[{}]: read aborted, connection attempt cancelled", self.link);
                return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
            }
            let slice = deadline.min(Instant::now() + CANCEL_SLICE);
            match self.inner.read(buf, slice) {
                Ok(0) if Instant::now() < deadline => continue,
                Ok(0) => return Ok(0),
                Ok(n) => {
                    log::debug!("transport[{}]: {}", self.link, describe_io("RX", &buf[..n], current()));
                    return Ok(n);
                }
                Err(e) => {
                    log::error!("transport[{}]: read failed: {e}", self.link);
                    return Err(e);
                }
            }
        }
    }

    fn clear_input(&mut self) -> io::Result<()> {
        log::trace!("transport[{}]: clear input", self.link);
        self.inner.clear_input()
    }
}

// ---------------------------------------------------------------- setup

pub const LOG_FILE_NAME: &str = "fmconfig";

pub fn plugin<R: Runtime>() -> tauri::plugin::TauriPlugin<R> {
    let level = if cfg!(debug_assertions) { log::LevelFilter::Debug } else { log::LevelFilter::Info };
    tauri_plugin_log::Builder::new()
        .level(level)
        .clear_targets()
        .targets([
            Target::new(TargetKind::Stdout),
            Target::new(TargetKind::LogDir { file_name: Some(LOG_FILE_NAME.into()) }),
        ])
        .max_file_size(2_000_000)
        .rotation_strategy(RotationStrategy::KeepSome(4))
        // Dependencies (window system, async runtime) are only interesting when they warn.
        .filter(|m| m.target().starts_with("fmconfig_lib") || m.target().starts_with("webview") || m.level() <= log::Level::Warn)
        .build()
}

pub fn log_dir<R: Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, String> {
    app.path().app_log_dir().map_err(|e| e.to_string())
}

fn log_file<R: Runtime>(app: &AppHandle<R>) -> Result<std::path::PathBuf, String> {
    Ok(log_dir(app)?.join(format!("{LOG_FILE_NAME}.log")))
}

fn read_tail(path: &std::path::Path, max_bytes: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return String::new() };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(max_bytes);
    let _ = f.seek(SeekFrom::Start(start));
    let mut buf = Vec::new();
    let _ = f.read_to_end(&mut buf);
    let text = String::from_utf8_lossy(&buf).into_owned();
    // Drop the partial first line.
    if start > 0 {
        text.split_once('\n').map(|(_, rest)| rest.to_string()).unwrap_or_default()
    } else {
        text
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogInfo {
    pub dir: String,
    pub file: String,
}

#[tauri::command]
pub fn diag_log_info(app: AppHandle) -> Result<LogInfo, String> {
    Ok(LogInfo { dir: log_dir(&app)?.display().to_string(), file: log_file(&app)?.display().to_string() })
}

/// The newest part of the app log (already redacted when written).
#[tauri::command]
pub fn diag_tail(app: AppHandle, max_bytes: Option<u64>) -> Result<String, String> {
    Ok(read_tail(&log_file(&app)?, max_bytes.unwrap_or(200_000).min(2_000_000)))
}

/// Frontend console.error / console.warn / unhandled rejections land in the Rust log.
#[tauri::command]
pub fn diag_frontend_log(level: String, message: String) {
    let message: String = redact_text(&message).chars().take(2000).collect();
    match level.as_str() {
        "error" => log::error!(target: "fmconfig_lib::frontend", "{message}"),
        "warn" => log::warn!(target: "fmconfig_lib::frontend", "{message}"),
        _ => log::info!(target: "fmconfig_lib::frontend", "{message}"),
    }
}

#[tauri::command]
pub fn diag_open_log_dir(app: AppHandle) -> Result<(), String> {
    let dir = log_dir(&app)?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener).arg(&dir).spawn().map(|_| ()).map_err(|e| format!("Could not open {}: {e}", dir.display()))
}

/// Writes the diagnostics report: app version, OS, transport info and the redacted log.
#[tauri::command]
pub fn diag_export(app: AppHandle, conn: tauri::State<'_, crate::connection::Connection>, path: String) -> Result<(), String> {
    let status = conn.status();
    let mut text = String::new();
    text.push_str(&format!("FM Config diagnostics\napp version: {}\n", app.package_info().version));
    text.push_str(&format!("os: {} {}\n", std::env::consts::OS, std::env::consts::ARCH));
    text.push_str(&format!("build: {}\n", if cfg!(debug_assertions) { "debug" } else { "release" }));
    text.push_str(&format!(
        "connection: state={:?} port={} kind={:?}\n",
        status.state,
        status.port.as_deref().unwrap_or("-"),
        status.kind
    ));
    text.push_str("\nNote: keywords, PINs and configuration values are never written to the log.\n\n--- log ---\n");
    text.push_str(&read_tail(&log_file(&app)?, 1_500_000));
    std::fs::write(&path, text).map_err(|e| format!("Could not save {path}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_commands_are_never_described() {
        assert_eq!(classify_command(b":sec_login:hunter2\r"), Ctx::Secret);
        assert_eq!(classify_command(b":sec_setkey:abc\r"), Ctx::Secret);
        assert_eq!(classify_command(b":sec_status\r"), Ctx::Normal);
        let line = describe_io("TX", b":sec_login:hunter2\r", Ctx::Secret);
        assert!(!line.contains("hunter2") && !line.contains("sec_login"));
        let reply = describe_io("RX", b"<SECSTAT>1,1,1,0,3\r", Ctx::Secret);
        assert!(!reply.contains("SECSTAT"));
    }

    #[test]
    fn setparam_values_are_masked() {
        let cmd = b":cfg_setparam:10:topsecret;2001:internet.example;11:12\r";
        assert_eq!(classify_command(cmd), Ctx::Config);
        let line = describe_io("TX", cmd, Ctx::Config);
        assert!(!line.contains("topsecret") && !line.contains("internet"), "{line}");
        assert!(line.contains("10:*") && line.contains("2001:*"), "{line}");
        assert!(line.contains(&format!("{} bytes", cmd.len())));
    }

    #[test]
    fn config_reply_is_summarised() {
        let reply = b"<CFG>\r\n1:supersecretvalue\r\n10:keyword99\r\n";
        let line = describe_io("RX", reply, Ctx::Config);
        assert!(!line.contains("supersecret") && !line.contains("keyword99"), "{line}");
        assert!(line.contains("<CFG>"));
        // Only the first 32 bytes are shown, with the total length.
        let long = vec![b'7'; 500];
        let line = describe_io("RX", &long, Ctx::Config);
        assert!(line.contains("500 bytes") && line.len() < 200, "{line}");
    }

    #[test]
    fn normal_traffic_is_hex_and_ascii_truncated() {
        let line = describe_io("TX", b":cfg_info:?\r", Ctx::Normal);
        assert!(line.contains("12 bytes") && line.contains("3a 63") && line.contains(":cfg_info:?\\r"));
        let long = vec![b'a'; 1000];
        let line = describe_io("RX", &long, Ctx::Normal);
        assert!(line.contains("1000 bytes") && line.ends_with("..."));
        assert!(line.len() < 1200);
    }

    #[test]
    fn stream_is_size_only() {
        assert_eq!(classify_command(b".log:1\r"), Ctx::Stream);
        let line = describe_io("RX", b"secret debug text", Ctx::Stream);
        assert!(!line.contains("debug text"));
    }

    #[test]
    fn frontend_text_loses_keyword_commands() {
        assert_eq!(redact_text("sent :sec_login:abc failed"), "sent [redacted]");
        assert_eq!(redact_text("plain"), "plain");
    }
}
