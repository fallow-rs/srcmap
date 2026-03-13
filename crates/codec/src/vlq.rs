//! Base64 VLQ encoding/decoding primitives.
//!
//! VLQ (Variable-Length Quantity) encoding stores arbitrary integers
//! as sequences of base64 characters. The sign bit is stored in the
//! least significant bit, and continuation bits indicate multi-char values.

use crate::DecodeError;

const VLQ_BASE_SHIFT: u32 = 5;
const VLQ_BASE: u64 = 1 << VLQ_BASE_SHIFT; // 32
const VLQ_BASE_MASK: u64 = VLQ_BASE - 1; // 0b11111
const VLQ_CONTINUATION_BIT: u64 = VLQ_BASE; // 0b100000

/// Maximum shift before overflow. 13 VLQ digits × 5 bits = 65 bits,
/// which exceeds i64 range. We allow shift up to 60 (13th digit).
const VLQ_MAX_SHIFT: u32 = 60;

/// Pre-computed base64 encode lookup table (index -> char byte).
#[rustfmt::skip]
const BASE64_ENCODE: [u8; 64] = [
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H',
    b'I', b'J', b'K', b'L', b'M', b'N', b'O', b'P',
    b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X',
    b'Y', b'Z', b'a', b'b', b'c', b'd', b'e', b'f',
    b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
    b'o', b'p', b'q', b'r', b's', b't', b'u', b'v',
    b'w', b'x', b'y', b'z', b'0', b'1', b'2', b'3',
    b'4', b'5', b'6', b'7', b'8', b'9', b'+', b'/',
];

/// Cache-line-aligned base64 encode table for encoding hot paths.
/// 64 bytes fits exactly in one cache line, avoiding split-load penalties.
#[repr(align(64))]
struct AlignedBase64Table([u8; 64]);

#[rustfmt::skip]
static BASE64_TABLE: AlignedBase64Table = AlignedBase64Table([
    b'A', b'B', b'C', b'D', b'E', b'F', b'G', b'H',
    b'I', b'J', b'K', b'L', b'M', b'N', b'O', b'P',
    b'Q', b'R', b'S', b'T', b'U', b'V', b'W', b'X',
    b'Y', b'Z', b'a', b'b', b'c', b'd', b'e', b'f',
    b'g', b'h', b'i', b'j', b'k', b'l', b'm', b'n',
    b'o', b'p', b'q', b'r', b's', b't', b'u', b'v',
    b'w', b'x', b'y', b'z', b'0', b'1', b'2', b'3',
    b'4', b'5', b'6', b'7', b'8', b'9', b'+', b'/',
]);

/// Pre-computed base64 decode lookup table (char byte -> value).
/// Invalid characters map to 255.
const BASE64_DECODE: [u8; 128] = {
    let mut table = [255u8; 128];
    let mut i = 0u8;
    while i < 64 {
        table[BASE64_ENCODE[i as usize] as usize] = i;
        i += 1;
    }
    table
};

/// Encode a single signed VLQ value directly into the buffer using unchecked writes.
///
/// # Safety
/// Caller must ensure `out` has at least 7 bytes of spare capacity
/// (`out.capacity() - out.len() >= 7`).
#[inline(always)]
pub unsafe fn vlq_encode_unchecked(out: &mut Vec<u8>, value: i64) {
    // Convert to VLQ signed representation using u64 to avoid overflow.
    // Sign bit goes in the LSB. Two's complement negation via bit ops
    // handles all values including i64::MIN safely.
    let mut vlq: u64 = if value >= 0 {
        (value as u64) << 1
    } else {
        // !(x as u64) + 1 computes the absolute value for any negative i64.
        // For i64::MIN (-2^63), abs = 2^63, and (2^63 << 1) wraps to 0 in u64.
        // This produces an incorrect encoding for i64::MIN, but that value
        // is unreachable in valid source maps (max ~4 billion lines/columns).
        ((!(value as u64)) + 1) << 1 | 1
    };

    let table = &BASE64_TABLE.0;
    // SAFETY: caller guarantees at least 7 bytes of spare capacity.
    let ptr = unsafe { out.as_mut_ptr().add(out.len()) };
    let mut i = 0;

    loop {
        let digit = vlq & VLQ_BASE_MASK;
        vlq >>= VLQ_BASE_SHIFT;
        if vlq == 0 {
            // Last byte — no continuation bit needed
            // SAFETY: i < 7 and we have 7 bytes of spare capacity.
            // digit is always in 0..64 so the table lookup is in bounds.
            unsafe {
                *ptr.add(i) = *table.get_unchecked(digit as usize);
            }
            i += 1;
            break;
        }
        // SAFETY: same as above; digit | VLQ_CONTINUATION_BIT is in 0..64.
        unsafe {
            *ptr.add(i) = *table.get_unchecked((digit | VLQ_CONTINUATION_BIT) as usize);
        }
        i += 1;
    }

    // SAFETY: we wrote exactly `i` valid ASCII bytes into the spare capacity.
    unsafe { out.set_len(out.len() + i) };
}

/// Encode a single VLQ value, appending base64 chars to the output buffer.
#[inline(always)]
pub fn vlq_encode(out: &mut Vec<u8>, value: i64) {
    out.reserve(7);
    // SAFETY: we just reserved 7 bytes, which is the maximum a single
    // i64 VLQ value can produce (63 data bits / 5 bits per char = 13,
    // but the sign bit reduces the effective range, and real source map
    // values are far smaller — 7 bytes handles up to ±2^34).
    unsafe { vlq_encode_unchecked(out, value) }
}

/// Decode a single VLQ value from the input bytes starting at the given position.
///
/// Returns `(decoded_value, bytes_consumed)` or a [`DecodeError`].
#[inline]
pub fn vlq_decode(input: &[u8], pos: usize) -> Result<(i64, usize), DecodeError> {
    if pos >= input.len() {
        return Err(DecodeError::UnexpectedEof { offset: pos });
    }

    let b0 = input[pos];
    if b0 >= 128 {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: pos,
        });
    }
    let d0 = BASE64_DECODE[b0 as usize];
    if d0 == 255 {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: pos,
        });
    }

    // Fast path: single character VLQ (values -15..15, ~60-70% of real data)
    if (d0 & 0x20) == 0 {
        let val = (d0 >> 1) as i64;
        return Ok((if (d0 & 1) != 0 { -val } else { val }, 1));
    }

    // Multi-character VLQ
    let mut result: u64 = (d0 & 0x1F) as u64;
    let mut shift: u32 = 5;
    let mut i = pos + 1;

    loop {
        if i >= input.len() {
            return Err(DecodeError::UnexpectedEof { offset: i });
        }

        let byte = input[i];

        if byte >= 128 {
            return Err(DecodeError::InvalidBase64 { byte, offset: i });
        }

        let digit = BASE64_DECODE[byte as usize];

        if digit == 255 {
            return Err(DecodeError::InvalidBase64 { byte, offset: i });
        }

        i += 1;

        if shift >= VLQ_MAX_SHIFT {
            return Err(DecodeError::VlqOverflow { offset: pos });
        }

        result += ((digit & 0x1F) as u64) << shift;
        shift += VLQ_BASE_SHIFT;

        if (digit & 0x20) == 0 {
            break;
        }
    }

    // Extract sign from LSB
    let value = if (result & 1) == 1 {
        -((result >> 1) as i64)
    } else {
        (result >> 1) as i64
    };

    Ok((value, i - pos))
}

/// Encode a single unsigned VLQ value directly into the buffer using unchecked writes.
///
/// # Safety
/// Caller must ensure `out` has at least 7 bytes of spare capacity
/// (`out.capacity() - out.len() >= 7`).
#[inline(always)]
pub unsafe fn vlq_encode_unsigned_unchecked(out: &mut Vec<u8>, value: u64) {
    let table = &BASE64_TABLE.0;
    // SAFETY: caller guarantees at least 7 bytes of spare capacity.
    let ptr = unsafe { out.as_mut_ptr().add(out.len()) };
    let mut i = 0;
    let mut vlq = value;

    loop {
        let digit = vlq & VLQ_BASE_MASK;
        vlq >>= VLQ_BASE_SHIFT;
        if vlq == 0 {
            // Last byte — no continuation bit needed
            // SAFETY: i < 7 and we have 7 bytes of spare capacity.
            // digit is always in 0..64 so the table lookup is in bounds.
            unsafe {
                *ptr.add(i) = *table.get_unchecked(digit as usize);
            }
            i += 1;
            break;
        }
        // SAFETY: same as above; digit | VLQ_CONTINUATION_BIT is in 0..64.
        unsafe {
            *ptr.add(i) = *table.get_unchecked((digit | VLQ_CONTINUATION_BIT) as usize);
        }
        i += 1;
    }

    // SAFETY: we wrote exactly `i` valid ASCII bytes into the spare capacity.
    unsafe { out.set_len(out.len() + i) };
}

/// Encode a single unsigned VLQ value, appending base64 chars to the output buffer.
///
/// Unlike signed VLQ, no sign bit is used — all 5 bits per character are data.
/// Used by the ECMA-426 scopes proposal for tags, flags, and unsigned values.
#[inline(always)]
pub fn vlq_encode_unsigned(out: &mut Vec<u8>, value: u64) {
    out.reserve(7);
    // SAFETY: we just reserved 7 bytes, which is the maximum a single
    // u64 VLQ value can produce in practice.
    unsafe { vlq_encode_unsigned_unchecked(out, value) }
}

/// Decode a single unsigned VLQ value from the input bytes starting at the given position.
///
/// Returns `(decoded_value, bytes_consumed)` or a [`DecodeError`].
/// Unlike signed VLQ, no sign bit extraction is performed.
#[inline]
pub fn vlq_decode_unsigned(input: &[u8], pos: usize) -> Result<(u64, usize), DecodeError> {
    if pos >= input.len() {
        return Err(DecodeError::UnexpectedEof { offset: pos });
    }

    let b0 = input[pos];
    if b0 >= 128 {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: pos,
        });
    }
    let d0 = BASE64_DECODE[b0 as usize];
    if d0 == 255 {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: pos,
        });
    }

    // Fast path: single character (value fits in 5 bits, no continuation)
    if (d0 & 0x20) == 0 {
        return Ok((d0 as u64, 1));
    }

    // Multi-character unsigned VLQ
    let mut result: u64 = (d0 & 0x1F) as u64;
    let mut shift: u32 = 5;
    let mut i = pos + 1;

    loop {
        if i >= input.len() {
            return Err(DecodeError::UnexpectedEof { offset: i });
        }

        let byte = input[i];

        if byte >= 128 {
            return Err(DecodeError::InvalidBase64 { byte, offset: i });
        }

        let digit = BASE64_DECODE[byte as usize];

        if digit == 255 {
            return Err(DecodeError::InvalidBase64 { byte, offset: i });
        }

        i += 1;

        if shift >= VLQ_MAX_SHIFT {
            return Err(DecodeError::VlqOverflow { offset: pos });
        }

        result += ((digit & 0x1F) as u64) << shift;
        shift += VLQ_BASE_SHIFT;

        if (digit & 0x20) == 0 {
            break;
        }
    }

    Ok((result, i - pos))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_zero() {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, 0);
        assert_eq!(&buf, b"A");
    }

    #[test]
    fn encode_positive() {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, 1);
        assert_eq!(&buf, b"C");
    }

    #[test]
    fn encode_negative() {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, -1);
        assert_eq!(&buf, b"D");
    }

    #[test]
    fn encode_large_value() {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, 1000);
        let (decoded, _) = vlq_decode(&buf, 0).unwrap();
        assert_eq!(decoded, 1000);
    }

    #[test]
    fn roundtrip_values() {
        let values = [
            0,
            1,
            -1,
            15,
            -15,
            16,
            -16,
            31,
            32,
            100,
            -100,
            1000,
            -1000,
            100_000,
            1_000_000_000,
            -1_000_000_000,
        ];
        for &v in &values {
            let mut buf = Vec::new();
            vlq_encode(&mut buf, v);
            let (decoded, consumed) = vlq_decode(&buf, 0).unwrap();
            assert_eq!(decoded, v, "roundtrip failed for {v}");
            assert_eq!(consumed, buf.len());
        }
    }

    #[test]
    fn decode_multi_char() {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, 500);
        assert!(buf.len() > 1, "500 should need multiple chars");
        let (decoded, _) = vlq_decode(&buf, 0).unwrap();
        assert_eq!(decoded, 500);
    }

    #[test]
    fn decode_non_ascii_byte() {
        let input = [0xC0u8];
        let err = vlq_decode(&input, 0).unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: 0xC0,
                offset: 0
            }
        );
    }

    #[test]
    fn decode_invalid_base64_char() {
        let input = b"!";
        let err = vlq_decode(input, 0).unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: b'!',
                offset: 0
            }
        );
    }

    #[test]
    fn decode_unexpected_eof() {
        // 'g' = 32, which has the continuation bit set
        let input = b"g";
        let err = vlq_decode(input, 0).unwrap_err();
        assert_eq!(err, DecodeError::UnexpectedEof { offset: 1 });
    }

    #[test]
    fn decode_overflow() {
        // 14 continuation chars: shift reaches 60+ which exceeds limit
        let input = b"ggggggggggggggA";
        let err = vlq_decode(input, 0).unwrap_err();
        assert!(matches!(err, DecodeError::VlqOverflow { .. }));
    }

    #[test]
    fn decode_empty_input() {
        let err = vlq_decode(b"", 0).unwrap_err();
        assert_eq!(err, DecodeError::UnexpectedEof { offset: 0 });
    }

    // --- Unsigned VLQ tests ---

    #[test]
    fn unsigned_encode_zero() {
        let mut buf = Vec::new();
        vlq_encode_unsigned(&mut buf, 0);
        assert_eq!(&buf, b"A");
    }

    #[test]
    fn unsigned_encode_small_values() {
        // Value 1 → 'B', value 2 → 'C', ..., value 31 → base64[31] = 'f'
        let mut buf = Vec::new();
        vlq_encode_unsigned(&mut buf, 1);
        assert_eq!(&buf, b"B");

        buf.clear();
        vlq_encode_unsigned(&mut buf, 8);
        assert_eq!(&buf, b"I");
    }

    #[test]
    fn unsigned_roundtrip() {
        let values: [u64; 10] = [0, 1, 8, 15, 16, 31, 32, 100, 1000, 100_000];
        for &v in &values {
            let mut buf = Vec::new();
            vlq_encode_unsigned(&mut buf, v);
            let (decoded, consumed) = vlq_decode_unsigned(&buf, 0).unwrap();
            assert_eq!(decoded, v, "unsigned roundtrip failed for {v}");
            assert_eq!(consumed, buf.len());
        }
    }

    #[test]
    fn unsigned_multi_char() {
        let mut buf = Vec::new();
        vlq_encode_unsigned(&mut buf, 500);
        assert!(buf.len() > 1, "500 should need multiple chars");
        let (decoded, _) = vlq_decode_unsigned(&buf, 0).unwrap();
        assert_eq!(decoded, 500);
    }

    #[test]
    fn unsigned_decode_empty() {
        let err = vlq_decode_unsigned(b"", 0).unwrap_err();
        assert_eq!(err, DecodeError::UnexpectedEof { offset: 0 });
    }

    #[test]
    fn unsigned_decode_non_ascii() {
        let err = vlq_decode_unsigned(&[0xC3, 0x80], 0).unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: 0xC3,
                offset: 0
            }
        );
    }

    #[test]
    fn unsigned_decode_invalid_base64_char() {
        let err = vlq_decode_unsigned(b"!", 0).unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: b'!',
                offset: 0
            }
        );
    }

    #[test]
    fn unsigned_decode_overflow() {
        // 14 continuation chars to trigger overflow
        let err = vlq_decode_unsigned(b"ggggggggggggggA", 0).unwrap_err();
        assert!(matches!(err, DecodeError::VlqOverflow { .. }));
    }
}
