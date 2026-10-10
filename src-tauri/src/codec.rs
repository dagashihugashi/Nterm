use encoding_rs::{CoderResult, Decoder, EncoderResult, Encoding, UTF_8};

/// 1セッション分の文字コード変換。
/// 受信側はストリーミングデコーダを保持し、チャンク境界で分断されたマルチバイト文字を正しく復元する。
pub struct TerminalCodec {
    encoding: &'static Encoding,
    decoder: Decoder,
}

impl TerminalCodec {
    /// `label` 省略時は UTF-8。`"Shift_JIS"` / `"EUC-JP"` 等の WHATWG ラベルを受け付ける。
    pub fn new(label: Option<&str>) -> Result<Self, String> {
        let encoding = match label {
            None => UTF_8,
            Some(label) => Encoding::for_label(label.as_bytes())
                .ok_or_else(|| format!("unsupported encoding: {label}"))?,
        };
        Ok(Self {
            encoding,
            decoder: encoding.new_decoder_without_bom_handling(),
        })
    }

    /// 受信バイト列をデコードする。末尾の不完全なマルチバイト文字は次回呼び出しまで保持される。
    pub fn decode(&mut self, bytes: &[u8]) -> String {
        let capacity = self
            .decoder
            .max_utf8_buffer_length(bytes.len())
            .unwrap_or(bytes.len() * 3 + 16);
        let mut text = String::with_capacity(capacity);
        let (result, _read, _had_replacements) =
            self.decoder.decode_to_string(bytes, &mut text, false);
        debug_assert_eq!(result, CoderResult::InputEmpty);
        text
    }

    /// 入力文字列を接続先の文字コードに変換する。変換できない文字は `?` に置き換える。
    pub fn encode(&self, text: &str) -> Vec<u8> {
        if self.encoding == UTF_8 {
            return text.as_bytes().to_vec();
        }
        let mut encoder = self.encoding.new_encoder();
        let mut bytes = Vec::with_capacity(text.len() * 2);
        let mut rest = text;
        loop {
            let needed = encoder
                .max_buffer_length_from_utf8_without_replacement(rest.len())
                .unwrap_or(rest.len() * 4 + 16);
            bytes.reserve(needed);
            let (result, read) =
                encoder.encode_from_utf8_to_vec_without_replacement(rest, &mut bytes, true);
            rest = &rest[read..];
            match result {
                EncoderResult::InputEmpty => return bytes,
                EncoderResult::OutputFull => {}
                EncoderResult::Unmappable(_) => bytes.push(b'?'),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_utf8() {
        let mut codec = TerminalCodec::new(None).unwrap();
        assert_eq!(codec.decode("abc 日本語".as_bytes()), "abc 日本語");
    }

    #[test]
    fn rejects_unknown_encoding() {
        assert!(TerminalCodec::new(Some("no-such-encoding")).is_err());
    }

    #[test]
    fn restores_utf8_char_split_across_chunks() {
        let mut codec = TerminalCodec::new(Some("UTF-8")).unwrap();
        let bytes = "日本".as_bytes(); // 6 bytes
        let first = codec.decode(&bytes[..1]);
        let second = codec.decode(&bytes[1..4]);
        let third = codec.decode(&bytes[4..]);
        assert_eq!(format!("{first}{second}{third}"), "日本");
        assert!(!first.contains('\u{FFFD}'));
    }

    #[test]
    fn restores_shift_jis_char_split_across_chunks() {
        let mut codec = TerminalCodec::new(Some("Shift_JIS")).unwrap();
        let bytes = [0x93, 0xFA, 0x96, 0x7B]; // "日本"
        let first = codec.decode(&bytes[..1]);
        let rest = codec.decode(&bytes[1..]);
        assert_eq!(format!("{first}{rest}"), "日本");
    }

    #[test]
    fn decodes_euc_jp() {
        let mut codec = TerminalCodec::new(Some("EUC-JP")).unwrap();
        assert_eq!(codec.decode(&[0xC6, 0xFC, 0xCB, 0xDC]), "日本");
    }

    #[test]
    fn encodes_utf8_as_is() {
        let codec = TerminalCodec::new(None).unwrap();
        assert_eq!(codec.encode("ls 日本"), "ls 日本".as_bytes());
    }

    #[test]
    fn encodes_to_shift_jis() {
        let codec = TerminalCodec::new(Some("Shift_JIS")).unwrap();
        assert_eq!(codec.encode("日本"), vec![0x93, 0xFA, 0x96, 0x7B]);
    }

    #[test]
    fn replaces_unmappable_chars_with_question_mark() {
        let codec = TerminalCodec::new(Some("Shift_JIS")).unwrap();
        assert_eq!(codec.encode("a😀b"), b"a?b".to_vec());
    }
}
