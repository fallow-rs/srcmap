//! High-performance VLQ source map codec.
//!
//! Encodes and decodes source map mappings using the Base64 VLQ format
//! as specified in the Source Map v3 specification (ECMA-426).
//!
//! # Features
//!
//! - **`parallel`** — enables `encode_parallel` for multi-threaded encoding via rayon.
//!   ~1.5x faster for large maps (5K+ lines).
//!
//! # Examples
//!
//! Decode and re-encode a mappings string:
//!
//! ```
//! use srcmap_codec::{decode, encode};
//!
//! let mappings = decode("AAAA;AACA,EAAE").unwrap();
//! assert_eq!(mappings.len(), 2); // 2 lines
//! assert_eq!(mappings[0][0], vec![0, 0, 0, 0]); // first segment
//!
//! let encoded = encode(&mappings);
//! assert_eq!(encoded, "AAAA;AACA,EAAE");
//! ```
//!
//! Low-level VLQ primitives:
//!
//! ```
//! use srcmap_codec::{vlq_decode, vlq_encode};
//!
//! let mut buf = Vec::new();
//! vlq_encode(&mut buf, 42);
//!
//! let (value, bytes_read) = vlq_decode(&buf, 0).unwrap();
//! assert_eq!(value, 42);
//! ```

mod decode;
mod encode;
mod vlq;

pub use decode::decode;
pub use encode::encode;
#[cfg(feature = "parallel")]
pub use encode::encode_parallel;
pub use vlq::{vlq_decode, vlq_decode_unsigned, vlq_encode, vlq_encode_unsigned};

use std::fmt;

/// A single source map segment stored inline (no heap allocation).
///
/// Segments have 1, 4, or 5 fields:
/// - 1 field:  `[generated_column]`
/// - 4 fields: `[generated_column, source_index, original_line, original_column]`
/// - 5 fields: `[generated_column, source_index, original_line, original_column, name_index]`
///
/// Implements `Deref<Target=[i64]>` so indexing, `len()`, `is_empty()`, and
/// iteration work identically to `Vec<i64>`.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    data: [i64; 5],
    len: u8,
}

impl std::hash::Hash for Segment {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl PartialOrd for Segment {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Segment {
    #[inline]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl Segment {
    /// Create a 1-field segment (generated column only).
    #[inline]
    pub fn one(a: i64) -> Self {
        Self {
            data: [a, 0, 0, 0, 0],
            len: 1,
        }
    }

    /// Create a 4-field segment (with source info, no name).
    #[inline]
    pub fn four(a: i64, b: i64, c: i64, d: i64) -> Self {
        Self {
            data: [a, b, c, d, 0],
            len: 4,
        }
    }

    /// Create a 5-field segment (with source info and name).
    #[inline]
    pub fn five(a: i64, b: i64, c: i64, d: i64, e: i64) -> Self {
        Self {
            data: [a, b, c, d, e],
            len: 5,
        }
    }

    /// Convert to a `Vec<i64>` (for interop with APIs that expect `Vec`).
    pub fn to_vec(&self) -> Vec<i64> {
        self.data[..self.len as usize].to_vec()
    }
}

impl std::ops::Deref for Segment {
    type Target = [i64];

    #[inline]
    fn deref(&self) -> &[i64] {
        &self.data[..self.len as usize]
    }
}

impl<'a> IntoIterator for &'a Segment {
    type Item = &'a i64;
    type IntoIter = std::slice::Iter<'a, i64>;

    #[inline]
    fn into_iter(self) -> Self::IntoIter {
        self.data[..self.len as usize].iter()
    }
}

impl PartialEq for Segment {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl Eq for Segment {}

impl PartialEq<Vec<i64>> for Segment {
    fn eq(&self, other: &Vec<i64>) -> bool {
        **self == **other
    }
}

impl PartialEq<Segment> for Vec<i64> {
    fn eq(&self, other: &Segment) -> bool {
        **self == **other
    }
}

impl From<Vec<i64>> for Segment {
    fn from(v: Vec<i64>) -> Self {
        let mut data = [0i64; 5];
        let len = v.len().min(5);
        data[..len].copy_from_slice(&v[..len]);
        Self { data, len: len as u8 }
    }
}

impl From<&[i64]> for Segment {
    fn from(s: &[i64]) -> Self {
        let mut data = [0i64; 5];
        let len = s.len().min(5);
        data[..len].copy_from_slice(&s[..len]);
        Self { data, len: len as u8 }
    }
}

/// A source map line is a list of segments.
pub type Line = Vec<Segment>;

/// Decoded source map mappings: a list of lines, each containing segments.
pub type SourceMapMappings = Vec<Line>;

/// Errors that can occur when decoding a VLQ-encoded mappings string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// A byte that is not a valid base64 character was encountered.
    InvalidBase64 { byte: u8, offset: usize },
    /// Input ended in the middle of a VLQ sequence (continuation bit was set).
    UnexpectedEof { offset: usize },
    /// A VLQ value exceeded the maximum representable range.
    VlqOverflow { offset: usize },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidBase64 { byte, offset } => {
                write!(
                    f,
                    "invalid base64 character 0x{byte:02x} at offset {offset}"
                )
            }
            Self::UnexpectedEof { offset } => {
                write!(f, "unexpected end of input at offset {offset}")
            }
            Self::VlqOverflow { offset } => {
                write!(f, "VLQ value overflow at offset {offset}")
            }
        }
    }
}

impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Roundtrip tests ---

    #[test]
    fn roundtrip_empty() {
        let decoded = decode("").unwrap();
        assert!(decoded.is_empty());
        assert_eq!(encode(&decoded), "");
    }

    #[test]
    fn roundtrip_simple() {
        let input = "AAAA;AACA";
        let decoded = decode(input).unwrap();
        let encoded = encode(&decoded);
        assert_eq!(encoded, input);
    }

    #[test]
    fn roundtrip_multiple_segments() {
        let input = "AAAA,GAAG,EAAE;AACA";
        let decoded = decode(input).unwrap();
        let encoded = encode(&decoded);
        assert_eq!(encoded, input);
    }

    #[test]
    fn roundtrip_large_values() {
        let mappings = vec![vec![Segment::five(1000, 50, 999, 500, 100)]];
        let encoded = encode(&mappings);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, mappings);
    }

    #[test]
    fn roundtrip_negative_deltas() {
        let mappings = vec![vec![Segment::four(10, 0, 10, 10), Segment::four(20, 0, 5, 5)]];
        let encoded = encode(&mappings);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, mappings);
    }

    // --- Decode structure tests ---

    #[test]
    fn decode_single_field_segment() {
        let decoded = decode("A").unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].len(), 1);
        assert_eq!(decoded[0][0], vec![0]);
    }

    #[test]
    fn decode_four_field_segment() {
        let decoded = decode("AAAA").unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].len(), 1);
        assert_eq!(decoded[0][0], vec![0, 0, 0, 0]);
    }

    #[test]
    fn decode_five_field_segment() {
        let decoded = decode("AAAAA").unwrap();
        assert_eq!(decoded.len(), 1);
        assert_eq!(decoded[0].len(), 1);
        assert_eq!(decoded[0][0], vec![0, 0, 0, 0, 0]);
    }

    #[test]
    fn decode_negative_values() {
        let decoded = decode("DADD").unwrap();
        assert_eq!(decoded[0][0], vec![-1, 0, -1, -1]);
    }

    #[test]
    fn decode_multiple_lines() {
        let decoded = decode("AAAA;AACA;AACA").unwrap();
        assert_eq!(decoded.len(), 3);
    }

    #[test]
    fn decode_empty_lines() {
        let decoded = decode("AAAA;;;AACA").unwrap();
        assert_eq!(decoded.len(), 4);
        assert!(decoded[1].is_empty());
        assert!(decoded[2].is_empty());
    }

    #[test]
    fn decode_trailing_semicolon() {
        // Trailing `;` means an empty line follows
        let decoded = decode("AAAA;").unwrap();
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].len(), 1);
        assert!(decoded[1].is_empty());
    }

    #[test]
    fn decode_only_semicolons() {
        let decoded = decode(";;;").unwrap();
        assert_eq!(decoded.len(), 4);
        for line in &decoded {
            assert!(line.is_empty());
        }
    }

    // --- Malformed input tests ---

    #[test]
    fn decode_invalid_ascii_char() {
        let err = decode("AA!A").unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: b'!',
                offset: 2
            }
        );
    }

    #[test]
    fn decode_non_ascii_byte() {
        // 'À' is UTF-8 bytes [0xC3, 0x80] — both >= 128, caught by non-ASCII guard
        let err = decode("AAÀ").unwrap_err();
        assert_eq!(
            err,
            DecodeError::InvalidBase64 {
                byte: 0xC3,
                offset: 2
            }
        );
    }

    #[test]
    fn decode_truncated_vlq() {
        // 'g' has value 32, which has the continuation bit set — needs more chars
        let err = decode("g").unwrap_err();
        assert_eq!(err, DecodeError::UnexpectedEof { offset: 1 });
    }

    #[test]
    fn decode_vlq_overflow() {
        // 14 continuation characters: each 'g' = value 32 (continuation bit set)
        // After 12 digits, shift reaches 60 which exceeds the VLQ_MAX_SHIFT limit
        let err = decode("gggggggggggggg").unwrap_err();
        assert!(matches!(err, DecodeError::VlqOverflow { .. }));
    }

    #[test]
    fn decode_truncated_segment() {
        // "AC" = two VLQ values (0, 1) — starts a 4-field segment but only has 2 values
        let err = decode("AC").unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnexpectedEof { .. } | DecodeError::InvalidBase64 { .. }
        ));
    }

    // --- Encode edge cases ---

    #[test]
    fn encode_empty_segments_no_dangling_comma() {
        // Empty segments should be skipped without producing dangling commas
        let empty = Segment::from(&[] as &[i64]);
        let mappings = vec![vec![empty, Segment::four(0, 0, 0, 0), empty, Segment::four(2, 0, 0, 1)]];
        let encoded = encode(&mappings);
        assert!(
            !encoded.contains(",,"),
            "should not contain dangling commas"
        );
        // Should encode as if empty segments don't exist
        let expected = encode(&vec![vec![Segment::four(0, 0, 0, 0), Segment::four(2, 0, 0, 1)]]);
        assert_eq!(encoded, expected);
    }

    #[test]
    fn encode_all_empty_segments() {
        let empty = Segment::from(&[] as &[i64]);
        let mappings = vec![vec![empty, empty, empty]];
        let encoded = encode(&mappings);
        assert_eq!(encoded, "");
    }

    // --- Parallel encoding tests ---

    #[cfg(feature = "parallel")]
    mod parallel_tests {
        use super::*;

        fn build_large_mappings(lines: usize, segments_per_line: usize) -> SourceMapMappings {
            let mut mappings = Vec::with_capacity(lines);
            for line in 0..lines {
                let mut line_segments = Vec::with_capacity(segments_per_line);
                for seg in 0..segments_per_line {
                    line_segments.push(Segment::five(
                        (seg * 10) as i64, // generated column
                        (seg % 5) as i64,  // source index
                        line as i64,       // original line
                        (seg * 5) as i64,  // original column
                        (seg % 3) as i64,  // name index
                    ));
                }
                mappings.push(line_segments);
            }
            mappings
        }

        #[test]
        fn parallel_matches_sequential_large() {
            let mappings = build_large_mappings(2000, 10);
            let sequential = encode(&mappings);
            let parallel = encode_parallel(&mappings);
            assert_eq!(sequential, parallel);
        }

        #[test]
        fn parallel_matches_sequential_with_empty_lines() {
            let mut mappings = build_large_mappings(1500, 8);
            // Insert empty lines
            for i in (0..mappings.len()).step_by(3) {
                mappings[i] = Vec::new();
            }
            let sequential = encode(&mappings);
            let parallel = encode_parallel(&mappings);
            assert_eq!(sequential, parallel);
        }

        #[test]
        fn parallel_matches_sequential_mixed_segments() {
            let mut mappings: SourceMapMappings = Vec::with_capacity(2000);
            for line in 0..2000 {
                let mut line_segments = Vec::new();
                for seg in 0..8 {
                    if seg % 4 == 0 {
                        line_segments.push(Segment::one((seg * 10) as i64));
                    } else if seg % 4 == 3 {
                        line_segments.push(Segment::five(
                            (seg * 10) as i64,
                            (seg % 3) as i64,
                            line as i64,
                            (seg * 5) as i64,
                            (seg % 2) as i64,
                        ));
                    } else {
                        line_segments.push(Segment::four(
                            (seg * 10) as i64,
                            (seg % 3) as i64,
                            line as i64,
                            (seg * 5) as i64,
                        ));
                    }
                }
                mappings.push(line_segments);
            }
            let sequential = encode(&mappings);
            let parallel = encode_parallel(&mappings);
            assert_eq!(sequential, parallel);
        }

        #[test]
        fn parallel_roundtrip() {
            let mappings = build_large_mappings(2000, 10);
            let encoded = encode_parallel(&mappings);
            let decoded = decode(&encoded).unwrap();
            assert_eq!(decoded, mappings);
        }

        #[test]
        fn parallel_fallback_for_small_maps() {
            // Below threshold — should still produce correct output
            let mappings = build_large_mappings(10, 5);
            let sequential = encode(&mappings);
            let parallel = encode_parallel(&mappings);
            assert_eq!(sequential, parallel);
        }
    }

    // --- DecodeError Display tests ---

    #[test]
    fn decode_error_display_invalid_base64() {
        let err = DecodeError::InvalidBase64 {
            byte: b'!',
            offset: 2,
        };
        assert_eq!(err.to_string(), "invalid base64 character 0x21 at offset 2");
    }

    #[test]
    fn decode_error_display_unexpected_eof() {
        let err = DecodeError::UnexpectedEof { offset: 5 };
        assert_eq!(err.to_string(), "unexpected end of input at offset 5");
    }

    #[test]
    fn decode_error_display_overflow() {
        let err = DecodeError::VlqOverflow { offset: 10 };
        assert_eq!(err.to_string(), "VLQ value overflow at offset 10");
    }

    // --- Decode edge case: 5-field segment with name ---

    #[test]
    fn decode_five_field_with_name_index() {
        // Ensure the name field (5th) is decoded correctly
        let input = "AAAAC"; // 0,0,0,0,1
        let decoded = decode(input).unwrap();
        assert_eq!(decoded[0][0], vec![0, 0, 0, 0, 1]);
    }

    // --- Encode edge case: encode with only 1 line ---

    #[test]
    fn encode_single_segment_one_field() {
        let mappings = vec![vec![Segment::one(5)]];
        let encoded = encode(&mappings);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, mappings);
    }
}
