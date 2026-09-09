//! Optional clipboard frames on a separate authenticated QUIC stream. Never
//! mixed into the latency-sensitive keyboard/mouse control stream.

pub const CAPABILITY_CLIPBOARD: u32 = 1 << 4;
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
pub const HEADER_BYTES: usize = 17;
pub const MAX_CLIPBOARD_FRAME: usize = HEADER_BYTES + MAX_IMAGE_BYTES;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardContent {
    Text(String),
    Png(Vec<u8>),
}

impl ClipboardContent {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Text(text) => text.as_bytes(),
            Self::Png(png) => png,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardPacket {
    pub revision: u64,
    pub content: ClipboardContent,
}

impl ClipboardPacket {
    pub fn encode(&self) -> Result<Vec<u8>, &'static str> {
        let (kind, limit) = match self.content {
            ClipboardContent::Text(_) => (1, MAX_TEXT_BYTES),
            ClipboardContent::Png(_) => (2, MAX_IMAGE_BYTES),
        };
        let bytes = self.content.bytes();
        if matches!(&self.content, ClipboardContent::Text(text) if text.contains('\0')) {
            return Err("clipboard text contains NUL");
        }
        if self.revision == 0 || bytes.len() > limit || bytes.is_empty() {
            return Err("invalid clipboard size or revision");
        }
        let mut frame = Vec::with_capacity(HEADER_BYTES + bytes.len());
        frame.extend_from_slice(b"EMC1");
        frame.push(kind);
        frame.extend_from_slice(&self.revision.to_be_bytes());
        frame.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        frame.extend_from_slice(bytes);
        Ok(frame)
    }

    pub fn decode(frame: &[u8]) -> Result<Self, &'static str> {
        if !(HEADER_BYTES..=MAX_CLIPBOARD_FRAME).contains(&frame.len()) || &frame[..4] != b"EMC1" {
            return Err("invalid clipboard frame");
        }
        let revision = u64::from_be_bytes(frame[5..13].try_into().unwrap());
        let length = u32::from_be_bytes(frame[13..17].try_into().unwrap()) as usize;
        if revision == 0 || length == 0 || length != frame.len() - HEADER_BYTES {
            return Err("invalid clipboard length or revision");
        }
        let bytes = &frame[HEADER_BYTES..];
        let content = match frame[4] {
            1 if length <= MAX_TEXT_BYTES && !bytes.contains(&0) => ClipboardContent::Text(
                std::str::from_utf8(bytes)
                    .map_err(|_| "invalid clipboard UTF-8")?
                    .to_owned(),
            ),
            2 if length <= MAX_IMAGE_BYTES && bytes.starts_with(b"\x89PNG\r\n\x1a\n") => {
                ClipboardContent::Png(bytes.to_vec())
            }
            _ => return Err("unsupported clipboard content"),
        };
        Ok(Self { revision, content })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_multiline_roundtrip_and_strict_framing() {
        let packet = ClipboardPacket {
            revision: 2,
            content: ClipboardContent::Text("你好 👋\nsecond line 尾".into()),
        };
        let encoded = packet.encode().unwrap();
        assert_eq!(ClipboardPacket::decode(&encoded).unwrap(), packet);
        for cut in 0..encoded.len() {
            assert!(ClipboardPacket::decode(&encoded[..cut]).is_err());
        }
        let mut extra = encoded.clone();
        extra.push(0);
        assert!(ClipboardPacket::decode(&extra).is_err());
        let mut bad_utf8 = encoded;
        *bad_utf8.last_mut().unwrap() = 0xff;
        assert!(ClipboardPacket::decode(&bad_utf8).is_err());
    }

    #[test]
    fn clipboard_limits_and_unknown_content_are_rejected() {
        assert!(
            ClipboardPacket {
                revision: 1,
                content: ClipboardContent::Text("x".repeat(MAX_TEXT_BYTES + 1))
            }
            .encode()
            .is_err()
        );
        assert!(
            ClipboardPacket {
                revision: 0,
                content: ClipboardContent::Text("x".into())
            }
            .encode()
            .is_err()
        );
        let mut frame = ClipboardPacket {
            revision: 1,
            content: ClipboardContent::Text("x".into()),
        }
        .encode()
        .unwrap();
        frame[4] = 255;
        assert!(ClipboardPacket::decode(&frame).is_err());
        frame[4] = 2;
        assert!(ClipboardPacket::decode(&frame).is_err());
    }
}
