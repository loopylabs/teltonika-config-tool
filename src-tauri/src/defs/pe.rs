//! Minimal PE/COFF reader (public PE/COFF specification): headers, sections,
//! RVA mapping, data directories and the version resource strings.

use std::fmt;

#[derive(Debug, PartialEq, Eq)]
pub enum PeError {
    NotMz,
    NoPeSignature,
    Truncated(&'static str),
    BadRva(u32),
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PeError::NotMz => write!(f, "not a Windows executable (no MZ header)"),
            PeError::NoPeSignature => write!(f, "not a PE image (no PE signature)"),
            PeError::Truncated(what) => write!(f, "PE image truncated ({what})"),
            PeError::BadRva(rva) => write!(f, "PE address 0x{rva:X} is outside every section"),
        }
    }
}

pub(crate) fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    b.get(o..o + 2).map(|s| u16::from_le_bytes([s[0], s[1]]))
}
pub(crate) fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}
pub(crate) fn u64_at(b: &[u8], o: usize) -> Option<u64> {
    b.get(o..o + 8).map(|s| u64::from_le_bytes(s.try_into().unwrap()))
}

#[derive(Debug, Clone)]
pub struct Section {
    pub va: u32,
    pub vsize: u32,
    pub raw_ptr: u32,
    pub raw_size: u32,
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))] // header facts: diagnostics and integration test
pub struct Pe<'a> {
    pub data: &'a [u8],
    pub machine: u16,
    pub magic: u16,
    pub pe_offset: u32,
    dirs: Vec<(u32, u32)>,
    sections: Vec<Section>,
}

impl<'a> Pe<'a> {
    pub fn parse(data: &'a [u8]) -> Result<Self, PeError> {
        if data.get(0..2) != Some(b"MZ") {
            return Err(PeError::NotMz);
        }
        let pe_offset = u32_at(data, 0x3C).ok_or(PeError::Truncated("DOS header"))?;
        let p = pe_offset as usize;
        if data.get(p..p + 4) != Some(b"PE\0\0") {
            return Err(PeError::NoPeSignature);
        }
        let t = PeError::Truncated("COFF header");
        let machine = u16_at(data, p + 4).ok_or(t)?;
        let nsec = u16_at(data, p + 6).ok_or(PeError::Truncated("COFF header"))? as usize;
        let opt_size = u16_at(data, p + 20).ok_or(PeError::Truncated("COFF header"))? as usize;
        let opt = p + 24;
        let magic = u16_at(data, opt).ok_or(PeError::Truncated("optional header"))?;
        let (ndirs_at, dirs_at) = if magic == 0x20B { (opt + 108, opt + 112) } else { (opt + 92, opt + 96) };
        let ndirs = u32_at(data, ndirs_at).ok_or(PeError::Truncated("optional header"))? as usize;
        let mut dirs = Vec::new();
        for i in 0..ndirs.min(16) {
            let rva = u32_at(data, dirs_at + i * 8).ok_or(PeError::Truncated("data directories"))?;
            let size = u32_at(data, dirs_at + i * 8 + 4).ok_or(PeError::Truncated("data directories"))?;
            dirs.push((rva, size));
        }
        let sec_at = opt + opt_size;
        let mut sections = Vec::new();
        for i in 0..nsec {
            let s = sec_at + i * 40;
            let tr = || PeError::Truncated("section table");
            sections.push(Section {
                vsize: u32_at(data, s + 8).ok_or_else(tr)?,
                va: u32_at(data, s + 12).ok_or_else(tr)?,
                raw_size: u32_at(data, s + 16).ok_or_else(tr)?,
                raw_ptr: u32_at(data, s + 20).ok_or_else(tr)?,
            });
        }
        Ok(Pe { data, machine, magic, pe_offset, dirs, sections })
    }

    pub fn dir(&self, i: usize) -> Option<(u32, u32)> {
        self.dirs.get(i).copied().filter(|d| d.0 != 0)
    }

    pub fn rva_to_offset(&self, rva: u32) -> Result<usize, PeError> {
        for s in &self.sections {
            let len = s.vsize.max(s.raw_size);
            if rva >= s.va && rva < s.va + len {
                return Ok((rva - s.va + s.raw_ptr) as usize);
            }
        }
        Err(PeError::BadRva(rva))
    }

    pub fn slice_rva(&self, rva: u32, size: u32) -> Result<&'a [u8], PeError> {
        let o = self.rva_to_offset(rva)?;
        self.data.get(o..o + size as usize).ok_or(PeError::Truncated("data at RVA"))
    }

    /// Looks up a `StringFileInfo` value (e.g. `ProductVersion`) in the resource
    /// section. Walks the UTF-16LE key/value layout of `VS_VERSIONINFO` String
    /// entries: wLength, wValueLength, wType, key (NUL-terminated), pad to 4, value.
    pub fn version_string(&self, key: &str) -> Option<String> {
        let (rva, size) = self.dir(2)?;
        let rsrc = self.slice_rva(rva, size).ok()?;
        let needle: Vec<u8> = key.encode_utf16().chain([0u16]).flat_map(|c| c.to_le_bytes()).collect();
        let mut from = 0;
        while let Some(pos) = find(&rsrc[from..], &needle) {
            let k = from + pos;
            from = k + 2;
            // String struct header is the 6 bytes before the key, 4-aligned.
            if k < 6 || (k - 6) % 4 != 0 {
                continue;
            }
            let wlen = u16_at(rsrc, k - 6)? as usize;
            let vlen = u16_at(rsrc, k - 4)? as usize;
            let wtype = u16_at(rsrc, k - 2)?;
            if wtype != 1 || vlen == 0 || wlen < 6 + needle.len() {
                continue;
            }
            let mut v = k + needle.len();
            v = (v + 3) & !3;
            let words: Vec<u16> = (0..vlen).filter_map(|i| u16_at(rsrc, v + i * 2)).take_while(|&c| c != 0).collect();
            return String::from_utf16(&words).ok();
        }
        None
    }
}

pub(crate) fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Builds a tiny PE32+ image with one section `.text` at RVA 0x1000 holding `body`,
    /// and data directories as given (rva, size).
    pub fn build_pe(body: &[u8], dirs: &[(usize, u32, u32)]) -> Vec<u8> {
        let mut b = vec![0u8; 0x200];
        b[0..2].copy_from_slice(b"MZ");
        b[0x3C..0x40].copy_from_slice(&0x80u32.to_le_bytes());
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&0x8664u16.to_le_bytes());
        b[0x86..0x88].copy_from_slice(&1u16.to_le_bytes());
        b[0x94..0x96].copy_from_slice(&240u16.to_le_bytes());
        let opt = 0x98;
        b[opt..opt + 2].copy_from_slice(&0x20Bu16.to_le_bytes());
        b[opt + 108..opt + 112].copy_from_slice(&16u32.to_le_bytes());
        for &(i, rva, size) in dirs {
            let d = opt + 112 + i * 8;
            b[d..d + 4].copy_from_slice(&rva.to_le_bytes());
            b[d + 4..d + 8].copy_from_slice(&size.to_le_bytes());
        }
        let s = opt + 240;
        b[s..s + 5].copy_from_slice(b".text");
        b[s + 8..s + 12].copy_from_slice(&(body.len() as u32).to_le_bytes());
        b[s + 12..s + 16].copy_from_slice(&0x1000u32.to_le_bytes());
        b[s + 16..s + 20].copy_from_slice(&(body.len() as u32).to_le_bytes());
        b[s + 20..s + 24].copy_from_slice(&0x200u32.to_le_bytes());
        b.extend_from_slice(body);
        b
    }

    fn utf16z(s: &str) -> Vec<u8> {
        s.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect()
    }

    #[test]
    fn rejects_non_pe() {
        assert_eq!(Pe::parse(b"PK\x03\x04").unwrap_err(), PeError::NotMz);
    }

    #[test]
    fn reads_version_string() {
        let mut body = vec![0u8; 4];
        let key = utf16z("ProductVersion");
        let val = utf16z("1.8.99.0");
        let mut entry = Vec::new();
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&((val.len() / 2) as u16).to_le_bytes());
        entry.extend_from_slice(&1u16.to_le_bytes());
        entry.extend_from_slice(&key);
        while entry.len() % 4 != 0 {
            entry.push(0);
        }
        entry.extend_from_slice(&val);
        let total = entry.len() as u16;
        entry[0..2].copy_from_slice(&total.to_le_bytes());
        body.extend_from_slice(&entry);
        let img = build_pe(&body, &[(2, 0x1000, body.len() as u32)]);
        let pe = Pe::parse(&img).unwrap();
        assert_eq!(pe.machine, 0x8664);
        assert_eq!(pe.version_string("ProductVersion").as_deref(), Some("1.8.99.0"));
        assert_eq!(pe.version_string("CompanyName"), None);
    }
}
