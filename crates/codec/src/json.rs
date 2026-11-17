//! JSON string output for the source map writers.

const WORD_BYTES: usize = 8;
const LOW_BITS: u64 = 0x0101_0101_0101_0101;
const HIGH_BITS: u64 = 0x8080_8080_8080_8080;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
/// The longest escape, `\u00XX`.
const MAX_ESCAPE_BYTES: usize = 6;
/// Spare bytes that the loop keeps beyond the input that is left. One
/// unchecked write is `WORD_BYTES` long, and the closing quote follows.
const SPARE_BYTES: usize = WORD_BYTES + 1;

/// The escape of one byte, padded to `WORD_BYTES`, and its length. A length of
/// 0 means that the byte needs no escape.
#[derive(Clone, Copy)]
struct Escape {
    bytes: [u8; WORD_BYTES],
    len: u8,
}

const fn short_escape(letter: u8) -> Escape {
    Escape { bytes: [b'\\', letter, 0, 0, 0, 0, 0, 0], len: 2 }
}

static ESCAPES: [Escape; 256] = {
    let mut table = [Escape { bytes: [0; WORD_BYTES], len: 0 }; 256];
    let mut byte = 0;
    while byte < 0x20 {
        table[byte] = Escape {
            bytes: [b'\\', b'u', b'0', b'0', HEX_DIGITS[byte >> 4], HEX_DIGITS[byte & 0xf], 0, 0],
            len: MAX_ESCAPE_BYTES as u8,
        };
        byte += 1;
    }
    table[b'\n' as usize] = short_escape(b'n');
    table[b'\r' as usize] = short_escape(b'r');
    table[b'\t' as usize] = short_escape(b't');
    table[b'"' as usize] = short_escape(b'"');
    table[b'\\' as usize] = short_escape(b'\\');
    table
};

/// Marks the bytes of `word` that need a JSON escape: a control character
/// below 0x20, `"` or `\`. A marked byte has its high bit set in the result.
///
/// A borrow can also mark a byte above a real match. The lowest marked byte is
/// always a real match.
#[inline(always)]
const fn escape_mask(word: u64) -> u64 {
    let control = word.wrapping_sub(LOW_BITS * 0x20) & !word;
    let quote = word ^ (LOW_BITS * b'"' as u64);
    let quote = quote.wrapping_sub(LOW_BITS) & !quote;
    let backslash = word ^ (LOW_BITS * b'\\' as u64);
    let backslash = backslash.wrapping_sub(LOW_BITS) & !backslash;
    (control | quote | backslash) & HIGH_BITS
}

/// Writes `WORD_BYTES` bytes at the end of `out` and adds `kept` of them to the vector.
///
/// # Safety
/// `out` must have at least `WORD_BYTES` bytes of spare capacity, and `kept`
/// must be at most `WORD_BYTES`.
#[inline(always)]
unsafe fn write_word(out: &mut Vec<u8>, word: &[u8; WORD_BYTES], kept: usize) {
    // SAFETY: the caller guarantees the spare capacity for the copy, and that
    // `kept` initialized bytes fit in it.
    unsafe {
        let end = out.as_mut_ptr().add(out.len());
        std::ptr::copy_nonoverlapping(word.as_ptr(), end, WORD_BYTES);
        out.set_len(out.len() + kept);
    }
}

/// Appends `s` to `out` as a JSON string, with the quotes.
///
/// The function escapes `"`, `\` and the control characters below 0x20. It
/// copies all other bytes as they are, so `out` stays valid UTF-8 when it was
/// valid UTF-8 before the call.
///
/// It reads the input 8 bytes at a time. It copies each 8-byte word to the
/// output first, and then keeps only the bytes before the first escape.
///
/// # Examples
///
/// ```
/// let mut out = Vec::new();
/// srcmap_codec::json_quote_into(&mut out, "a\"b\n");
/// assert_eq!(out, br#""a\"b\n""#);
/// ```
pub fn json_quote_into(out: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    out.push(b'"');
    out.reserve(bytes.len() + SPARE_BYTES);

    // The loop keeps this much spare capacity: the input that is left, plus `SPARE_BYTES`.
    let mut index = 0;
    while let Some((chunk, _)) = bytes[index..].split_first_chunk::<WORD_BYTES>() {
        let mask = escape_mask(u64::from_le_bytes(*chunk));
        if mask == 0 {
            // SAFETY: the spare capacity covers the input that is left, which
            // is at least this word.
            unsafe { write_word(out, chunk, WORD_BYTES) };
            index += WORD_BYTES;
            continue;
        }

        let clean = (mask.trailing_zeros() / u8::BITS) as usize;
        // SAFETY: as above, and `clean` is less than `WORD_BYTES`.
        unsafe { write_word(out, chunk, clean) };
        index += clean;

        // Source text often has several escapes in a row (tabs, line breaks),
        // so handle them here and go back to the word loop at a plain byte.
        while let Some(escape) = bytes.get(index).map(|&byte| &ESCAPES[byte as usize]) {
            if escape.len == 0 {
                break;
            }
            // An escape is longer than its input byte. Restore the spare
            // capacity before the write, which the escape then uses in part.
            out.reserve(bytes.len() - index + SPARE_BYTES + MAX_ESCAPE_BYTES);
            // SAFETY: the `reserve` above gives more than `WORD_BYTES` bytes,
            // and an escape is at most `MAX_ESCAPE_BYTES` long.
            unsafe { write_word(out, &escape.bytes, escape.len as usize) };
            index += 1;
        }
    }

    for &byte in &bytes[index..] {
        let escape = &ESCAPES[byte as usize];
        if escape.len == 0 {
            out.push(byte);
        } else {
            out.extend_from_slice(&escape.bytes[..escape.len as usize]);
        }
    }
    out.push(b'"');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(s: &str) -> String {
        let mut out = Vec::new();
        json_quote_into(&mut out, s);
        String::from_utf8(out).unwrap()
    }

    /// The escape that a JSON writer without the 8-byte step gives.
    fn reference_quote(s: &str) -> String {
        let mut out = String::from("\"");
        for ch in s.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    out.push_str("\\u00");
                    out.push(char::from(HEX_DIGITS[(c as usize) >> 4]));
                    out.push(char::from(HEX_DIGITS[(c as usize) & 0xf]));
                }
                c => out.push(c),
            }
        }
        out.push('"');
        out
    }

    #[test]
    fn plain_strings_keep_their_bytes() {
        assert_eq!(quote(""), "\"\"");
        assert_eq!(quote("abc"), "\"abc\"");
        assert_eq!(quote("src/components/App.tsx"), "\"src/components/App.tsx\"");
    }

    #[test]
    fn every_escape_has_the_json_form() {
        assert_eq!(quote("\""), r#""\"""#);
        assert_eq!(quote("\\"), r#""\\""#);
        assert_eq!(quote("\n\r\t"), r#""\n\r\t""#);
        assert_eq!(quote("\u{0}\u{1f}\u{b}"), r#""\u0000\u001f\u000b""#);
    }

    #[test]
    fn bytes_that_look_close_to_an_escape_stay_plain() {
        // 0x20 is the first byte that needs no escape. 0x7f and the bytes of a
        // multi-byte character have no escape in JSON.
        assert_eq!(quote(" !#[]\u{7f}"), "\" !#[]\u{7f}\"");
        assert_eq!(quote("héllo wörld ✓ 日本語"), "\"héllo wörld ✓ 日本語\"");
    }

    #[test]
    fn an_escape_at_each_position_of_a_word_is_found() {
        for escape in ['"', '\\', '\n', '\u{1}'] {
            for length in 0..40 {
                for position in 0..length {
                    let mut s: String = "x".repeat(length);
                    s.replace_range(position..=position, &escape.to_string());
                    assert_eq!(
                        quote(&s),
                        reference_quote(&s),
                        "{escape:?} at {position} of {length}"
                    );
                }
            }
        }
    }

    #[test]
    fn mixed_text_matches_the_reference_writer() {
        let source = "const a = \"x\\y\";\n\tif (a) {\r\n  // ✓ naïve \u{7}\n}\n".repeat(7);
        for end in 0..source.len() {
            if source.is_char_boundary(end) {
                assert_eq!(quote(&source[..end]), reference_quote(&source[..end]));
            }
        }
    }

    #[test]
    fn output_parses_as_the_same_string() {
        let source = "line 1\nline \"2\"\t\\ end \u{1}\u{1f} ü";
        let parsed: String = serde_json_parse(&quote(source));
        assert_eq!(parsed, source);
    }

    /// A minimal JSON string reader, so that the test needs no JSON crate.
    fn serde_json_parse(json: &str) -> String {
        let inner = &json[1..json.len() - 1];
        let mut out = String::new();
        let mut chars = inner.chars();
        while let Some(c) = chars.next() {
            if c != '\\' {
                out.push(c);
                continue;
            }
            match chars.next().unwrap() {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                }
                other => out.push(other),
            }
        }
        out
    }
}
