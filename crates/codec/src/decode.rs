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

    // Pre-count lines for capacity hint
    let line_count = bytes.iter().filter(|&&b| b == b';').count() + 1;
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
        let mut line: Line = Vec::new();
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

            // Decode a segment
            let mut segment: Segment = Vec::with_capacity(5);

            // Field 1: generated column (always present)
            let (delta, consumed) = vlq_decode(bytes, pos)?;
            generated_column += delta;
            segment.push(generated_column);
            pos += consumed;

            // Check if there are more fields in this segment
            if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                // Field 2: source index
                let (delta, consumed) = vlq_decode(bytes, pos)?;
                source_index += delta;
                segment.push(source_index);
                pos += consumed;

                // Field 3: original line
                let (delta, consumed) = vlq_decode(bytes, pos)?;
                original_line += delta;
                segment.push(original_line);
                pos += consumed;

                // Field 4: original column
                let (delta, consumed) = vlq_decode(bytes, pos)?;
                original_column += delta;
                segment.push(original_column);
                pos += consumed;

                // Field 5: name index (optional)
                if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                    let (delta, consumed) = vlq_decode(bytes, pos)?;
                    name_index += delta;
                    segment.push(name_index);
                    pos += consumed;
                }
            }

            debug_assert!(
                segment.len() == 1 || segment.len() == 4 || segment.len() == 5,
                "invalid segment length {}",
                segment.len()
            );

            line.push(segment);
        }

        mappings.push(line);

        if !saw_semicolon {
            break;
        }
    }

    Ok(mappings)
}
