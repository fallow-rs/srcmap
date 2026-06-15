use crate::vlq::vlq_decode;
use crate::{DecodeError, Line, Segment, SourceMapMappings};

#[derive(Default)]
struct DecodeState {
    source_index: i64,
    original_line: i64,
    original_column: i64,
    name_index: i64,
}

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

    // Pre-count lines and segments for capacity hints. memchr's count is
    // SIMD-accelerated; the equivalent scalar byte loop is not auto-vectorized
    // and measured ~7x slower.
    let (line_count, avg_segments_per_line) = decode_capacity_hints(bytes);
    let mut mappings: SourceMapMappings = Vec::with_capacity(line_count);

    let mut state = DecodeState::default();
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
                decode_sourced_segment(bytes, &mut pos, generated_column, &mut state)?
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

fn decode_capacity_hints(bytes: &[u8]) -> (usize, usize) {
    let semicolons = memchr::memchr_iter(b';', bytes).count();
    let commas = memchr::memchr_iter(b',', bytes).count();
    let line_count = semicolons + 1;
    let approx_segments = commas + line_count;
    (line_count, approx_segments / line_count)
}

fn decode_sourced_segment(
    bytes: &[u8],
    pos: &mut usize,
    generated_column: i64,
    state: &mut DecodeState,
) -> Result<Segment, DecodeError> {
    let (delta, consumed) = vlq_decode(bytes, *pos)?;
    state.source_index += delta;
    *pos += consumed;

    if *pos >= bytes.len() || bytes[*pos] == b',' || bytes[*pos] == b';' {
        return Err(DecodeError::InvalidSegmentLength { fields: 2, offset: *pos });
    }

    let (delta, consumed) = vlq_decode(bytes, *pos)?;
    state.original_line += delta;
    *pos += consumed;

    if *pos >= bytes.len() || bytes[*pos] == b',' || bytes[*pos] == b';' {
        return Err(DecodeError::InvalidSegmentLength { fields: 3, offset: *pos });
    }

    let (delta, consumed) = vlq_decode(bytes, *pos)?;
    state.original_column += delta;
    *pos += consumed;

    if *pos < bytes.len() && bytes[*pos] != b',' && bytes[*pos] != b';' {
        let (delta, consumed) = vlq_decode(bytes, *pos)?;
        state.name_index += delta;
        *pos += consumed;
        return Ok(Segment::five(
            generated_column,
            state.source_index,
            state.original_line,
            state.original_column,
            state.name_index,
        ));
    }

    Ok(Segment::four(
        generated_column,
        state.source_index,
        state.original_line,
        state.original_column,
    ))
}
