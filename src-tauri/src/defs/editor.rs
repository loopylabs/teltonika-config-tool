//! Editor view of a composed definition set: a page/group tree plus per-parameter
//! metadata (label, type, bounds, enumeration items, validation rule text).
//! Validation itself runs in the frontend against the current document values.

use std::collections::BTreeMap;

use serde::Serialize;

use super::compose::{own_params, Composed, Param};
use super::model::{Element, Family, HardwareRow};

#[derive(Debug, Serialize)]
pub struct EnumItem {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Serialize, Default)]
pub struct ParamDef {
    pub id: u32,
    pub label: String,
    /// Lower-case value type (`int32`, `byte`, `uint16`, `uint32`, `int64`, `float`, `double`, `string`, ...).
    pub value_type: String,
    /// `number`, `text`, `enum` or `other`.
    pub control: String,
    pub default: Option<String>,
    pub min: Option<String>,
    pub max: Option<String>,
    pub min_id: Option<u32>,
    pub max_id: Option<u32>,
    pub units: Option<String>,
    pub text_min: Option<u32>,
    pub text_max: Option<u32>,
    /// Rule names exactly as in the definition (`;`-separated in the source).
    pub validate: Vec<String>,
    pub options: Vec<EnumItem>,
    /// Bit-mask enumeration (several items may be ticked).
    pub bitmask: bool,
    /// Raw `Mask` text of the enumeration (hex mask or multi-field notation).
    pub mask: Option<String>,
    /// Display converter name (`hh:mm` minutes etc.), if any.
    pub converter: Option<String>,
    /// List name and 1-based slot, for entries of a list.
    pub list: Option<String>,
    pub slot: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct Node {
    /// `page`, `group` or `tab`.
    pub kind: String,
    pub title: String,
    pub ids: Vec<u32>,
    pub children: Vec<Node>,
}

/// One I/O element of the composed definitions (04 §4): the live status view looks
/// records up by `id` and shows `name` and `unit`. Per document, never global.
#[derive(Debug, Serialize)]
pub struct IoElement {
    pub id: u32,
    pub name: String,
    pub unit: Option<String>,
    /// The I/O group the element belongs to, if the document names one.
    pub group: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EditorModel {
    pub model_name: String,
    pub hw_code: String,
    pub build: String,
    pub chain: Vec<String>,
    pub pages: Vec<Node>,
    pub params: BTreeMap<u32, ParamDef>,
    pub io_items: Vec<IoElement>,
}

fn io_elements(e: &Element, group: Option<&str>, out: &mut Vec<IoElement>) {
    if e.name == "IOItem" {
        if let Some(id) = n(e.attr("IoId")) {
            out.push(IoElement {
                id,
                name: e.attr("Name").unwrap_or("IO element").to_string(),
                unit: e.attr("Units").filter(|u| !u.trim().is_empty()).map(str::to_string),
                group: group.map(str::to_string),
            });
        }
        return;
    }
    let group = if matches!(e.name.as_str(), "IO" | "GroupBox" | "Group") { e.attr("GroupName").or(e.attr("Name")).or(group) } else { group };
    for c in &e.children {
        io_elements(c, group, out);
    }
}

fn n(a: Option<&str>) -> Option<u32> {
    a.and_then(|v| v.trim().parse().ok())
}

fn def_of(p: &Param, f: &Family, row: &HardwareRow) -> ParamDef {
    let a = |k: &str| p.attrs.get(k).cloned();
    let collection = p.attrs.get("Collection").and_then(|c| f.collections.get(c));
    let list_type = p.attrs.get("ListType").cloned();
    let value_type = a("ParameterType")
        .or(list_type)
        .or_else(|| collection.and_then(|c| c.attr("CollectionType").map(str::to_string)))
        .unwrap_or_default()
        .to_ascii_lowercase();
    let options: Vec<EnumItem> = collection
        .map(|c| {
            c.kids("CollectionItem")
                // 04 §3.6: items needing a missing hardware attribute are hidden.
                .filter(|i| i.attr("HWAttribute").is_none_or(|h| row.attributes.iter().any(|x| x.eq_ignore_ascii_case(h))))
                .map(|i| EnumItem { value: i.attr("Value").unwrap_or("").to_string(), label: i.attr("Text").unwrap_or("").to_string() })
                .collect()
        })
        .unwrap_or_default();
    let is_list = p.attrs.contains_key("StartId");
    let control = if collection.is_some() {
        "enum"
    } else if value_type == "string" || value_type == "lls" || p.attrs.contains_key("TextMaxLength") || is_list && value_type != "int32" {
        "text"
    } else if matches!(value_type.as_str(), "int32" | "byte" | "uint16" | "uint32" | "int64" | "float" | "double") {
        "number"
    } else {
        "other"
    };
    ParamDef {
        id: p.id,
        label: a("Label").or_else(|| p.label_hint.clone()).unwrap_or_else(|| format!("{} {}", p.kind, p.id)),
        value_type,
        control: control.into(),
        default: a("Default").or_else(|| a("DefaultText")),
        min: a("Min"),
        max: a("Max"),
        min_id: n(p.attrs.get("MinId").map(String::as_str)),
        max_id: n(p.attrs.get("MaxId").map(String::as_str)),
        units: a("Units"),
        text_min: n(p.attrs.get("TextMinLength").map(String::as_str)),
        text_max: n(p.attrs.get("TextMaxLength").map(String::as_str)),
        validate: a("Validate").map(|v| v.split(';').map(str::to_string).filter(|s| !s.trim().is_empty()).collect()).unwrap_or_default(),
        options,
        bitmask: collection.is_some_and(|c| c.attr("Mask").is_some()),
        mask: collection.and_then(|c| c.attr("Mask").map(str::to_string)),
        converter: a("Converter"),
        list: p.slot.and(p.label_hint.clone()),
        slot: p.slot,
    }
}

fn layout(e: &Element, f: &Family, out: &mut Vec<Node>, own: &mut Vec<u32>) {
    let (kind, title) = match e.name.as_str() {
        "ViewConfiguration" => ("page", e.attr("Name")),
        "GroupBox" => ("group", e.attr("GroupName")),
        "TabGroup" | "Group" => ("tab", e.attr("GroupName").or(e.attr("Name"))),
        _ => ("", None),
    };
    let ids: Vec<u32> = own_params(e, f).iter().map(|x| x.0).collect();
    if kind.is_empty() {
        own.extend(ids);
        for c in &e.children {
            layout(c, f, out, own);
        }
        return;
    }
    let mut node = Node { kind: kind.into(), title: title.unwrap_or("").to_string(), ids, children: Vec::new() };
    let mut mine = Vec::new();
    for c in &e.children {
        layout(c, f, &mut node.children, &mut mine);
    }
    node.ids.extend(mine);
    out.push(node);
}

pub fn editor_model(c: &Composed, f: &Family, build: &str) -> EditorModel {
    let row = super::compose::resolve_row(f, &c.hw_code).expect("composed hardware row exists");
    let mut pages = Vec::new();
    let mut loose = Vec::new();
    layout(&c.tree, f, &mut pages, &mut loose);
    if !loose.is_empty() {
        pages.push(Node { kind: "page".into(), title: "Other".into(), ids: loose, children: Vec::new() });
    }
    let params = c.params.values().map(|p| (p.id, def_of(p, f, row))).collect();
    let mut io_items = Vec::new();
    io_elements(&c.tree, None, &mut io_items);
    EditorModel { model_name: c.model_name.clone(), hw_code: c.hw_code.clone(), build: build.to_string(), chain: c.chain.clone(), pages, params, io_items }
}

#[cfg(test)]
mod io_tests {
    use super::*;
    use std::collections::BTreeMap;

    fn el(name: &str, attrs: &[(&str, &str)], children: Vec<Element>) -> Element {
        Element { name: name.into(), attrs: attrs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>(), children }
    }

    #[test]
    fn io_elements_carry_name_unit_and_group() {
        let tree = el("Root", &[], vec![el("IO", &[("Name", "Base I/O")], vec![
            el("IOItem", &[("IoId", "66"), ("Name", "External voltage"), ("Units", "mV")], vec![]),
            el("IOItem", &[("IoId", "239"), ("Name", "Ignition"), ("Units", " ")], vec![]),
            el("IOItem", &[("Name", "no id")], vec![]),
        ])]);
        let mut out = Vec::new();
        io_elements(&tree, None, &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].id, out[0].name.as_str(), out[0].unit.as_deref(), out[0].group.as_deref()), (66, "External voltage", Some("mV"), Some("Base I/O")));
        assert_eq!(out[1].unit, None);
    }
}
