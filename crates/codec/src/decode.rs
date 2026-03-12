use crate::vlq::vlq_decode;
use crate::{DecodeError, Line, Segment, SourceMapMappings};

/// Decode a VLQ-encoded source map mappings string into structured data.
///
/// The mappings string uses `;` to separate lines and `,` to separate
/// segments within a line. Each segment is a sequence of VLQ-encoded
/// values representing column offsets, source indices, and name indices.
///
/// Values are relative (delta-encoded) within the mappings string:
/// - Generated column resets to 0 for each new line
/// - Source index, original line, original column, and name index
///   are cumulative across the entire mappings string
///
/// # Errors
///
/// Returns [`DecodeError`] if the input contains invalid base64 characters,
/// truncated VLQ sequences, or values that overflow i64.
pub fn decode(input: &str) -> Result<SourceMapMappings, DecodeError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    // Pre-count lines and segments in a single pass for capacity hints
    let mut semicolons = 0usize;
    let mut commas = 0usize;
    for &b in bytes {
        semicolons += (b == b';') as usize;
        commas += (b == b',') as usize;
    }
    let line_count = semicolons + 1;
    let approx_segments = commas + line_count;
    let avg_segments_per_line = approx_segments / line_count;
    let mut mappings: SourceMapMappings = Vec::with_capacity(line_count);

    // Cumulative state across the entire mappings string
    let mut source_index: i64 = 0;
    let mut original_line: i64 = 0;
    let mut original_column: i64 = 0;
    let mut name_index: i64 = 0;

    let mut pos: usize = 0;

    loop {
        // Generated column resets per line
        let mut generated_column: i64 = 0;
        let mut line: Line = Vec::with_capacity(avg_segments_per_line);
        let mut saw_semicolon = false;

        while pos < len {
            let byte = bytes[pos];

            if byte == b';' {
                pos += 1;
                saw_semicolon = true;
                break;
            }

            if byte == b',' {
                pos += 1;
                continue;
            }

            // Field 1: generated column (always present)
            let (delta, consumed) = vlq_decode(bytes, pos)?;
            generated_column += delta;
            pos += consumed;

            // Build segment with exact allocation (1, 4, or 5 fields)
            let segment: Segment = if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                // Fields 2-4: source, original line, original column
                let (delta, consumed) = vlq_decode(bytes, pos)?;
                source_index += delta;
                pos += consumed;

                let (delta, consumed) = vlq_decode(bytes, pos)?;
                original_line += delta;
                pos += consumed;

                let (delta, consumed) = vlq_decode(bytes, pos)?;
                original_column += delta;
                pos += consumed;

                // Field 5: name index (optional)
                if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                    let (delta, consumed) = vlq_decode(bytes, pos)?;
                    name_index += delta;
                    pos += consumed;
                    Segment::five(
                        generated_column,
                        source_index,
                        original_line,
                        original_column,
                        name_index,
                    )
                } else {
                    Segment::four(
                        generated_column,
                        source_index,
                        original_line,
                        original_column,
                    )
                }
            } else {
                Segment::one(generated_column)
            };

            line.push(segment);
        }

        mappings.push(line);

        if !saw_semicolon {
            break;
        }
    }

    Ok(mappings)
}
