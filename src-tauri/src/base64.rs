//! 標準 Base64（RFC 4648）。known_hosts のフィンガープリントと credentials.json で使う。

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const PAD: char = '=';

/// パディングあり
pub fn encode(bytes: &[u8]) -> String {
    let mut encoded = encode_no_pad(bytes);
    while !encoded.len().is_multiple_of(4) {
        encoded.push(PAD);
    }
    encoded
}

/// パディングなし（OpenSSH のフィンガープリント表記）
pub fn encode_no_pad(bytes: &[u8]) -> String {
    let mut encoded = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b1 = chunk.get(1).copied().unwrap_or(0);
        let b2 = chunk.get(2).copied().unwrap_or(0);
        let group = (u32::from(chunk[0]) << 16) | (u32::from(b1) << 8) | u32::from(b2);
        // 入力 n バイトに対して n+1 文字を出力する
        for i in 0..=chunk.len() {
            let index = (group >> (18 - 6 * i)) & 0x3F;
            encoded.push(char::from(ALPHABET[index as usize]));
        }
    }
    encoded
}

/// パディングの有無どちらも受け付ける
pub fn decode(text: &str) -> Result<Vec<u8>, String> {
    let body = text.trim().trim_end_matches(PAD);
    let mut decoded = Vec::with_capacity(body.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits = 0;
    for byte in body.bytes() {
        let value = ALPHABET
            .iter()
            .position(|&c| c == byte)
            .ok_or_else(|| format!("invalid base64 character: {:?}", char::from(byte)))?;
        buffer = (buffer << 6) | value as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            decoded.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VECTORS: [(&str, &str); 7] = [
        ("", ""),
        ("f", "Zg=="),
        ("fo", "Zm8="),
        ("foo", "Zm9v"),
        ("foob", "Zm9vYg=="),
        ("fooba", "Zm9vYmE="),
        ("foobar", "Zm9vYmFy"),
    ];

    #[test]
    fn encodes_rfc4648_vectors() {
        for (plain, encoded) in VECTORS {
            assert_eq!(encode(plain.as_bytes()), encoded);
        }
    }

    #[test]
    fn encodes_without_padding() {
        assert_eq!(encode_no_pad(b"f"), "Zg");
        assert_eq!(encode_no_pad(b"fo"), "Zm8");
        assert_eq!(encode_no_pad(&[0xFB, 0xFF]), "+/8");
    }

    #[test]
    fn decodes_rfc4648_vectors() {
        for (plain, encoded) in VECTORS {
            assert_eq!(decode(encoded).unwrap(), plain.as_bytes());
        }
    }

    #[test]
    fn decodes_unpadded_input() {
        assert_eq!(decode("Zm8").unwrap(), b"fo");
    }

    #[test]
    fn round_trips_binary_data() {
        let bytes: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&encode(&bytes)).unwrap(), bytes);
    }

    #[test]
    fn rejects_invalid_characters() {
        assert!(decode("Zm9v!").is_err());
    }
}
