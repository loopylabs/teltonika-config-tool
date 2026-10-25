//! ECMA-335 Partition II subset: CLI header, metadata root, table stream row sizes,
//! and the ManifestResource table (0x28) with embedded resource bytes.

use std::fmt;

use super::pe::{u16_at, u32_at, u64_at, Pe, PeError};

#[derive(Debug)]
pub enum MetaError {
    Pe(PeError),
    NotManaged,
    BadMetadata(&'static str),
    BadResource(String),
}

impl fmt::Display for MetaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MetaError::Pe(e) => write!(f, "definition library: {e}"),
            MetaError::NotManaged => write!(f, "definition library is not a managed assembly"),
            MetaError::BadMetadata(w) => write!(f, "definition library metadata invalid ({w})"),
            MetaError::BadResource(n) => write!(f, "definition resource out of range: {n}"),
        }
    }
}

impl From<PeError> for MetaError {
    fn from(e: PeError) -> Self {
        MetaError::Pe(e)
    }
}

// Column kinds.
#[derive(Clone, Copy)]
enum C {
    U16,
    U32,
    Str,
    Guid,
    Blob,
    T(u8),
    Ci(&'static [u8], u8), // coded index: tables (0xFF = unused tag), tag bits
}

const TYPEDEF_OR_REF: C = C::Ci(&[0x02, 0x01, 0x1B], 2);
const HAS_CONSTANT: C = C::Ci(&[0x04, 0x08, 0x17], 2);
const HAS_CA: C = C::Ci(
    &[0x06, 0x04, 0x01, 0x02, 0x08, 0x09, 0x0A, 0x00, 0x0E, 0x17, 0x14, 0x11, 0x1A, 0x1B, 0x20, 0x23, 0x26, 0x27, 0x28, 0x2A, 0x2C, 0x2B],
    5,
);
const HAS_FIELD_MARSHAL: C = C::Ci(&[0x04, 0x08], 1);
const HAS_DECL_SECURITY: C = C::Ci(&[0x02, 0x06, 0x20], 2);
const MEMBER_REF_PARENT: C = C::Ci(&[0x02, 0x01, 0x1A, 0x06, 0x1B], 3);
const HAS_SEMANTICS: C = C::Ci(&[0x14, 0x17], 1);
const METHOD_DEF_OR_REF: C = C::Ci(&[0x06, 0x0A], 1);
const MEMBER_FORWARDED: C = C::Ci(&[0x04, 0x06], 1);
const IMPLEMENTATION: C = C::Ci(&[0x26, 0x23, 0x27], 2);
const CA_TYPE: C = C::Ci(&[0xFF, 0xFF, 0x06, 0x0A, 0xFF], 3);
const RESOLUTION_SCOPE: C = C::Ci(&[0x00, 0x1A, 0x23, 0x01], 2);

/// Column layout of tables 0x00..=0x28 (ECMA-335 §II.22).
fn schema(t: usize) -> &'static [C] {
    use C::*;
    match t {
        0x00 => &[U16, Str, Guid, Guid, Guid],
        0x01 => &[RESOLUTION_SCOPE, Str, Str],
        0x02 => &[U32, Str, Str, TYPEDEF_OR_REF, T(0x04), T(0x06)],
        0x03 => &[T(0x04)],
        0x04 => &[U16, Str, Blob],
        0x05 => &[T(0x06)],
        0x06 => &[U32, U16, U16, Str, Blob, T(0x08)],
        0x07 => &[T(0x08)],
        0x08 => &[U16, U16, Str],
        0x09 => &[T(0x02), TYPEDEF_OR_REF],
        0x0A => &[MEMBER_REF_PARENT, Str, Blob],
        0x0B => &[U16, HAS_CONSTANT, Blob],
        0x0C => &[HAS_CA, CA_TYPE, Blob],
        0x0D => &[HAS_FIELD_MARSHAL, Blob],
        0x0E => &[U16, HAS_DECL_SECURITY, Blob],
        0x0F => &[U16, U32, T(0x02)],
        0x10 => &[U32, T(0x04)],
        0x11 => &[Blob],
        0x12 => &[T(0x02), T(0x14)],
        0x13 => &[T(0x14)],
        0x14 => &[U16, Str, TYPEDEF_OR_REF],
        0x15 => &[T(0x02), T(0x17)],
        0x16 => &[T(0x17)],
        0x17 => &[U16, Str, Blob],
        0x18 => &[U16, T(0x06), HAS_SEMANTICS],
        0x19 => &[T(0x02), METHOD_DEF_OR_REF, METHOD_DEF_OR_REF],
        0x1A => &[Str],
        0x1B => &[Blob],
        0x1C => &[U16, MEMBER_FORWARDED, Str, T(0x1A)],
        0x1D => &[U32, T(0x04)],
        0x1E => &[U32, U32],
        0x1F => &[U32],
        0x20 => &[U32, U16, U16, U16, U16, U32, Blob, Str, Str],
        0x21 => &[U32],
        0x22 => &[U32, U32, U32],
        0x23 => &[U16, U16, U16, U16, U32, Blob, Str, Str, Blob],
        0x24 => &[U32, T(0x23)],
        0x25 => &[U32, U32, U32, T(0x23)],
        0x26 => &[U32, Str, Blob],
        0x27 => &[U32, U32, Str, Str, IMPLEMENTATION],
        0x28 => &[U32, U32, Str, IMPLEMENTATION],
        _ => &[],
    }
}

#[derive(Debug, Clone)]
pub struct Resource {
    pub name: String,
    pub data: Vec<u8>,
}

/// Returns the embedded manifest resources (Implementation = 0) of a managed PE.
pub fn manifest_resources(dll: &[u8]) -> Result<Vec<Resource>, MetaError> {
    let pe = Pe::parse(dll)?;
    let (cli_rva, cli_size) = pe.dir(14).ok_or(MetaError::NotManaged)?;
    let cli = pe.slice_rva(cli_rva, cli_size.max(32))?;
    let bad = MetaError::BadMetadata;
    let md_rva = u32_at(cli, 8).ok_or(bad("CLI header"))?;
    let md_size = u32_at(cli, 12).ok_or(bad("CLI header"))?;
    let res_rva = u32_at(cli, 24).ok_or(bad("CLI header"))?;
    let md = pe.slice_rva(md_rva, md_size)?;
    if md.get(0..4) != Some(b"BSJB") {
        return Err(bad("no BSJB signature"));
    }
    let vlen = u32_at(md, 12).ok_or(bad("root"))? as usize;
    let mut p = 16 + vlen;
    let nstreams = u16_at(md, p + 2).ok_or(bad("root"))? as usize;
    p += 4;
    let (mut tables, mut strings) = (None, None);
    for _ in 0..nstreams {
        let off = u32_at(md, p).ok_or(bad("stream header"))? as usize;
        let size = u32_at(md, p + 4).ok_or(bad("stream header"))? as usize;
        let name_start = p + 8;
        let name_len = md[name_start..].iter().position(|&c| c == 0).ok_or(bad("stream name"))?;
        let name = &md[name_start..name_start + name_len];
        p = name_start + ((name_len + 4) & !3);
        let s = md.get(off..off + size).ok_or(bad("stream bounds"))?;
        match name {
            b"#~" | b"#-" => tables = Some(s),
            b"#Strings" => strings = Some(s),
            _ => {}
        }
    }
    let tables = tables.ok_or(bad("no table stream"))?;
    let strings = strings.ok_or(bad("no #Strings heap"))?;
    let heap = tables[6];
    let valid = u64_at(tables, 8).ok_or(bad("table header"))?;
    let mut rows = [0u32; 64];
    let mut q = 24;
    for (t, r) in rows.iter_mut().enumerate() {
        if valid >> t & 1 == 1 {
            *r = u32_at(tables, q).ok_or(bad("row counts"))?;
            q += 4;
        }
    }
    if heap & 0x40 != 0 {
        q += 4; // extra data (uncompressed #- streams)
    }
    let str_w = if heap & 1 != 0 { 4 } else { 2 };
    let guid_w = if heap & 2 != 0 { 4 } else { 2 };
    let blob_w = if heap & 4 != 0 { 4 } else { 2 };
    let width = |c: C| -> usize {
        match c {
            C::U16 => 2,
            C::U32 => 4,
            C::Str => str_w,
            C::Guid => guid_w,
            C::Blob => blob_w,
            C::T(t) => if rows[t as usize] < 0x10000 { 2 } else { 4 },
            C::Ci(ts, bits) => {
                let max = ts.iter().filter(|&&t| t != 0xFF).map(|&t| rows[t as usize]).max().unwrap_or(0);
                if max < (1 << (16 - bits)) { 2 } else { 4 }
            }
        }
    };
    for t in 0..0x28 {
        if rows[t] == 0 {
            continue;
        }
        let s = schema(t);
        if s.is_empty() {
            return Err(bad("unknown table before ManifestResource"));
        }
        q += rows[t] as usize * s.iter().map(|&c| width(c)).sum::<usize>();
    }
    let cols: Vec<usize> = schema(0x28).iter().map(|&c| width(c)).collect();
    let row_w: usize = cols.iter().sum();
    let read = |o: usize, w: usize| -> Option<u32> {
        if w == 2 { u16_at(tables, o).map(u32::from) } else { u32_at(tables, o) }
    };
    let res_base = if res_rva != 0 { Some(pe.rva_to_offset(res_rva)?) } else { None };
    let mut out = Vec::new();
    for i in 0..rows[0x28] as usize {
        let r = q + i * row_w;
        let offset = read(r, 4).ok_or(bad("ManifestResource row"))? as usize;
        let name_idx = read(r + 8, cols[2]).ok_or(bad("ManifestResource row"))? as usize;
        let implementation = read(r + 8 + cols[2], cols[3]).ok_or(bad("ManifestResource row"))?;
        let name_bytes = strings.get(name_idx..).ok_or(bad("string index"))?;
        let end = name_bytes.iter().position(|&c| c == 0).unwrap_or(name_bytes.len());
        let name = String::from_utf8_lossy(&name_bytes[..end]).into_owned();
        if implementation != 0 {
            continue;
        }
        let base = res_base.ok_or(bad("no resources directory"))?;
        let at = base + offset;
        let len = u32_at(dll, at).ok_or_else(|| MetaError::BadResource(name.clone()))? as usize;
        let data = dll.get(at + 4..at + 4 + len).ok_or_else(|| MetaError::BadResource(name.clone()))?.to_vec();
        out.push(Resource { name, data });
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::defs::pe::tests::build_pe;

    /// Builds a minimal managed PE whose only metadata table is ManifestResource.
    pub fn build_assembly(resources: &[(&str, &[u8])]) -> Vec<u8> {
        // Layout inside section body (RVA 0x1000 + offset):
        // 0x000 CLI header (72 bytes), 0x100 resources blob, then metadata.
        let mut res_blob = Vec::new();
        let mut strings = vec![0u8];
        let mut rows = Vec::new();
        for (name, data) in resources {
            let off = res_blob.len() as u32;
            res_blob.extend_from_slice(&(data.len() as u32).to_le_bytes());
            res_blob.extend_from_slice(data);
            let sidx = strings.len() as u16;
            strings.extend_from_slice(name.as_bytes());
            strings.push(0);
            rows.push((off, sidx));
        }
        while strings.len() % 4 != 0 {
            strings.push(0);
        }
        let mut tbl = Vec::new();
        tbl.extend_from_slice(&[0, 0, 0, 0, 2, 0, 0, 1]);
        tbl.extend_from_slice(&(1u64 << 0x28).to_le_bytes());
        tbl.extend_from_slice(&0u64.to_le_bytes());
        tbl.extend_from_slice(&(rows.len() as u32).to_le_bytes());
        for (off, sidx) in &rows {
            tbl.extend_from_slice(&off.to_le_bytes());
            tbl.extend_from_slice(&1u32.to_le_bytes());
            tbl.extend_from_slice(&sidx.to_le_bytes());
            tbl.extend_from_slice(&0u16.to_le_bytes());
        }
        while tbl.len() % 4 != 0 {
            tbl.push(0);
        }
        let mut md = Vec::new();
        md.extend_from_slice(b"BSJB");
        md.extend_from_slice(&[1, 0, 1, 0, 0, 0, 0, 0]);
        md.extend_from_slice(&4u32.to_le_bytes());
        md.extend_from_slice(b"v4\0\0");
        md.extend_from_slice(&[0, 0, 2, 0]);
        let hdr_len = md.len() + 12 + 20;
        md.extend_from_slice(&(hdr_len as u32).to_le_bytes());
        md.extend_from_slice(&(tbl.len() as u32).to_le_bytes());
        md.extend_from_slice(b"#~\0\0");
        md.extend_from_slice(&((hdr_len + tbl.len()) as u32).to_le_bytes());
        md.extend_from_slice(&(strings.len() as u32).to_le_bytes());
        md.extend_from_slice(b"#Strings\0\0\0\0");
        assert_eq!(md.len(), hdr_len);
        md.extend_from_slice(&tbl);
        md.extend_from_slice(&strings);

        let mut body = vec![0u8; 0x100];
        body.extend_from_slice(&res_blob);
        while body.len() % 16 != 0 {
            body.push(0);
        }
        let md_off = body.len() as u32;
        body.extend_from_slice(&md);
        body[0..4].copy_from_slice(&72u32.to_le_bytes());
        body[8..12].copy_from_slice(&(0x1000 + md_off).to_le_bytes());
        body[12..16].copy_from_slice(&(md.len() as u32).to_le_bytes());
        body[24..28].copy_from_slice(&0x1100u32.to_le_bytes());
        body[28..32].copy_from_slice(&(res_blob.len() as u32).to_le_bytes());
        build_pe(&body, &[(14, 0x1000, 72)])
    }

    #[test]
    fn lists_embedded_resources() {
        let dll = build_assembly(&[("A.x.xml", b"<a/>"), ("B.g.resources", b"zz")]);
        let r = manifest_resources(&dll).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!((r[0].name.as_str(), r[0].data.as_slice()), ("A.x.xml", &b"<a/>"[..]));
        assert_eq!(r[1].name, "B.g.resources");
    }

    #[test]
    fn unmanaged_pe_is_refused() {
        let img = build_pe(&[0u8; 16], &[]);
        assert!(matches!(manifest_resources(&img), Err(MetaError::NotManaged)));
    }
}
