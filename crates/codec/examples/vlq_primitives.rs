//! Low-level VLQ encode/decode primitives.
//!
//! Demonstrates the signed and unsigned VLQ functions that underpin the
//! ECMA-426 source map format:
//!
//!   - **Signed VLQ** is used for source map mappings. The sign bit occupies
//!     the LSB of the first character, leaving 4 data bits in the first char
//!     and 5 data bits in each continuation char.
//!
//!   - **Unsigned VLQ** is used by the ECMA-426 scopes proposal for tags,
//!     flags, and unsigned position values. All 5 bits per character are data
//!     (no sign bit), so it is more compact for non-negative values.
//!
//! Run with: cargo run -p srcmap-codec --example vlq_primitives

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_codec::{vlq_decode, vlq_decode_unsigned, vlq_encode, vlq_encode_unsigned};

fn main() {
    println!("=== VLQ Primitives ===\n");

    // -----------------------------------------------------------------------
    // 1. Signed VLQ encode/decode
    // -----------------------------------------------------------------------
    //
    // Signed VLQ encoding (ECMA-426 / Source Map v3):
    //   - The LSB of the first base64 digit carries the sign (0 = positive, 1 = negative)
    //   - Remaining bits carry magnitude
    //   - Bit 5 (0x20) of each digit is the continuation flag
    //
    // Single-character range: -15..=15 (4 data bits + 1 sign bit = 5 bits in one char)
    //
    // Base64 alphabet: A=0, B=1, C=2, ..., Z=25, a=26, ..., z=51, 0=52, ..., 9=61, +=62, /=63

    println!("--- Signed VLQ ---\n");

    let signed_values: &[i64] = &[0, 1, -1, 15, -15, 16, -16, 100, -100, 1000, -1_000_000];

    for &value in signed_values {
        let mut buf = Vec::new();
        vlq_encode(&mut buf, value);

        let base64_str = std::str::from_utf8(&buf).expect("VLQ output is always valid ASCII");
        let (decoded, bytes_consumed) = vlq_decode(&buf, 0).expect("roundtrip must succeed");

        println!(
            "  {value:>12} -> {base64_str:<8} ({} byte{}, raw: {:?})",
            bytes_consumed,
            if bytes_consumed == 1 { "" } else { "s" },
            buf,
        );

        assert_eq!(decoded, value, "signed roundtrip failed for {value}");
        assert_eq!(bytes_consumed, buf.len(), "must consume all bytes for {value}");
    }

    // -----------------------------------------------------------------------
    // 2. Show the byte-level structure of a multi-char signed VLQ
    // -----------------------------------------------------------------------

    println!("\n--- Byte-level breakdown: signed VLQ of 1000 ---\n");

    // 1000 in signed VLQ:
    //   abs = 1000, VLQ representation = 1000 << 1 | 0 = 2000 (positive, sign bit = 0)
    //   2000 in binary: 11111010000
    //   Split into 5-bit groups (LSB first): 10000, 11101, 01 -> with continuation: 110000, 111101, 01
    //   Base64 digits: 48 = 'w', 61 = '9', 1 = 'B' -> but let's just show what the encoder produces

    let mut buf = Vec::new();
    vlq_encode(&mut buf, 1000);

    println!("  Value: 1000");
    println!("  Encoded bytes: {buf:?}");
    println!("  Base64 string: {}", std::str::from_utf8(&buf).unwrap());
    println!();

    for (i, &byte) in buf.iter().enumerate() {
        let digit = byte; // the base64 character
        let is_last = i == buf.len() - 1;

        // Decode the base64 character back to its 6-bit value for display
        let six_bit = match digit {
            b'A'..=b'Z' => digit - b'A',
            b'a'..=b'z' => digit - b'a' + 26,
            b'0'..=b'9' => digit - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => unreachable!(),
        };
        let continuation = (six_bit & 0x20) != 0;
        let data_bits = six_bit & 0x1F;

        println!(
            "  byte[{i}]: '{}' (base64 value {six_bit:>2}, 0b{six_bit:06b}) -> continuation={}, data=0b{data_bits:05b}{}",
            char::from(digit),
            continuation,
            if i == 0 { format!(" (sign bit={})", data_bits & 1) } else { String::new() },
        );

        if is_last {
            assert!(!continuation, "last byte must not have continuation bit");
        }
    }

    // -----------------------------------------------------------------------
    // 3. Unsigned VLQ encode/decode
    // -----------------------------------------------------------------------
    //
    // Unsigned VLQ (ECMA-426 scopes proposal):
    //   - No sign bit — all 5 data bits per character carry magnitude
    //   - Single-character range: 0..=31 (vs 0..=15 for signed)
    //   - Used for scope tags, binding flags, and unsigned indices
    //   - More compact than signed VLQ for the same non-negative value

    println!("\n--- Unsigned VLQ ---\n");

    let unsigned_values: &[u64] = &[0, 1, 8, 31, 32, 100, 1000, 100_000, 1_000_000];

    for &value in unsigned_values {
        let mut buf = Vec::new();
        vlq_encode_unsigned(&mut buf, value);

        let base64_str = std::str::from_utf8(&buf).expect("VLQ output is always valid ASCII");
        let (decoded, bytes_consumed) =
            vlq_decode_unsigned(&buf, 0).expect("unsigned roundtrip must succeed");

        println!(
            "  {value:>12} -> {base64_str:<8} ({} byte{}, raw: {:?})",
            bytes_consumed,
            if bytes_consumed == 1 { "" } else { "s" },
            buf,
        );

        assert_eq!(decoded, value, "unsigned roundtrip failed for {value}");
        assert_eq!(bytes_consumed, buf.len(), "must consume all bytes for {value}");
    }

    // -----------------------------------------------------------------------
    // 4. Compare signed vs unsigned encoding size
    // -----------------------------------------------------------------------
    //
    // For non-negative values, unsigned VLQ is always equal to or more compact
    // than signed VLQ because it does not spend a bit on the sign.

    println!("\n--- Signed vs Unsigned size comparison ---\n");
    println!("  {:>10}  {:>8}  {:>10}", "Value", "Signed", "Unsigned");
    println!("  {:>10}  {:>8}  {:>10}", "-----", "------", "--------");

    let comparison_values: &[i64] = &[0, 15, 16, 31, 32, 100, 1000, 100_000];

    for &value in comparison_values {
        let mut signed_buf = Vec::new();
        vlq_encode(&mut signed_buf, value);

        let mut unsigned_buf = Vec::new();
        vlq_encode_unsigned(&mut unsigned_buf, value as u64);

        let signed_str = std::str::from_utf8(&signed_buf).unwrap();
        let unsigned_str = std::str::from_utf8(&unsigned_buf).unwrap();

        println!("  {value:>10}  {signed_str:>8}  {unsigned_str:>10}",);

        // Unsigned is always <= signed in byte count for non-negative values
        assert!(
            unsigned_buf.len() <= signed_buf.len(),
            "unsigned must be at most as large as signed for value {value}"
        );
    }

    // -----------------------------------------------------------------------
    // 5. Decoding from a specific offset (useful for parsing streams)
    // -----------------------------------------------------------------------
    //
    // Both vlq_decode and vlq_decode_unsigned accept an offset parameter,
    // allowing you to decode consecutive values from a single byte buffer
    // without slicing.

    println!("\n--- Sequential decoding from a buffer ---\n");

    let values_to_encode: &[i64] = &[42, -7, 0, 256];
    let mut stream = Vec::new();

    for &v in values_to_encode {
        vlq_encode(&mut stream, v);
    }

    println!(
        "  Encoded {} values into {} bytes: {:?}",
        values_to_encode.len(),
        stream.len(),
        std::str::from_utf8(&stream).unwrap(),
    );

    let mut offset = 0;
    let mut decoded_values = Vec::new();

    while offset < stream.len() {
        let (value, consumed) = vlq_decode(&stream, offset).expect("valid stream");
        println!("  offset {offset:>2}: decoded {value:>4} ({consumed} byte(s))");
        decoded_values.push(value);
        offset += consumed;
    }

    assert_eq!(
        decoded_values.as_slice(),
        values_to_encode,
        "sequential decode must recover all values"
    );

    println!("\nAll assertions passed.");
}
