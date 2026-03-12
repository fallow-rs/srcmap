//! Roundtrip example: decode, inspect, manipulate, and re-encode source map mappings.
//!
//! Simulates what a build tool does when it needs to inspect or transform
//! individual mappings — for example, prepending whitespace to every generated
//! line and adjusting the source map to match.
//!
//! Run with: cargo run -p srcmap-codec --example roundtrip

use srcmap_codec::{DecodeError, Segment, decode, encode};

fn main() {
    // -----------------------------------------------------------------------
    // 1. Decode a realistic mappings string
    // -----------------------------------------------------------------------
    //
    // This mappings string represents 5 generated lines from a small bundled
    // module. It uses a mix of segment types:
    //
    //   - 1-field segments: only a generated column (no source mapping)
    //   - 4-field segments: generated column + source/line/column
    //   - 5-field segments: same as 4-field + a name index
    //
    // ECMA-426 VLQ mappings are delta-encoded on the wire:
    //   - Generated column resets to 0 at each new line (`;`)
    //   - Source index, original line, original column, and name index
    //     are cumulative across the entire string
    //
    // After decoding, segments contain ABSOLUTE values — the codec handles
    // all delta accumulation internally.
    //
    // Line 0: two 4-field segments — "AAAA,IAAA"
    //          (col 0 -> src 0, line 0, col 0) and (col 4 -> src 0, line 0, col 0)
    // Line 1: one 5-field segment with a name — "EACEC"
    //          (col 2 -> src 0, line 1, col 2, name 1)
    // Line 2: one 4-field segment — "IAEE"
    //          (col 4 -> src 0, line 3, col 4)
    // Line 3: empty line (no mappings)
    // Line 4: one 1-field segment (col 0, no source info) — "A"

    let mappings_str = "AAAA,IAAA;EACEC;IAEE;;A";

    println!("=== Source Map Codec Roundtrip ===\n");
    println!("Input mappings: {mappings_str:?}\n");

    let mappings = decode(mappings_str).expect("valid mappings string");

    // -----------------------------------------------------------------------
    // 2. Inspect the decoded structure
    // -----------------------------------------------------------------------

    println!("Decoded {} lines:\n", mappings.len());

    for (line_idx, line) in mappings.iter().enumerate() {
        if line.is_empty() {
            println!("  Line {line_idx}: (empty)");
            continue;
        }

        println!("  Line {line_idx}: {} segment(s)", line.len());

        for (seg_idx, segment) in line.iter().enumerate() {
            // Segment implements Deref<Target=[i64]>, so len() and indexing work
            match segment.len() {
                1 => {
                    println!("    [{seg_idx}] 1-field: generated_col={}", segment[0],);
                }
                4 => {
                    println!(
                        "    [{seg_idx}] 4-field: generated_col={}, source={}, orig_line={}, orig_col={}",
                        segment[0], segment[1], segment[2], segment[3],
                    );
                }
                5 => {
                    println!(
                        "    [{seg_idx}] 5-field: generated_col={}, source={}, orig_line={}, orig_col={}, name={}",
                        segment[0], segment[1], segment[2], segment[3], segment[4],
                    );
                }
                other => {
                    println!("    [{seg_idx}] unexpected {other}-field segment");
                }
            }
        }
    }

    // -----------------------------------------------------------------------
    // 3. Manipulate: shift all generated columns by +4
    // -----------------------------------------------------------------------
    //
    // This simulates a build tool that prepends "    " (4 spaces) to every
    // generated line. Each segment's generated column (field 0) increases by 4.
    // Source-side fields are unchanged — they still point to the original code.

    println!("\n--- Shifting all generated columns by +4 ---\n");

    let shifted: Vec<Vec<Segment>> = mappings
        .iter()
        .map(|line| {
            line.iter()
                .map(|seg| match seg.len() {
                    1 => Segment::one(seg[0] + 4),
                    4 => Segment::four(seg[0] + 4, seg[1], seg[2], seg[3]),
                    5 => Segment::five(seg[0] + 4, seg[1], seg[2], seg[3], seg[4]),
                    _ => *seg,
                })
                .collect()
        })
        .collect();

    // -----------------------------------------------------------------------
    // 4. Re-encode and verify roundtrip
    // -----------------------------------------------------------------------

    let encoded_original = encode(&mappings);
    let encoded_shifted = encode(&shifted);

    println!("Original re-encoded: {encoded_original:?}");
    println!("Shifted  re-encoded: {encoded_shifted:?}\n");

    // Verify the original roundtrips exactly
    assert_eq!(
        encoded_original, mappings_str,
        "roundtrip must produce identical output"
    );
    println!("Roundtrip verified: original encodes back to the same string.");

    // Verify the shifted version decodes correctly
    let re_decoded = decode(&encoded_shifted).expect("shifted mappings should be valid");

    for (line_idx, (orig_line, shifted_line)) in mappings.iter().zip(re_decoded.iter()).enumerate()
    {
        assert_eq!(
            orig_line.len(),
            shifted_line.len(),
            "line {line_idx} segment count must match"
        );
        for (seg_idx, (orig_seg, shifted_seg)) in
            orig_line.iter().zip(shifted_line.iter()).enumerate()
        {
            assert_eq!(
                shifted_seg[0],
                orig_seg[0] + 4,
                "line {line_idx} segment {seg_idx}: generated column must be shifted by 4"
            );
        }
    }
    println!("Shift verified: all generated columns increased by 4.\n");

    // -----------------------------------------------------------------------
    // 5. Error handling for invalid input
    // -----------------------------------------------------------------------

    println!("--- Error handling ---\n");

    // Invalid base64 character
    match decode("AA!A") {
        Err(DecodeError::InvalidBase64 { byte, offset }) => {
            println!(
                "InvalidBase64: byte 0x{byte:02x} ({:?}) at offset {offset}",
                char::from(byte),
            );
        }
        other => panic!("expected InvalidBase64, got {other:?}"),
    }

    // Truncated VLQ (continuation bit set, but no more input)
    match decode("g") {
        Err(DecodeError::UnexpectedEof { offset }) => {
            println!("UnexpectedEof: at offset {offset}");
        }
        other => panic!("expected UnexpectedEof, got {other:?}"),
    }

    // Invalid segment length (2-field segments are not allowed per ECMA-426)
    match decode("AC") {
        Err(DecodeError::InvalidSegmentLength { fields, offset }) => {
            println!("InvalidSegmentLength: {fields} fields at offset {offset}");
        }
        other => panic!("expected InvalidSegmentLength, got {other:?}"),
    }

    // VLQ overflow (too many continuation bytes)
    match decode("gggggggggggggg") {
        Err(DecodeError::VlqOverflow { offset }) => {
            println!("VlqOverflow: at offset {offset}");
        }
        other => panic!("expected VlqOverflow, got {other:?}"),
    }

    println!("\nAll assertions passed.");
}
