//! High-performance VLQ source map codec.
//!
//! Encodes and decodes source map mappings using the Base64 VLQ format
//! as specified in the Source Map v3 specification (ECMA-426).
//!
//! # Features
//!
//! - **`parallel`** — enables [`encode_parallel`] for multi-threaded encoding via rayon.
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
pub use vlq::{vlq_decode, vlq_encode};

use std::fmt;

/// A single source map segment.
///
/// Segments have 1, 4, or 5 fields:
/// - 1 field:  `[generated_column]`
/// - 4 fields: `[generated_column, source_index, original_line, original_column]`
/// - 5 fields: `[generated_column, source_index, original_line, original_column, name_index]`
pub type Segment = Vec<i64>;

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
        let mappings = vec![vec![vec![1000_i64, 50, 999, 500, 100]]];
        let encoded = encode(&mappings);
        let decoded = decode(&encoded).unwrap();
        assert_eq!(decoded, mappings);
    }

    #[test]
    fn roundtrip_negative_deltas() {
        let mappings = vec![vec![vec![10_i64, 0, 10, 10], vec![20, 0, 5, 5]]];
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
        // After 13 digits, shift reaches 65 which exceeds i64 range
        let err = decode("gggggggggggggg").unwrap_err();
        matches!(err, DecodeError::VlqOverflow { .. });
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
        let mappings = vec![vec![vec![], vec![0, 0, 0, 0], vec![], vec![2, 0, 0, 1]]];
        let encoded = encode(&mappings);
        assert!(
            !encoded.contains(",,"),
            "should not contain dangling commas"
        );
        // Should encode as if empty segments don't exist
        let expected = encode(&vec![vec![vec![0, 0, 0, 0], vec![2, 0, 0, 1]]]);
        assert_eq!(encoded, expected);
    }

    #[test]
    fn encode_all_empty_segments() {
        let mappings = vec![vec![vec![], vec![], vec![]]];
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
                    line_segments.push(vec![
                        (seg * 10) as i64,        // generated column
                        (seg % 5) as i64,          // source index
                        line as i64,               // original line
                        (seg * 5) as i64,          // original column
                        (seg % 3) as i64,          // name index
                    ]);
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
                        // 1-field segment (generated-only)
                        line_segments.push(vec![(seg * 10) as i64]);
                    } else if seg % 4 == 3 {
                        // 5-field segment (with name)
                        line_segments.push(vec![
                            (seg * 10) as i64,
                            (seg % 3) as i64,
                            line as i64,
                            (seg * 5) as i64,
                            (seg % 2) as i64,
                        ]);
                    } else {
                        // 4-field segment
                        line_segments.push(vec![
                            (seg * 10) as i64,
                            (seg % 3) as i64,
                            line as i64,
                            (seg * 5) as i64,
                        ]);
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
}
