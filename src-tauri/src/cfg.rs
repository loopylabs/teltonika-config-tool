//! Teltonika `.cfg` files: gzip-compressed text of `key:value;` pairs.
//!
//! The first few keys are a header (`ConfigurationVersion`, `HwVersion`,
//! `Title`, `FmType`, `SpecId`); the rest are numeric parameter IDs.
//! Entry order is preserved so a load/save round trip is byte-identical.

use std::fs;
use std::io::{Read, Write};

use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};

const GZIP_MAGIC: [u8; 2] = [0x1f, 0x8b];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CfgFile {
    pub entries: Vec<Entry>,
}

/// Loading splits each piece at the **first** `:` and keeps everything after it as
/// the value. The vendor's reader takes the text after the *last* `:` instead
/// (docs/spec/05-cfg-files.md §3), which truncates values such as URLs. We are
/// deliberately stricter so `http://host:8080/x` survives; files we write and
/// files the vendor tool wrote without `:` in values load identically either way.
pub fn parse(text: &str) -> Result<CfgFile, String> {
    let entries = text
        .split(';')
        .filter(|segment| !segment.is_empty())
        .map(|segment| {
            segment
                .split_once(':')
                .map(|(key, value)| Entry {
                    key: key.to_string(),
                    value: value.to_string(),
                })
                .ok_or_else(|| format!("malformed entry: {segment:?}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(CfgFile { entries })
}

pub fn serialize(file: &CfgFile) -> String {
    file.entries
        .iter()
        .map(|entry| format!("{}:{};", entry.key, entry.value))
        .collect()
}

/// Accepts both gzip-compressed and plain-text files.
pub fn decode(bytes: &[u8]) -> Result<String, String> {
    if bytes.starts_with(&GZIP_MAGIC) {
        let mut text = String::new();
        GzDecoder::new(bytes)
            .read_to_string(&mut text)
            .map_err(|e| format!("could not decompress: {e}"))?;
        Ok(text)
    } else {
        String::from_utf8(bytes.to_vec()).map_err(|e| format!("not a text file: {e}"))
    }
}

pub fn encode(text: &str) -> Result<Vec<u8>, String> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(text.as_bytes())
        .and_then(|_| encoder.finish())
        .map_err(|e| format!("could not compress: {e}"))
}

pub fn read_file(path: &str) -> Result<CfgFile, String> {
    let bytes = fs::read(path).map_err(|e| format!("could not read {path}: {e}"))?;
    parse(&decode(&bytes)?)
}

/// The keyword parameter; left out of saved files unless asked for (05 §4).
pub const KEYWORD_KEY: &str = "10";

const HEADER_KEYS: [&str; 5] = ["ConfigurationVersion", "HwVersion", "Title", "FmType", "SpecId"];

fn is_parameter(entry: &Entry) -> bool {
    !entry.key.is_empty() && entry.key.bytes().all(|b| b.is_ascii_digit())
}

fn join_ids(ids: &[String]) -> String {
    ids.join(", ")
}

/// What saving needs: the entries to write, and non-blocking warnings.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub file: CfgFile,
    pub warnings: Vec<String>,
}

/// Applies the save rules. A `;` in any value would corrupt the file (05 §2.4), so
/// that refuses the save and names the parameters. Non-ASCII text and repeated IDs
/// only produce warnings. The keyword (parameter 10) is dropped unless included.
pub fn prepare_save(file: &CfgFile, include_keyword: bool) -> Result<Prepared, String> {
    let entries: Vec<Entry> = file
        .entries
        .iter()
        .filter(|e| include_keyword || e.key != KEYWORD_KEY)
        .cloned()
        .collect();

    let mut with_semicolon: Vec<String> = Vec::new();
    let mut non_ascii: Vec<String> = Vec::new();
    for e in &entries {
        if e.value.contains(';') && !with_semicolon.contains(&e.key) {
            with_semicolon.push(e.key.clone());
        }
        if !e.value.is_ascii() && !non_ascii.contains(&e.key) {
            non_ascii.push(e.key.clone());
        }
    }
    if !with_semicolon.is_empty() {
        let (noun, verb) = if with_semicolon.len() == 1 { ("parameter", "has") } else { ("parameters", "have") };
        return Err(format!(
            "Not saved: {noun} {} {verb} a value containing \";\", which would corrupt the file. Remove the semicolon and save again.",
            join_ids(&with_semicolon)
        ));
    }

    let mut warnings = Vec::new();
    if !non_ascii.is_empty() {
        warnings.push(format!(
            "Parameter(s) {} contain non-ASCII characters. The vendor Configurator writes ASCII only, so other tools may not read them correctly.",
            join_ids(&non_ascii)
        ));
    }
    let mut seen: Vec<&str> = Vec::new();
    let mut duplicates: Vec<String> = Vec::new();
    for e in entries.iter().filter(|e| is_parameter(e) || HEADER_KEYS.contains(&e.key.as_str())) {
        if seen.contains(&e.key.as_str()) {
            if !duplicates.contains(&e.key) {
                duplicates.push(e.key.clone());
            }
        } else {
            seen.push(&e.key);
        }
    }
    if !duplicates.is_empty() {
        warnings.push(format!(
            "ID(s) {} appear more than once. Which copy a device or another tool uses is not defined.",
            join_ids(&duplicates)
        ));
    }
    Ok(Prepared { file: CfgFile { entries }, warnings })
}

pub fn write_file(path: &str, file: &CfgFile, include_keyword: bool) -> Result<Vec<String>, String> {
    let prepared = prepare_save(file, include_keyword)?;
    let bytes = encode(&serialize(&prepared.file))?;
    fs::write(path, bytes).map_err(|e| format!("could not write {path}: {e}"))?;
    Ok(prepared.warnings)
}

/// Result of checking a file-sourced configuration against the connected device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "verdict", content = "message", rename_all = "camelCase")]
pub enum WriteCheck {
    Ok,
    /// Wrong hardware: must not be written.
    Block(String),
    /// Possibly unsupported parameters: the user decides.
    Confirm(String),
}

fn header<'a>(file: &'a CfgFile, name: &str) -> Option<&'a str> {
    file.entries.iter().find(|e| e.key.eq_ignore_ascii_case(name)).map(|e| e.value.trim())
}

/// Header text values escape `:` as `_` in files (05 §2.3); undo that.
fn unescape_header(value: &str) -> String {
    value.replace('_', ":")
}

/// Dotted version to numbers; `None` if any part is not a number.
fn version_parts(v: &str) -> Option<Vec<u64>> {
    let parts: Option<Vec<u64>> = v.trim().split('.').map(|p| p.trim().parse().ok()).collect();
    parts.filter(|p| !p.is_empty())
}

/// True if `a` is strictly newer than `b`, comparing all components (missing = 0).
fn version_newer(a: &str, b: &str) -> Option<bool> {
    let (mut a, mut b) = (version_parts(a)?, version_parts(b)?);
    let n = a.len().max(b.len());
    a.resize(n, 0);
    b.resize(n, 0);
    Some(a > b)
}

/// Writing a file to a device: a different hardware code blocks, a newer
/// `ConfigurationVersion` than the device's asks for confirmation (05 §3.1).
/// Missing or unreadable information never blocks.
pub fn check_write_target(
    file: &CfgFile,
    device_hardware: Option<&str>,
    device_config_version: Option<&str>,
) -> WriteCheck {
    if let (Some(file_hw), Some(dev_hw)) = (header(file, "HwVersion"), device_hardware) {
        let file_hw = unescape_header(file_hw);
        let dev_hw = dev_hw.trim();
        if !file_hw.is_empty() && !dev_hw.is_empty() && !file_hw.eq_ignore_ascii_case(dev_hw) {
            return WriteCheck::Block(format!(
                "This file is for hardware {file_hw}, but the connected device is {dev_hw}. It can't be written to this device."
            ));
        }
    }
    if let (Some(file_ver), Some(dev_ver)) = (header(file, "ConfigurationVersion"), device_config_version) {
        if version_newer(file_ver, dev_ver) == Some(true) {
            return WriteCheck::Confirm(format!(
                "This file uses configuration version {file_ver}, newer than the device's {}. Some parameters may not be supported by the device.",
                dev_ver.trim()
            ));
        }
    }
    WriteCheck::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "ConfigurationVersion:10.0.0.0;HwVersion:FMB1_6;Title:03.29.00 or higher;\
FmType:FMB140;SpecId:1;101:8;804:;902:avl1.example.com;";

    #[test]
    fn parses_header_and_parameters_in_order() {
        let file = parse(SAMPLE).unwrap();
        assert_eq!(file.entries.len(), 8);
        assert_eq!(file.entries[0].key, "ConfigurationVersion");
        assert_eq!(file.entries[2].value, "03.29.00 or higher");
        assert_eq!(file.entries[6].value, "");
        assert_eq!(file.entries[7].value, "avl1.example.com");
    }

    #[test]
    fn values_may_contain_colons() {
        let file = parse("2004:http://host:8080/path;").unwrap();
        assert_eq!(file.entries[0].value, "http://host:8080/path");
    }

    #[test]
    fn rejects_entries_without_a_separator() {
        assert!(parse("101:8;garbage;").is_err());
    }

    #[test]
    fn text_round_trip_is_identical() {
        assert_eq!(serialize(&parse(SAMPLE).unwrap()), SAMPLE);
    }

    #[test]
    fn gzip_round_trip_is_identical() {
        let bytes = encode(SAMPLE).unwrap();
        assert!(bytes.starts_with(&GZIP_MAGIC));
        assert_eq!(decode(&bytes).unwrap(), SAMPLE);
    }

    #[test]
    fn decodes_plain_text() {
        assert_eq!(decode(SAMPLE.as_bytes()).unwrap(), SAMPLE);
    }

    /// Set FMCONFIG_SAMPLE_CFG to a real Configurator export to check it round-trips.
    #[test]
    fn real_file_round_trip() {
        let Ok(path) = std::env::var("FMCONFIG_SAMPLE_CFG") else {
            return;
        };
        let text = decode(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(serialize(&parse(&text).unwrap()), text);
    }

    fn file(pairs: &[(&str, &str)]) -> CfgFile {
        CfgFile {
            entries: pairs.iter().map(|(k, v)| Entry { key: k.to_string(), value: v.to_string() }).collect(),
        }
    }

    #[test]
    fn load_splits_at_the_first_colon_keeping_urls() {
        let f = parse("2004:https://a.example:8443/x:y;").unwrap();
        assert_eq!(f.entries[0].key, "2004");
        assert_eq!(f.entries[0].value, "https://a.example:8443/x:y");
    }

    #[test]
    fn save_refuses_semicolons_and_names_every_id() {
        let err = prepare_save(&file(&[("101", "1"), ("2004", "a;b"), ("2005", "c;d"), ("2006", "ok")]), false)
            .unwrap_err();
        assert!(err.contains("2004, 2005") && !err.contains("2006"), "{err}");
        let one = prepare_save(&file(&[("7", "x;")]), false).unwrap_err();
        assert!(one.contains("parameter 7 has"), "{one}");
    }

    #[test]
    fn save_warns_on_non_ascii_without_blocking() {
        let p = prepare_save(&file(&[("2001", "caf\u{e9}"), ("2002", "plain")]), false).unwrap();
        assert_eq!(p.file.entries.len(), 2);
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("2001") && !p.warnings[0].contains("2002"));
    }

    #[test]
    fn save_warns_on_duplicate_ids_once_each() {
        let p = prepare_save(&file(&[("101", "1"), ("102", "2"), ("101", "3"), ("101", "4")]), false).unwrap();
        assert_eq!(p.warnings.len(), 1);
        assert!(p.warnings[0].contains("101") && !p.warnings[0].contains("102"));
        assert_eq!(p.file.entries.len(), 4);
    }

    #[test]
    fn no_warnings_for_clean_files() {
        assert!(prepare_save(&file(&[("101", "1"), ("2001", "x")]), false).unwrap().warnings.is_empty());
    }

    #[test]
    fn keyword_is_left_out_unless_included() {
        let f = file(&[("9", "a"), ("10", "secret"), ("11", "b")]);
        let without = prepare_save(&f, false).unwrap();
        assert!(without.file.entries.iter().all(|e| e.key != "10"));
        assert_eq!(without.file.entries.len(), 2);
        let with = prepare_save(&f, true).unwrap();
        assert_eq!(with.file.entries.len(), 3);
    }

    #[test]
    fn keyword_with_semicolon_does_not_block_when_left_out() {
        assert!(prepare_save(&file(&[("10", "a;b")]), false).is_ok());
        assert!(prepare_save(&file(&[("10", "a;b")]), true).is_err());
    }

    #[test]
    fn write_file_saves_and_returns_warnings() {
        let path = std::env::temp_dir().join(format!("fmconfig-test-{}.cfg", std::process::id()));
        let path = path.to_str().unwrap().to_string();
        let warnings = write_file(&path, &file(&[("10", "k"), ("2001", "\u{e9}")]), false).unwrap();
        assert_eq!(warnings.len(), 1);
        let back = read_file(&path).unwrap();
        let _ = fs::remove_file(&path);
        assert_eq!(back.entries.len(), 1);
        assert!(write_file(&path, &file(&[("1", "a;b")]), false).is_err());
        assert!(!std::path::Path::new(&path).exists());
    }

    fn header_file(hw: &str, ver: &str) -> CfgFile {
        file(&[("ConfigurationVersion", ver), ("HwVersion", hw), ("101", "1")])
    }

    #[test]
    fn hardware_mismatch_blocks() {
        let f = header_file("FMB1_6", "13.0.0.0");
        assert_eq!(check_write_target(&f, Some("FMB1:6"), Some("13.0.0.0")), WriteCheck::Ok);
        assert_eq!(check_write_target(&f, Some("fmb1:6"), None), WriteCheck::Ok);
        match check_write_target(&f, Some("FMC1:2"), Some("13.0.0.0")) {
            WriteCheck::Block(m) => assert!(m.contains("FMB1:6") && m.contains("FMC1:2"), "{m}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hardware_mismatch_beats_version_prompt() {
        let f = header_file("A:1", "99.0.0.0");
        assert!(matches!(check_write_target(&f, Some("B:1"), Some("1.0.0.0")), WriteCheck::Block(_)));
    }

    #[test]
    fn newer_file_version_asks_for_confirmation() {
        let hw = Some("FMB1:6");
        assert!(matches!(
            check_write_target(&header_file("FMB1_6", "13.1.0.0"), hw, Some("13.0.0.0")),
            WriteCheck::Confirm(_)
        ));
        assert!(matches!(
            check_write_target(&header_file("FMB1_6", "13.0.0.1"), hw, Some("13.0.0")),
            WriteCheck::Confirm(_)
        ));
        assert_eq!(check_write_target(&header_file("FMB1_6", "13.0.0.0"), hw, Some("13.0.0")), WriteCheck::Ok);
        assert_eq!(check_write_target(&header_file("FMB1_6", "9.0.0.0"), hw, Some("13.0.0.0")), WriteCheck::Ok);
        // 10 is newer than 9 (numeric, not text, comparison)
        assert!(matches!(
            check_write_target(&header_file("FMB1_6", "10.0.0.0"), hw, Some("9.0.0.0")),
            WriteCheck::Confirm(_)
        ));
    }

    #[test]
    fn missing_or_odd_information_never_blocks() {
        assert_eq!(check_write_target(&file(&[("101", "1")]), Some("FMB1:6"), Some("13.0.0.0")), WriteCheck::Ok);
        assert_eq!(check_write_target(&header_file("FMB1_6", "n/a"), None, Some("13.0.0.0")), WriteCheck::Ok);
        assert_eq!(check_write_target(&header_file("FMB1_6", "13.0.0.0"), None, None), WriteCheck::Ok);
    }
}
