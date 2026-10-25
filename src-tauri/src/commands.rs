//! Device commands and session flow from docs/spec/03-commands.md (text protocol).

use std::collections::BTreeMap;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Serialize, Serializer};

use crate::cfg::Entry;
use crate::device::{DeviceError, Session};
use crate::framing::{ascii, text_command, TextCodec, CONFIG_END_MARKER, CR};

const LOG_OFF: &str = ".log:0";
const INFO: &str = ":cfg_info:?";
const CONNECT: &str = ":cfg_connect";
const CONNECT_REPLY: &[u8] = b"<CFG_CONNECT>\r";
const DISCONNECT: &str = ":cfg_disconnect";
const SEC_STATUS: &str = ":sec_status";
const SEC_LOGIN: &str = ":sec_login:";
const GET_CONFIG: &str = ":cfg_getcfg";
const SET_PARAM: &str = ":cfg_setparam:";
const SAVE: &str = ":cfg_save";
const DEFAULTS: &str = ":cfg_default";
const DEFAULTS_DONE: &str = "<DEFAULT_CFG_END>";
const REBOOT: &str = ".reset";
const ASSET_INFO: &str = "@asset_send:asset info";
const ASSET_VERSION_PREFIX: &str = "Code Version:";

/// Most bytes in one set-parameter command, prefix and CR included (03-commands §4.1).
const SET_PARAM_LIMIT: usize = 1024;
/// Info feature bit that enables the BLE-chip version query (03-commands §2).
const FEATURE_ASSET_INFO: u32 = 14;

// ---------------------------------------------------------------- device info

/// Parsed `:cfg_info:?` reply (03-commands §2).
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceInfo {
    pub firmware: Option<String>,
    pub config_version: Option<String>,
    /// e.g. `FMB1:6`.
    // TODO(spec): mapping the hardware code to a model name needs the hardware
    // table from unreleased 04-parameters.
    pub hardware_code: Option<String>,
    pub imei: Option<String>,
    /// 1 or 2: key 4 is `0` for SIM1 and anything else for SIM2.
    pub active_sim: Option<u8>,
    /// A status word such as `WORKING` or `NEED PIN`, or other text shown verbatim.
    pub sim1_status: String,
    pub sim2_status: String,
    /// For the active SIM.
    pub sim_pin_retries: Option<i64>,
    pub keyword_supported: bool,
    pub keyword_authorized: bool,
    pub sim_lock: bool,
    pub record_reading: bool,
    pub spec_id: i64,
    /// Kept as a string in JSON: JavaScript numbers can't hold 64 bits.
    #[serde(serialize_with = "as_string")]
    pub features: u64,
    /// Free text, for display only.
    pub firmware_revision: Option<String>,
    /// Option tokens, only meaningful to parameter definitions (04-parameters).
    pub hardware_options: Vec<String>,
    pub compressed_firmware_upload: bool,
    pub ble_chip_version: Option<String>,
}

fn as_string<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

impl DeviceInfo {
    pub fn has_feature(&self, bit: u32) -> bool {
        bit < 64 && self.features & (1 << bit) != 0
    }
}

/// Keys whose values must be decimal integers; lines that don't parse are ignored.
const INTEGER_KEYS: [u32; 7] = [4, 7, 8, 9, 10, 11, 16];

pub fn parse_info(text: &str) -> Result<DeviceInfo, String> {
    let mut values = BTreeMap::new();
    for line in text.split('\r') {
        let Some(rest) = line.trim_start_matches('\n').strip_prefix("cfg_info:") else {
            continue;
        };
        let Some((key, value)) = rest.split_once(':') else {
            continue;
        };
        let Ok(key) = key.parse::<u32>() else {
            continue;
        };
        let printable = value.chars().all(|c| c.is_ascii() && !c.is_ascii_control());
        let integer_ok = !INTEGER_KEYS.contains(&key) || value.parse::<i64>().is_ok();
        // Keys the spec doesn't list (15, for one) are ignored.
        let listed = key <= 16 && key != 15;
        if printable && integer_ok && listed {
            values.insert(key, value.to_string());
        }
    }
    let present = |key: u32| values.get(&key).filter(|v| !v.is_empty()).is_some();
    if !present(5) || !present(6) {
        return Err("not a Teltonika tracker (no valid info reply)".into());
    }
    let text = |key: u32| values.get(&key).cloned();
    let int = |key: u32| values.get(&key).and_then(|v| v.parse::<i64>().ok());
    Ok(DeviceInfo {
        firmware: text(0),
        config_version: text(1),
        hardware_code: text(2),
        imei: text(3),
        active_sim: int(4).map(|n| if n == 0 { 1 } else { 2 }),
        sim1_status: text(5).unwrap_or_default(),
        sim2_status: text(6).unwrap_or_default(),
        sim_pin_retries: int(7),
        keyword_supported: int(8).is_some(),
        keyword_authorized: int(8) == Some(1),
        sim_lock: int(9) == Some(1),
        record_reading: int(10) == Some(1),
        spec_id: int(11).unwrap_or(1),
        features: text(12).and_then(|v| v.parse().ok()).unwrap_or(0),
        firmware_revision: text(13),
        hardware_options: text(14)
            .map(|v| v.split(':').filter(|t| !t.is_empty()).map(String::from).collect())
            .unwrap_or_default(),
        compressed_firmware_upload: int(16) == Some(1),
        ble_chip_version: None,
    })
}

/// Text after `Code Version:` in the asset info reply, up to the end of its line.
fn parse_asset_version(text: &str) -> Option<String> {
    let (_, rest) = text.split_once(ASSET_VERSION_PREFIX)?;
    let version = rest.split(['\r', '\n']).next()?.trim();
    (!version.is_empty()).then(|| version.to_string())
}

// ---------------------------------------------------------------- keyword

/// `<SECSTAT>r,s,a,l,n` (03-commands §6).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityStatus {
    pub last_ok: bool,
    pub secured: bool,
    pub authorized: bool,
    pub locked: bool,
    pub retries_left: i64,
}

impl SecurityStatus {
    pub fn needs_keyword(&self) -> bool {
        self.secured && !self.authorized
    }
}

pub fn parse_secstat(text: &str) -> Result<SecurityStatus, String> {
    let tag = "<secstat>";
    let start = text
        .to_ascii_lowercase()
        .find(tag)
        .ok_or("no keyword status in reply")?
        + tag.len();
    let fields: Vec<i64> = text[start..]
        .trim_end_matches(['\r', '\n'])
        .split(',')
        .map(|f| f.trim().parse::<i64>())
        .collect::<Result<_, _>>()
        .map_err(|_| "keyword status isn't five numbers")?;
    let [r, s, a, l, n] = fields[..] else {
        return Err("keyword status isn't five numbers".into());
    };
    Ok(SecurityStatus {
        last_ok: r == 1,
        secured: s == 1,
        authorized: a == 1,
        locked: l == 1,
        retries_left: n,
    })
}

fn query_security(session: &mut Session) -> Result<SecurityStatus, DeviceError> {
    log_off(session)?;
    let timeout = session.timeouts.command();
    let reply = session.request(&mut TextCodec::until(CR), SEC_STATUS, timeout, 0)?;
    parse_secstat(&reply).map_err(DeviceError::Malformed)
}

/// Sends `:sec_login:<keyword>`; the reply is a status line (03-commands §6).
/// The keyword never appears in errors or logs.
pub fn login(session: &mut Session, keyword: &str) -> Result<SecurityStatus, DeviceError> {
    // TODO(spec): allowed keyword characters and length come from unreleased
    // 04-parameters (open question 15); only what the framing can't carry is refused.
    if keyword.is_empty() || !keyword.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) {
        return Err(DeviceError::Invalid("the keyword must be printable ASCII".into()));
    }
    let timeout = session.timeouts.command();
    let command = format!("{SEC_LOGIN}{keyword}");
    let reply = session.request(&mut TextCodec::until(CR), command.as_str(), timeout, 0)?;
    let status = parse_secstat(&reply).map_err(DeviceError::Malformed)?;
    session.security = Some(status.clone());
    if status.locked {
        Err(DeviceError::Rejected("the device is locked after too many wrong keywords".into()))
    } else if !status.last_ok || !status.authorized {
        Err(DeviceError::Rejected(format!(
            "wrong keyword ({} attempts left)",
            status.retries_left
        )))
    } else {
        Ok(status)
    }
}

// ---------------------------------------------------------------- session

/// Extra wait, beyond the idle window, for a reply to `.log:0` before the line counts
/// as quiet. The reply is ignored anyway (01-transport §1.4), so this stays short.
const LOG_OFF_QUIET_CAP: Duration = Duration::from_millis(1500);

fn log_off(session: &mut Session) -> Result<(), DeviceError> {
    // The reply is ignored (01-transport §1.4); a silent device must not stall the
    // connect, so the wait is bounded by the idle window plus a short cap.
    let started = Instant::now();
    let reply = session.request_until_quiet(&text_command(LOG_OFF).map_err(DeviceError::Invalid)?, LOG_OFF_QUIET_CAP)?;
    log::debug!("connect: log off done in {} ms ({} bytes of trailing log text)", started.elapsed().as_millis(), reply.len());
    Ok(())
}

/// Runs one connect step with timing; failures become a message naming the step.
fn step<T>(
    session: &mut Session,
    name: &str,
    limit: Duration,
    run: impl FnOnce(&mut Session) -> Result<T, DeviceError>,
) -> Result<T, DeviceError> {
    log::info!("connect: {name} ...");
    let started = Instant::now();
    match run(session) {
        Ok(v) => {
            log::info!("connect: {name} ok in {} ms", started.elapsed().as_millis());
            Ok(v)
        }
        Err(e) => Err(session.step_failed(name, &e, started, limit)),
    }
}

/// Probes the device and opens a configuration session (01-transport §1.4,
/// 03-commands §0): log off, info, optional BLE-chip query, connect, keyword status.
/// The probe's closing "log on" and port re-open are skipped: we stay in session.
/// Every step is bounded (03-commands §2: 30 s for the info reply).
pub fn open_session(session: &mut Session) -> Result<(), DeviceError> {
    let timeout = session.timeouts.command();
    let total = Instant::now();
    log::info!("connect: opening session over {} (idle {} ms, command timeout {} ms)", session.link, session.timeouts.idle_ms, session.timeouts.command_ms);
    step(session, "log-off request", LOG_OFF_QUIET_CAP, log_off)?;
    let reply = step(session, "device-info request", timeout, |s| {
        s.request_until_idle(&text_command(INFO).map_err(DeviceError::Invalid)?, timeout)
    })?;
    let mut info = parse_info(&ascii(&reply)).map_err(|m| {
        log::warn!("connect: info reply rejected: {m} ({} bytes)", reply.len());
        DeviceError::Rejected(m)
    })?;
    log::info!(
        "connect: device identified: hardware {:?}, firmware {:?}, keyword supported {}, features 0x{:x}",
        info.hardware_code, info.firmware, info.keyword_supported, info.features
    );
    if info.has_feature(FEATURE_ASSET_INFO) {
        let asset = text_command(ASSET_INFO).map_err(DeviceError::Invalid)?;
        match step(session, "BLE-chip version request", timeout, |s| s.request_until_idle(&asset, timeout)) {
            Ok(reply) => info.ble_chip_version = parse_asset_version(&ascii(&reply)),
            Err(e) => log::warn!("connect: BLE-chip version unavailable: {e}"),
        }
    }
    step(session, "configuration-session request", timeout, |s| {
        s.request(&mut TextCodec::until(CONNECT_REPLY), CONNECT, timeout, 0)
    })?;
    session.farewell = Some(text_command(DISCONNECT).map_err(DeviceError::Invalid)?);
    if info.keyword_supported {
        let status = step(session, "keyword-status request", timeout, query_security)?;
        log::info!("connect: keyword status: secured {}, authorized {}, locked {}", status.secured, status.authorized, status.locked);
        session.security = Some(status);
    }
    session.info = Some(info);
    log::info!("connect: session open in {} ms", total.elapsed().as_millis());
    Ok(())
}

pub fn reboot(session: &mut Session) -> Result<(), DeviceError> {
    // TODO(spec): whether reboot needs an authorized session is open question 18.
    // The device restarts instead of ending the session, so no disconnect follows.
    session.farewell = None;
    session.send(&text_command(REBOOT).map_err(DeviceError::Invalid)?)
}

// ---------------------------------------------------------------- configuration

/// Reads every parameter the device has (03-commands §3): up to
/// `config_read_attempts` tries, repeating only the read command.
/// `attempt(n, total)` is called before each try.
pub fn read_config(session: &mut Session, mut attempt: impl FnMut(u32, u32)) -> Result<Vec<Entry>, DeviceError> {
    let timeout = Duration::from_millis(session.timeouts.config_read_ms);
    let delay = Duration::from_millis(session.timeouts.config_read_retry_delay_ms);
    let attempts = session.timeouts.config_read_attempts.max(1);
    let mut n = 1;
    loop {
        attempt(n, attempts);
        log::info!("read: configuration attempt {n} of {attempts}, timeout {} ms", timeout.as_millis());
        let result = session
            .request(&mut TextCodec::until(CONFIG_END_MARKER), GET_CONFIG, timeout, 0)
            .and_then(|reply| parse_config(&reply).map_err(DeviceError::Malformed));
        match result {
            Ok(entries) => {
                log::info!("read: {} parameters", entries.len());
                return Ok(entries);
            }
            Err(e) if e.is_retryable() && n < attempts => {
                log::warn!("read: attempt {n} failed ({e}), retrying");
                n += 1;
                thread::sleep(delay);
            }
            result => return result,
        }
    }
}

/// Parses a configuration dump by structure; tag text is never compared
/// (03-commands §3). `text` excludes the end marker.
pub fn parse_config(text: &str) -> Result<Vec<Entry>, String> {
    let open_start = text.find('<').ok_or("no opening tag")?;
    let open_end = open_start + text[open_start..].find('>').ok_or("no opening tag")?;
    let close_start = text.rfind('<').filter(|&i| i > open_end).ok_or("no closing tag")?;
    text[close_start..].find('>').ok_or("no closing tag")?;
    let count_start = text.rfind(':').ok_or("no parameter count")? + 1;
    let count_end = count_start + text[count_start..].find('\r').ok_or("no parameter count")?;
    let count: usize = text[count_start..count_end]
        .parse()
        .map_err(|_| "parameter count isn't a number")?;

    let entries = text[open_end + 1..close_start]
        .split('\r')
        .map(|line| line.trim_matches('\n'))
        .filter(|line| !line.is_empty())
        .map(|line| match line.split_once(':') {
            Some((id, value)) if !id.is_empty() => Ok(Entry { key: id.into(), value: value.into() }),
            _ => Err(format!("malformed parameter line {line:?}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
    if entries.len() != count {
        return Err(format!("got {} parameters, device said {count}", entries.len()));
    }
    Ok(entries)
}

/// Groups `id:value` items into `:cfg_setparam:` commands (03-commands §4.1).
/// Each returned command excludes the CR; the count is its number of items.
pub fn set_param_commands(params: &[Entry]) -> Result<Vec<(String, usize)>, String> {
    let mut commands: Vec<(String, usize)> = Vec::new();
    for param in params {
        if param.key.is_empty() || !param.key.bytes().all(|b| b.is_ascii_digit()) {
            return Err(format!("parameter ID {:?} isn't a number", param.key));
        }
        if !param.value.chars().all(|c| c.is_ascii() && !c.is_ascii_control()) || param.value.contains(';') {
            return Err(format!(
                "parameter {} has a value the device can't take (non-ASCII, line break or ';')",
                param.key
            ));
        }
        let item = format!("{}:{}", param.key, param.value);
        // Start a new command once "current + ';' + item" would reach the limit, so
        // with its CR a command is at most 1024 bytes.
        match commands.last_mut() {
            Some((command, count)) if command.len() + 1 + item.len() < SET_PARAM_LIMIT => {
                command.push(';');
                command.push_str(&item);
                *count += 1;
            }
            _ => commands.push((format!("{SET_PARAM}{item}"), 1)),
        }
    }
    Ok(commands)
}

/// The number in a `<WORD>:<digits>` reply (03-commands §4.1, §4.3).
pub fn parse_result(text: &str) -> Result<u64, String> {
    let text = text.trim_end_matches(['\r', '\n']);
    let malformed = || format!("expected <TAG>:<number>, got {text:?}");
    let start = text.find('<').ok_or_else(malformed)?;
    let (tag, rest) = text[start + 1..].split_once('>').ok_or_else(malformed)?;
    let digits = rest.strip_prefix(':').ok_or_else(malformed)?;
    let tag_ok = !tag.is_empty() && tag.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
    let digits_ok = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if !tag_ok || !digits_ok {
        return Err(malformed());
    }
    digits.parse().map_err(|_| malformed())
}

/// Writes the given parameters in chunks, then commits them to flash
/// (03-commands §4). `progress(done, total)` counts chunks, with the save last.
/// Any failure stops at once without saving, as the spec requires.
pub fn write_config(
    session: &mut Session,
    params: &[Entry],
    mut progress: impl FnMut(usize, usize),
) -> Result<(), DeviceError> {
    let commands = set_param_commands(params).map_err(DeviceError::Invalid)?;
    let total = commands.len() + 1;
    let timeout = session.timeouts.command();
    log::info!("write: {} parameters in {} chunks", params.len(), commands.len());
    for (i, (command, count)) in commands.iter().enumerate() {
        progress(i, total);
        let result = session
            .request(&mut TextCodec::until(CR), command.as_str(), timeout, 0)
            .and_then(|reply| parse_result(&reply).map_err(DeviceError::Malformed))
            .and_then(|accepted| match accepted == *count as u64 {
                true => Ok(()),
                false => Err(DeviceError::Rejected(format!(
                    "the device accepted {accepted} of {count} parameters"
                ))),
            });
        if let Err(e) = result {
            log::error!("write: chunk {} of {} failed: {e}", i + 1, commands.len());
            return Err(write_aborted(e, i, commands.len()));
        }
        log::debug!("write: chunk {} of {} accepted ({count} parameters, {} bytes)", i + 1, commands.len(), command.len());
    }
    progress(commands.len(), total);
    log::info!("write: saving");
    save(session)?;
    progress(total, total);
    Ok(())
}

/// Says plainly what state an aborted write leaves the device in (03-commands
/// §4.1, open question 24): nothing saved, the working copy maybe partly changed.
fn write_aborted(cause: DeviceError, chunk: usize, chunks: usize) -> DeviceError {
    if let DeviceError::Transport(_) = cause {
        return cause;
    }
    DeviceError::Rejected(format!(
        "Write stopped at part {} of {chunks}: {cause}. Nothing was saved to the device's \
         flash, but its unsaved working configuration may be partly changed. Load from \
         the device to compare, or reboot it to discard the unsaved changes.",
        chunk + 1
    ))
}

fn save(session: &mut Session) -> Result<(), DeviceError> {
    let timeout = Duration::from_millis(session.timeouts.save_ms);
    let reply = session.request(&mut TextCodec::until(CR), SAVE, timeout, 0)?;
    match parse_result(&reply).map_err(DeviceError::Malformed)? {
        1 => Ok(()),
        _ => Err(DeviceError::Rejected("the device couldn't save the configuration".into())),
    }
}

/// `:cfg_default`: an end line, then a save-style result (03-commands §5).
pub fn reset_to_defaults(session: &mut Session) -> Result<(), DeviceError> {
    let timeout = session.timeouts.command();
    let mut codec = TextCodec::until(CR);
    let first = session.request(&mut codec, DEFAULTS, timeout, 0)?;
    if first != DEFAULTS_DONE {
        return Err(DeviceError::Malformed(format!("expected {DEFAULTS_DONE}, got {first:?}")));
    }
    let result = session.receive(&mut codec, timeout)?;
    match parse_result(&result).map_err(DeviceError::Malformed)? {
        1 => Ok(()),
        _ => Err(DeviceError::Rejected("the device couldn't reset to defaults".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::Timeouts;
    use crate::ports::PortKind;
    use crate::transport::mock::{MockTransport, WriteLog};

    const INFO_REPLY: &[u8] = b"cfg_info:0:03.29.00\rcfg_info:2:FMB1:6\rcfg_info:3:356307042441013\r\
cfg_info:4:0\rcfg_info:5:WORKING\rcfg_info:6:ABSENT\rcfg_info:11:3\rcfg_info:12:5\rcfg_info:15:x\r";

    fn session(mock: MockTransport, kind: PortKind) -> (Session, WriteLog) {
        let log = mock.log();
        let mut timeouts = Timeouts::for_kind(kind);
        timeouts.command_ms = 20;
        timeouts.config_read_ms = 20;
        timeouts.config_read_retry_delay_ms = 0;
        // The mock never waits, so only the order of deadlines matters: quiet comes first.
        timeouts.idle_ms = 1;
        (Session::new(Box::new(mock), kind, timeouts), log)
    }

    fn writes(log: &WriteLog) -> Vec<String> {
        log.writes().iter().map(|w| String::from_utf8_lossy(w).into_owned()).collect()
    }

    fn entry(key: &str, value: &str) -> Entry {
        Entry { key: key.into(), value: value.into() }
    }

    fn with_marker(text: &str) -> Vec<u8> {
        [text.as_bytes(), CONFIG_END_MARKER].concat()
    }

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    // ------------------------------------------------ 03-commands §9.1 request vectors

    #[test]
    fn text_request_vectors() {
        let vectors = [
            (CONNECT, "3A 63 66 67 5F 63 6F 6E 6E 65 63 74 0D"),
            (INFO, "3A 63 66 67 5F 69 6E 66 6F 3A 3F 0D"),
            (GET_CONFIG, "3A 63 66 67 5F 67 65 74 63 66 67 0D"),
            (SAVE, "3A 63 66 67 5F 73 61 76 65 0D"),
            (DEFAULTS, "3A 63 66 67 5F 64 65 66 61 75 6C 74 0D"),
            (DISCONNECT, "3A 63 66 67 5F 64 69 73 63 6F 6E 6E 65 63 74 0D"),
            (REBOOT, "2E 72 65 73 65 74 0D"),
            (LOG_OFF, "2E 6C 6F 67 3A 30 0D"),
            (".log:1", "2E 6C 6F 67 3A 31 0D"),
            (SEC_STATUS, "3A 73 65 63 5F 73 74 61 74 75 73 0D"),
        ];
        for (command, bytes) in vectors {
            assert_eq!(text_command(command).unwrap(), hex(bytes), "{command}");
        }
    }

    #[test]
    fn set_param_request_vector() {
        let params = [entry("101", "4"), entry("102", "2"), entry("2001", "internet")];
        let (command, count) = set_param_commands(&params).unwrap().remove(0);
        assert_eq!(count, 3);
        assert_eq!(
            text_command(&command).unwrap(),
            hex("3A 63 66 67 5F 73 65 74 70 61 72 61 6D 3A 31 30 31 3A 34 3B 31 30 32 3A 32 3B \
                 32 30 30 31 3A 69 6E 74 65 72 6E 65 74 0D")
        );
    }

    #[test]
    fn login_request_vector() {
        let (mut s, log) = session(MockTransport::new().reply(&[b"<SECSTAT>1,1,1,0,3\r"]), PortKind::Usb);
        login(&mut s, "abcd").unwrap();
        assert_eq!(log.writes(), [hex("3A 73 65 63 5F 6C 6F 67 69 6E 3A 61 62 63 64 0D")]);
    }

    // ------------------------------------------------ 03-commands §9.2 reply vectors

    #[test]
    fn connect_reply_vector() {
        let mock = MockTransport::new().reply(&[b"<CFG_CONNECT>\r"]);
        let (mut s, _) = session(mock, PortKind::Usb);
        s.request(&mut TextCodec::until(CONNECT_REPLY), CONNECT, s.timeouts.command(), 0).unwrap();
    }

    #[test]
    fn info_reply_vector() {
        let info = parse_info("cfg_info:3:123456789012345\rcfg_info:5:WORKING\rcfg_info:6:ABSENT\r").unwrap();
        assert_eq!(info.imei.as_deref(), Some("123456789012345"));
        assert_eq!((info.sim1_status.as_str(), info.sim2_status.as_str()), ("WORKING", "ABSENT"));
    }

    #[test]
    fn config_reply_vectors() {
        let valid = with_marker("<X>\r101:4\r102:2\r</X>:2\r");
        let invalid = with_marker("<X>\r101:4\r</X>:2\r");
        let text = |bytes: &[u8]| ascii(&bytes[..bytes.len() - CONFIG_END_MARKER.len()]);
        assert_eq!(parse_config(&text(&valid)).unwrap(), [entry("101", "4"), entry("102", "2")]);
        assert!(parse_config(&text(&invalid)).is_err());

        // The invalid reply is retried; the valid one then succeeds.
        let mock = MockTransport::new().reply(&[&invalid]).reply(&[&valid]);
        let (mut s, log) = session(mock, PortKind::Usb);
        assert_eq!(read_config(&mut s, |_, _| {}).unwrap().len(), 2);
        assert_eq!(writes(&log), [":cfg_getcfg\r", ":cfg_getcfg\r"]);
    }

    #[test]
    fn set_param_and_save_reply_vectors() {
        let mock = MockTransport::new()
            .reply(&[b"<SETPARAM_RESULT>:3\r"])
            .reply(&[b"<CFG_SAVE>:1\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        let params = [entry("101", "4"), entry("102", "2"), entry("2001", "internet")];
        write_config(&mut s, &params, |_, _| {}).unwrap();
        assert_eq!(writes(&log), [":cfg_setparam:101:4;102:2;2001:internet\r", ":cfg_save\r"]);
    }

    #[test]
    fn keyword_status_reply_vector() {
        let status = parse_secstat("<SECSTAT>1,1,0,0,3\r").unwrap();
        assert_eq!(
            status,
            SecurityStatus { last_ok: true, secured: true, authorized: false, locked: false, retries_left: 3 }
        );
    }

    // ------------------------------------------------ device info rules (§2)

    #[test]
    fn parses_info_keys() {
        let info = parse_info(&ascii(INFO_REPLY)).unwrap();
        assert_eq!(info.firmware.as_deref(), Some("03.29.00"));
        assert_eq!(info.hardware_code.as_deref(), Some("FMB1:6"));
        assert_eq!(info.active_sim, Some(1));
        assert_eq!(info.spec_id, 3);
        assert!(info.has_feature(0) && info.has_feature(2) && !info.has_feature(1));
        assert!(!info.keyword_supported);
    }

    #[test]
    fn info_defaults_and_validity() {
        let info = parse_info("cfg_info:5:a\rcfg_info:6:b\rcfg_info:8:1\rcfg_info:4:2\r").unwrap();
        assert_eq!(info.spec_id, 1);
        assert_eq!(info.active_sim, Some(2));
        assert!(info.keyword_supported && info.keyword_authorized);
        assert!(parse_info("cfg_info:5:a\rcfg_info:6:\r").is_err());
        assert!(parse_info("hello\r").is_err());
        // Lines with non-printable values are ignored.
        assert!(parse_info("cfg_info:5:a\x01\rcfg_info:6:b\r").is_err());
    }

    #[test]
    fn info_ignores_bad_integers_and_splits_hardware_options() {
        let info = parse_info("cfg_info:5:a\rcfg_info:6:b\rcfg_info:8:yes\rcfg_info:11:x\rcfg_info:14:OPT1:OPT2\r")
            .unwrap();
        assert!(!info.keyword_supported);
        assert_eq!(info.spec_id, 1);
        assert_eq!(info.hardware_options, ["OPT1", "OPT2"]);
    }

    #[test]
    fn info_json_keeps_64_bit_features_exact() {
        let info = parse_info("cfg_info:5:a\rcfg_info:6:b\rcfg_info:12:18446744073709551615\r").unwrap();
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["features"], "18446744073709551615");
    }

    // ------------------------------------------------ session flow

    #[test]
    fn opens_a_session_over_usb_and_bluetooth() {
        for kind in [PortKind::Usb, PortKind::Bluetooth] {
            let mock = MockTransport::new()
                .reply(&[b"log off\r"])
                .reply(&[INFO_REPLY])
                .reply(&[b"<CFG_CONNECT>\r"]);
            let (mut s, log) = session(mock, kind);
            open_session(&mut s).unwrap();
            assert_eq!(writes(&log), [".log:0\r", ":cfg_info:?\r", ":cfg_connect\r"]);
            assert_eq!(s.info.as_ref().unwrap().imei.as_deref(), Some("356307042441013"));
            assert!(s.security.is_none());
            drop(s);
            assert_eq!(writes(&log).last().unwrap(), ":cfg_disconnect\r");
        }
    }

    #[test]
    fn session_checks_keyword_status_when_supported() {
        let mock = MockTransport::new()
            .silence()
            .reply(&[b"cfg_info:5:a\rcfg_info:6:b\rcfg_info:8:0\r"])
            .reply(&[b"<CFG_CONNECT>\r"])
            .silence()
            .reply(&[b"<SECSTAT>1,1,0,0,5\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        open_session(&mut s).unwrap();
        assert_eq!(
            writes(&log),
            [".log:0\r", ":cfg_info:?\r", ":cfg_connect\r", ".log:0\r", ":sec_status\r"]
        );
        assert!(s.security.as_ref().unwrap().needs_keyword());
    }

    #[test]
    fn asset_query_follows_feature_bit_14() {
        let info = format!("cfg_info:5:a\rcfg_info:6:b\rcfg_info:12:{}\r", 1u64 << 14);
        let mock = MockTransport::new()
            .silence()
            .reply(&[info.as_bytes()])
            .reply(&[b"BLE chip\r\nCode Version: 1.2.3\r\n"])
            .reply(&[b"<CFG_CONNECT>\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        open_session(&mut s).unwrap();
        assert_eq!(writes(&log)[2], "@asset_send:asset info\r");
        assert_eq!(s.info.as_ref().unwrap().ble_chip_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn silent_log_off_does_not_stall_the_connect() {
        let mock = MockTransport::new().silence().reply(&[INFO_REPLY]).reply(&[b"<CFG_CONNECT>\r"]);
        let (mut s, log) = session(mock, PortKind::Bluetooth);
        open_session(&mut s).unwrap();
        assert_eq!(writes(&log), [".log:0\r", ":cfg_info:?\r", ":cfg_connect\r"]);
    }

    #[test]
    fn silent_info_names_the_step_link_and_wait() {
        let mock = MockTransport::new().silence().silence();
        let (mut s, _) = session(mock, PortKind::Bluetooth);
        s.timeouts.command_ms = 30_000; // the mock never waits, so this only feeds the message
        let err = open_session(&mut s).unwrap_err().to_string();
        assert!(err.contains("device-info request") && err.contains("Bluetooth serial port") && err.contains("30 s"), "{err}");
    }

    #[test]
    fn rejects_a_port_that_is_not_a_tracker() {
        let mock = MockTransport::new().silence().reply(&[b"AT\r\nOK\r\n"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        assert!(matches!(open_session(&mut s), Err(DeviceError::Rejected(_))));
        assert_eq!(writes(&log).len(), 2);
        drop(s);
        // No session was opened, so no disconnect is sent.
        assert_eq!(writes(&log).len(), 2);
    }

    #[test]
    fn login_failure_reports_retries_without_the_keyword() {
        let mock = MockTransport::new().reply(&[b"<SECSTAT>0,1,0,0,2\r"]);
        let (mut s, _) = session(mock, PortKind::Usb);
        let err = login(&mut s, "s3cr3t").unwrap_err().to_string();
        assert!(err.contains("2 attempts left"));
        assert!(!err.contains("s3cr3t"));
        assert!(s.security.as_ref().unwrap().needs_keyword());

        let mock = MockTransport::new().reply(&[b"<SECSTAT>0,1,0,1,0\r"]);
        let (mut s, _) = session(mock, PortKind::Usb);
        assert!(login(&mut s, "x").unwrap_err().to_string().contains("locked"));

        let (mut s, log) = session(MockTransport::new(), PortKind::Usb);
        assert!(login(&mut s, "bad\rkey").is_err());
        assert!(log.writes().is_empty());
    }

    // ------------------------------------------------ configuration (§3-5)

    #[test]
    fn parses_config_by_structure() {
        let text = "<CFG>\r101:4\r2004:http://host:8080/x\r804:\r<CFG_END>:3\r";
        let entries = parse_config(text).unwrap();
        assert_eq!(entries, [entry("101", "4"), entry("2004", "http://host:8080/x"), entry("804", "")]);
        assert!(parse_config("101:4\r").is_err());
        assert!(parse_config("<CFG>\r:4\r<CFG_END>:1\r").is_err());
    }

    #[test]
    fn reads_config_split_across_chunks_over_bluetooth() {
        let reply = with_marker("<X>\r101:4\r102:2\r</X>:2\r");
        let (first, second) = reply.split_at(9);
        let mock = MockTransport::new().reply(&[first, second]);
        let (mut s, log) = session(mock, PortKind::Bluetooth);
        let entries = read_config(&mut s, |_, _| {}).unwrap();
        assert_eq!(entries, [entry("101", "4"), entry("102", "2")]);
        assert_eq!(writes(&log), [":cfg_getcfg\r"]);
    }

    #[test]
    fn config_read_retries_after_timeouts() {
        let good = with_marker("<X>\r101:4\r</X>:1\r");
        let mock = MockTransport::new().silence().silence().reply(&[&good]);
        let (mut s, log) = session(mock, PortKind::Usb);
        let mut attempts = vec![];
        read_config(&mut s, |n, total| attempts.push((n, total))).unwrap();
        assert_eq!(log.writes().len(), 3);
        assert_eq!(attempts, [(1, 10), (2, 10), (3, 10)]);
    }

    #[test]
    fn config_read_gives_up_after_ten_attempts() {
        let mut mock = MockTransport::new();
        for _ in 0..12 {
            mock = mock.silence();
        }
        let (mut s, log) = session(mock, PortKind::Usb);
        assert_eq!(read_config(&mut s, |_, _| {}), Err(DeviceError::Timeout));
        assert_eq!(log.writes().len(), 10);
    }

    #[test]
    fn chunking_example_from_the_spec() {
        // Ten items of 155 characters ("8000:" plus 150) go out as 6 + 4 (§4.1).
        let params: Vec<Entry> = (0..10).map(|_| entry("8000", &"v".repeat(150))).collect();
        let commands = set_param_commands(&params).unwrap();
        let sizes: Vec<(usize, usize)> = commands.iter().map(|(c, n)| (c.len() + 1, *n)).collect();
        assert_eq!(sizes, [(950, 6), (638, 4)]);
    }

    #[test]
    fn chunks_never_exceed_1024_bytes_with_the_cr() {
        for len in [1, 7, 50, 300, 1000] {
            let params: Vec<Entry> = (0..40).map(|i| entry(&(1000 + i).to_string(), &"x".repeat(len))).collect();
            let commands = set_param_commands(&params).unwrap();
            assert_eq!(commands.iter().map(|(_, n)| n).sum::<usize>(), 40);
            for (command, count) in &commands {
                assert!(command.len() + 1 <= SET_PARAM_LIMIT || *count == 1, "len {len}");
                assert_eq!(command.matches(';').count() + 1, *count);
            }
        }
    }

    #[test]
    fn set_param_refuses_values_it_cannot_send() {
        assert!(set_param_commands(&[entry("101", "a;b")]).is_err());
        assert!(set_param_commands(&[entry("101", "a\rb")]).is_err());
        assert!(set_param_commands(&[entry("Title", "x")]).is_err());
    }

    #[test]
    fn parses_result_replies() {
        assert_eq!(parse_result("<SETPARAM_RESULT>:12"), Ok(12));
        assert_eq!(parse_result("<X>:1\r"), Ok(1));
        assert!(parse_result("<X>:").is_err());
        assert!(parse_result("X:1").is_err());
        assert!(parse_result("<X>:1a").is_err());
    }

    #[test]
    fn write_reports_progress() {
        let mock = MockTransport::new().reply(&[b"<R>:2\r"]).reply(&[b"<S>:1\r"]);
        let (mut s, _) = session(mock, PortKind::Bluetooth);
        let mut steps = vec![];
        write_config(&mut s, &[entry("101", "4"), entry("102", "2")], |d, t| steps.push((d, t))).unwrap();
        assert_eq!(steps, [(0, 2), (1, 2), (2, 2)]);
    }

    #[test]
    fn short_count_aborts_without_saving() {
        let params: Vec<Entry> = (0..10).map(|_| entry("8000", &"v".repeat(150))).collect();
        let mock = MockTransport::new().reply(&[b"<R>:6\r"]).reply(&[b"<R>:3\r"]).reply(&[b"<S>:1\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        let err = write_config(&mut s, &params, |_, _| {}).unwrap_err().to_string();
        assert_eq!(log.writes().len(), 2, "no save after a failed chunk");
        assert!(err.contains("part 2 of 2") && err.contains("accepted 3 of 4"));
        assert!(err.contains("Nothing was saved"));
    }

    #[test]
    fn timeout_aborts_without_saving() {
        let mock = MockTransport::new().silence().reply(&[b"<S>:1\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        assert!(write_config(&mut s, &[entry("101", "4")], |_, _| {}).is_err());
        assert_eq!(writes(&log), [":cfg_setparam:101:4\r"]);
    }

    #[test]
    fn failed_save_is_reported() {
        let mock = MockTransport::new().reply(&[b"<R>:1\r"]).reply(&[b"<S>:0\r"]);
        let (mut s, _) = session(mock, PortKind::Usb);
        assert!(matches!(write_config(&mut s, &[entry("101", "4")], |_, _| {}), Err(DeviceError::Rejected(_))));
    }

    #[test]
    fn resets_to_defaults() {
        let mock = MockTransport::new().reply(&[b"<DEFAULT_CFG_END>\r<RES>:1\r"]);
        let (mut s, log) = session(mock, PortKind::Usb);
        reset_to_defaults(&mut s).unwrap();
        assert_eq!(writes(&log), [":cfg_default\r"]);

        // Case-sensitive first line.
        let mock = MockTransport::new().reply(&[b"<default_cfg_end>\r<RES>:1\r"]);
        let (mut s, _) = session(mock, PortKind::Usb);
        assert!(reset_to_defaults(&mut s).is_err());

        let mock = MockTransport::new().reply(&[b"<DEFAULT_CFG_END>\r", b"<RES>:0\r"]);
        let (mut s, _) = session(mock, PortKind::Bluetooth);
        assert!(matches!(reset_to_defaults(&mut s), Err(DeviceError::Rejected(_))));
    }

    #[test]
    fn reboot_skips_the_disconnect() {
        let (mut s, log) = session(MockTransport::new(), PortKind::Usb);
        s.farewell = Some(b":cfg_disconnect\r".to_vec());
        reboot(&mut s).unwrap();
        drop(s);
        assert_eq!(writes(&log), [".reset\r"]);
    }
}
