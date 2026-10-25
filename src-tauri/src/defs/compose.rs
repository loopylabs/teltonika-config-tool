//! Composition (04 §3): select hardware row and version chain, apply layer
//! operations and filters, and collect the parameter set.

use std::collections::BTreeMap;

use serde::Serialize;

use super::model::{Element, Family, HardwareRow, Ops};

/// Where the spec ID comes from. There is deliberately no way to pick an
/// arbitrary spec ID: standard (1) or whatever the connected device reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecSource {
    Standard,
    ReportedByDevice(u32),
}

impl SpecSource {
    pub fn id(self) -> u32 {
        match self {
            SpecSource::Standard => 1,
            SpecSource::ReportedByDevice(n) => n.max(1),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Param {
    pub id: u32,
    pub kind: String,
    pub attrs: BTreeMap<String, String>,
    pub label_hint: Option<String>,
    /// Slot number (1-based) when the parameter is an entry of a list.
    pub slot: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Composed {
    pub model_name: String,
    pub hw_code: String,
    pub chain: Vec<String>,
    pub tree: Element,
    pub params: BTreeMap<u32, Param>,
    /// Add tags that found no anchor (04 §3.10 expects about 8 to 13 per model).
    pub unanchored_adds: Vec<String>,
    /// IO elements dropped because their custom group has no custom IO table (04 §3.9).
    pub custom_io_dropped: usize,
}

pub fn resolve_row<'a>(f: &'a Family, code: &str) -> Option<&'a HardwareRow> {
    f.hardware
        .iter()
        .find(|r| r.alias_of.as_deref().is_some_and(|a| a.eq_ignore_ascii_case(code)))
        .or_else(|| f.hardware.iter().find(|r| r.hw_code.eq_ignore_ascii_case(code)))
}

/// `FMB1:[1-4],FMB2:3` style hardware code lists.
pub fn code_matches(list: &str, code: &str) -> bool {
    let Some((cp, cn)) = code.split_once(':') else { return false };
    let Ok(cn) = cn.trim().parse::<u32>() else { return false };
    list.split(',').any(|item| {
        let Some((p, n)) = item.trim().split_once(':') else { return false };
        if !p.trim().eq_ignore_ascii_case(cp.trim()) {
            return false;
        }
        let n = n.trim();
        if let Some(r) = n.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            let Some((a, b)) = r.split_once('-') else { return false };
            matches!((a.trim().parse::<u32>(), b.trim().parse::<u32>()), (Ok(a), Ok(b)) if a <= cn && cn <= b)
        } else {
            n.parse::<u32>() == Ok(cn)
        }
    })
}

pub fn version_key(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

pub fn chain_for(f: &Family, hw: &str, cfg: &str) -> Vec<String> {
    let want = version_key(cfg);
    let selected = f
        .versions
        .iter()
        .filter(|v| v.blocks.iter().any(|b| code_matches(&b.codes, hw)) && version_key(&v.version) <= want)
        .max_by(|a, b| version_key(&a.version).cmp(&version_key(&b.version)));
    let mut chain = Vec::new();
    let mut cur = selected;
    while let Some(v) = cur {
        if chain.contains(&v.version) {
            break;
        }
        chain.push(v.version.clone());
        cur = v.parent.as_ref().and_then(|p| f.versions.iter().find(|x| &x.version == p));
    }
    chain.reverse();
    chain
}

fn apply_add(tree: &mut Element, item: &Element) -> usize {
    let Some(tag) = item.attr("Guid") else { return 0 };
    let mut hits = 0;
    let mut i = 0;
    while i < tree.children.len() {
        let c = &tree.children[i];
        if c.name == "Placeholder" && c.attr("Guid") == Some(tag) {
            let new = item.children.clone();
            let n = new.len();
            tree.children.splice(i..i, new);
            i += n + 1;
            hits += 1;
        } else {
            hits += apply_add(&mut tree.children[i], item);
            i += 1;
        }
    }
    hits
}

fn apply_remove(tree: &mut Element, tag: &str) {
    tree.children.retain(|c| c.name == "Placeholder" || c.attr("Guid") != Some(tag));
    for c in &mut tree.children {
        apply_remove(c, tag);
    }
}

/// TODO(spec): 04 §3.3 / open question 30 — Modify matching is only partly known;
/// children of the tagged element whose kind matches a replacement child are replaced.
fn apply_modify(tree: &mut Element, item: &Element) {
    let Some(tag) = item.attr("Guid") else { return };
    for c in &mut tree.children {
        if c.name != "Placeholder" && c.attr("Guid") == Some(tag) {
            for r in &item.children {
                if let Some(slot) = c.children.iter_mut().find(|x| x.name == r.name) {
                    *slot = r.clone();
                } else {
                    c.children.push(r.clone());
                }
            }
        }
        apply_modify(c, item);
    }
}

/// Feature filter (04 §3.5); also discards anchors.
fn filter(tree: &mut Element, mask: u64) {
    tree.children.retain(|c| {
        if c.name == "Placeholder" {
            return false;
        }
        if let Some(bit) = c.attr("Feature").and_then(|b| b.trim().parse::<u32>().ok()) {
            if mask >> (bit % 64) & 1 == 0 {
                return false;
            }
        }
        true
    });
    for c in &mut tree.children {
        filter(c, mask);
    }
}

fn expand_templates(tree: &mut Element, templates: &BTreeMap<String, Element>) {
    if tree.name == "IOItem" {
        if let Some(t) = tree.attr("IOTemplate").and_then(|n| templates.get(n)).and_then(|t| t.child("IOItem")).cloned() {
            for (k, v) in &t.attrs {
                tree.attrs.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for sub in t.children {
                if !tree.children.iter().any(|c| c.name == sub.name) {
                    tree.children.push(sub);
                }
            }
        }
    }
    for c in &mut tree.children {
        expand_templates(c, templates);
    }
}

const NOT_PARAMS: [&str; 5] = ["Keyword", "Thumb", "If", "NoAction", "Rule"];
const IO_SUB: [(&str, u32); 7] =
    [("Priority", 0), ("Operand", 1), ("HighLevel", 2), ("LowLevel", 3), ("EventOnly", 4), ("AveragingConstant", 5), ("TimeInterval", 6)];
const CAN_SUB: [(&str, u32); 4] = [("CanType", 0), ("DataMask", 1), ("CanId", 2), ("CanSrc", 3)];
const ZONE_FIELDS: [&str; 12] = [
    "Priority", "GenerateEvent", "EventualRecords", "FrameBorder", "ShapeType", "Radius",
    "LongitudeX1", "LatitudeY1", "LongitudeX2", "LatitudeY2", "Overspeeding", "MaxSpeed",
];

fn num(e: &Element, k: &str) -> Option<u32> {
    e.attr(k).and_then(|v| v.trim().parse().ok())
}

fn insert(out: &mut BTreeMap<u32, Param>, id: u32, e: &Element, label: Option<String>, slot: Option<u32>) {
    out.entry(id).or_insert_with(|| Param { id, kind: e.name.clone(), attrs: e.attrs.clone(), label_hint: label, slot });
}

/// Parameters an element defines itself (not its descendants'), with a label hint for derived IDs.
/// (id, defining element, label hint, list slot)
pub(crate) type Own<'a> = (u32, &'a Element, Option<String>, Option<u32>);

/// `HighLevel` -> `High level`.
fn humanize(s: &str) -> String {
    let mut out = String::new();
    for (i, ch) in s.chars().enumerate() {
        if ch.is_uppercase() && i > 0 {
            out.push(' ');
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

pub(crate) fn own_params<'a>(e: &'a Element, f: &'a Family) -> Vec<Own<'a>> {
    let mut v = Vec::new();
    if let Some(id) = num(e, "ParameterId") {
        if !NOT_PARAMS.contains(&e.name.as_str()) {
            v.push((id, e, None, None));
        }
    }
    match e.name.as_str() {
        "IOItem" => {
            let name = e.attr("Name").unwrap_or("IO element");
            if let Some(io) = num(e, "IoId") {
                for c in &e.children {
                    let fixed = IO_SUB.iter().chain(CAN_SUB.iter()).find(|(n, _)| *n == c.name).map(|x| x.1);
                    if let Some(off) = num(c, "Offset").or(fixed) {
                        v.push((io + off, c, Some(format!("{name} · {}", humanize(&c.name))), None));
                    }
                }
            }
            if let Some(sms) = num(e, "SmsEventId") {
                if let Some(t) = e.child("Text") {
                    v.push((sms, t, Some(format!("{name} · SMS text")), None));
                }
                if let (Some(p), Some(id)) = (e.child("Phone"), sms.checked_sub(1000)) {
                    v.push((id, p, Some(format!("{name} · SMS phone")), None));
                }
            }
        }
        "List" => {
            if let Some((n, l)) = e.attr("List").and_then(|n| f.lists.get(n).map(|l| (n, l))) {
                if let (Some(s), Some(cnt)) = (num(l, "StartId"), num(l, "NumberOfItems")) {
                    for i in 0..cnt {
                        v.push((s + i, l, Some(n.to_string()), Some(i + 1)));
                    }
                }
            }
        }
        "Geozones" => {
            // TODO(spec): open question 31 — sub-offsets 12–16 are low certainty; only
            // the fields actually present as children are counted, in sub-index order.
            if let (Some(s), Some(stride), Some(n)) = (num(e, "StartId"), num(e, "Offset"), num(e, "Count")) {
                for k in 0..n {
                    // A polygon block child is not a zone sub-field (its point layout is UNKNOWN);
                    // 08 v3 §6 FMB640-family figures count no IDs for it.
                    for (off, c) in e.children.iter().filter(|c| c.name != "Polygon").enumerate() {
                        let off = ZONE_FIELDS.iter().position(|z| *z == c.name).unwrap_or(off) as u32;
                        v.push((s + k * stride + off, c, Some(format!("Zone {} · {}", k + 1, humanize(&c.name))), None));
                    }
                }
            }
        }
        // TODO(spec): open question 31 — polygon point ID layout UNKNOWN; not expanded.
        _ => {}
    }
    v
}

fn collect(e: &Element, f: &Family, out: &mut BTreeMap<u32, Param>) {
    for (id, el, label, slot) in own_params(e, f) {
        insert(out, id, el, label, slot);
    }
    for c in &e.children {
        collect(c, f, out);
    }
}

/// Effective hardware code for each version of the chain (04 §3.2 extends rule),
/// walking from the newest version back: once a version's matching block declares
/// `extends = <older>`, every older version contributes its blocks for `<older>`.
pub fn effective_codes(f: &Family, chain: &[String], hw: &str) -> Vec<String> {
    let mut cur = hw.to_string();
    let mut out = vec![String::new(); chain.len()];
    for (i, ver) in chain.iter().enumerate().rev() {
        out[i] = cur.clone();
        let Some(v) = f.versions.iter().find(|x| &x.version == ver) else { continue };
        if let Some(ext) = v.blocks.iter().filter(|b| code_matches(&b.codes, &cur)).find_map(|b| b.extends.clone()) {
            cur = ext.trim().to_string();
        }
    }
    out
}

/// Parse a spec or patch number as an integer (04 §3.4); blank -> None.
fn patch_number(s: &str) -> Result<Option<u32>, String> {
    let t = s.trim();
    if t.is_empty() {
        return Ok(None);
    }
    t.parse::<u32>().map(Some).map_err(|_| format!("spec patch number {t:?} is not a number"))
}

fn find_patch<'a>(f: &'a Family, spec_id: u32, patch: u32, ver: &str) -> Result<&'a Ops, String> {
    let patches = f.specs.get(&spec_id).ok_or_else(|| format!("version {ver} references spec {spec_id}, which has no spec document"))?;
    patches
        .iter()
        .find(|(no, _)| no.trim().parse::<u32>().ok() == Some(patch))
        .map(|(_, o)| o)
        .ok_or_else(|| format!("version {ver} references spec {spec_id} patch {patch}, which does not exist"))
}

/// Operations of one version in collection order (04 §3.3): version-level, then the
/// hardware blocks matching the effective code, then the matching spec patches.
fn collect_version_ops<'a>(f: &'a Family, ver: &str, code: &str, spec_id: u32, out: &mut Vec<&'a Ops>) -> Result<(), String> {
    let v = f.versions.iter().find(|x| x.version == ver).ok_or_else(|| format!("version {ver} missing from the chain"))?;
    out.push(&v.family_ops);
    let blocks: Vec<_> = v.blocks.iter().filter(|b| code_matches(&b.codes, code)).collect();
    for b in &blocks {
        out.push(&b.ops);
    }
    if spec_id <= 1 {
        return Ok(()); // spec 1 never patches (04 §3.4, inferred)
    }
    // Version-level references: all that match apply.
    for (sid, no) in &v.family_specs {
        if *sid == spec_id {
            if let Some(n) = patch_number(no)? {
                out.push(find_patch(f, spec_id, n, ver)?);
            }
        }
    }
    // Per hardware block: only the first reference with the device spec ID, chosen
    // AFTER references with a blank or missing patch number are discarded (04 §3.4),
    // so a blank reference never takes the "first" place.
    for b in &blocks {
        let mut numbered = Vec::new();
        for (_, no) in b.specs.iter().filter(|(sid, _)| *sid == spec_id) {
            if let Some(n) = patch_number(no)? {
                numbered.push(n);
            }
        }
        if let Some(n) = numbered.first() {
            out.push(find_patch(f, spec_id, *n, ver)?);
        }
    }
    Ok(())
}

/// Insert every add, resolving nested adds (an add whose anchor is supplied by
/// another add) by repeating passes until no pending add finds an anchor.
/// Returns the tags of adds that never found an anchor (04 §3.10: not counted).
fn apply_adds(tree: &mut Element, adds: &[&Element]) -> Vec<String> {
    let mut pending: Vec<&Element> = adds.to_vec();
    loop {
        let before = pending.len();
        pending.retain(|a| apply_add(tree, a) == 0);
        if pending.len() == before || pending.is_empty() {
            break;
        }
    }
    pending.iter().filter_map(|a| a.attr("Guid").map(str::to_string)).collect()
}

/// Hardware-attribute filter (04 §3.6) on non-enumeration elements.
/// TODO(spec): enumeration items with a hardware condition should be hidden, not
/// removed; the editor still sees them here.
fn hw_filter(tree: &mut Element, attrs: &[String]) {
    tree.children.retain(|c| c.attr("HWAttribute").is_none_or(|a| attrs.iter().any(|x| x.eq_ignore_ascii_case(a.trim()))));
    for c in &mut tree.children {
        hw_filter(c, attrs);
    }
}

/// Custom-group IO (04 §3.9, "Custom-group rule in full"). An IO element's effective
/// custom-group name is its own attribute if it carries one (even when empty), else the
/// name copied from its IO template during template expansion, which must already have
/// run: a template-supplied name counts exactly like one written on the element. An
/// empty effective name makes an ordinary IO element. A non-empty name yields
/// parameters only when the composed tree has a custom IO table with exactly that name
/// (case-sensitive); otherwise the element is dropped. Returns the number dropped.
fn drop_unmatched_custom_io(tree: &mut Element) -> usize {
    let mut groups = Vec::new();
    custom_tables(tree, &mut groups);
    fn walk(e: &mut Element, groups: &[String]) -> usize {
        let before = e.children.len();
        e.children.retain(|c| c.name != "IOItem" || c.attr(CUSTOM_GROUP_ATTR).is_none_or(|g| g.is_empty() || groups.iter().any(|x| x == g)));
        let mut n = before - e.children.len();
        for c in &mut e.children {
            n += walk(c, groups);
        }
        n
    }
    walk(tree, &groups)
}

/// 04 §3.9 names the custom IO table and the custom-group name but not their data
/// form; these element and attribute names are read from the definition documents.
const CUSTOM_GROUP_ATTR: &str = "CustomIoItemName";
const CUSTOM_TABLE: &str = "CustomIoTable";

fn custom_tables(e: &Element, out: &mut Vec<String>) {
    if e.name == CUSTOM_TABLE {
        if let Some(g) = e.attr(CUSTOM_GROUP_ATTR) {
            out.push(g.to_string());
        }
    }
    for c in &e.children {
        custom_tables(c, out);
    }
}

#[cfg(test)]
pub fn compose(f: &Family, hw_code: &str, cfg_version: &str, spec: SpecSource, feature_mask: u64) -> Option<Composed> {
    try_compose(f, hw_code, cfg_version, spec, feature_mask).ok()
}

/// Full composition (04 §3.2-§3.9). Errors are readable (e.g. an invalid spec reference).
pub fn try_compose(f: &Family, hw_code: &str, cfg_version: &str, spec: SpecSource, feature_mask: u64) -> Result<Composed, String> {
    let row = resolve_row(f, hw_code).ok_or_else(|| format!("hardware code {hw_code} is not in the {} family", f.name))?;
    let hw = row.hw_code.clone();
    let chain = chain_for(f, &hw, cfg_version);
    if chain.is_empty() {
        return Err(format!("no configuration version for {hw} at or below {cfg_version}"));
    }
    let codes = effective_codes(f, &chain, &hw);
    let spec_id = spec.id();
    let mut ops: Vec<&Ops> = Vec::new();
    for (ver, code) in chain.iter().zip(&codes) {
        collect_version_ops(f, ver, code, spec_id, &mut ops)?;
    }
    // 04 §3.3: adds, then modifies, then removes, on the definition tree.
    let mut tree = f.base.clone();
    let adds: Vec<&Element> = ops.iter().flat_map(|o| o.add.iter()).collect();
    let unanchored = apply_adds(&mut tree, &adds);
    // TODO(spec): 04 §3.3 Modify matching is UNKNOWN beyond same-kind children.
    for m in ops.iter().flat_map(|o| o.modify.iter()) {
        apply_modify(&mut tree, m);
    }
    let removes: Vec<&String> = ops.iter().flat_map(|o| o.remove.iter()).collect();
    for r in &removes {
        apply_remove(&mut tree, r);
    }
    // Removes also apply to IO templates before expansion.
    let mut templates = f.templates.clone();
    for t in templates.values_mut() {
        for r in &removes {
            apply_remove(t, r);
        }
        hw_filter(t, &row.attributes);
    }
    hw_filter(&mut tree, &row.attributes);
    // TODO(spec): spec-limited enumeration items (04 §3.4) are not filtered (unused in 1.8.21).
    // TODO(spec): 04 §3.3 geofence output-list length adjustment: effect UNKNOWN, not applied.
    expand_templates(&mut tree, &templates);
    // Feature filter last (04 §3.5); hardware options (§3.7) are unused in 1.8.21.
    filter(&mut tree, feature_mask);
    // Custom-group rule (04 §3.9) after all filters: a filtered-out table supplies nothing.
    let custom_io_dropped = drop_unmatched_custom_io(&mut tree);
    let mut params = BTreeMap::new();
    collect(&tree, f, &mut params);
    Ok(Composed { model_name: row.model_name.clone(), hw_code: hw, chain, tree, params, unanchored_adds: unanchored, custom_io_dropped })
}

#[cfg_attr(not(test), allow(dead_code))] // integration test check (08 §6)
/// Distinct parameter IDs across every definition document of a family (no composition).
pub fn union_ids(f: &Family) -> usize {
    let mut out = BTreeMap::new();
    collect(&f.base, f, &mut out);
    for v in &f.versions {
        for o in std::iter::once(&v.family_ops).chain(v.blocks.iter().map(|b| &b.ops)) {
            for e in o.add.iter().chain(o.modify.iter()) {
                collect(e, f, &mut out);
            }
        }
    }
    for patches in f.specs.values() {
        for o in patches.values() {
            for e in o.add.iter().chain(o.modify.iter()) {
                collect(e, f, &mut out);
            }
        }
    }
    for t in f.templates.values() {
        collect(t, f, &mut out);
    }
    out.len()
}
