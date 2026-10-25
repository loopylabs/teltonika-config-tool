//! .NET single-file bundle reader (public dotnet/runtime single-file bundle format).

use std::fmt;
use std::io::Read;

use super::pe::{find, u32_at, u64_at};

pub const SIGNATURE: [u8; 32] = [
    0x8B, 0x12, 0x02, 0xB9, 0x6A, 0x61, 0x20, 0x38, 0x72, 0x7B, 0x93, 0x02, 0x14, 0xD7, 0xA0, 0x32,
    0x13, 0xF5, 0xB9, 0xE6, 0xEF, 0xAE, 0x33, 0x18, 0xEE, 0x3B, 0x2D, 0xCE, 0x24, 0xB3, 0x6A, 0xAE,
];

#[derive(Debug, PartialEq, Eq)]
pub enum BundleError {
    NoMarker,
    Truncated(&'static str),
    BadEntry(String),
    Inflate(String),
}

impl fmt::Display for BundleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BundleError::NoMarker => write!(f, "unsupported Configurator build (no application bundle)"),
            BundleError::Truncated(w) => write!(f, "application bundle truncated ({w})"),
            BundleError::BadEntry(p) => write!(f, "application bundle entry out of range: {p}"),
            BundleError::Inflate(e) => write!(f, "application bundle entry failed to inflate: {e}"),
        }
    }
}

#[derive(Debug, Clone)]
#[cfg_attr(not(test), allow(dead_code))] // kind: diagnostics and integration test
pub struct Entry {
    pub offset: u64,
    pub size: u64,
    pub compressed_size: u64,
    pub kind: u8,
    pub path: String,
}

#[derive(Debug)]
#[cfg_attr(not(test), allow(dead_code))] // offsets: diagnostics and integration test
pub struct Bundle {
    pub signature_offset: usize,
    pub header_offset: u64,
    pub major: u32,
    pub minor: u32,
    pub entries: Vec<Entry>,
}

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl Cur<'_> {
    fn u8(&mut self) -> Result<u8, BundleError> {
        let v = *self.b.get(self.p).ok_or(BundleError::Truncated("header"))?;
        self.p += 1;
        Ok(v)
    }
    fn u32(&mut self) -> Result<u32, BundleError> {
        let v = u32_at(self.b, self.p).ok_or(BundleError::Truncated("header"))?;
        self.p += 4;
        Ok(v)
    }
    fn u64(&mut self) -> Result<u64, BundleError> {
        let v = u64_at(self.b, self.p).ok_or(BundleError::Truncated("header"))?;
        self.p += 8;
        Ok(v)
    }
    /// 7-bit-encoded length prefixed UTF-8 string.
    fn string(&mut self) -> Result<String, BundleError> {
        let mut len = 0usize;
        let mut shift = 0;
        loop {
            let b = self.u8()?;
            len |= ((b & 0x7F) as usize) << shift;
            if b & 0x80 == 0 {
                break;
            }
            shift += 7;
            if shift > 28 {
                return Err(BundleError::Truncated("string length"));
            }
        }
        let s = self.b.get(self.p..self.p + len).ok_or(BundleError::Truncated("string"))?;
        self.p += len;
        Ok(String::from_utf8_lossy(s).into_owned())
    }
}

impl Bundle {
    pub fn parse(exe: &[u8]) -> Result<Self, BundleError> {
        let sig = find(exe, &SIGNATURE).ok_or(BundleError::NoMarker)?;
        if sig < 8 {
            return Err(BundleError::NoMarker);
        }
        let header_offset = u64_at(exe, sig - 8).unwrap();
        if header_offset == 0 || header_offset as usize >= exe.len() {
            return Err(BundleError::NoMarker);
        }
        let mut c = Cur { b: exe, p: header_offset as usize };
        let major = c.u32()?;
        let minor = c.u32()?;
        let count = c.u32()? as i32;
        let _bundle_id = c.string()?;
        if major >= 2 {
            c.p += 40;
        }
        let mut entries = Vec::new();
        for _ in 0..count.max(0) {
            let offset = c.u64()?;
            let size = c.u64()?;
            let compressed_size = if major >= 6 { c.u64()? } else { 0 };
            let kind = c.u8()?;
            let path = c.string()?.replace('\\', "/");
            entries.push(Entry { offset, size, compressed_size, kind, path });
        }
        Ok(Bundle { signature_offset: sig, header_offset, major, minor, entries })
    }

    pub fn find(&self, path: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.path.eq_ignore_ascii_case(path))
    }

    pub fn read(&self, exe: &[u8], e: &Entry) -> Result<Vec<u8>, BundleError> {
        let stored = if e.compressed_size != 0 { e.compressed_size } else { e.size };
        let start = e.offset as usize;
        let raw = exe
            .get(start..start.saturating_add(stored as usize))
            .ok_or_else(|| BundleError::BadEntry(e.path.clone()))?;
        if e.compressed_size == 0 {
            return Ok(raw.to_vec());
        }
        let mut out = Vec::with_capacity(e.size as usize);
        flate2::read::DeflateDecoder::new(raw)
            .read_to_end(&mut out)
            .map_err(|err| BundleError::Inflate(err.to_string()))?;
        if out.len() as u64 != e.size {
            return Err(BundleError::Inflate(format!("{}: size mismatch", e.path)));
        }
        Ok(out)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    fn put_str(v: &mut Vec<u8>, s: &str) {
        v.push(s.len() as u8);
        v.extend_from_slice(s.as_bytes());
    }

    /// Fake host: signature preceded by header offset, files, then a v6 header.
    pub fn build_bundle(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut exe = crate::defs::pe::tests::build_pe(&[0u8; 16], &[]);
        let hdr_slot = exe.len();
        exe.extend_from_slice(&[0u8; 8]);
        exe.extend_from_slice(&SIGNATURE);
        let mut meta = Vec::new();
        for (path, data, compress) in files {
            let off = exe.len() as u64;
            let (stored, csize) = if *compress {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                let c = e.finish().unwrap();
                let n = c.len() as u64;
                (c, n)
            } else {
                (data.to_vec(), 0)
            };
            exe.extend_from_slice(&stored);
            meta.push((off, data.len() as u64, csize, path.to_string()));
        }
        let hdr = exe.len() as u64;
        exe[hdr_slot..hdr_slot + 8].copy_from_slice(&hdr.to_le_bytes());
        exe.extend_from_slice(&6u32.to_le_bytes());
        exe.extend_from_slice(&0u32.to_le_bytes());
        exe.extend_from_slice(&(meta.len() as u32).to_le_bytes());
        put_str(&mut exe, "bundle-id");
        exe.extend_from_slice(&[0u8; 40]);
        for (off, size, csize, path) in meta {
            exe.extend_from_slice(&off.to_le_bytes());
            exe.extend_from_slice(&size.to_le_bytes());
            exe.extend_from_slice(&csize.to_le_bytes());
            exe.push(1);
            put_str(&mut exe, &path);
        }
        exe
    }

    #[test]
    fn reads_stored_and_deflated_entries() {
        let exe = build_bundle(&[("a.dll", b"hello", false), ("sub\\b.dll", b"world world world", true)]);
        let b = Bundle::parse(&exe).unwrap();
        assert_eq!((b.major, b.entries.len()), (6, 2));
        assert_eq!(b.read(&exe, b.find("a.dll").unwrap()).unwrap(), b"hello");
        assert_eq!(b.read(&exe, b.find("sub/b.dll").unwrap()).unwrap(), b"world world world");
    }

    #[test]
    fn no_marker() {
        assert_eq!(Bundle::parse(b"MZ nothing here").unwrap_err(), BundleError::NoMarker);
    }
}
