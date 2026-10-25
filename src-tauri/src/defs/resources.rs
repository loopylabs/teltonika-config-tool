//! Binary resource-set reader (docs/spec/10-device-presentation.md §1.2). Reads the
//! entry names and the byte range of each stream/byte-array value. All offsets come
//! from the file and are bounds-checked; malformed input yields an error, never a panic.

use std::fmt;

const MAGIC: u32 = 0xBEEF_CACE;
const TYPE_BYTES: u32 = 0x20;
const TYPE_STREAM: u32 = 0x21;

#[derive(Debug, PartialEq, Eq)]
pub enum ResError {
    Magic,
    Truncated(&'static str),
    Bad(&'static str),
}

impl fmt::Display for ResError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResError::Magic => write!(f, "not a binary resource set"),
            ResError::Truncated(w) => write!(f, "resource set truncated ({w})"),
            ResError::Bad(w) => write!(f, "resource set malformed ({w})"),
        }
    }
}

/// One entry holding picture-style bytes: its (as stored) name and the byte range in the blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub start: usize,
    pub len: usize,
}

struct Cur<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Cur<'a> {
    fn i32(&mut self, what: &'static str) -> Result<i32, ResError> {
        let s = self.b.get(self.p..self.p + 4).ok_or(ResError::Truncated(what))?;
        self.p += 4;
        Ok(i32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    fn nonneg(&mut self, what: &'static str) -> Result<usize, ResError> {
        usize::try_from(self.i32(what)?).map_err(|_| ResError::Bad(what))
    }

    fn v7(&mut self, what: &'static str) -> Result<usize, ResError> {
        let (mut v, mut shift) = (0usize, 0u32);
        loop {
            let c = *self.b.get(self.p).ok_or(ResError::Truncated(what))?;
            self.p += 1;
            v |= usize::from(c & 0x7F) << shift;
            if c & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
            if shift > 28 {
                return Err(ResError::Bad(what));
            }
        }
    }

    fn skip(&mut self, n: usize, what: &'static str) -> Result<(), ResError> {
        let end = self.p.checked_add(n).filter(|&e| e <= self.b.len()).ok_or(ResError::Truncated(what))?;
        self.p = end;
        Ok(())
    }
}

/// Lists the stream (0x21) and byte-array (0x20) entries; other value types are skipped.
pub fn entries(blob: &[u8]) -> Result<Vec<Entry>, ResError> {
    let mut c = Cur { b: blob, p: 0 };
    if c.i32("magic")? as u32 != MAGIC {
        return Err(ResError::Magic);
    }
    let _header_version = c.i32("header version")?;
    let header_skip = c.nonneg("header skip")?;
    c.skip(header_skip, "reader header")?;
    let _set_version = c.i32("set version")?;
    let count = c.nonneg("entry count")?;
    let types = c.nonneg("type count")?;
    // Every entry needs at least 8 bytes of tables; reject absurd counts early.
    if count > blob.len() / 8 || types > blob.len() {
        return Err(ResError::Bad("counts"));
    }
    for _ in 0..types {
        let n = c.v7("type name length")?;
        c.skip(n, "type name")?;
    }
    c.p = (c.p + 7) & !7;
    c.skip(count * 4, "name hashes")?;
    let mut positions = Vec::with_capacity(count);
    for _ in 0..count {
        positions.push(c.nonneg("name position")?);
    }
    let data_start = c.nonneg("data section offset")?;
    let names_start = c.p;
    let mut out = Vec::new();
    for pos in positions {
        let mut n = Cur { b: blob, p: names_start.checked_add(pos).ok_or(ResError::Bad("name position"))? };
        let bytes = n.v7("name length")?;
        let raw = n.b.get(n.p..n.p.checked_add(bytes).ok_or(ResError::Bad("name length"))?).ok_or(ResError::Truncated("name"))?;
        n.p += bytes;
        let units: Vec<u16> = raw.chunks_exact(2).map(|u| u16::from_le_bytes([u[0], u[1]])).collect();
        let name = String::from_utf16_lossy(&units);
        let data_off = n.nonneg("data offset")?;
        let mut d = Cur { b: blob, p: data_start.checked_add(data_off).ok_or(ResError::Bad("data offset"))? };
        let code = d.v7("type code")? as u32;
        if code == TYPE_STREAM || code == TYPE_BYTES {
            let len = d.nonneg("value length")?;
            d.skip(len, "value")?;
            out.push(Entry { name, start: d.p - len, len });
        }
    }
    Ok(out)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn v7(mut n: usize) -> Vec<u8> {
        let mut o = Vec::new();
        loop {
            let b = (n & 0x7F) as u8;
            n >>= 7;
            if n == 0 {
                o.push(b);
                return o;
            }
            o.push(b | 0x80);
        }
    }

    /// Builds a resource set per spec 10 §1.2. `values`: (name, type code, payload).
    pub fn build_set(values: &[(&str, u32, &[u8])], type_names: &[&str]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&MAGIC.to_le_bytes());
        b.extend_from_slice(&1i32.to_le_bytes());
        b.extend_from_slice(&5i32.to_le_bytes());
        b.extend_from_slice(b"ab\0cd");
        b.extend_from_slice(&2i32.to_le_bytes());
        b.extend_from_slice(&(values.len() as i32).to_le_bytes());
        b.extend_from_slice(&(type_names.len() as i32).to_le_bytes());
        for t in type_names {
            b.extend(v7(t.len()));
            b.extend_from_slice(t.as_bytes());
        }
        while b.len() % 8 != 0 {
            b.push(b'P');
        }
        for i in 0..values.len() {
            b.extend_from_slice(&(i as i32 * 17).to_le_bytes());
        }
        // Name records and data, in reverse order so positions are not monotonic.
        let mut names = Vec::new();
        let mut data = Vec::new();
        let mut pos = vec![0i32; values.len()];
        for (i, (name, code, payload)) in values.iter().enumerate().rev() {
            pos[i] = names.len() as i32;
            let u: Vec<u8> = name.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
            names.extend(v7(u.len()));
            names.extend(u);
            names.extend_from_slice(&(data.len() as i32).to_le_bytes());
            data.extend(v7(*code as usize));
            if *code == TYPE_STREAM || *code == TYPE_BYTES {
                data.extend_from_slice(&(payload.len() as i32).to_le_bytes());
            }
            data.extend_from_slice(payload);
        }
        for p in pos {
            b.extend_from_slice(&p.to_le_bytes());
        }
        let data_start = b.len() + 4 + names.len();
        b.extend_from_slice(&(data_start as i32).to_le_bytes());
        b.extend(names);
        b.extend(data);
        b
    }

    #[test]
    fn reads_streams_and_byte_arrays_and_skips_other_types() {
        let big = vec![7u8; 300];
        let blob = build_set(&[("devices/fmb1_6.png", 0x21, b"\x89PNGone"), ("misc/n.txt", 0x01, b"str"), ("devices/unknown.png", 0x20, &big), ("brands/é.baml", 0x21, b"")], &[]);
        let e = entries(&blob).unwrap();
        let names: Vec<&str> = e.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["devices/fmb1_6.png", "devices/unknown.png", "brands/é.baml"]);
        assert_eq!(&blob[e[0].start..e[0].start + e[0].len], b"\x89PNGone");
        assert_eq!(e[1].len, 300);
        assert!(blob[e[1].start..e[1].start + 300].iter().all(|&x| x == 7));
        assert_eq!(e[2].len, 0);
    }

    #[test]
    fn handles_type_names_and_alignment() {
        for names in [&["System.String"][..], &["a", "bc", "def"][..]] {
            let blob = build_set(&[("x", 0x21, b"y")], names);
            assert_eq!(entries(&blob).unwrap().len(), 1);
        }
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        assert_eq!(entries(b""), Err(ResError::Truncated("magic")));
        assert_eq!(entries(&[0u8; 64]), Err(ResError::Magic));
        let blob = build_set(&[("a", 0x21, b"12345678")], &[]);
        for cut in 0..blob.len() {
            let _ = entries(&blob[..cut]);
        }
        let mut bad = blob.clone();
        let n = bad.len();
        bad[n - 12..n - 8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(entries(&bad).is_err());
        let mut huge = blob;
        huge[21..25].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(entries(&huge).is_err());
    }
}
