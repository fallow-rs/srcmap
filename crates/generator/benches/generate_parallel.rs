use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use srcmap_generator::SourceMapGenerator;

fn build_generator(lines: u32, cols_per_line: u32, with_content: bool) -> SourceMapGenerator {
    let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));

    for i in 0..10 {
        let src = builder.add_source(&format!("src/file{i}.js"));
        if with_content {
            builder.set_source_content(
                src,
                format!("// source file {i}\n{}", "const x = 1;\n".repeat(500)),
            );
        }
    }

    for i in 0..20 {
        builder.add_name(&format!("var{i}"));
    }

    for line in 0..lines {
        for col in 0..cols_per_line {
            let src = (line * cols_per_line + col) % 10;
            if col % 3 == 0 {
                let name = col % 20;
                builder.add_named_mapping(line, col * 10, src, line, col * 5, name);
            } else {
                builder.add_mapping(line, col * 10, src, line, col * 5);
            }
        }
    }

    builder
}

fn build_sorted_generator(lines: u32, cols_per_line: u32) -> SourceMapGenerator {
    let mut builder = build_generator(lines, cols_per_line, false);
    builder.set_assume_sorted(true);
    builder
}

fn bench_parallel_generate(criterion: &mut Criterion) {
    criterion.bench_function("parallel_feature_generate_100000_mappings", |b| {
        b.iter_batched_ref(
            || build_generator(5000, 20, false),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("parallel_feature_generate_100000_mappings_assume_sorted", |b| {
        b.iter_batched_ref(
            || build_sorted_generator(5000, 20),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function(
        "parallel_feature_generate_100000_mappings_with_sources_content",
        |b| {
            b.iter_batched_ref(
                || build_generator(5000, 20, true),
                |builder| black_box(builder.to_json()),
                BatchSize::LargeInput,
            );
        },
    );

    criterion.bench_function("parallel_feature_generate_100000_mappings_to_writer", |b| {
        b.iter_batched_ref(
            || build_generator(5000, 20, false),
            |builder| {
                let mut out = Vec::new();
                builder.to_writer(&mut out).unwrap();
                black_box(out);
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_parallel_generate);
criterion_main!(benches);
