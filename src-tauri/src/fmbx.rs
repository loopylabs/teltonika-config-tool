//! Binary "FMBX" frames, docs/spec/02-framing.md §2. Used for status pages and
//! other extras; configuration itself goes over text commands.
// Nothing sends binary commands yet: their payloads are mostly unspecified (open question 8).
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};

use crate::framing::{Codec, Decoded};

/// ASCII "FMBX" (02-framing §2.1).
pub const PROTOCOL_ID: [u8; 4] = *b"FMBX";
pub const HEADER_LEN: usize = 12;
const CRC_LEN: usize = 2;

/// Generic response IDs (03-commands §7).
pub const RESPONSE_ACK: u16 = 0x0001;
pub const RESPONSE_ERROR: u16 = 0x0014;
pub const RESPONSE_UNKNOWN: u16 = 0xFFFF;

/// CRC-16/ARC: reflected poly 0xA001, init 0, no final XOR (02-framing §2.4).
pub fn crc16_arc(data: &[u8]) -> u16 {
    data.iter().fold(0u16, |crc, &byte| {
        (0..8).fold(crc ^ byte as u16, |crc, _| {
            if crc & 1 != 0 {
                (crc >> 1) ^ 0xA001
            } else {
                crc >> 1
            }
        })
    })
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub request_id: u32,
    /// Command ID in requests, response ID in responses.
    pub id: u16,
    pub payload: Vec<u8>,
}

/// Big-endian header, payload, then CRC over everything after the protocol ID.
pub fn encode_frame(frame: &Frame) -> Result<Vec<u8>, String> {
    let length = u16::try_from(frame.payload.len()).map_err(|_| "payload over 65535 bytes")?;
    let mut bytes = Vec::with_capacity(HEADER_LEN + frame.payload.len() + CRC_LEN);
    bytes.extend_from_slice(&PROTOCOL_ID);
    bytes.extend_from_slice(&frame.request_id.to_be_bytes());
    bytes.extend_from_slice(&frame.id.to_be_bytes());
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&frame.payload);
    let crc = crc16_arc(&bytes[PROTOCOL_ID.len()..]);
    bytes.extend_from_slice(&crc.to_be_bytes());
    Ok(bytes)
}

/// Decodes one frame from the front of `buf`, checking the protocol ID as soon as
/// its bytes arrive and the CRC once the frame is complete (02-framing §2.3).
pub fn decode_frame(buf: &[u8]) -> Result<Decoded<Frame>, String> {
    let magic = &buf[..buf.len().min(PROTOCOL_ID.len())];
    if magic != &PROTOCOL_ID[..magic.len()] {
        return Err("bad protocol ID".into());
    }
    if buf.len() < HEADER_LEN {
        return Ok(Decoded::Incomplete);
    }
    let length = u16::from_be_bytes([buf[10], buf[11]]) as usize;
    let total = HEADER_LEN + length + CRC_LEN;
    if buf.len() < total {
        return Ok(Decoded::Incomplete);
    }
    let expected = u16::from_be_bytes([buf[total - 2], buf[total - 1]]);
    if crc16_arc(&buf[PROTOCOL_ID.len()..total - CRC_LEN]) != expected {
        return Err("checksum mismatch".into());
    }
    Ok(Decoded::Frame {
        value: Frame {
            request_id: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
            id: u16::from_be_bytes([buf[8], buf[9]]),
            payload: buf[HEADER_LEN..HEADER_LEN + length].to_vec(),
        },
        consumed: total,
    })
}

/// Process-wide request counter, incremented before each command, so the first
/// command uses 1 (02-framing §2.2).
static LAST_REQUEST_ID: AtomicU32 = AtomicU32::new(0);

pub fn next_request_id() -> u32 {
    LAST_REQUEST_ID.fetch_add(1, Ordering::Relaxed).wrapping_add(1)
}

pub struct Request {
    pub command: u16,
    pub payload: Vec<u8>,
}

/// One binary exchange: stamps a fresh request ID and accepts only the reply
/// that echoes it. Checking the response ID against the command's expected set
/// (03-commands §7) is up to the caller.
#[derive(Default)]
pub struct FmbxCodec {
    sent_id: Option<u32>,
    /// Use this request ID instead of a fresh one (tests need predictable replies).
    pub fixed_id: Option<u32>,
}

impl FmbxCodec {
    pub fn with_id(fixed_id: Option<u32>) -> Self {
        Self { sent_id: None, fixed_id }
    }
}

impl Codec for FmbxCodec {
    type Request = Request;
    type Response = Frame;

    fn encode(&mut self, request: &Request) -> Result<Vec<u8>, String> {
        let request_id = self.fixed_id.unwrap_or_else(next_request_id);
        self.sent_id = Some(request_id);
        encode_frame(&Frame {
            request_id,
            id: request.command,
            payload: request.payload.clone(),
        })
    }

    fn decode(&mut self, buf: &[u8]) -> Result<Decoded<Frame>, String> {
        if let (Some(sent), true) = (self.sent_id, buf.len() >= 8) {
            if buf[4..8] != sent.to_be_bytes() {
                return Err("reply has a different request ID".into());
            }
        }
        decode_frame(buf)
    }
}

// ---------------------------------------------------------------- status data

/// One record of a status-data payload (03-commands §7.2).
#[derive(Debug, Clone, PartialEq)]
pub struct StatusRecord {
    pub id: u32,
    pub type_code: u8,
    pub value: StatusValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StatusValue {
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
    /// Byte arrays and the "hex rendering" types.
    Bytes(Vec<u8>),
}

/// Parses the payload of a status-data response: 0x0003 / 0x0023 have 2-byte IDs,
/// 0x0103 (`wide_ids`) 4-byte IDs. Byte 0 is skipped (03 §7.2); parsing stops quietly
/// at an unknown type code or a truncated record. Numbers are big-endian.
pub fn parse_status_data(payload: &[u8], wide_ids: bool) -> Vec<StatusRecord> {
    parse_records(payload, 1, wide_ids)
}

/// 0x003B reply (03 §7.1): the same record layout, but with no skipped first byte.
pub fn parse_single_status(payload: &[u8]) -> Option<StatusRecord> {
    parse_records(payload, 0, false).into_iter().next()
}

/// Count frame payload (03 §7.6.2): 1, 2 or 4 bytes big-endian; any other width is 0.
pub fn frame_count(payload: &[u8]) -> u32 {
    match payload.len() {
        1 | 2 | 4 => payload.iter().fold(0u32, |acc, &b| acc << 8 | b as u32),
        _ => 0,
    }
}

/// ISO-8859-1: one byte per character.
fn latin1(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| b as char).collect()
}

fn parse_records(payload: &[u8], start: usize, wide_ids: bool) -> Vec<StatusRecord> {
    let mut records = Vec::new();
    let mut at = start;
    let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
        let bytes = payload.get(*at..*at + n)?;
        *at += n;
        Some(bytes)
    };
    let uint = |bytes: &[u8]| bytes.iter().fold(0u64, |acc, &b| acc << 8 | b as u64);
    let int = |bytes: &[u8]| {
        let shift = 64 - 8 * bytes.len() as u32;
        ((uint(bytes) << shift) as i64) >> shift
    };
    loop {
        let Some(id) = take(&mut at, if wide_ids { 4 } else { 2 }).map(uint) else {
            break;
        };
        let Some(&[type_code]) = take(&mut at, 1) else {
            break;
        };
        let value = match type_code {
            0 | 3 | 5 | 7 | 17 => {
                let size = match type_code {
                    0 => 1,
                    3 => 2,
                    7 => 8,
                    _ => 4,
                };
                take(&mut at, size).map(|b| StatusValue::Unsigned(uint(b)))
            }
            1 | 2 | 4 | 6 => {
                let size = 1 << (type_code / 2).min(3);
                let size = if type_code == 1 { 1 } else { size };
                take(&mut at, size).map(|b| StatusValue::Signed(int(b)))
            }
            8 => take(&mut at, 4).map(|b| StatusValue::Float(f32::from_bits(uint(b) as u32) as f64)),
            9 => take(&mut at, 8).map(|b| StatusValue::Float(f64::from_bits(uint(b)))),
            18 => take(&mut at, 6).map(|b| StatusValue::Bytes(b.to_vec())),
            20..=23 => take(&mut at, 1 << (type_code - 20)).map(|b| StatusValue::Bytes(b.to_vec())),
            16 | 19 | 24 | 25 | 32 => {
                let len_size = if type_code == 24 { 2 } else { 1 };
                take(&mut at, len_size)
                    .map(uint)
                    .and_then(|len| take(&mut at, len as usize))
                    .map(|b| match type_code {
                        19 | 25 => StatusValue::Bytes(b.to_vec()),
                        _ => StatusValue::Text(latin1(b)),
                    })
            }
            _ => None,
        };
        let Some(value) = value else {
            break;
        };
        records.push(StatusRecord { id: id as u32, type_code, value });
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Vec<u8> {
        text.split_whitespace()
            .map(|b| u8::from_str_radix(b, 16).unwrap())
            .collect()
    }

    fn frame(request_id: u32, id: u16, payload: &[u8]) -> Frame {
        Frame { request_id, id, payload: payload.to_vec() }
    }

    // Test vectors from 02-framing §2.5.

    #[test]
    fn crc16_arc_check_values() {
        assert_eq!(crc16_arc(b"123456789"), 0xBB3D);
        assert_eq!(crc16_arc(b""), 0x0000);
    }

    #[test]
    fn vector_get_status_data() {
        let bytes = hex("46 4D 42 58 00 00 00 01 00 02 00 00 00 9C");
        assert_eq!(encode_frame(&frame(1, 0x0002, &[])).unwrap(), bytes);
        assert_decodes(&bytes, frame(1, 0x0002, &[]));
    }

    #[test]
    fn vector_request_device_info() {
        let bytes = hex("46 4D 42 58 00 00 00 01 00 1C 00 00 06 FC");
        assert_eq!(encode_frame(&frame(1, 0x001C, &[])).unwrap(), bytes);
        assert_decodes(&bytes, frame(1, 0x001C, &[]));
    }

    #[test]
    fn vector_single_status_parameter_781() {
        let bytes = hex("46 4D 42 58 00 00 00 02 00 38 00 02 03 0D F0 E2");
        assert_eq!(encode_frame(&frame(2, 0x0038, &[0x03, 0x0D])).unwrap(), bytes);
        assert_decodes(&bytes, frame(2, 0x0038, &[0x03, 0x0D]));
    }

    #[test]
    fn vector_single_status_parameter_784() {
        let bytes = hex("46 4D 42 58 00 00 00 03 00 38 00 02 03 10 39 32");
        assert_eq!(encode_frame(&frame(3, 0x0038, &[0x03, 0x10])).unwrap(), bytes);
        assert_decodes(&bytes, frame(3, 0x0038, &[0x03, 0x10]));
    }

    fn assert_decodes(bytes: &[u8], expected: Frame) {
        assert_eq!(
            decode_frame(bytes).unwrap(),
            Decoded::Frame { value: expected, consumed: bytes.len() }
        );
    }

    // Test vectors from 03-commands §9.3 and §9.4.

    #[test]
    fn command_class_request_vectors() {
        let vectors: [(u32, u16, &[u8], &str); 6] = [
            (1, 0x0002, &[], "46 4D 42 58 00 00 00 01 00 02 00 00 00 9C"),
            (2, 0x0102, &[0, 0, 0, 0, 0, 0, 0, 0x01], "46 4D 42 58 00 00 00 02 01 02 00 08 00 00 00 00 00 00 00 01 39 57"),
            (3, 0x0102, &[0, 0, 0, 0, 0, 0, 0xFF, 0xFF], "46 4D 42 58 00 00 00 03 01 02 00 08 00 00 00 00 00 00 FF FF C8 95"),
            (4, 0x0038, &[0x03, 0x0D], "46 4D 42 58 00 00 00 04 00 38 00 02 03 0D F0 84"),
            (5, 0x0004, &[], "46 4D 42 58 00 00 00 05 00 04 00 00 C1 8D"),
            (6, 0x001C, &[], "46 4D 42 58 00 00 00 06 00 1C 00 00 C6 49"),
        ];
        for (request_id, command, payload, bytes) in vectors {
            let bytes = hex(bytes);
            assert_eq!(encode_frame(&frame(request_id, command, payload)).unwrap(), bytes, "{command:#06x}");
            assert_decodes(&bytes, frame(request_id, command, payload));
        }
    }

    #[test]
    fn ack_response_vector() {
        assert_decodes(&hex("46 4D 42 58 00 00 00 05 00 01 00 00 C0 9D"), frame(5, RESPONSE_ACK, &[]));
    }

    #[test]
    fn status_data_response_vector() {
        let bytes = hex(
            "46 4D 42 58 00 00 00 01 00 03 00 15 \
             00 \
             03 0D 03 00 01 \
             03 10 05 00 00 00 05 \
             00 03 10 04 54 45 53 54 \
             46 5E",
        );
        let Decoded::Frame { value, consumed } = decode_frame(&bytes).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!((value.request_id, value.id, consumed), (1, 0x0003, bytes.len()));
        assert_eq!(
            parse_status_data(&value.payload, false),
            [
                StatusRecord { id: 781, type_code: 3, value: StatusValue::Unsigned(1) },
                StatusRecord { id: 784, type_code: 5, value: StatusValue::Unsigned(5) },
                StatusRecord { id: 3, type_code: 16, value: StatusValue::Text("TEST".into()) },
            ]
        );
    }

    #[test]
    fn status_data_types_and_stops() {
        let payload = [
            &[0x00][..],
            &[0, 0, 0, 9, 2, 0xFF, 0xFE],    // wide ID 9, i16 -2
            &[0, 0, 0, 10, 1, 0x80],         // i8 -128
            &[0, 0, 0, 11, 24, 0, 2, b'h', b'i'], // string, 2-byte length
            &[0, 0, 0, 12, 99, 1, 2, 3],     // unknown type: stop
        ]
        .concat();
        let records = parse_status_data(&payload, true);
        assert_eq!(
            records.iter().map(|r| r.value.clone()).collect::<Vec<_>>(),
            [StatusValue::Signed(-2), StatusValue::Signed(-128), StatusValue::Text("hi".into())]
        );
        // A truncated record ends parsing too.
        assert!(parse_status_data(&[0, 0x03, 0x0D, 0x05, 0x00], false).is_empty());
    }

    #[test]
    fn partial_frames_wait_for_more_bytes() {
        let bytes = hex("46 4D 42 58 00 00 00 02 00 38 00 02 03 0D F0 E2");
        for len in 0..bytes.len() {
            assert_eq!(decode_frame(&bytes[..len]).unwrap(), Decoded::Incomplete, "len {len}");
        }
    }

    #[test]
    fn extra_bytes_after_a_frame_are_left_over() {
        let mut bytes = hex("46 4D 42 58 00 00 00 01 00 02 00 00 00 9C");
        bytes.extend_from_slice(b"FMB");
        let Decoded::Frame { consumed, .. } = decode_frame(&bytes).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(consumed, 14);
    }

    #[test]
    fn rejects_bad_protocol_id_and_checksum() {
        assert!(decode_frame(b"FMBY").is_err());
        assert!(decode_frame(b"X").is_err());
        let mut bytes = hex("46 4D 42 58 00 00 00 01 00 1C 00 00 06 FC");
        bytes[13] ^= 0xFF;
        assert!(decode_frame(&bytes).is_err());
    }

    #[test]
    fn codec_uses_increasing_request_ids_and_checks_the_echo() {
        let mut codec = FmbxCodec::default();
        let request = Request { command: 0x0002, payload: vec![] };
        let first = codec.encode(&request).unwrap();
        let second = codec.encode(&request).unwrap();
        let id = |b: &[u8]| u32::from_be_bytes([b[4], b[5], b[6], b[7]]);
        assert!(id(&second) > id(&first));

        // A reply echoing the second request is accepted; one echoing the first is not.
        let reply = encode_frame(&frame(id(&second), 0x0003, &[0])).unwrap();
        assert!(matches!(codec.decode(&reply), Ok(Decoded::Frame { .. })));
        let stale = encode_frame(&frame(id(&first), 0x0003, &[0])).unwrap();
        assert!(codec.decode(&stale).is_err());
    }

    // ---- 03-commands v6 §9.3 live-status requests and §9.4 replies.

    #[test]
    fn live_status_request_vectors() {
        let be = |n: u16| n.to_be_bytes().to_vec();
        let groups = |g: u8| vec![0, 0, 0, 0, 0, 0, 0, g];
        let vectors: [(u32, u16, Vec<u8>, &str); 16] = [
            (10, 0x0038, be(784), "46 4D 42 58 00 00 00 0A 00 38 00 02 03 10 39 AB"),
            (11, 0x0038, be(773), "46 4D 42 58 00 00 00 0B 00 38 00 02 03 05 36 7A"),
            (12, 0x0038, be(790), "46 4D 42 58 00 00 00 0C 00 38 00 02 03 16 3B 4D"),
            (13, 0x0038, be(803), "46 4D 42 58 00 00 00 0D 00 38 00 02 03 23 EC 9D"),
            (14, 0x0038, be(810), "46 4D 42 58 00 00 00 0E 00 38 00 02 03 2A EA 6E"),
            (15, 0x0102, groups(0x10), "46 4D 42 58 00 00 00 0F 01 02 00 08 00 00 00 00 00 00 00 10 78 84"),
            (16, 0x0102, groups(0x20), "46 4D 42 58 00 00 00 10 01 02 00 08 00 00 00 00 00 00 00 20 F3 AE"),
            (17, 0x001C, vec![], "46 4D 42 58 00 00 00 11 00 1C 00 00 C5 3D"),
            (18, 0x001E, vec![], "46 4D 42 58 00 00 00 12 00 1E 00 00 05 D8"),
            (19, 0x0020, vec![], "46 4D 42 58 00 00 00 13 00 20 00 00 09 84"),
            (20, 0x0022, vec![], "46 4D 42 58 00 00 00 14 00 22 00 00 09 90"),
            (21, 0x0024, vec![], "46 4D 42 58 00 00 00 15 00 24 00 00 C8 4D"),
            (22, 0x0026, vec![], "46 4D 42 58 00 00 00 16 00 26 00 00 08 A8"),
            (7, 0x0010, vec![], "46 4D 42 58 00 00 00 07 00 10 00 00 05 B4"),
            (8, 0x0012, vec![], "46 4D 42 58 00 00 00 08 00 12 00 00 C4 41"),
            (9, 0x0015, vec![], "46 4D 42 58 00 00 00 09 00 15 00 00 C5 CD"),
        ];
        for (request_id, command, payload, bytes) in vectors {
            let bytes = hex(bytes);
            assert_eq!(encode_frame(&frame(request_id, command, &payload)).unwrap(), bytes, "{command:#06x} #{request_id}");
            assert_decodes(&bytes, frame(request_id, command, &payload));
        }
        assert_eq!(&b".log:1\r"[..], &hex("2E 6C 6F 67 3A 31 0D")[..]);
        assert_eq!(&b".log:0\r"[..], &hex("2E 6C 6F 67 3A 30 0D")[..]);
    }

    fn decoded(bytes: &str) -> Frame {
        match decode_frame(&hex(bytes)).unwrap() {
            Decoded::Frame { value, .. } => value,
            Decoded::Incomplete => panic!("incomplete"),
        }
    }

    #[test]
    fn count_frame_vectors() {
        let f = decoded("46 4D 42 58 00 00 00 0F 00 01 00 02 00 02 C1 E2");
        assert_eq!((f.request_id, f.id, frame_count(&f.payload)), (15, RESPONSE_ACK, 2));
        let f = decoded("46 4D 42 58 00 00 00 14 00 01 00 01 01 B8 C2");
        assert_eq!((f.request_id, f.id, frame_count(&f.payload)), (20, RESPONSE_ACK, 1));
        let f = decoded("46 4D 42 58 00 00 00 01 00 01 00 01 01 7D C0");
        assert_eq!(frame_count(&f.payload), 1);
        // Widths: u8, u16, u32; anything else is 0.
        assert_eq!(frame_count(&[7]), 7);
        assert_eq!(frame_count(&[1, 0]), 256);
        assert_eq!(frame_count(&[0, 1, 0, 0]), 65_536);
        assert_eq!((frame_count(&[]), frame_count(&[0, 0, 1])), (0, 0));
    }

    #[test]
    fn status_reply_vectors_with_signed_unsigned_and_float() {
        let f = decoded("46 4D 42 58 00 00 00 01 00 03 00 0D 00 03 05 03 31 38 03 03 05 65 53 F1 00 5A F7");
        assert_eq!(f.id, 0x0003);
        assert_eq!(
            parse_status_data(&f.payload, false),
            [
                StatusRecord { id: 773, type_code: 3, value: StatusValue::Unsigned(12600) },
                StatusRecord { id: 771, type_code: 5, value: StatusValue::Unsigned(1_700_000_000) },
            ]
        );
        let f = decoded("46 4D 42 58 00 00 00 0F 01 03 00 15 00 00 00 01 09 09 40 4B 57 F6 2B 6A E7 D5 00 00 01 0C 03 00 39 C6 5B");
        assert_eq!(f.id, 0x0103);
        let records = parse_status_data(&f.payload, true);
        assert_eq!(records.len(), 2);
        assert_eq!((records[0].id, &records[0].value), (265, &StatusValue::Float(54.6872)));
        assert_eq!((records[1].id, &records[1].value), (268, &StatusValue::Unsigned(57)));
        let f = decoded("46 4D 42 58 00 00 00 14 00 23 00 0A 00 00 01 00 01 00 09 03 10 1B 3E 31");
        assert_eq!(f.id, 0x0023);
        let records = parse_status_data(&f.payload, false);
        assert_eq!(records.iter().map(|r| (r.id, r.value.clone())).collect::<Vec<_>>(), [(1, StatusValue::Unsigned(1)), (9, StatusValue::Unsigned(4123))]);
    }

    #[test]
    fn single_status_reply_vectors_have_no_skipped_byte() {
        let f = decoded("46 4D 42 58 00 00 00 0A 00 3B 00 0B 03 10 07 00 00 00 00 00 00 0F FF D6 34");
        assert_eq!(f.id, 0x003B);
        let r = parse_single_status(&f.payload).unwrap();
        assert_eq!((r.id, r.type_code, r.value), (784, 7, StatusValue::Unsigned(0x0FFF)));
        let f = decoded("46 4D 42 58 00 00 00 0C 00 3B 00 04 03 16 00 01 43 0E");
        let r = parse_single_status(&f.payload).unwrap();
        assert_eq!((r.id, r.value), (790, StatusValue::Unsigned(1)));
        let f = decoded("46 4D 42 58 00 00 00 0D 00 3B 00 09 03 23 18 00 04 DE AD BE EF A0 DD");
        let r = parse_single_status(&f.payload).unwrap();
        assert_eq!((r.id, r.type_code), (803, 24));
        assert_eq!(r.value, StatusValue::Text("\u{DE}\u{AD}\u{BE}\u{EF}".into()));
        let f = decoded("46 4D 42 58 00 00 00 0B 00 3D 00 00 0D 34");
        assert_eq!((f.request_id, f.id, f.payload.len()), (11, 0x003D, 0));
    }

    #[test]
    fn signed_and_big_endian_types() {
        let payload = [
            &[0x00][..],
            &[0, 1, 2, 0x80, 0x00],             // i16 -32768
            &[0, 2, 4, 0xFF, 0xFF, 0xFF, 0xFE], // i32 -2
            &[0, 3, 6, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], // i64 -1
            &[0, 4, 8, 0x3F, 0x80, 0x00, 0x00], // f32 1.0
            &[0, 5, 5, 0x00, 0x00, 0x01, 0x00], // u32 256
            &[0, 6, 17, 0x00, 0x00, 0x01, 0x02], // alternate u32 258
        ]
        .concat();
        let values: Vec<_> = parse_status_data(&payload, false).into_iter().map(|r| r.value).collect();
        assert_eq!(
            values,
            [
                StatusValue::Signed(-32768),
                StatusValue::Signed(-2),
                StatusValue::Signed(-1),
                StatusValue::Float(1.0),
                StatusValue::Unsigned(256),
                StatusValue::Unsigned(258)
            ]
        );
    }
}
