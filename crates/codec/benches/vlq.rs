use std::hint::black_box;
use criterion::{Criterion, criterion_group, criterion_main};
use srcmap_codec::{decode, encode};

/// Synthetic all-zero mappings (best case: single-char VLQ values).
fn make_synthetic_mappings() -> String {
    let line = (0..50).map(|_| "AAAA").collect::<Vec<_>>().join(",");
    (0..1000).map(|_| line.as_str()).collect::<Vec<_>>().join(";")
}

/// Realistic mappings with varied deltas and multi-byte VLQ sequences.
/// Simulates a typical transpiled JS file with increasing columns,
/// jumping between source files, and occasional name mappings.
fn make_realistic_mappings() -> String {
    let mut mappings = Vec::with_capacity(500);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64 = 0;
    let mut name: i64 = 0;

    for line_idx in 0..500_i64 {
        let segments_per_line = 10 + (line_idx % 40) as usize;
        let mut line = Vec::with_capacity(segments_per_line);
        let mut gen_col: i64 = 0;

        for seg in 0..segments_per_line {
            let seg = seg as i64;

            // Varied generated columns (2-30 chars apart)
            gen_col += 2 + (seg * 3) % 28;

            // Occasional source file changes
            if seg % 15 == 0 {
                src += 1;
            }

            // Source line generally increases, sometimes jumps back
            src_line += if seg % 7 == 0 { -3 } else { 1 };
            src_line = src_line.max(0);

            // Source column varies widely
            src_col += (seg * 7 + 3) % 50 - 10;
            src_col = src_col.max(0);

            // ~20% of segments have names
            if seg % 5 == 0 {
                name += 1;
                line.push(vec![gen_col, src, src_line, src_col, name]);
            } else {
                line.push(vec![gen_col, src, src_line, src_col]);
            }
        }

        mappings.push(line);
    }

    encode(&mappings)
}

fn bench_decode(c: &mut Criterion) {
    let small = "AAAA;AACA,GAAG;AACA,IAAI,EAAE";
    let synthetic = make_synthetic_mappings();
    let realistic = make_realistic_mappings();

    c.bench_function("decode small", |b| {
        b.iter(|| decode(black_box(small)).unwrap());
    });

    c.bench_function("decode synthetic (50K segments)", |b| {
        b.iter(|| decode(black_box(&synthetic)).unwrap());
    });

    c.bench_function("decode realistic (500 lines)", |b| {
        b.iter(|| decode(black_box(&realistic)).unwrap());
    });
}

fn bench_encode(c: &mut Criterion) {
    let synthetic = make_synthetic_mappings();
    let decoded_synthetic = decode(&synthetic).unwrap();

    let realistic = make_realistic_mappings();
    let decoded_realistic = decode(&realistic).unwrap();

    c.bench_function("encode synthetic (50K segments)", |b| {
        b.iter(|| encode(black_box(&decoded_synthetic)));
    });

    c.bench_function("encode realistic (500 lines)", |b| {
        b.iter(|| encode(black_box(&decoded_realistic)));
    });
}

fn bench_roundtrip(c: &mut Criterion) {
    let realistic = make_realistic_mappings();

    c.bench_function("roundtrip realistic (500 lines)", |b| {
        b.iter(|| {
            let decoded = decode(black_box(&realistic)).unwrap();
            encode(black_box(&decoded))
        });
    });
}

criterion_group!(benches, bench_decode, bench_encode, bench_roundtrip);
criterion_main!(benches);
