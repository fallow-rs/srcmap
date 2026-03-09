use crate::vlq::vlq_encode;
use crate::SourceMapMappings;

/// Encode decoded source map mappings back into a VLQ-encoded string.
///
/// This is the inverse of [`decode`](crate::decode). Values are delta-encoded:
/// generated column resets per line, all other fields are cumulative.
///
/// Empty segments are silently skipped.
pub fn encode(mappings: &SourceMapMappings) -> String {
    if mappings.is_empty() {
        return String::new();
    }

    // Estimate capacity: ~4 bytes per segment on average
    let segment_count: usize = mappings.iter().map(|line| line.len()).sum();
    let mut buf: Vec<u8> = Vec::with_capacity(segment_count * 4 + mappings.len());

    // Cumulative state
    let mut prev_source: i64 = 0;
    let mut prev_original_line: i64 = 0;
    let mut prev_original_column: i64 = 0;
    let mut prev_name: i64 = 0;

    for (line_idx, line) in mappings.iter().enumerate() {
        if line_idx > 0 {
            buf.push(b';');
        }

        // Generated column resets per line
        let mut prev_generated_column: i64 = 0;
        let mut wrote_segment = false;

        for segment in line.iter() {
            if segment.is_empty() {
                continue;
            }

            if wrote_segment {
                buf.push(b',');
            }
            wrote_segment = true;

            // Field 1: generated column (delta from previous in this line)
            vlq_encode(&mut buf, segment[0] - prev_generated_column);
            prev_generated_column = segment[0];

            if segment.len() >= 4 {
                // Field 2: source index (cumulative delta)
                vlq_encode(&mut buf, segment[1] - prev_source);
                prev_source = segment[1];

                // Field 3: original line (cumulative delta)
                vlq_encode(&mut buf, segment[2] - prev_original_line);
                prev_original_line = segment[2];

                // Field 4: original column (cumulative delta)
                vlq_encode(&mut buf, segment[3] - prev_original_column);
                prev_original_column = segment[3];

                if segment.len() >= 5 {
                    // Field 5: name index (cumulative delta)
                    vlq_encode(&mut buf, segment[4] - prev_name);
                    prev_name = segment[4];
                }
            }
        }
    }

    // SAFETY: buf contains only ASCII base64 chars, semicolons, and commas
    unsafe { String::from_utf8_unchecked(buf) }
}
