use criterion::{Criterion, black_box, criterion_group, criterion_main};
use srcmap_sourcemap::SourceMap;

fn generate_sourcemap_json(lines: usize, segs_per_line: usize, num_sources: usize) -> String {
    let sources: Vec<String> = (0..num_sources)
        .map(|i| format!("src/file{i}.js"))
        .collect();
    let names: Vec<String> = (0..20).map(|i| format!("var{i}")).collect();
    let sources_content: Vec<String> = (0..num_sources)
        .map(|i| format!("// source file {i}\n{}", "const x = 1;\n".repeat(lines)))
        .collect();

    let mut mappings_parts: Vec<Vec<Vec<i64>>> = Vec::with_capacity(lines);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64 = 0;
    let mut name: i64 = 0;

    for _ in 0..lines {
        let mut gen_col: i64 = 0;
        let mut line_parts = Vec::with_capacity(segs_per_line);

        for s in 0..segs_per_line {
            gen_col += 2 + (s as i64 * 3) % 20;
            if s % 7 == 0 {
                src = (src + 1) % num_sources as i64;
            }
            src_line += 1;
            src_col = (s as i64 * 5 + 1) % 30;

            if s % 4 == 0 {
                name = (name + 1) % names.len() as i64;
                line_parts.push(vec![gen_col, src, src_line, src_col, name]);
            } else {
                line_parts.push(vec![gen_col, src, src_line, src_col]);
            }
        }
        mappings_parts.push(line_parts);
    }

    let encoded = srcmap_codec::encode(&mappings_parts);

    format!(
        r#"{{"version":3,"sources":[{}],"sourcesContent":[{}],"names":[{}],"mappings":"{}"}}"#,
        sources
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(","),
        sources_content
            .iter()
            .map(|s| format!("{}", serde_json::to_string(s).unwrap()))
            .collect::<Vec<_>>()
            .join(","),
        names
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(","),
        encoded,
    )
}

fn generate_sourcemap_json_no_content(
    lines: usize,
    segs_per_line: usize,
    num_sources: usize,
) -> String {
    let sources: Vec<String> = (0..num_sources)
        .map(|i| format!("src/file{i}.js"))
        .collect();
    let names: Vec<String> = (0..20).map(|i| format!("var{i}")).collect();

    let mut mappings_parts: Vec<Vec<Vec<i64>>> = Vec::with_capacity(lines);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64 = 0;
    let mut name: i64 = 0;

    for _ in 0..lines {
        let mut gen_col: i64 = 0;
        let mut line_parts = Vec::with_capacity(segs_per_line);

        for s in 0..segs_per_line {
            gen_col += 2 + (s as i64 * 3) % 20;
            if s % 7 == 0 {
                src = (src + 1) % num_sources as i64;
            }
            src_line += 1;
            src_col = (s as i64 * 5 + 1) % 30;

            if s % 4 == 0 {
                name = (name + 1) % names.len() as i64;
                line_parts.push(vec![gen_col, src, src_line, src_col, name]);
            } else {
                line_parts.push(vec![gen_col, src, src_line, src_col]);
            }
        }
        mappings_parts.push(line_parts);
    }

    let encoded = srcmap_codec::encode(&mappings_parts);

    format!(
        r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
        sources
            .iter()
            .map(|s| format!("\"{s}\""))
            .collect::<Vec<_>>()
            .join(","),
        names
            .iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(","),
        encoded,
    )
}

fn bench_parse(c: &mut Criterion) {
    let small = generate_sourcemap_json(50, 10, 3);
    let medium = generate_sourcemap_json(500, 20, 5);
    let large = generate_sourcemap_json(2000, 50, 10);
    let large_no_content = generate_sourcemap_json_no_content(2000, 50, 10);

    let mut group = c.benchmark_group("parse");

    group.bench_function("small (500 segs)", |b| {
        b.iter(|| SourceMap::from_json(black_box(&small)).unwrap())
    });

    group.bench_function("medium (10K segs)", |b| {
        b.iter(|| SourceMap::from_json(black_box(&medium)).unwrap())
    });

    group.bench_function("large (100K segs)", |b| {
        b.iter(|| SourceMap::from_json(black_box(&large)).unwrap())
    });

    group.bench_function("large no sourcesContent", |b| {
        b.iter(|| SourceMap::from_json(black_box(&large_no_content)).unwrap())
    });

    group.finish();
}

fn bench_lookup(c: &mut Criterion) {
    let medium = generate_sourcemap_json(500, 20, 5);
    let sm = SourceMap::from_json(&medium).unwrap();

    let mut group = c.benchmark_group("lookup");

    group.bench_function("single original_position_for", |b| {
        b.iter(|| sm.original_position_for(black_box(250), black_box(30)))
    });

    group.bench_function("1000x original_position_for", |b| {
        let lookups: Vec<(u32, u32)> = (0..1000).map(|i| ((i * 7) % 500, (i * 13) % 200)).collect();
        b.iter(|| {
            for &(line, col) in &lookups {
                black_box(sm.original_position_for(line, col));
            }
        })
    });

    group.finish();
}

fn bench_vlq_only(c: &mut Criterion) {
    // Extract just the mappings string to benchmark VLQ decode in isolation
    let json = generate_sourcemap_json_no_content(2000, 50, 10);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let mappings_str = parsed["mappings"].as_str().unwrap().to_string();

    let mut group = c.benchmark_group("vlq_decode");

    group.bench_function("large mappings only", |b| {
        b.iter(|| srcmap_sourcemap::SourceMap::from_json(black_box(&json)).unwrap())
    });

    group.bench_function("serde_json parse only", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(&json)).unwrap();
        })
    });

    group.finish();
}

criterion_group!(benches, bench_parse, bench_lookup, bench_vlq_only);
criterion_main!(benches);
