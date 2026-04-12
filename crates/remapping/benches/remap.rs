use criterion::{Criterion, black_box, criterion_group, criterion_main};
use srcmap_generator::SourceMapGenerator;
use srcmap_remapping::{remap, remap_streaming};
use srcmap_sourcemap::{MappingsIter, SourceMap};

fn build_chain(mapping_count: u32) -> (SourceMap, SourceMap) {
    // Outer: generated -> intermediate
    let mut outer_gen = SourceMapGenerator::new(Some("output.js".to_string()));
    let src = outer_gen.add_source("intermediate.js");
    for i in 0..mapping_count {
        let line = i / 20;
        let col = (i % 20) * 5;
        outer_gen.add_mapping(line, col, src, line, col);
    }
    let outer = outer_gen.to_decoded_map();

    // Inner: intermediate -> original
    let mut inner_gen = SourceMapGenerator::new(Some("intermediate.js".to_string()));
    let src = inner_gen.add_source("original.ts");
    for i in 0..mapping_count {
        let line = i / 20;
        let col = (i % 20) * 5;
        inner_gen.add_mapping(line, col, src, line + 1, col + 2);
    }
    let inner = inner_gen.to_decoded_map();

    (outer, inner)
}

fn bench_remap(c: &mut Criterion) {
    for &count in &[500u32, 10_000, 60_000] {
        let (outer, inner) = build_chain(count);

        // Pre-encode VLQ for the streaming variant
        let vlq = outer.encode_mappings();

        c.bench_function(&format!("remap_{count}"), |b| {
            b.iter(|| {
                black_box(remap(&outer, |_| Some(inner.clone())));
            })
        });

        c.bench_function(&format!("remap_streaming_{count}"), |b| {
            b.iter(|| {
                let iter = MappingsIter::new(&vlq);
                black_box(remap_streaming(
                    iter,
                    &outer.sources,
                    &outer.names,
                    &outer.sources_content,
                    &outer.ignore_list,
                    outer.file.clone(),
                    |_| Some(inner.clone()),
                ));
            })
        });
    }
}

/// Simulate a bundler workload: multiple source files each with their own
/// source map, composed through a single bundler output map.
fn bench_remap_bundler(c: &mut Criterion) {
    let source_count = 20;
    let mappings_per_source = 3000; // 60K total

    // Build inner maps (one per source file, simulating TS → JS transforms)
    let inner_maps: Vec<(String, SourceMap)> = (0..source_count)
        .map(|s| {
            let source_name = format!("src/module_{s}.ts");
            let intermediate_name = format!("dist/module_{s}.js");
            let mut builder = SourceMapGenerator::new(Some(intermediate_name.clone()));
            let src = builder.add_source(&source_name);
            for i in 0..mappings_per_source {
                let line = i / 15;
                let col = (i % 15) * 4;
                builder.add_mapping(line, col, src, line + 1, col);
            }
            (intermediate_name, builder.to_decoded_map())
        })
        .collect();

    // Build outer map (bundler output referencing all intermediate files)
    let mut outer_gen = SourceMapGenerator::new(Some("bundle.js".to_string()));
    let src_indices: Vec<u32> =
        inner_maps.iter().map(|(name, _)| outer_gen.add_source(name)).collect();
    for (s, &src) in src_indices.iter().enumerate() {
        let line_offset = (s as u32) * (mappings_per_source / 15);
        for i in 0..mappings_per_source {
            let orig_line = i / 15;
            let col = (i % 15) * 4;
            outer_gen.add_mapping(line_offset + orig_line, col, src, orig_line, col);
        }
    }
    let outer = outer_gen.to_decoded_map();
    let vlq = outer.encode_mappings();

    c.bench_function("remap_bundler_60k_20src", |b| {
        b.iter(|| {
            black_box(remap(&outer, |source| {
                inner_maps.iter().find(|(name, _)| name == source).map(|(_, sm)| sm.clone())
            }));
        })
    });

    c.bench_function("remap_streaming_bundler_60k_20src", |b| {
        b.iter(|| {
            let iter = MappingsIter::new(&vlq);
            black_box(remap_streaming(
                iter,
                &outer.sources,
                &outer.names,
                &outer.sources_content,
                &outer.ignore_list,
                outer.file.clone(),
                |source| {
                    inner_maps.iter().find(|(name, _)| name == source).map(|(_, sm)| sm.clone())
                },
            ));
        })
    });
}

criterion_group!(benches, bench_remap, bench_remap_bundler);
criterion_main!(benches);
