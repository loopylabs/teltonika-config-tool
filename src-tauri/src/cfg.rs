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

pub fn write_file(path: &str, file: &CfgFile) -> Result<(), String> {
    let bytes = encode(&serialize(file))?;
    fs::write(path, bytes).map_err(|e| format!("could not write {path}: {e}"))
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
}
