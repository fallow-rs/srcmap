use criterion::{black_box, criterion_group, criterion_main, Criterion};
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
                    outer.file.clone(),
                    |_| Some(inner.clone()),
                ));
            })
        });
    }
}

criterion_group!(benches, bench_remap);
criterion_main!(benches);
