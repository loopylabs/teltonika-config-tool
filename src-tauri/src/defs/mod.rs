//! Definitions extractor (docs/spec/08-definitions-source.md) producing the
//! definition model of docs/spec/04-parameters.md from the user's own
//! Configurator download. Vendor data is never bundled, committed or uploaded;
//! the parsed model is cached only in the app data dir.

pub mod bundle;
pub mod compose;
pub mod editor;
pub mod images;
pub mod metadata;
pub mod model;
pub mod naming;
pub mod pe;
pub mod present;
pub mod resources;

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use model::Family;

const PREFIX: &str = "Brand.Configurations.";
const FIXED: [&str; 7] = ["Base", "BaseExtension", "Collections", "Lists", "Hardware", "Templates", "UpdateConfiguration"];
pub const VERIFIED_BUILD: &str = "1.8.21.0";

#[derive(Debug)]
pub enum DefsError {
    Io(String),
    Zip(String),
    Pe(pe::PeError),
    Bundle(bundle::BundleError),
    Meta(metadata::MetaError),
    NoDefinitions,
    Schema(String, model::SchemaError),
}

impl fmt::Display for DefsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DefsError::Io(e) => write!(f, "cannot read input: {e}"),
            DefsError::Zip(e) => write!(f, "archive: {e}"),
            DefsError::Pe(e) => write!(f, "{e}"),
            DefsError::Bundle(e) => write!(f, "{e}"),
            DefsError::Meta(e) => write!(f, "{e}"),
            DefsError::NoDefinitions => write!(f, "unsupported Configurator build (no definition data found)"),
            DefsError::Schema(v, e) => write!(f, "{e} (Configurator {v})"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Definitions {
    pub product_version: String,
    pub sha256: String,
    pub warnings: Vec<String>,
    pub families: BTreeMap<String, Family>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub product_version: String,
    pub families: Vec<String>,
    pub document_count: usize,
    pub warnings: Vec<String>,
    pub from_cache: bool,
    /// The newest loaded build; others only add what it lacks.
    #[serde(default)]
    pub primary: bool,
    /// Hardware codes this build contributes that no newer loaded build defines.
    #[serde(default)]
    pub extra_hardware: Vec<String>,
}

impl Definitions {
    pub fn summary(&self, from_cache: bool) -> Summary {
        Summary {
            product_version: self.product_version.clone(),
            families: self.families.keys().cloned().collect(),
            document_count: self.families.values().map(|f| f.document_count).sum(),
            warnings: self.warnings.clone(),
            from_cache,
            primary: false,
            extra_hardware: Vec::new(),
        }
    }
}

/// Several loaded Configurator builds (09 §6). The newest is primary; a hardware code
/// that only an older build defines (e.g. `FMM0:31`) is served from that build.
#[derive(Default)]
pub struct Library {
    /// Newest first (by product version), with the "loaded from cache" flag.
    builds: Vec<(std::sync::Arc<Definitions>, bool)>,
}

impl Library {
    pub fn add(&mut self, d: Definitions, from_cache: bool) {
        self.builds.retain(|(b, _)| b.product_version != d.product_version);
        self.builds.push((std::sync::Arc::new(d), from_cache));
        self.builds.sort_by(|a, b| compose::version_key(&b.0.product_version).cmp(&compose::version_key(&a.0.product_version)));
    }

    pub fn remove(&mut self, product_version: &str) {
        self.builds.retain(|(b, _)| b.product_version != product_version);
    }

    pub fn is_empty(&self) -> bool {
        self.builds.is_empty()
    }

    /// The build that defines `hw_code` (newest first) and its family. The FMB640 family
    /// is preferred over the FMB family within a build (04 §3.1).
    pub fn find(&self, hw_code: &str) -> Option<(std::sync::Arc<Definitions>, String)> {
        for (b, _) in &self.builds {
            let mut fams: Vec<&Family> = b.families.values().collect();
            fams.sort_by_key(|f| f.name == "FMB");
            if let Some(f) = fams.into_iter().find(|f| compose::resolve_row(f, hw_code).is_some()) {
                return Some((b.clone(), f.name.clone()));
            }
        }
        None
    }

    pub fn summaries(&self) -> Vec<Summary> {
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        let mut out = Vec::new();
        for (i, (b, cached)) in self.builds.iter().enumerate() {
            let mut s = b.summary(*cached);
            s.primary = i == 0;
            for f in b.families.values() {
                for r in &f.hardware {
                    let key = r.hw_code.to_ascii_uppercase();
                    if seen.insert(key) && i > 0 {
                        s.extra_hardware.push(r.hw_code.clone());
                    }
                }
            }
            out.push(s);
        }
        out
    }
}

/// Executable bytes from a `.zip` (member `Teltonika.Configurator_*.exe`) or the raw input.
pub fn exe_bytes(input: &[u8]) -> Result<Vec<u8>, DefsError> {
    if !input.starts_with(b"PK\x03\x04") {
        return Ok(input.to_vec());
    }
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(input)).map_err(|e| DefsError::Zip(e.to_string()))?;
    for i in 0..z.len() {
        let mut m = z.by_index(i).map_err(|e| DefsError::Zip(e.to_string()))?;
        let name = m.name().rsplit('/').next().unwrap_or("").to_ascii_lowercase();
        if name.starts_with("teltonika.configurator_") && name.ends_with(".exe") {
            let mut v = Vec::with_capacity(m.size() as usize);
            m.read_to_end(&mut v).map_err(|e| DefsError::Zip(e.to_string()))?;
            return Ok(v);
        }
    }
    Err(DefsError::Zip("no Teltonika.Configurator_*.exe inside the archive".into()))
}

/// Groups definition resources by family and file kind (08 §2.4).
pub fn group_resources(res: Vec<metadata::Resource>) -> BTreeMap<String, BTreeMap<String, Vec<u8>>> {
    let mut fams: BTreeMap<String, BTreeMap<String, Vec<u8>>> = BTreeMap::new();
    for r in res {
        let Some(rest) = r.name.strip_prefix(PREFIX).and_then(|s| s.strip_suffix(".xml")) else { continue };
        let Some((fam, file)) = rest.split_once('.') else { continue };
        fams.entry(fam.to_string()).or_default().insert(file.to_string(), r.data);
    }
    fams
}

pub fn extract(input: &[u8]) -> Result<Definitions, DefsError> {
    let sha256 = hex(&Sha256::digest(input));
    let exe = exe_bytes(input)?;
    let mut warnings = Vec::new();
    let (product_version, dll) = if exe.starts_with(b"MZ") && bundle::Bundle::parse(&exe).is_err() && metadata::manifest_resources(&exe).is_ok() {
        // An already-extracted definition library (08 §1.1).
        warnings.push("input is a bare definition library; build version unknown".into());
        ("unknown".to_string(), exe.clone())
    } else {
        let img = pe::Pe::parse(&exe).map_err(DefsError::Pe)?;
        let pv = img.version_string("ProductVersion").unwrap_or_else(|| "unknown".into());
        let company = img.version_string("CompanyName").unwrap_or_default();
        if company != "JSC Teltonika" {
            warnings.push(format!("unexpected CompanyName '{company}'"));
        }
        let b = bundle::Bundle::parse(&exe).map_err(DefsError::Bundle)?;
        if b.major != 6 {
            warnings.push(format!("bundle format version {}.{} not verified", b.major, b.minor));
        }
        let e = b.find("Brand.dll").ok_or(DefsError::NoDefinitions)?;
        // TODO(spec): open question 40 — fallback into embedded libraries not implemented.
        (pv, b.read(&exe, e).map_err(DefsError::Bundle)?)
    };
    if product_version != VERIFIED_BUILD {
        warnings.push(format!("Configurator {product_version}: only {VERIFIED_BUILD} was verified"));
    }
    let res = metadata::manifest_resources(&dll).map_err(DefsError::Meta)?;
    let grouped = group_resources(res);
    if grouped.is_empty() {
        return Err(DefsError::NoDefinitions);
    }
    let mut families = BTreeMap::new();
    for (name, docs) in &grouped {
        for k in FIXED {
            if !docs.contains_key(k) {
                return Err(DefsError::Schema(product_version, model::SchemaError(format!("{name}: missing {k} document"))));
            }
        }
        let fam = model::build_family(name, docs).map_err(|e| DefsError::Schema(product_version.clone(), e))?;
        families.insert(name.clone(), fam);
    }
    Ok(Definitions { product_version, sha256, warnings, families })
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Loads from the cache (`<dir>/definitions/<version>-<sha256>.json`) or extracts and caches.
pub fn load(path: &Path, cache_dir: Option<&Path>) -> Result<(Definitions, bool), DefsError> {
    let input = std::fs::read(path).map_err(|e| DefsError::Io(e.to_string()))?;
    let sha = hex(&Sha256::digest(&input));
    let cache_file = |pv: &str| -> Option<PathBuf> { cache_dir.map(|d| d.join("definitions").join(format!("{pv}-{sha}.json"))) };
    if let Some(dir) = cache_dir.map(|d| d.join("definitions")) {
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().ends_with(&format!("-{sha}.json")) {
                    if let Ok(d) = std::fs::read(e.path()).map_err(|_| ()).and_then(|b| serde_json::from_slice::<Definitions>(&b).map_err(|_| ())) {
                        return Ok((d, true));
                    }
                }
            }
        }
    }
    let defs = extract(&input)?;
    if let Some(f) = cache_file(&defs.product_version) {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        if let Ok(b) = serde_json::to_vec(&defs) {
            let _ = std::fs::write(f, b);
        }
    }
    Ok((defs, false))
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod presentation_tests;
