use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use srcmap_codec::{Segment, encode, encode_parallel};

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

fn bench_parallel_encode(criterion: &mut Criterion) {
    criterion.bench_function("parallel_feature_encode_sequential_5k_lines", |b| {
        b.iter_batched_ref(
            make_large_realistic_mappings,
            |mappings| encode(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("parallel_feature_encode_parallel_5k_lines", |b| {
        b.iter_batched_ref(
            make_large_realistic_mappings,
            |mappings| encode_parallel(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("parallel_feature_encode_sequential_50k_lines", |b| {
        b.iter_batched_ref(
            make_very_large_realistic_mappings,
            |mappings| encode(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("parallel_feature_encode_parallel_50k_lines", |b| {
        b.iter_batched_ref(
            make_very_large_realistic_mappings,
            |mappings| encode_parallel(black_box(mappings)),
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_parallel_encode);
criterion_main!(benches);
