use criterion::{Criterion, black_box, criterion_group, criterion_main};
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

fn bench_generate(c: &mut Criterion) {
    let mut group = c.benchmark_group("generate");

    group.bench_function("1000 mappings", |b| {
        let builder = build_generator(100, 10, false);
        b.iter(|| black_box(builder.to_json()))
    });

    group.bench_function("10000 mappings", |b| {
        let builder = build_generator(500, 20, false);
        b.iter(|| black_box(builder.to_json()))
    });

    group.bench_function("100000 mappings", |b| {
        let builder = build_generator(5000, 20, false);
        b.iter(|| black_box(builder.to_json()))
    });

    group.bench_function("100000 mappings + sourcesContent", |b| {
        let builder = build_generator(5000, 20, true);
        b.iter(|| black_box(builder.to_json()))
    });

    group.finish();
}

criterion_group!(benches, bench_generate);
criterion_main!(benches);
