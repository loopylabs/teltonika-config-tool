//! Tests for device pictures and model names (docs/spec/10-device-presentation.md).

use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use super::*;

fn family(hardware: &str) -> Family {
    let mut d: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        d.insert(k.to_string(), v.as_bytes().to_vec());
    };
    put("Base", "<Configuration/>");
    put("BaseExtension", r#"<BaseExtensions><Software Version="1.0.0.0"><Hardware Version="FMB1:[1-9]"/></Software></BaseExtensions>"#);
    put("Collections", "<CollectionConfigurations/>");
    put("Lists", "<ListConfigurations/>");
    put("Hardware", hardware);
    put("Templates", "<IOTemplates/>");
    put("UpdateConfiguration", "<Configuration/>");
    model::build_family("FMB", &d).unwrap()
}

fn library(version: &str, hardware: &str) -> Library {
    let mut lib = Library::default();
    let mut families = BTreeMap::new();
    families.insert("FMB".to_string(), family(hardware));
    lib.add(Definitions { product_version: version.into(), sha256: String::new(), warnings: vec![], families }, false);
    lib
}

const HW: &str = r#"<HardwareVersionTypes>
  <HardwareVersionType Version="FMB1:6" FmType="FMB140"/>
  <HardwareVersionType Version="FMB9:21" FmType="FMB900" BaseOf="FMB9:1"/>
  <HardwareVersionType Version="FMB1:4" FmType="FMB125 no BAT DualSIM"/>
  <HardwareVersionType Version="FMB1:2" FmType="FMB920"/>
  <HardwareVersionType Version="FMB1:3" FmType="FMB920"/>
</HardwareVersionTypes>"#;

#[test]
fn model_names_alias_resolve_for_the_name_only() {
    let lib = library("1.8.21.0", HW);
    assert_eq!(lib.model_name("FMB1:6"), naming::ModelName { name: Some("FMB140".into()), known: true });
    assert_eq!(lib.model_name("FMB9:1").name.as_deref(), Some("FMB900"));
    assert_eq!(lib.model_name("fmb1:4").name.as_deref(), Some("FMB125 no BAT DualSIM"));
    assert_eq!(lib.model_name("FMB7:7"), naming::ModelName { name: None, known: false });
    assert_eq!(Library::default().model_name("FMB1:6").known, false);
}

#[test]
fn bluetooth_name_hint_needs_exactly_one_code_for_a_picture() {
    let lib = library("1.8.21.0", HW);
    let h = lib.hint_for_model("fmb140").unwrap();
    assert_eq!((h.model.as_str(), h.hw_code.as_deref()), ("FMB140", Some("FMB1:6")));
    let two = lib.hint_for_model("FMB920").unwrap();
    assert_eq!((two.model.as_str(), two.hw_code), ("FMB920", None));
    assert_eq!(lib.hint_for_model("FMB999"), None);
}

/// Gated on FMCONFIG_CONFIGURATOR (the user's own download). Counts, sizes and hashes only.
#[test]
fn integration_device_pictures() {
    let Ok(path) = std::env::var("FMCONFIG_CONFIGURATOR") else { return };
    let input = std::fs::read(&path).unwrap();
    let x = images::extract_pictures(&input).unwrap();
    let get = |k: &str| x.pictures.iter().find(|p| p.0 == k).map(|p| p.1.clone());
    let dims = |k: &str| get(k).map(|b| (b.len(), u32::from_be_bytes(b[16..20].try_into().unwrap()), u32::from_be_bytes(b[20..24].try_into().unwrap())));
    let mut report = Vec::new();
    let mut check = |name: &str, expected: String, actual: String| {
        report.push(format!("{} {name}: expected {expected}, actual {actual}", if expected == actual { "PASS" } else { "FAIL" }));
    };
    check("blob size", "2772396".into(), x.stats.blob_len.to_string());
    check("blob sha256", "1feaed8bfa7064ba45cfd5c03954a8f5e9d32e6b7756fd3f394e8c63f79978a3".into(), x.stats.blob_sha256.clone());
    check("entries", "179".into(), x.stats.entries.to_string());
    check("device pictures", "131".into(), x.stats.device_pictures.to_string());
    check("device bytes", "2486615".into(), x.stats.device_bytes.to_string());
    let sizes: Vec<usize> = x.pictures.iter().map(|p| p.1.len()).collect();
    check("smallest/largest", "2500/439499".into(), format!("{}/{}", sizes.iter().min().unwrap(), sizes.iter().max().unwrap()));
    let distinct: std::collections::BTreeSet<String> = x.pictures.iter().map(|p| hex(&Sha256::digest(&p.1))).collect();
    check("distinct by content", "90".into(), distinct.len().to_string());
    check("fmb1_6 size/w/h", "(2500, 128, 100)".into(), format!("{:?}", dims("fmb1_6").unwrap()));
    check("fmb0_1 size/w/h", "(5903, 134, 101)".into(), format!("{:?}", dims("fmb0_1").unwrap()));
    check("fmb640_1 size/w/h", "(6833, 127, 100)".into(), format!("{:?}", dims("fmb640_1").unwrap()));
    check("unknown size/w/h", "(3719, 101, 101)".into(), format!("{:?}", dims("unknown").unwrap()));
    check("unknown sha256", "9ad71e2bfeb7cb8cf3e38bba18bc7c6674c19236a42b3932f6297286722944a2".into(), hex(&Sha256::digest(get("unknown").unwrap())));
    let widest = x.pictures.iter().map(|p| u32::from_be_bytes(p.1[16..20].try_into().unwrap())).max().unwrap();
    check("widest picture px", "4232".into(), widest.to_string());
    let tat = x.pictures.iter().filter(|p| p.0.starts_with("tat1_")).count();
    check("tat1_* pictures", ">=5".into(), if tat >= 5 { ">=5".into() } else { tat.to_string() });
    // Hardware-table coverage: every code in the loaded definitions has a picture.
    let (d, _) = load(std::path::Path::new(&path), None).unwrap();
    let codes: Vec<String> = d.families.values().flat_map(|f| f.hardware.iter().map(|r| r.hw_code.clone())).collect();
    let missing = codes.iter().filter(|c| get(&images::image_key(c).unwrap()).is_none()).count();
    check("hardware codes without a picture", "0".into(), missing.to_string());
    // Cache round trip and selection.
    let dir = std::env::temp_dir().join(format!("fmcfg-img-it-{}", std::process::id()));
    let set = images::load(std::path::Path::new(&path), &dir).unwrap();
    check("cached pictures", "131".into(), set.count().to_string());
    let mut imgs = images::Images::default();
    imgs.add(set);
    check("FMB1:6 source", "Exact".into(), format!("{:?}", imgs.find(Some("FMB1:6")).0));
    check("FMB9:99 source", "Unknown".into(), format!("{:?}", imgs.find(Some("FMB9:99")).0));
    let mut lib = Library::default();
    lib.add(d, false);
    check("FMB1:6 model", "FMB140".into(), lib.model_name("FMB1:6").name.unwrap_or_default());
    let _ = std::fs::remove_dir_all(&dir);
    println!("{}", report.join("\n"));
    assert!(report.iter().all(|r| r.starts_with("PASS")), "{}", report.join("\n"));
}
