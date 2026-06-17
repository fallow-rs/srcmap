#[cfg(feature = "parallel")]
use srcmap_codec::encode_parallel;
use srcmap_codec::{Segment, decode, encode};
use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};

/// Synthetic all-zero mappings (best case: single-char VLQ values).
fn make_synthetic_mappings() -> String {
    let line = std::iter::repeat_n("AAAA", 50).collect::<Vec<_>>().join(",");
    std::iter::repeat_n(line.as_str(), 1000).collect::<Vec<_>>().join(";")
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
                line.push(Segment::five(gen_col, src, src_line, src_col, name));
            } else {
                line.push(Segment::four(gen_col, src, src_line, src_col));
            }
        }

        mappings.push(line);
    }

    encode(&mappings)
}

fn bench_decode(criterion: &mut Criterion) {
    criterion.bench_function("decode_small", |b| {
        b.iter(|| decode(black_box("AAAA;AACA,GAAG;AACA,IAAI,EAAE")).unwrap());
    });

    criterion.bench_function("decode_synthetic_50k_segments", |b| {
        b.iter_batched_ref(
            make_synthetic_mappings,
            |mappings| decode(black_box(mappings)).unwrap(),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("decode_realistic_500_lines", |b| {
        b.iter_batched_ref(
            make_realistic_mappings,
            |mappings| decode(black_box(mappings)).unwrap(),
            BatchSize::LargeInput,
        );
    });
}

#[cfg(feature = "parallel")]
fn make_large_realistic_mappings() -> srcmap_codec::SourceMapMappings {
    let mut mappings = Vec::with_capacity(5000);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64 = 0;
    let mut name: i64 = 0;

    for line_idx in 0..5000_i64 {
        let segments_per_line = 10 + (line_idx % 40) as usize;
        let mut line = Vec::with_capacity(segments_per_line);
        let mut gen_col: i64 = 0;

        for seg in 0..segments_per_line {
            let seg = seg as i64;
            gen_col += 2 + (seg * 3) % 28;
            if seg % 15 == 0 {
                src += 1;
            }
            src_line += if seg % 7 == 0 { -3 } else { 1 };
            src_line = src_line.max(0);
            src_col += (seg * 7 + 3) % 50 - 10;
            src_col = src_col.max(0);

            if seg % 5 == 0 {
                name += 1;
                line.push(Segment::five(gen_col, src, src_line, src_col, name));
            } else {
                line.push(Segment::four(gen_col, src, src_line, src_col));
            }
        }
        mappings.push(line);
    }
    mappings
}

fn make_decoded_synthetic_mappings() -> srcmap_codec::SourceMapMappings {
    let synthetic = make_synthetic_mappings();
    decode(&synthetic).unwrap()
}

fn make_decoded_realistic_mappings() -> srcmap_codec::SourceMapMappings {
    let realistic = make_realistic_mappings();
    decode(&realistic).unwrap()
}

fn bench_encode(criterion: &mut Criterion) {
    criterion.bench_function("encode_synthetic_50k_segments", |b| {
        b.iter_batched_ref(
            make_decoded_synthetic_mappings,
            |mappings| encode(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("encode_realistic_500_lines", |b| {
        b.iter_batched_ref(
            make_decoded_realistic_mappings,
            |mappings| encode(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });

    #[cfg(feature = "parallel")]
    {
        criterion.bench_function("encode_sequential_5k_lines", |b| {
            b.iter_batched_ref(
                make_large_realistic_mappings,
                |mappings| encode(black_box(mappings)),
                BatchSize::LargeInput,
            );
        });

        criterion.bench_function("encode_parallel_5k_lines", |b| {
            b.iter_batched_ref(
                make_large_realistic_mappings,
                |mappings| encode_parallel(black_box(mappings)),
                BatchSize::LargeInput,
            );
        });
    }
}

#[cfg(feature = "parallel")]
fn make_very_large_realistic_mappings() -> srcmap_codec::SourceMapMappings {
    let mut mappings = Vec::with_capacity(50000);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64 = 0;
    let mut name: i64 = 0;
    for line_idx in 0..50000_i64 {
        let segments_per_line = 5 + (line_idx % 20) as usize;
        let mut line = Vec::with_capacity(segments_per_line);
        let mut gen_col: i64 = 0;
        for seg in 0..segments_per_line {
            let seg = seg as i64;
            gen_col += 2 + (seg * 3) % 28;
            if seg % 15 == 0 {
                src += 1;
            }
            src_line += if seg % 7 == 0 { -3 } else { 1 };
            src_line = src_line.max(0);
            src_col += (seg * 7 + 3) % 50 - 10;
            src_col = src_col.max(0);
            if seg % 5 == 0 {
                name += 1;
                line.push(Segment::five(gen_col, src, src_line, src_col, name));
            } else {
                line.push(Segment::four(gen_col, src, src_line, src_col));
            }
        }
        mappings.push(line);
    }
    mappings
}

fn bench_large_encode(criterion: &mut Criterion) {
    #[cfg(feature = "parallel")]
    {
        criterion.bench_function("encode_sequential_50k_lines", |b| {
            b.iter_batched_ref(
                make_very_large_realistic_mappings,
                |mappings| encode(black_box(mappings)),
                BatchSize::LargeInput,
            );
        });

        criterion.bench_function("encode_parallel_50k_lines", |b| {
            b.iter_batched_ref(
                make_very_large_realistic_mappings,
                |mappings| encode_parallel(black_box(mappings)),
                BatchSize::LargeInput,
            );
        });
    }

    criterion.bench_function("roundtrip_realistic_500_lines", |b| {
        b.iter_batched_ref(
            make_realistic_mappings,
            |realistic| {
                let decoded = decode(black_box(realistic)).unwrap();
                encode(black_box(&decoded))
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_decode, bench_encode, bench_large_encode);
criterion_main!(benches);
