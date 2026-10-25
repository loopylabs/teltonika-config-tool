//! Definition documents (08 §3) parsed into an owned model (04).

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct SchemaError(pub String);

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "definition format not recognised: {}", self.0)
    }
}

/// Generic element: the definition tree keeps every element (presentation included).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Element {
    pub name: String,
    pub attrs: BTreeMap<String, String>,
    pub children: Vec<Element>,
}

impl Element {
    pub fn attr(&self, k: &str) -> Option<&str> {
        self.attrs.get(k).map(String::as_str)
    }
    pub fn child(&self, n: &str) -> Option<&Element> {
        self.children.iter().find(|c| c.name == n)
    }
    pub fn kids<'a>(&'a self, n: &str) -> impl Iterator<Item = &'a Element> + 'a {
        let n = n.to_string();
        self.children.iter().filter(move |c| c.name == n)
    }
}

pub fn parse_xml(bytes: &[u8], want_root: &str) -> Result<Element, SchemaError> {
    let text = std::str::from_utf8(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
        .map_err(|_| SchemaError("document is not UTF-8".into()))?;
    let doc = roxmltree::Document::parse(text).map_err(|e| SchemaError(format!("XML: {e}")))?;
    let root = doc.root_element();
    if root.tag_name().name() != want_root {
        return Err(SchemaError(format!("expected root <{want_root}>, found <{}>", root.tag_name().name())));
    }
    Ok(convert(root))
}

fn convert(n: roxmltree::Node) -> Element {
    Element {
        name: n.tag_name().name().to_string(),
        attrs: n
            .attributes()
            .filter(|a| a.namespace().is_none())
            .map(|a| (a.name().to_string(), a.value().to_string()))
            .collect(),
        children: n.children().filter(|c| c.is_element()).map(convert).collect(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareRow {
    pub hw_code: String,
    pub model_name: String,
    pub attributes: Vec<String>,
    pub alias_of: Option<String>,
    pub technology: Option<String>,
    pub legacy: Option<String>,
    pub help_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Ops {
    pub add: Vec<Element>,
    pub remove: Vec<String>,
    pub modify: Vec<Element>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareBlock {
    pub codes: String,
    pub extends: Option<String>,
    pub ops: Ops,
    /// Raw `(spec_id, patch_number)` references in document order; the patch
    /// number is kept as text (it may be blank) and matched as an integer (04 §3.4).
    pub specs: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Version {
    pub version: String,
    pub parent: Option<String>,
    pub title: Option<String>,
    pub end_of_life: bool,
    pub blocks: Vec<HardwareBlock>,
    /// Operations directly under the version node (04 §3.3: every code of the family).
    pub family_ops: Ops,
    /// Spec references directly under the version node (04 §3.4: all apply).
    #[serde(default)]
    pub family_specs: Vec<(u32, String)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Family {
    pub name: String,
    pub keyword_id: Option<u32>,
    pub hardware: Vec<HardwareRow>,
    pub versions: Vec<Version>,
    pub base: Element,
    pub collections: BTreeMap<String, Element>,
    pub lists: BTreeMap<String, Element>,
    pub templates: BTreeMap<String, Element>,
    /// spec id -> patch number -> ops
    pub specs: BTreeMap<u32, BTreeMap<String, Ops>>,
    pub presets: BTreeMap<String, Element>,
    pub document_count: usize,
}

fn named(doc: &Element, kid: &str) -> BTreeMap<String, Element> {
    doc.kids(kid).filter_map(|e| Some((e.attr("Name")?.to_string(), e.clone()))).collect()
}

pub fn ops_of(parent: &Element) -> Ops {
    let items = |n: &str| -> Vec<&Element> { parent.kids(n).flat_map(|o| o.kids("Item")).collect() };
    Ops {
        add: items("Add").into_iter().cloned().collect(),
        remove: items("Remove").into_iter().filter_map(|i| i.attr("Guid").map(str::to_string)).collect(),
        modify: items("Modify").into_iter().cloned().collect(),
    }
}

/// `(spec_id, patch_number)` references of a version or hardware block. A
/// reference without a numeric spec ID cannot match any device and is dropped.
fn spec_refs(parent: &Element) -> Vec<(u32, String)> {
    parent
        .kids("Spec")
        .filter_map(|sp| Some((sp.attr("Id")?.trim().parse().ok()?, sp.attr("PatchNo").unwrap_or("").to_string())))
        .collect()
}

/// Raw documents of one family: (file kind, bytes).
pub fn build_family(name: &str, docs: &BTreeMap<String, Vec<u8>>) -> Result<Family, SchemaError> {
    let get = |k: &str| docs.get(k).ok_or_else(|| SchemaError(format!("{name}: missing {k} document")));
    for k in ["UpdateConfiguration"] {
        get(k)?;
    }
    let base = parse_xml(get("Base")?, "Configuration")?;
    let keyword_id = base.child("Keyword").and_then(|k| k.attr("ParameterId")).and_then(|v| v.parse().ok());
    let hw = parse_xml(get("Hardware")?, "HardwareVersionTypes")?;
    let opt = |e: &Element, k: &str| e.attr(k).map(str::to_string);
    let hardware = hw
        .kids("HardwareVersionType")
        .map(|r| HardwareRow {
            hw_code: r.attr("Version").unwrap_or_default().to_string(),
            model_name: r.attr("FmType").unwrap_or_default().to_string(),
            attributes: r.attr("Attributes").unwrap_or("").split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect(),
            alias_of: opt(r, "BaseOf"),
            technology: opt(r, "Technology"),
            legacy: opt(r, "Legacy"),
            help_url: opt(r, "WikiUrl"),
        })
        .collect();
    let ext = parse_xml(get("BaseExtension")?, "BaseExtensions")?;
    let versions: Vec<Version> = ext
        .kids("Software")
        .map(|s| Version {
            version: s.attr("Version").unwrap_or_default().to_string(),
            parent: opt(s, "DiffFor"),
            title: opt(s, "Title"),
            end_of_life: s.attr("EOL") == Some("true"),
            blocks: s
                .kids("Hardware")
                .map(|h| HardwareBlock {
                    codes: h.attr("Version").unwrap_or_default().to_string(),
                    extends: opt(h, "Extends"),
                    ops: ops_of(h),
                    specs: spec_refs(h),
                })
                .collect(),
            family_ops: ops_of(s),
            family_specs: spec_refs(s),
        })
        .collect();
    // 08 §5.3 chain sanity.
    if versions.iter().any(|v| v.version.is_empty()) {
        return Err(SchemaError(format!("{name}: version node without Version")));
    }
    let roots = versions.iter().filter(|v| v.parent.is_none()).count();
    if roots != 1 {
        return Err(SchemaError(format!("{name}: {roots} root versions (expected 1)")));
    }
    for v in &versions {
        if let Some(p) = &v.parent {
            if !versions.iter().any(|x| &x.version == p) {
                return Err(SchemaError(format!("{name}: version {} extends unknown {p}", v.version)));
            }
        }
    }
    let collections = named(&parse_xml(get("Collections")?, "CollectionConfigurations")?, "CollectionConfiguration");
    let lists = named(&parse_xml(get("Lists")?, "ListConfigurations")?, "ListConfiguration");
    let templates = named(&parse_xml(get("Templates")?, "IOTemplates")?, "IOTemplate");
    let mut specs = BTreeMap::new();
    let mut presets = BTreeMap::new();
    for (k, bytes) in docs {
        if let Some(id) = k.strip_prefix("Specs.") {
            let doc = parse_xml(bytes, "Spec")?;
            let id: u32 = id.parse().map_err(|_| SchemaError(format!("{name}: bad spec id {id}")))?;
            let patches = doc.kids("Patch").filter_map(|p| Some((p.attr("No")?.to_string(), ops_of(p)))).collect();
            specs.insert(id, patches);
        } else if let Some(p) = k.strip_prefix("Presets.") {
            presets.insert(p.to_string(), parse_xml(bytes, "Preset")?);
        }
    }
    Ok(Family {
        name: name.to_string(),
        keyword_id,
        hardware,
        versions,
        base,
        collections,
        lists,
        templates,
        specs,
        presets,
        document_count: docs.len(),
    })
}
