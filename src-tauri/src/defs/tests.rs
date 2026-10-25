use super::compose::{chain_for, code_matches, compose, SpecSource};
use super::*;

const BASE: &str = r#"<?xml version="1.0"?>
<!-- <NOT A TAG> -->
<Configuration>
  <Keyword ParameterId="10"/>
  <ViewConfiguration Name="System">
    <GroupBox GroupName="G">
      <NumberInput ParameterId="100" ParameterType="Int32" Min="0" Max="5"/>
      <NumberInput ParameterId="101" Feature="3"/>
      <NumberInput ParameterId="102" HWAttribute="OBD"/>
      <Placeholder Guid="Add_x"/>
      <NumberInput ParameterId="103" Guid="Remove_y"/>
      <List List="Phones"/>
      <IO Name="io"><IOItem IoId="50000" SmsEventId="8000" IOTemplate="T"><Priority Default="1"/><Text/><Phone/></IOItem></IO>
    </GroupBox>
  </ViewConfiguration>
</Configuration>"#;

const EXT: &str = r#"<BaseExtensions>
  <Software Version="1.0.0.0"><Hardware Version="FMB1:[1-4]"/></Software>
  <Software Version="2.0.0.0" DiffFor="1.0.0.0">
    <Hardware Version="FMB1:[1-4]">
      <Add><Item Guid="Add_x"><NumberInput ParameterId="200"/></Item></Add>
      <Remove><Item Guid="Remove_y"/></Remove>
      <Spec Id="107" PatchNo="1"/>
    </Hardware>
  </Software>
  <Software Version="3.0.0.0" DiffFor="2.0.0.0"><Hardware Version="FMB2:1"/></Software>
</BaseExtensions>"#;

fn docs() -> BTreeMap<String, Vec<u8>> {
    let mut d = BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        d.insert(k.to_string(), v.as_bytes().to_vec());
    };
    put("Base", BASE);
    put("BaseExtension", EXT);
    put("Collections", "<CollectionConfigurations/>");
    put("Lists", r#"<ListConfigurations><ListConfiguration Name="Phones" StartId="4000" NumberOfItems="3"/></ListConfigurations>"#);
    put("Hardware", r#"<HardwareVersionTypes><HardwareVersionType Version="FMB1:2" FmType="FMBX" Attributes="DIN1,OBD" BaseOf="FMB1:9"/><HardwareVersionType Version="FMB1:3" FmType="FMBY" Attributes="DIN1"/></HardwareVersionTypes>"#);
    put("Templates", r#"<IOTemplates><IOTemplate Name="T"><IOItem><Priority Default="9"/><Operand/><EventOnly Offset="9"/></IOItem></IOTemplate></IOTemplates>"#);
    put("UpdateConfiguration", "<Configuration/>");
    put("Specs.107", r#"<Spec Id="107"><Patch No="1"><Add><Item Guid="Add_x"><NumberInput ParameterId="300"/></Item></Add></Patch></Spec>"#);
    d
}

fn family() -> Family {
    model::build_family("FMB", &docs()).unwrap()
}

#[test]
fn hardware_codes_and_chain() {
    assert!(code_matches("FMB1:[1-4],FMB2:3", "fmb1:4"));
    assert!(!code_matches("FMB1:[1-4]", "FMB1:5"));
    let f = family();
    assert_eq!(f.keyword_id, Some(10));
    assert_eq!(chain_for(&f, "FMB1:2", "2.5.0.0"), ["1.0.0.0", "2.0.0.0"]);
    assert_eq!(chain_for(&f, "FMB1:2", "1.9.0.0"), ["1.0.0.0"]);
}

#[test]
fn composes_with_filters_and_derived_ids() {
    let f = family();
    let c = compose(&f, "FMB1:9", "9.0.0.0", SpecSource::Standard, 0).unwrap(); // alias
    assert_eq!(c.model_name, "FMBX");
    let ids: Vec<u32> = c.params.keys().copied().collect();
    // 101 feature-gated off, 103 removed, 200 added, list 4000..4002, IO 50000/1/9, SMS 8000/7000.
    assert_eq!(ids, [100, 102, 200, 4000, 4001, 4002, 7000, 8000, 50000, 50001, 50009]);
    assert_eq!(c.params[&50000].attrs.get("Default").map(String::as_str), Some("1"));
    let all = compose(&f, "FMB1:2", "9.0.0.0", SpecSource::ReportedByDevice(107), u64::MAX).unwrap();
    assert!(all.params.contains_key(&101) && all.params.contains_key(&300));
    let no_obd = compose(&f, "FMB1:3", "9.0.0.0", SpecSource::Standard, u64::MAX).unwrap();
    assert!(!no_obd.params.contains_key(&102) && !no_obd.params.contains_key(&300));
}

#[test]
fn full_container_walk_and_refusals() {
    let mut res: Vec<(String, Vec<u8>)> = docs().into_iter().map(|(k, v)| (format!("{PREFIX}FMB.{k}.xml"), v)).collect();
    res.push(("Brand.g.resources".into(), b"x".to_vec()));
    let refs: Vec<(&str, &[u8])> = res.iter().map(|(k, v)| (k.as_str(), v.as_slice())).collect();
    let dll = metadata::tests::build_assembly(&refs);
    let exe = bundle::tests::build_bundle(&[("App.dll", b"x", false), ("Brand.dll", &dll, true)]);
    let d = extract(&exe).unwrap();
    assert_eq!(d.summary(false).document_count, 8);
    assert!(d.warnings.iter().any(|w| w.contains("only 1.8.21.0")));
    let mut lib = Library::default();
    lib.add(d, false);
    assert!(lib.find("FMB1:2").is_some());
    assert!(lib.find("ZZZ9:9").is_none());
    assert_eq!(lib.summaries().len(), 1);
    assert!(lib.summaries()[0].primary);

    let no_defs = bundle::tests::build_bundle(&[("App.dll", b"x", false)]);
    assert_eq!(extract(&no_defs).unwrap_err().to_string(), "unsupported Configurator build (no definition data found)");
    assert!(extract(b"hello").is_err());

    let mut broken = docs();
    broken.insert("Base".into(), b"<Other/>".to_vec());
    assert!(model::build_family("FMB", &broken).is_err());
}

/// Gated on FMCONFIG_CONFIGURATOR (the user's own download). Prints counts only.
#[test]
fn integration_user_configurator() {
    let Ok(path) = std::env::var("FMCONFIG_CONFIGURATOR") else { return };
    let input = std::fs::read(&path).unwrap();
    let exe = exe_bytes(&input).unwrap();
    let mut report = Vec::new();
    let mut check = |name: &str, expected: String, actual: String| {
        report.push(format!("{} {name}: expected {expected}, actual {actual}", if expected == actual { "PASS" } else { "FAIL" }));
    };
    check("exe size", "41086351".into(), exe.len().to_string());
    check("exe sha256", "92c8d43bdbbad476e190d54ee4fb56495744d5fe1036aa7cef3282fe9b6de123".into(), hex(&Sha256::digest(&exe)));
    let img = pe::Pe::parse(&exe).unwrap();
    check("pe header/machine/magic", "0xE8/0x8664/0x20B".into(), format!("0x{:X}/0x{:X}/0x{:X}", img.pe_offset, img.machine, img.magic));
    check("ProductVersion", "1.8.21.0".into(), img.version_string("ProductVersion").unwrap_or_default());
    check("CompanyName", "JSC Teltonika".into(), img.version_string("CompanyName").unwrap_or_default());
    let b = bundle::Bundle::parse(&exe).unwrap();
    check("bundle sig/header/ver/entries", "0x204D0/0x272E124/6.0/60".into(), format!("0x{:X}/0x{:X}/{}.{}/{}", b.signature_offset, b.header_offset, b.major, b.minor, b.entries.len()));
    check("compressed entries", "0".into(), b.entries.iter().filter(|e| e.compressed_size != 0).count().to_string());
    let kinds: Vec<usize> = (1..=4).map(|k| b.entries.iter().filter(|e| e.kind == k).count()).collect();
    check("entry kinds 1..4", "[56, 2, 1, 1]".into(), format!("{kinds:?}"));
    let e = b.find("Brand.dll").unwrap();
    check("Brand.dll offset/size", "0x17BC000/6386176".into(), format!("0x{:X}/{}", e.offset, e.size));
    let dll = b.read(&exe, e).unwrap();
    check("Brand.dll sha256", "0c2b8c613b5799976cc1029765416f9fedb5040b0f6d78e165bbd749542385b7".into(), hex(&Sha256::digest(&dll)));
    let res = metadata::manifest_resources(&dll).unwrap();
    check("manifest resources", "148".into(), res.len().to_string());
    let defs: Vec<_> = res.iter().filter(|r| r.name.starts_with(PREFIX)).collect();
    check("definition documents", "147".into(), defs.len().to_string());
    check("definition bytes", "3601111".into(), defs.iter().map(|r| r.data.len()).sum::<usize>().to_string());
    let sizes: BTreeMap<String, usize> = defs.iter().map(|r| (r.name.trim_start_matches(PREFIX).to_string(), r.data.len())).collect();
    let fixed = "FMB: 202156 556381 62939 14701 4097 20884 483; FMB640: 37861 1377550 296158 981 4487 8848 483";
    let got = ["FMB", "FMB640"]
        .iter()
        .map(|f| {
            let v: Vec<String> = ["Base", "BaseExtension", "Collections", "Hardware", "Lists", "Templates", "UpdateConfiguration"]
                .iter()
                .map(|k| sizes.get(&format!("{f}.{k}.xml")).copied().unwrap_or(0).to_string())
                .collect();
            format!("{f}: {}", v.join(" "))
        })
        .collect::<Vec<_>>()
        .join("; ");
    check("fixed-file sizes", fixed.into(), got);
    let d = extract(&input).unwrap();
    let fmb = &d.families["FMB"];
    let fmb640 = &d.families["FMB640"];
    check("documents FMB/FMB640", "76/71".into(), format!("{}/{}", fmb.document_count, fmb640.document_count));
    check("specs FMB/FMB640, presets FMB", "32/64/37".into(), format!("{}/{}/{}", fmb.specs.len(), fmb640.specs.len(), fmb.presets.len()));
    check("keyword ids", "10/10".into(), format!("{:?}/{:?}", fmb.keyword_id.unwrap_or(0), fmb640.keyword_id.unwrap_or(0)));
    check("FMB software nodes / FMB640", "33/187".into(), format!("{}/{}", fmb.versions.len(), fmb640.versions.len()));
    let chain = compose::chain_for(fmb, "FMB1:6", "13.0.0.0");
    check("FMB1:6 chain len/root/newest", "22/3.21.6.0/13.0.0.0".into(), format!("{}/{}/{}", chain.len(), chain.first().unwrap(), chain.last().unwrap()));
    let row = compose::resolve_row(fmb, "FMB9:1").unwrap();
    check("alias FMB9:1", "FMB9:21/FMB900".into(), format!("{}/{}", row.hw_code, row.model_name));
    // 08 v3 §6 figures: exact targets (distinct IDs, all feature bits, hardware-attribute filter).
    let c1 = compose(fmb, "FMB1:6", "13.0.0.0", SpecSource::Standard, u64::MAX).unwrap();
    let figures: [(&str, &str, &str, u32, usize); 5] = [
        ("FMB140 @13.0.0.0 spec 1", "FMB1:6", "13.0.0.0", 1, 8885),
        ("FMB140 @13.0.0.0 spec 107", "FMB1:6", "13.0.0.0", 107, 8887),
        ("FMB140 @10.0.0.0 spec 1", "FMB1:6", "10.0.0.0", 1, 8514),
        ("FMB920 @13.0.0.0 spec 1", "FMB9:22", "13.0.0.0", 1, 2741),
        ("FMB010 @13.0.0.0 spec 1", "FMB0:1", "13.0.0.0", 1, 2480),
    ];
    let mut fails = Vec::new();
    for (n, code, ver, spec, target) in figures {
        let src = if spec <= 1 { SpecSource::Standard } else { SpecSource::ReportedByDevice(spec) };
        let c = compose::try_compose(fmb, code, ver, src, u64::MAX).unwrap();
        let count = c.params.len();
        report.push(format!("{} {n}: expected {target}, actual {count} (unanchored adds {}, custom-group IO dropped {})", if count == target { "PASS" } else { "FAIL" }, c.unanchored_adds.len(), c.custom_io_dropped));
        if count != target { fails.push(n.to_string()); }
    }
    let mut union = std::collections::BTreeSet::new();
    let mut codes = 0;
    for row in &fmb.hardware {
        if let Ok(c) = compose::try_compose(fmb, &row.hw_code, "13.0.0.0", SpecSource::Standard, u64::MAX) {
            codes += 1;
            union.extend(c.params.keys().copied());
        }
    }
    report.push(format!("{} composed union over {codes} of {} FMB codes @13.0.0.0 spec 1: expected 10070, actual {}", if union.len() == 10070 { "PASS" } else { "FAIL" }, fmb.hardware.len(), union.len()));
    if union.len() != 10070 { fails.push("FMB union".into()); }
    // FMB640 family at its newest version, spec 1.
    let fam640: [(&str, usize); 6] = [("FMB640:1", 6025), ("FMB641:1", 7438), ("FMC640:1", 6744), ("FMM640:1", 6737), ("FMC650:1", 11265), ("FMM650:1", 11264)];
    let mut u640 = std::collections::BTreeSet::new();
    for (code, target) in fam640 {
        let c = compose::try_compose(fmb640, code, "4.45.00.0", SpecSource::Standard, u64::MAX).unwrap();
        u640.extend(c.params.keys().copied());
        report.push(format!("{} {code} @4.45.00.0 spec 1: expected {target}, actual {}", if c.params.len() == target { "PASS" } else { "FAIL" }, c.params.len()));
        if c.params.len() != target { fails.push(code.to_string()); }
    }
    report.push(format!("{} FMB640 family union: expected 11272, actual {}", if u640.len() == 11272 { "PASS" } else { "FAIL" }, u640.len()));
    if u640.len() != 11272 { fails.push("FMB640 union".into()); }
    // TODO(spec): move the raw union to the 08 v3 §6 counting rules (target 11,717): lists at
    // the length each reference names, IO elements with template-supplied sub-fields, geofence
    // at full zone count, presets/collections/hardware excluded.
    report.push(format!("INFO raw definition union (current counting, 08 v3 §6 target 11,717): {}", compose::union_ids(fmb)));
    let mut kinds: BTreeMap<&str, usize> = BTreeMap::new();
    for p in c1.params.values() {
        *kinds.entry(p.kind.as_str()).or_default() += 1;
    }
    report.push(format!("INFO FMB140 spec 1 count by element kind: {kinds:?}"));
    let mut no_family_ops = fmb.clone();
    for v in &mut no_family_ops.versions {
        v.family_ops = model::Ops { add: vec![], remove: vec![], modify: vec![] };
    }
    let c_nf = compose(&no_family_ops, "FMB1:6", "13.0.0.0", SpecSource::Standard, u64::MAX).unwrap();
    report.push(format!("INFO FMB140 spec 1 without version-level ops: {}", c_nf.params.len()));
    for ver in ["10.0.0.0", "13.0.0.0"] {
        let c = compose(fmb, "FMB1:6", ver, SpecSource::Standard, u64::MAX).unwrap();
        let m = editor::editor_model(&c, fmb, "1.8.21.0");
        let listed: usize = m.pages.iter().map(|p| p.ids.len()).sum();
        let labelled = m.params.values().filter(|p| !p.label.is_empty()).count();
        let enums = m.params.values().filter(|p| p.control == "enum" && !p.options.is_empty()).count();
        report.push(format!("INFO FMB140 @{ver}: chain {}, pages {}, params {}, labelled {labelled}, enums with options {enums}, ids directly on pages {listed}", c.chain.len(), m.pages.len(), m.params.len()));
    }
    let r = report.join("\n");
    println!("{r}");
    std::fs::write(std::env::temp_dir().join("fmconfig-defs-report.txt"), &r).ok();
    assert!(fails.is_empty(), "figures differ: {fails:?}");
}

/// Gated on FMCONFIG_CONFIGURATOR_OLD (a 1.8.10 download); checks 09's stated facts. Counts only.
#[test]
fn integration_old_configurator() {
    let Ok(path) = std::env::var("FMCONFIG_CONFIGURATOR_OLD") else { return };
    let input = std::fs::read(&path).unwrap();
    let d = extract(&input).unwrap();
    let fmb = &d.families["FMB"];
    let fmb640 = &d.families["FMB640"];
    let newest = |f: &Family| f.versions.iter().map(|v| v.version.clone()).max_by_key(|v| compose::version_key(v)).unwrap_or_default();
    let mut report = Vec::new();
    let mut check = |name: &str, expected: &str, actual: String| {
        report.push(format!("{} {name}: expected {expected}, actual {actual}", if expected == actual { "PASS" } else { "FAIL" }));
    };
    check("documents FMB/FMB640", "75/63", format!("{}/{}", fmb.document_count, fmb640.document_count));
    check("newest FMB version", "12.0.0.0", newest(fmb));
    check("newest FMB640 version", "4.41.0.0", newest(fmb640));
    check("FMM0:31 in FMB hardware table", "true", compose::resolve_row(fmb, "FMM0:31").is_some().to_string());
    check("hardware rows, all families (09 §2.2)", "77", d.families.values().map(|f| f.hardware.len()).sum::<usize>().to_string());
    // Multi-build: with the user's newest build also loaded, FMM0:31 comes from the old build only.
    if let Ok(new_path) = std::env::var("FMCONFIG_CONFIGURATOR") {
        let (newer, _) = load(Path::new(&new_path), None).unwrap();
        let mut lib = Library::default();
        lib.add(d.clone(), false);
        lib.add(newer, false);
        let s = lib.summaries();
        check("primary is newest", "1.8.21.0", s[0].product_version.clone());
        check("old build contributes", "[\"FMM0:31\"]", format!("{:?}", s[1].extra_hardware));
        check("FMM0:31 served by old build", "1.8.10", lib.find("FMM0:31").map(|x| x.0.product_version.chars().take(6).collect()).unwrap_or_default());
    }
    let r = report.join("\n");
    println!("{r}");
    assert!(!r.contains("FAIL"), "{r}");
}

/// Gated on FMCONFIG_CONFIGURATOR and FMCONFIG_SAMPLE_CFG (the user's own files).
/// Prints COUNTS AND IDS ONLY: never parameter values, labels or catalogue content.
#[test]
fn integration_cfg_coverage() {
    let (Ok(zip), Ok(cfg_path)) = (std::env::var("FMCONFIG_CONFIGURATOR"), std::env::var("FMCONFIG_SAMPLE_CFG")) else { return };
    let (d, _) = load(std::path::Path::new(&zip), None).unwrap();
    let cfg = crate::cfg::parse(&crate::cfg::decode(&std::fs::read(&cfg_path).unwrap()).unwrap()).unwrap();
    let hdr = |k: &str| cfg.entries.iter().find(|e| e.key == k).map(|e| e.value.clone()).unwrap_or_default();
    let hw = hdr("HwVersion").replace('_', ":");
    let ver = hdr("ConfigurationVersion");
    let spec: u32 = hdr("SpecId").trim().parse().unwrap_or(1);
    let fm = hdr("FmType");
    let mut out = vec![format!("header: FmType={fm} HwCode={hw} ConfigVersion={ver} SpecId={spec}")];
    let fam = d.families.values().filter(|f| compose::resolve_row(f, &hw).is_some()).max_by_key(|f| f.name != "FMB").expect("family for hardware code");
    let spec_src = if spec <= 1 { SpecSource::Standard } else { SpecSource::ReportedByDevice(spec) };
    let c = compose(fam, &hw, &ver, spec_src, u64::MAX).unwrap();
    let model = editor::editor_model(&c, fam, "x");
    out.push(format!("family={} composed ids={} chain len={}", fam.name, c.params.len(), c.chain.len()));
    let params: Vec<(u32, &str)> = cfg.entries.iter().filter_map(|e| e.key.parse::<u32>().ok().map(|k| (k, e.value.as_str()))).collect();
    out.push(format!("numeric parameter entries in file: {} (all entries {})", params.len(), cfg.entries.len()));
    let bucket = |id: u32| if id < 1000 { "0-999" } else if id < 10000 { "1000-9999" } else { "10000+" };
    let mut unknown: Vec<u32> = Vec::new();
    let mut covered = 0usize;
    let mut fails: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
    let (mut labelled, mut unit, mut enums, mut checked_range_skipped) = (0, 0, 0, 0);
    let mut seen = std::collections::BTreeSet::new();
    for (id, v) in &params {
        seen.insert(*id);
        let Some(p) = model.params.get(id) else { unknown.push(*id); continue };
        covered += 1;
        // A label counts only when it is not the synthetic "<kind> <id>" fallback.
        if c.params.get(id).is_some_and(|cp| cp.attrs.contains_key("Label") || cp.label_hint.is_some()) { labelled += 1; }
        if p.units.is_some() { unit += 1; }
        if !p.options.is_empty() { enums += 1; }
        let mut f = |k: &'static str| fails.entry(k).or_default().push(*id);
        let t = p.value_type.as_str();
        let num: Option<f64> = match t {
            "int32" => v.parse::<i32>().ok().map(f64::from),
            "byte" => v.parse::<u8>().ok().map(f64::from),
            "uint16" => v.parse::<u16>().ok().map(f64::from),
            "uint32" => v.parse::<u32>().ok().map(f64::from),
            "int64" => v.parse::<i64>().ok().map(|x| x as f64),
            "float" | "double" => v.parse::<f64>().ok(),
            _ => None,
        };
        let is_num_type = matches!(t, "int32" | "byte" | "uint16" | "uint32" | "int64" | "float" | "double");
        if p.control == "enum" {
            if p.bitmask {
                if !is_num_type || num.is_none() { f("enum-bitmask-not-numeric"); }
            } else if !p.options.iter().any(|o| o.value == *v) { f("enum-not-in-options"); }
        } else if is_num_type {
            match num {
                None => f("type-mismatch"),
                Some(x) => {
                    if p.min_id.is_some() || p.max_id.is_some() { checked_range_skipped += 1; }
                    if p.min.as_deref().and_then(|m| m.parse::<f64>().ok()).is_some_and(|m| x < m) { f("below-min"); }
                    if p.max.as_deref().and_then(|m| m.parse::<f64>().ok()).is_some_and(|m| x > m) { f("above-max"); }
                }
            }
        } else {
            let len = v.chars().count() as u32;
            if p.text_min.is_some_and(|m| len < m) { f("text-too-short"); }
            if p.text_max.is_some_and(|m| len > m) { f("text-too-long"); }
        }
    }
    out.push(format!("file ids WITH definition: {covered}"));
    out.push(format!("file ids with NO definition: {}", unknown.len()));
    let mut b: BTreeMap<&str, usize> = BTreeMap::new();
    for id in &unknown { *b.entry(bucket(*id)).or_default() += 1; }
    out.push(format!("  by range: {b:?}"));
    out.push(format!("  example ids (max 20): {:?}", unknown.iter().take(20).collect::<Vec<_>>()));
    let mut thousands: BTreeMap<u32, usize> = BTreeMap::new();
    for id in &unknown { *thousands.entry(id / 1000).or_default() += 1; }
    out.push(format!("  by thousand-block (id/1000): {thousands:?}"));
    let absent: Vec<u32> = c.params.keys().filter(|k| !seen.contains(k)).copied().collect();
    let mut ab: BTreeMap<&str, usize> = BTreeMap::new();
    for id in &absent { *ab.entry(bucket(*id)).or_default() += 1; }
    out.push(format!("composed ids absent from file: {} by range {ab:?}", absent.len()));
    out.push(format!("  ids (max 60): {:?}", absent.iter().take(60).collect::<Vec<_>>()));
    out.push(format!("validation failures (covered ids): total kinds {}", fails.len()));
    for (k, ids) in &fails { out.push(format!("  {k}: {} ids, examples {:?}", ids.len(), ids.iter().take(10).collect::<Vec<_>>())); }
    out.push(format!("covered ids with min/max-by-id (not checked): {checked_range_skipped}"));
    out.push(format!("covered ids with label: {labelled}, with unit: {unit}, with enum options: {enums}"));
    println!("{}", out.join("\n"));
}

// ---- 04 v2 composition rules (synthetic data) ----

const V2_BASE: &str = r#"<Configuration>
  <Keyword ParameterId="10"/>
  <ViewConfiguration Name="P">
    <NumberInput ParameterId="100"/>
    <Placeholder Guid="Add_a"/>
    <Placeholder Guid="Add_v"/>
    <Placeholder Guid="Add_b"/>
    <IO Name="io">
      <IOItem IoId="50000" SmsEventId="8000" IOTemplate="T"><Priority/><Text Guid="Sms_t"/><Phone Guid="Sms_t"/></IOItem>
      <IOItem IoId="51000" SmsEventId="8100" IOTemplate="T"/>
      <IOItem IoId="52000" CustomIoItemName="G1"><Priority/></IOItem>
      <IOItem IoId="53000" CustomIoItemName="G2"><Priority/></IOItem>
    </IO>
    <CustomIoTable CustomIoItemName="G1"/>
  </ViewConfiguration>
</Configuration>"#;

const V2_EXT: &str = r#"<BaseExtensions>
  <Software Version="1.0.0.0">
    <Hardware Version="FMB1:22">
      <Add><Item Guid="Add_a"><NumberInput ParameterId="500"/></Item><Item Guid="Add_n"><NumberInput ParameterId="650"/></Item></Add>
      <Spec Id="107" PatchNo="07"/>
    </Hardware>
    <Hardware Version="FMB2:1"/>
  </Software>
  <Software Version="2.0.0.0" DiffFor="1.0.0.0">
    <Add><Item Guid="Add_v"><NumberInput ParameterId="600"/><Placeholder Guid="Add_n"/></Item></Add>
    <Remove><Item Guid="Sms_t"/></Remove>
    <Hardware Version="FMB1:6" Extends="FMB1:22">
      <Add><Item Guid="Add_b"><NumberInput ParameterId="700"/></Item></Add>
      <Spec Id="107" PatchNo="2"/>
      <Spec Id="107" PatchNo="3"/>
    </Hardware>
    <Hardware Version="FMB2:1">
      <Spec Id="105" PatchNo=""/>
      <Spec Id="108" PatchNo="x"/>
      <Spec Id="109" PatchNo="5"/>
      <Spec Id="110" PatchNo="1"/>
      <Spec Id="111" PatchNo=""/>
      <Spec Id="111" PatchNo="1"/>
      <Spec Id="111" PatchNo="2"/>
    </Hardware>
  </Software>
</BaseExtensions>"#;

fn v2_family() -> Family {
    let mut d = BTreeMap::new();
    let mut put = |k: &str, v: &str| {
        d.insert(k.to_string(), v.as_bytes().to_vec());
    };
    put("Base", V2_BASE);
    put("BaseExtension", V2_EXT);
    put("Collections", "<CollectionConfigurations/>");
    put("Lists", "<ListConfigurations/>");
    put("Hardware", r#"<HardwareVersionTypes><HardwareVersionType Version="FMB1:6" FmType="A"/><HardwareVersionType Version="FMB1:22" FmType="B"/><HardwareVersionType Version="FMB2:1" FmType="C"/></HardwareVersionTypes>"#);
    put("Templates", r#"<IOTemplates><IOTemplate Name="T"><IOItem><Priority/><Operand/><Text Guid="Sms_t"/><Phone Guid="Sms_t"/></IOItem></IOTemplate></IOTemplates>"#);
    put("UpdateConfiguration", "<Configuration/>");
    let patch = |id: u32, no: &str| format!(r#"<Patch No="{no}"><Add><Item Guid="Add_b"><NumberInput ParameterId="{id}"/></Item></Add></Patch>"#);
    let s107 = format!("<Spec>{}{}{}</Spec>", patch(800, "7"), patch(820, "2"), patch(830, "3"));
    put("Specs.107", &s107);
    put("Specs.105", &format!("<Spec>{}</Spec>", patch(850, "1")));
    put("Specs.108", &format!("<Spec>{}</Spec>", patch(860, "1")));
    put("Specs.111", &format!("<Spec>{}{}</Spec>", patch(880, "1"), patch(881, "2")));
    put("Specs.109", &format!("<Spec>{}</Spec>", patch(870, "1")));
    model::build_family("FMB", &d).unwrap()
}

fn ids(f: &Family, code: &str, ver: &str, spec: SpecSource) -> Vec<u32> {
    compose::try_compose(f, code, ver, spec, u64::MAX).unwrap().params.keys().copied().collect()
}

#[test]
fn extends_inherits_older_history() {
    let f = v2_family();
    let chain = chain_for(&f, "FMB1:6", "2.0.0.0");
    assert_eq!(compose::effective_codes(&f, &chain, "FMB1:6"), ["FMB1:22", "FMB1:6"]);
    let got = ids(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard);
    // 500 comes from FMB1:22 at 1.0.0.0 through the extends rule; 700 from its own block.
    assert!(got.contains(&500) && got.contains(&700), "{got:?}");
    // Without extends, FMB2:1 does not get FMB1:22's history.
    assert!(!ids(&f, "FMB2:1", "2.0.0.0", SpecSource::Standard).contains(&500));
}

#[test]
fn version_level_ops_apply_to_every_code_and_nested_adds_resolve() {
    let f = v2_family();
    for code in ["FMB1:6", "FMB2:1"] {
        let got = ids(&f, code, "2.0.0.0", SpecSource::Standard);
        assert!(got.contains(&600), "{code}: {got:?}");
    }
    // Add_n is collected at 1.0.0.0 but its anchor arrives with the 2.0.0.0 version-level add.
    assert!(ids(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard).contains(&650));
    let c = compose::try_compose(&f, "FMB2:1", "2.0.0.0", SpecSource::Standard, u64::MAX).unwrap();
    assert_eq!(c.unanchored_adds, Vec::<String>::new());
    // FMB1:22's chain stops at 1.0.0.0, so the 2.0.0.0 version-level ops do not apply.
    assert!(!ids(&f, "FMB1:22", "2.0.0.0", SpecSource::Standard).contains(&600));
}

#[test]
fn sms_fields_removed_from_elements_and_templates_at_a_version() {
    let f = v2_family();
    let before = ids(&f, "FMB1:22", "1.0.0.0", SpecSource::Standard);
    for id in [8000, 7000, 8100, 7100] {
        assert!(before.contains(&id), "{id} before");
    }
    let after = ids(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard);
    for id in [8000, 7000, 8100, 7100] {
        assert!(!after.contains(&id), "{id} after");
    }
    // Template sub-fields other than SMS still expand.
    assert!(after.contains(&51000) && after.contains(&51001));
}

#[test]
fn custom_group_io_needs_a_matching_table() {
    let f = v2_family();
    let got = ids(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard);
    assert!(got.contains(&52000) && !got.contains(&53000));
    let c = compose::try_compose(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard, u64::MAX).unwrap();
    assert_eq!(c.custom_io_dropped, 1);
}

#[test]
fn spec_references_match_as_integers_first_per_block() {
    let f = v2_family();
    let got = ids(&f, "FMB1:6", "2.0.0.0", SpecSource::ReportedByDevice(107));
    // "07" matches patch "7" (via the extended code's block); only the first FMB1:6 reference (2) applies.
    assert!(got.contains(&800) && got.contains(&820) && !got.contains(&830), "{got:?}");
    // Spec 1 never patches.
    assert!(!ids(&f, "FMB1:6", "2.0.0.0", SpecSource::Standard).contains(&800));
    // A blank patch number is skipped.
    assert!(!ids(&f, "FMB2:1", "2.0.0.0", SpecSource::ReportedByDevice(105)).contains(&850));
    // A blank reference before a numbered one for the same spec ID never takes the "first" place.
    let got111 = ids(&f, "FMB2:1", "2.0.0.0", SpecSource::ReportedByDevice(111));
    assert!(got111.contains(&880) && !got111.contains(&881), "{got111:?}");
    // Invalid references give readable errors.
    let err = |s| compose::try_compose(&f, "FMB2:1", "2.0.0.0", SpecSource::ReportedByDevice(s), u64::MAX).err().unwrap();
    assert!(err(108).contains("not a number"), "{}", err(108));
    assert!(err(109).contains("patch 5, which does not exist"), "{}", err(109));
    assert!(err(110).contains("no spec document"), "{}", err(110));
}

