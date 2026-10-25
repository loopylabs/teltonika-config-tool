//! Device pictures (docs/spec/10-device-presentation.md §1-§2). Read at run time from the
//! user's own Configurator download and cached only in the app-data dir
//! (`device-images/<ProductVersion>-<sha256>/<image_key>.png`). Never bundled or committed.

use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::{bundle, exe_bytes, hex, metadata, pe, resources, DefsError};

const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const UNKNOWN: &str = "unknown";

/// `FMB1:6` -> `fmb1_6`. `None` when the code is empty or has characters that cannot be a file name.
pub fn image_key(hw_code: &str) -> Option<String> {
    let key = hw_code.trim().replace(':', "_").to_ascii_lowercase();
    (!key.is_empty() && key.len() <= 64 && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.') && !key.starts_with('.')).then_some(key)
}

/// Counts and sizes of one extraction (never the image bytes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stats {
    pub blob_len: usize,
    pub blob_sha256: String,
    pub entries: usize,
    pub device_pictures: usize,
    pub device_bytes: usize,
}

pub struct Extracted {
    pub product_version: String,
    pub stats: Stats,
    /// (image_key, PNG bytes) for each `devices/*.png` entry.
    pub pictures: Vec<(String, Vec<u8>)>,
}

/// The definition library's `.g.resources` blob (spec 10 §1.1).
fn resource_blob(input: &[u8]) -> Result<(String, Vec<u8>), DefsError> {
    let exe = exe_bytes(input)?;
    let (pv, dll) = if exe.starts_with(b"MZ") && bundle::Bundle::parse(&exe).is_err() && metadata::manifest_resources(&exe).is_ok() {
        ("unknown".to_string(), exe)
    } else {
        let img = pe::Pe::parse(&exe).map_err(DefsError::Pe)?;
        let pv = img.version_string("ProductVersion").unwrap_or_else(|| "unknown".into());
        let b = bundle::Bundle::parse(&exe).map_err(DefsError::Bundle)?;
        let e = b.find("Brand.dll").ok_or(DefsError::NoDefinitions)?;
        (pv, b.read(&exe, e).map_err(DefsError::Bundle)?)
    };
    let res = metadata::manifest_resources(&dll).map_err(DefsError::Meta)?;
    let blob = res.into_iter().find(|r| r.name.ends_with(".g.resources")).ok_or(DefsError::NoDefinitions)?;
    Ok((pv, blob.data))
}

pub fn extract_pictures(input: &[u8]) -> Result<Extracted, DefsError> {
    let (product_version, blob) = resource_blob(input)?;
    let list = resources::entries(&blob).map_err(|e| DefsError::Io(e.to_string()))?;
    let mut pictures = Vec::new();
    let mut stats = Stats { blob_len: blob.len(), blob_sha256: hex(&Sha256::digest(&blob)), entries: list.len(), ..Stats::default() };
    for e in &list {
        let lower = e.name.to_ascii_lowercase();
        let Some(key) = lower.strip_prefix("devices/").and_then(|n| n.strip_suffix(".png")) else { continue };
        let bytes = &blob[e.start..e.start + e.len];
        // Spec 10 UNKNOWN 6: a changed format is skipped, not cached as junk.
        if !bytes.starts_with(PNG_SIGNATURE) || image_key(key).as_deref() != Some(key) {
            continue;
        }
        stats.device_pictures += 1;
        stats.device_bytes += bytes.len();
        pictures.push((key.to_string(), bytes.to_vec()));
    }
    Ok(Extracted { product_version, stats, pictures })
}

fn cache_name(pv: &str, sha: &str) -> String {
    let pv: String = pv.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.').collect();
    format!("{pv}-{sha}")
}

/// One loaded build's cached pictures.
#[derive(Debug, Clone)]
pub struct ImageSet {
    pub product_version: String,
    pub dir: PathBuf,
    keys: std::collections::BTreeSet<String>,
}

impl ImageSet {
    #[cfg(test)]
    pub fn count(&self) -> usize {
        self.keys.len()
    }

    pub fn has(&self, key: &str) -> bool {
        self.keys.contains(key)
    }

    fn scan(product_version: String, dir: PathBuf) -> Option<Self> {
        dir.join(".complete").exists().then_some(())?;
        let keys = std::fs::read_dir(&dir).ok()?.flatten().filter_map(|e| e.file_name().to_str()?.strip_suffix(".png").map(str::to_string)).collect();
        Some(ImageSet { product_version, dir, keys })
    }
}

/// Loads the pictures of the Configurator at `path` from the cache, or extracts and caches them.
pub fn load(path: &Path, cache_dir: &Path) -> Result<ImageSet, DefsError> {
    let input = std::fs::read(path).map_err(|e| DefsError::Io(e.to_string()))?;
    let sha = hex(&Sha256::digest(&input));
    let root = cache_dir.join("device-images");
    if let Ok(rd) = std::fs::read_dir(&root) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if let Some(pv) = name.strip_suffix(&format!("-{sha}")) {
                if let Some(set) = ImageSet::scan(pv.to_string(), e.path()) {
                    return Ok(set);
                }
            }
        }
    }
    let x = extract_pictures(&input)?;
    let dir = root.join(cache_name(&x.product_version, &sha));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| DefsError::Io(e.to_string()))?;
    for (key, bytes) in &x.pictures {
        std::fs::write(dir.join(format!("{key}.png")), bytes).map_err(|e| DefsError::Io(e.to_string()))?;
    }
    std::fs::write(dir.join(".complete"), x.stats.device_pictures.to_string()).map_err(|e| DefsError::Io(e.to_string()))?;
    ImageSet::scan(x.product_version, dir).ok_or(DefsError::NoDefinitions)
}

/// Which picture a device gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// The entry for the reported hardware code.
    Exact,
    /// The code has no entry (or no code): the Configurator's generic picture.
    Unknown,
    /// No Configurator loaded or no usable picture: the caller draws its own icon.
    None,
}

/// Loaded builds' pictures, newest first (spec 10 §2; the primary build wins, older builds fill in).
#[derive(Default)]
pub struct Images {
    sets: Vec<ImageSet>,
}

impl Images {
    pub fn add(&mut self, s: ImageSet) {
        self.sets.retain(|x| x.product_version != s.product_version);
        self.sets.push(s);
        self.sets.sort_by(|a, b| super::compose::version_key(&b.product_version).cmp(&super::compose::version_key(&a.product_version)));
    }

    pub fn remove(&mut self, product_version: &str) {
        self.sets.retain(|s| s.product_version != product_version);
    }

    /// File to show for `hw_code` (as reported, not alias-resolved) and which rule picked it.
    pub fn find(&self, hw_code: Option<&str>) -> (Source, Option<PathBuf>) {
        let pick = |key: &str| self.sets.iter().find(|s| s.has(key)).map(|s| s.dir.join(format!("{key}.png")));
        if let Some(p) = hw_code.and_then(image_key).and_then(|k| pick(&k)) {
            return (Source::Exact, Some(p));
        }
        match pick(UNKNOWN) {
            Some(p) => (Source::Unknown, Some(p)),
            None => (Source::None, None),
        }
    }
}

pub fn base64(data: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16) | (u32::from(*c.get(1).unwrap_or(&0)) << 8) | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            out.push(if i <= c.len() { A[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::metadata::tests::build_assembly;
    use crate::defs::resources::tests::build_set;

    fn png(tag: u8) -> Vec<u8> {
        let mut v = PNG_SIGNATURE.to_vec();
        v.extend_from_slice(&[tag; 10]);
        v
    }

    fn input() -> Vec<u8> {
        let set = build_set(
            &[("devices/fmb1_6.png", 0x21, &png(1)), ("Devices/UNKNOWN.png", 0x21, &png(2)), ("devices/bad.png", 0x21, b"not a png"), ("misc/usb.png", 0x21, &png(3)), ("devices/../x.png", 0x21, &png(4))],
            &[],
        );
        build_assembly(&[("Brand.Configurations.FMB.Base.xml", b"<x/>"), ("Brand.g.resources", &set)])
    }

    #[test]
    fn image_keys() {
        assert_eq!(image_key("FMB1:6").as_deref(), Some("fmb1_6"));
        assert_eq!(image_key("FMB640:1").as_deref(), Some("fmb640_1"));
        assert_eq!(image_key(""), None);
        assert_eq!(image_key("../etc"), None);
        assert_eq!(image_key("A/B"), None);
    }

    #[test]
    fn extracts_only_device_pngs_from_a_bare_library() {
        let x = extract_pictures(&input()).unwrap();
        let keys: Vec<&str> = x.pictures.iter().map(|p| p.0.as_str()).collect();
        assert_eq!(keys, ["fmb1_6", "unknown"]);
        assert_eq!(x.stats.entries, 5);
        assert_eq!(x.stats.device_pictures, 2);
        assert_eq!(x.stats.device_bytes, 2 * (PNG_SIGNATURE.len() + 10));
        assert_eq!(x.product_version, "unknown");
    }

    #[test]
    fn caches_and_selects() {
        let dir = std::env::temp_dir().join(format!("fmcfg-img-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("in.exe");
        std::fs::write(&cfg, input()).unwrap();
        let cache = dir.join("cache");
        let set = load(&cfg, &cache).unwrap();
        assert_eq!(set.count(), 2);
        let again = load(&cfg, &cache).unwrap();
        assert_eq!(again.dir, set.dir);
        let mut imgs = Images::default();
        assert_eq!(imgs.find(Some("FMB1:6")).0, Source::None);
        imgs.add(set);
        let (s, p) = imgs.find(Some("FMB1:6"));
        assert_eq!(s, Source::Exact);
        assert_eq!(std::fs::read(p.unwrap()).unwrap(), png(1));
        assert_eq!(imgs.find(Some("FMB9:9")).0, Source::Unknown);
        assert_eq!(imgs.find(None).0, Source::Unknown);
        assert_eq!(imgs.find(Some("../x")).0, Source::Unknown);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn base64_matches_known_vectors() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foob"), "Zm9vYg==");
    }
}
