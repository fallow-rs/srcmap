use crate::SourceMapMappings;
use crate::vlq::vlq_encode;

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

    // SAFETY: vlq_encode only pushes bytes from BASE64_ENCODE (all ASCII),
    // and we only add b';' and b',' — all valid UTF-8.
    debug_assert!(buf.is_ascii());
    unsafe { String::from_utf8_unchecked(buf) }
}

/// Encode a single line's segments to VLQ bytes.
///
/// Generated column resets per line (starts at 0).
/// Cumulative state (source, original line/column, name) is passed in.
#[cfg(feature = "parallel")]
fn encode_line_to_bytes(
    segments: &[crate::Segment],
    init_source: i64,
    init_original_line: i64,
    init_original_column: i64,
    init_name: i64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(segments.len() * 6);
    let mut prev_generated_column: i64 = 0;
    let mut prev_source = init_source;
    let mut prev_original_line = init_original_line;
    let mut prev_original_column = init_original_column;
    let mut prev_name = init_name;
    let mut wrote_segment = false;

    for segment in segments {
        if segment.is_empty() {
            continue;
        }

        if wrote_segment {
            buf.push(b',');
        }
        wrote_segment = true;

        vlq_encode(&mut buf, segment[0] - prev_generated_column);
        prev_generated_column = segment[0];

        if segment.len() >= 4 {
            vlq_encode(&mut buf, segment[1] - prev_source);
            prev_source = segment[1];

            vlq_encode(&mut buf, segment[2] - prev_original_line);
            prev_original_line = segment[2];

            vlq_encode(&mut buf, segment[3] - prev_original_column);
            prev_original_column = segment[3];

            if segment.len() >= 5 {
                vlq_encode(&mut buf, segment[4] - prev_name);
                prev_name = segment[4];
            }
        }
    }

    buf
}

/// Encode source map mappings using parallel encoding with rayon.
///
/// Uses the same delta-encoding as [`encode`], but distributes line encoding
/// across threads. Falls back to sequential [`encode`] for small maps.
///
/// Two-phase approach:
/// 1. **Sequential scan** — compute cumulative state at each line boundary
/// 2. **Parallel encode** — encode each line independently via rayon
#[cfg(feature = "parallel")]
pub fn encode_parallel(mappings: &SourceMapMappings) -> String {
    use rayon::prelude::*;

    if mappings.is_empty() {
        return String::new();
    }

    let total_segments: usize = mappings.iter().map(|l| l.len()).sum();
    if mappings.len() < 1024 || total_segments < 4096 {
        return encode(mappings);
    }

    // Pass 1 (sequential): compute cumulative state at each line boundary
    let mut states: Vec<(i64, i64, i64, i64)> = Vec::with_capacity(mappings.len());
    let mut prev_source: i64 = 0;
    let mut prev_original_line: i64 = 0;
    let mut prev_original_column: i64 = 0;
    let mut prev_name: i64 = 0;

    for line in mappings.iter() {
        states.push((
            prev_source,
            prev_original_line,
            prev_original_column,
            prev_name,
        ));
        for segment in line.iter() {
            if segment.len() >= 4 {
                prev_source = segment[1];
                prev_original_line = segment[2];
                prev_original_column = segment[3];
                if segment.len() >= 5 {
                    prev_name = segment[4];
                }
            }
        }
    }

    // Pass 2 (parallel): encode each line independently
    let encoded_lines: Vec<Vec<u8>> = mappings
        .par_iter()
        .zip(states.par_iter())
        .map(|(line, &(src, ol, oc, name))| encode_line_to_bytes(line, src, ol, oc, name))
        .collect();

    // Join with semicolons
    let total_len: usize =
        encoded_lines.iter().map(|l| l.len()).sum::<usize>() + encoded_lines.len() - 1;
    let mut buf: Vec<u8> = Vec::with_capacity(total_len);
    for (i, line_bytes) in encoded_lines.iter().enumerate() {
        if i > 0 {
            buf.push(b';');
        }
        buf.extend_from_slice(line_bytes);
    }

    // SAFETY: vlq_encode only pushes bytes from BASE64_ENCODE (all ASCII),
    // and we only add b';' — all valid UTF-8.
    debug_assert!(buf.is_ascii());
    unsafe { String::from_utf8_unchecked(buf) }
}
