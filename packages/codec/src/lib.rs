use napi::bindgen_prelude::Buffer;
use napi_derive::napi;
use srcmap_codec::SourceMapMappings;

// ── Approach 1: NAPI nested arrays (current baseline) ──────────────

#[napi]
pub fn decode(mappings: String) -> napi::Result<Vec<Vec<Vec<i64>>>> {
    srcmap_codec::decode(&mappings).map_err(|e| napi::Error::from_reason(e.to_string()))
}

#[napi]
pub fn encode(mappings: Vec<Vec<Vec<i64>>>) -> String {
    srcmap_codec::encode(&mappings)
}

// ── Approach 2: JSON string (V8's JSON.parse is very fast) ─────────

#[napi]
pub fn decode_json(mappings: String) -> napi::Result<String> {
    let decoded =
        srcmap_codec::decode(&mappings).map_err(|e| napi::Error::from_reason(e.to_string()))?;
    Ok(to_json(&decoded))
}

#[napi]
pub fn encode_json(json: String) -> napi::Result<String> {
    let mappings = from_json(json.as_bytes())?;
    Ok(srcmap_codec::encode(&mappings))
}

// ── Approach 3: Packed i32 buffer ──────────────────────────────────
// Format: [n_lines, seg_count_line0, seg_count_line1, ...,
//          n_fields_seg0, val0, val1, ..., n_fields_seg1, val0, ...]

#[napi]
pub fn decode_buf(mappings: String) -> napi::Result<Buffer> {
    let decoded =
        srcmap_codec::decode(&mappings).map_err(|e| napi::Error::from_reason(e.to_string()))?;
    Ok(to_packed_buffer(&decoded).into())
}

#[napi]
pub fn encode_buf(buf: Buffer) -> String {
    let mappings = from_packed_buffer(&buf);
    srcmap_codec::encode(&mappings)
}

// ── JSON helpers ───────────────────────────────────────────────────

fn to_json(mappings: &SourceMapMappings) -> String {
    // Estimate: ~6 chars per field value on average
    let total_fields: usize = mappings
        .iter()
        .flat_map(|l| l.iter())
        .map(|s| s.len())
        .sum();
    let mut buf: Vec<u8> = Vec::with_capacity(total_fields * 6 + mappings.len() * 4);

    buf.push(b'[');
    for (li, line) in mappings.iter().enumerate() {
        if li > 0 {
            buf.push(b',');
        }
        buf.push(b'[');
        for (si, seg) in line.iter().enumerate() {
            if si > 0 {
                buf.push(b',');
            }
            buf.push(b'[');
            for (fi, &field) in seg.iter().enumerate() {
                if fi > 0 {
                    buf.push(b',');
                }
                write_i64(&mut buf, field);
            }
            buf.push(b']');
        }
        buf.push(b']');
    }
    buf.push(b']');

    // SAFETY: only ASCII digits, brackets, commas, minus signs
    unsafe { String::from_utf8_unchecked(buf) }
}

fn write_i64(buf: &mut Vec<u8>, value: i64) {
    if value == 0 {
        buf.push(b'0');
        return;
    }

    let negative = value < 0;
    let mut v: u64 = if negative {
        (!(value as u64)).wrapping_add(1)
    } else {
        value as u64
    };

    let start = buf.len();
    while v > 0 {
        buf.push(b'0' + (v % 10) as u8);
        v /= 10;
    }
    if negative {
        buf.push(b'-');
    }
    buf[start..].reverse();
}

fn from_json(bytes: &[u8]) -> napi::Result<SourceMapMappings> {
    let mut pos = 0;
    let len = bytes.len();

    let skip_ws = |pos: &mut usize| {
        while *pos < len && matches!(bytes[*pos], b' ' | b'\n' | b'\r' | b'\t') {
            *pos += 1;
        }
    };

    let expect = |pos: &mut usize, ch: u8| -> napi::Result<()> {
        skip_ws(pos);
        if *pos >= len || bytes[*pos] != ch {
            return Err(napi::Error::from_reason(format!(
                "expected '{}' at position {}",
                ch as char, *pos
            )));
        }
        *pos += 1;
        Ok(())
    };

    let parse_i64 = |pos: &mut usize| -> napi::Result<i64> {
        skip_ws(pos);
        let negative = *pos < len && bytes[*pos] == b'-';
        if negative {
            *pos += 1;
        }
        let start = *pos;
        let mut val: i64 = 0;
        while *pos < len && bytes[*pos].is_ascii_digit() {
            val = val * 10 + (bytes[*pos] - b'0') as i64;
            *pos += 1;
        }
        if *pos == start {
            return Err(napi::Error::from_reason(format!(
                "expected number at position {start}"
            )));
        }
        Ok(if negative { -val } else { val })
    };

    let mut mappings = Vec::new();
    expect(&mut pos, b'[')?;

    loop {
        skip_ws(&mut pos);
        if pos >= len {
            break;
        }
        if bytes[pos] == b']' {
            break;
        }
        if bytes[pos] == b',' {
            pos += 1;
            continue;
        }

        expect(&mut pos, b'[')?;
        let mut line = Vec::new();

        loop {
            skip_ws(&mut pos);
            if pos >= len {
                break;
            }
            if bytes[pos] == b']' {
                pos += 1;
                break;
            }
            if bytes[pos] == b',' {
                pos += 1;
                continue;
            }

            expect(&mut pos, b'[')?;
            let mut seg = Vec::new();

            loop {
                skip_ws(&mut pos);
                if pos >= len {
                    break;
                }
                if bytes[pos] == b']' {
                    pos += 1;
                    break;
                }
                if bytes[pos] == b',' {
                    pos += 1;
                    continue;
                }
                seg.push(parse_i64(&mut pos)?);
            }

            line.push(seg);
        }

        mappings.push(line);
    }

    Ok(mappings)
}

// ── Buffer helpers ─────────────────────────────────────────────────

fn to_packed_buffer(mappings: &SourceMapMappings) -> Vec<u8> {
    let n_lines = mappings.len();
    let total_segments: usize = mappings.iter().map(|l| l.len()).sum();
    let total_fields: usize = mappings
        .iter()
        .flat_map(|l| l.iter())
        .map(|s| s.len())
        .sum();

    // 4 bytes per: n_lines header + segment counts + field counts + values
    let capacity = (1 + n_lines + total_segments + total_fields) * 4;
    let mut buf = Vec::with_capacity(capacity);

    buf.extend_from_slice(&(n_lines as i32).to_le_bytes());

    for line in mappings {
        buf.extend_from_slice(&(line.len() as i32).to_le_bytes());
    }

    for line in mappings {
        for seg in line {
            buf.extend_from_slice(&(seg.len() as i32).to_le_bytes());
            for &field in seg {
                buf.extend_from_slice(&(field as i32).to_le_bytes());
            }
        }
    }

    buf
}

fn from_packed_buffer(buf: &[u8]) -> SourceMapMappings {
    let mut pos = 0;
    let n_lines = read_i32(buf, &mut pos) as usize;

    let mut line_seg_counts = Vec::with_capacity(n_lines);
    for _ in 0..n_lines {
        line_seg_counts.push(read_i32(buf, &mut pos) as usize);
    }

    let mut mappings = Vec::with_capacity(n_lines);
    for &seg_count in &line_seg_counts {
        let mut line = Vec::with_capacity(seg_count);
        for _ in 0..seg_count {
            let n_fields = read_i32(buf, &mut pos) as usize;
            let mut seg = Vec::with_capacity(n_fields);
            for _ in 0..n_fields {
                seg.push(read_i32(buf, &mut pos) as i64);
            }
            line.push(seg);
        }
        mappings.push(line);
    }

    mappings
}

#[inline]
fn read_i32(buf: &[u8], pos: &mut usize) -> i32 {
    let val = i32::from_le_bytes([buf[*pos], buf[*pos + 1], buf[*pos + 2], buf[*pos + 3]]);
    *pos += 4;
    val
}
