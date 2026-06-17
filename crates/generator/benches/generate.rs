use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use srcmap_generator::{SourceMapGenerator, StreamingGenerator};
use srcmap_scopes::{Binding, GeneratedRange, OriginalScope, Position, ScopeInfo};

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

fn build_range_generator(lines: u32, cols_per_line: u32) -> SourceMapGenerator {
    let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));
    let src = builder.add_source("src/ranged.ts");
    let name = builder.add_name("ranged");

    for line in 0..lines {
        for col in 0..cols_per_line {
            if col % 4 == 0 {
                builder.add_named_range_mapping(line, col * 10, src, line, col * 5, name);
            } else {
                builder.add_mapping(line, col * 10, src, line, col * 5);
            }
        }
    }

    builder
}

fn build_scoped_generator(lines: u32, cols_per_line: u32) -> SourceMapGenerator {
    let mut builder = build_generator(lines, cols_per_line, true);
    builder.set_scopes(ScopeInfo {
        scopes: vec![Some(OriginalScope {
            start: Position { line: 0, column: 0 },
            end: Position { line: lines, column: 0 },
            name: None,
            kind: Some("global".to_string()),
            is_stack_frame: false,
            variables: vec!["entry".to_string(), "state".to_string()],
            children: vec![OriginalScope {
                start: Position { line: 1, column: 0 },
                end: Position { line: lines.saturating_sub(1), column: 0 },
                name: Some("render".to_string()),
                kind: Some("function".to_string()),
                is_stack_frame: true,
                variables: vec!["props".to_string(), "result".to_string()],
                children: Vec::new(),
            }],
        })],
        ranges: vec![GeneratedRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: lines, column: 0 },
            is_stack_frame: false,
            is_hidden: false,
            definition: Some(0),
            call_site: None,
            bindings: vec![
                Binding::Expression("entry".to_string()),
                Binding::Expression("state".to_string()),
            ],
            children: vec![GeneratedRange {
                start: Position { line: 1, column: 0 },
                end: Position { line: lines.saturating_sub(1), column: 0 },
                is_stack_frame: true,
                is_hidden: false,
                definition: Some(1),
                call_site: None,
                bindings: vec![
                    Binding::Expression("props".to_string()),
                    Binding::Expression("result".to_string()),
                ],
                children: Vec::new(),
            }],
        }],
    });
    builder
}

fn build_streaming_generator(lines: u32, cols_per_line: u32) -> StreamingGenerator {
    let mut sg = StreamingGenerator::new(Some("bundle.js".to_string()));
    for i in 0..10 {
        sg.add_source(&format!("src/file{i}.js"));
    }
    for i in 0..20 {
        sg.add_name(&format!("var{i}"));
    }
    for line in 0..lines {
        for col in 0..cols_per_line {
            let src = (line * cols_per_line + col) % 10;
            if col % 3 == 0 {
                let name = col % 20;
                sg.add_named_mapping(line, col * 10, src, line, col * 5, name);
            } else {
                sg.add_mapping(line, col * 10, src, line, col * 5);
            }
        }
    }
    sg
}

fn bench_generate(criterion: &mut Criterion) {
    criterion.bench_function("generate_1000_mappings", |b| {
        b.iter_batched_ref(
            || build_generator(100, 10, false),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_10000_mappings", |b| {
        b.iter_batched_ref(
            || build_generator(500, 20, false),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings", |b| {
        b.iter_batched_ref(
            || build_generator(5000, 20, false),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_with_sources_content", |b| {
        b.iter_batched_ref(
            || build_generator(5000, 20, true),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_1000_mappings_assume_sorted", |b| {
        b.iter_batched_ref(
            || build_sorted_generator(100, 10),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_10000_mappings_assume_sorted", |b| {
        b.iter_batched_ref(
            || build_sorted_generator(500, 20),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_assume_sorted", |b| {
        b.iter_batched_ref(
            || build_sorted_generator(5000, 20),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_construct_encode", |b| {
        b.iter(|| black_box(build_generator(5000, 20, false).to_json()));
    });

    criterion.bench_function("generate_100000_mappings_assume_sorted_construct_encode", |b| {
        b.iter(|| black_box(build_sorted_generator(5000, 20).to_json()));
    });

    criterion.bench_function("generate_100000_mappings_streaming_construct_encode", |b| {
        b.iter(|| {
            let sg = build_streaming_generator(5000, 20);
            black_box(sg.to_json());
        });
    });

    criterion.bench_function("generate_100000_mappings_to_writer", |b| {
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

    criterion.bench_function("generate_100000_mappings_into_parts", |b| {
        b.iter_batched(
            || build_generator(5000, 20, false),
            |builder| black_box(builder.into_parts()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_streaming_to_writer", |b| {
        b.iter_batched_ref(
            || build_streaming_generator(5000, 20),
            |builder| {
                let mut out = Vec::new();
                builder.to_writer(&mut out).unwrap();
                black_box(out);
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_streaming_into_parts", |b| {
        b.iter_batched(
            || build_streaming_generator(5000, 20),
            |builder| black_box(builder.into_parts()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_with_range_mappings", |b| {
        b.iter_batched_ref(
            || build_range_generator(5000, 20),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("generate_100000_mappings_with_scopes", |b| {
        b.iter_batched_ref(
            || build_scoped_generator(5000, 20),
            |builder| black_box(builder.to_json()),
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_generate);
criterion_main!(benches);
