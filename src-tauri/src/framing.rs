//! Turning requests into bytes and byte streams back into replies.
//! Text commands follow docs/spec/02-framing.md §1; binary frames live in `fmbx.rs`.

/// Result of looking for one reply at the front of a receive buffer.
#[derive(Debug, PartialEq)]
pub enum Decoded<T> {
    /// Not enough bytes yet.
    Incomplete,
    /// A whole reply; `consumed` bytes belong to it and anything after is left over.
    Frame { value: T, consumed: usize },
}

pub trait Codec {
    type Request: ?Sized;
    type Response;

    fn encode(&mut self, request: &Self::Request) -> Result<Vec<u8>, String>;

    /// Errors mean the reply is malformed and can't become valid with more bytes.
    fn decode(&mut self, buf: &[u8]) -> Result<Decoded<Self::Response>, String>;
}

/// Terminators for text replies (02-framing §1.2, 03-commands).
pub const CR: &[u8] = b"\r";
/// Ends the full-configuration dump: ETB FS DC4 NUL CR.
pub const CONFIG_END_MARKER: &[u8] = &[0x17, 0x1C, 0x14, 0x00, 0x0D];

/// Encodes a text command: 7-bit ASCII, ended by a single CR (02-framing §1.1).
pub fn text_command(command: &str) -> Result<Vec<u8>, String> {
    if !command.is_ascii() || command.contains(['\r', '\n']) {
        return Err("commands must be ASCII without line breaks".into());
    }
    let mut bytes = Vec::with_capacity(command.len() + 1);
    bytes.extend_from_slice(command.as_bytes());
    bytes.extend_from_slice(CR);
    Ok(bytes)
}

/// Decodes bytes as ASCII text (replies are ASCII, 01-transport §3).
pub fn ascii(bytes: &[u8]) -> String {
    bytes.iter().map(|&b| if b.is_ascii() { b as char } else { '\u{FFFD}' }).collect()
}

/// A text command whose reply is complete once the given terminator arrives.
/// The reply text excludes the terminator.
pub struct TextCodec {
    pub until: &'static [u8],
}

impl TextCodec {
    pub const fn until(until: &'static [u8]) -> Self {
        Self { until }
    }
}

impl Codec for TextCodec {
    type Request = str;
    type Response = String;

    fn encode(&mut self, command: &str) -> Result<Vec<u8>, String> {
        text_command(command)
    }

    fn decode(&mut self, buf: &[u8]) -> Result<Decoded<String>, String> {
        let end = buf.windows(self.until.len()).position(|w| w == self.until);
        Ok(match end {
            Some(at) => Decoded::Frame {
                value: ascii(&buf[..at]),
                consumed: at + self.until.len(),
            },
            None => Decoded::Incomplete,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_end_in_a_single_cr() {
        assert_eq!(text_command(":cfg_info:?").unwrap(), b":cfg_info:?\r");
        assert!(text_command("bad\rcmd").is_err());
        assert!(text_command("caf\u{e9}").is_err());
    }

    #[test]
    fn text_reply_waits_for_the_terminator() {
        let mut codec = TextCodec::until(CR);
        assert_eq!(codec.decode(b"<SECSTAT>1,0").unwrap(), Decoded::Incomplete);
        assert_eq!(
            codec.decode(b"<SECSTAT>1,0,1,0,3\rextra").unwrap(),
            Decoded::Frame { value: "<SECSTAT>1,0,1,0,3".into(), consumed: 19 }
        );
    }

    #[test]
    fn config_dump_ends_at_the_marker_not_at_the_first_cr() {
        let mut codec = TextCodec::until(CONFIG_END_MARKER);
        let mut reply = b"<A>\r101:4\r<B>:1\r".to_vec();
        assert_eq!(codec.decode(&reply).unwrap(), Decoded::Incomplete);
        reply.extend_from_slice(CONFIG_END_MARKER);
        let Decoded::Frame { value, consumed } = codec.decode(&reply).unwrap() else {
            panic!("expected a frame");
        };
        assert_eq!(value, "<A>\r101:4\r<B>:1\r");
        assert_eq!(consumed, reply.len());
    }
}
