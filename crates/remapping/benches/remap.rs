use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use srcmap_generator::SourceMapGenerator;
use srcmap_remapping::{remap, remap_streaming};
use srcmap_sourcemap::{MappingsIter, SourceMap};

struct ChainInput {
    outer: SourceMap,
    inner: SourceMap,
    vlq: String,
}

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

fn build_chain_input(mapping_count: u32) -> ChainInput {
    let (outer, inner) = build_chain(mapping_count);
    let vlq = outer.encode_mappings();

    ChainInput { outer, inner, vlq }
}

fn bench_remap_input(input: &mut ChainInput) {
    black_box(remap(&input.outer, |_| Some(input.inner.clone())));
}

fn bench_remap_streaming_input(input: &mut ChainInput) {
    let iter = MappingsIter::new(&input.vlq);
    black_box(remap_streaming(
        iter,
        &input.outer.sources,
        &input.outer.names,
        &input.outer.sources_content,
        &input.outer.ignore_list,
        input.outer.file.clone(),
        |_| Some(input.inner.clone()),
    ));
}

fn bench_chain(criterion: &mut Criterion) {
    criterion.bench_function("remap_500", |b| {
        b.iter_batched_ref(|| build_chain_input(500), bench_remap_input, BatchSize::LargeInput);
    });

    criterion.bench_function("remap_streaming_500", |b| {
        b.iter_batched_ref(
            || build_chain_input(500),
            bench_remap_streaming_input,
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_10000", |b| {
        b.iter_batched_ref(|| build_chain_input(10_000), bench_remap_input, BatchSize::LargeInput);
    });

    criterion.bench_function("remap_streaming_10000", |b| {
        b.iter_batched_ref(
            || build_chain_input(10_000),
            bench_remap_streaming_input,
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_60000", |b| {
        b.iter_batched_ref(|| build_chain_input(60_000), bench_remap_input, BatchSize::LargeInput);
    });

    criterion.bench_function("remap_streaming_60000", |b| {
        b.iter_batched_ref(
            || build_chain_input(60_000),
            bench_remap_streaming_input,
            BatchSize::LargeInput,
        );
    });
}

struct BundlerInput {
    outer: SourceMap,
    vlq: String,
    outer_json: String,
    inner_maps: Vec<(String, SourceMap)>,
}

fn build_bundler_input() -> BundlerInput {
    let source_count = 20;
    let mappings_per_source = 3000;

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
    let outer_json = outer.to_json();

    BundlerInput { outer, vlq, outer_json, inner_maps }
}

fn load_inner_map(input: &BundlerInput, source: &str) -> Option<SourceMap> {
    input.inner_maps.iter().find(|(name, _)| name == source).map(|(_, sm)| sm.clone())
}

fn load_fixture(name: &str) -> Option<String> {
    let path = format!("../../benchmarks/fixtures/{name}.js.map");
    std::fs::read_to_string(&path).ok()
}

fn max_original_lines_by_source(outer: &SourceMap) -> Vec<u32> {
    let mut max_lines = vec![0; outer.sources.len()];
    for mapping in outer.all_mappings() {
        if let Some(slot) = max_lines.get_mut(mapping.source as usize) {
            *slot = (*slot).max(mapping.original_line.saturating_add(1));
        }
    }
    max_lines
}

fn build_inner_maps_for_outer(outer: &SourceMap) -> Vec<(String, SourceMap)> {
    max_original_lines_by_source(outer)
        .into_iter()
        .enumerate()
        .map(|(source_idx, max_line)| {
            let source_name = outer.source(source_idx as u32).to_string();
            let original_name = format!("original/{source_name}");
            let line_count = max_line.clamp(1, 2000);
            let mut builder = SourceMapGenerator::with_capacity(
                Some(source_name.clone()),
                line_count as usize * 4,
            );
            let src = builder.add_source(&original_name);

            for line in 0..line_count {
                for col in 0..4 {
                    builder.add_mapping(line, col * 20, src, line + 1, col * 20 + 2);
                }
            }

            (source_name, builder.to_decoded_map())
        })
        .collect()
}

fn build_fixture_input(name: &str) -> BundlerInput {
    let outer_json = load_fixture(name).unwrap_or_else(|| build_bundler_input().outer_json);
    let outer = SourceMap::from_json(&outer_json).unwrap();
    let vlq = outer.encode_mappings();
    let inner_maps = build_inner_maps_for_outer(&outer);

    BundlerInput { outer, vlq, outer_json, inner_maps }
}

fn bench_bundler(criterion: &mut Criterion) {
    criterion.bench_function("remap_bundler_60k_20src", |b| {
        b.iter_batched_ref(
            build_bundler_input,
            |input| {
                black_box(remap(&input.outer, |source| load_inner_map(input, source)));
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_streaming_bundler_60k_20src", |b| {
        b.iter_batched_ref(
            build_bundler_input,
            |input| {
                let iter = MappingsIter::new(&input.vlq);
                black_box(remap_streaming(
                    iter,
                    &input.outer.sources,
                    &input.outer.names,
                    &input.outer.sources_content,
                    &input.outer.ignore_list,
                    input.outer.file.clone(),
                    |source| load_inner_map(input, source),
                ));
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_bundler_60k_20src_to_json", |b| {
        b.iter_batched_ref(
            build_bundler_input,
            |input| {
                black_box(remap(&input.outer, |source| load_inner_map(input, source)).to_json());
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_streaming_bundler_60k_20src_to_json", |b| {
        b.iter_batched_ref(
            build_bundler_input,
            |input| {
                let iter = MappingsIter::new(&input.vlq);
                black_box(
                    remap_streaming(
                        iter,
                        &input.outer.sources,
                        &input.outer.names,
                        &input.outer.sources_content,
                        &input.outer.ignore_list,
                        input.outer.file.clone(),
                        |source| load_inner_map(input, source),
                    )
                    .to_json(),
                );
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_json_input_bundler_60k_20src", |b| {
        b.iter_batched_ref(
            build_bundler_input,
            |input| {
                let outer = SourceMap::from_json(&input.outer_json).unwrap();
                black_box(remap(&outer, |source| load_inner_map(input, source)).to_json());
            },
            BatchSize::LargeInput,
        );
    });
}

fn bench_real_world(criterion: &mut Criterion) {
    criterion.bench_function("remap_real_world_preact_fixture_chain", |b| {
        b.iter_batched_ref(
            || build_fixture_input("preact"),
            |input| {
                black_box(remap(&input.outer, |source| load_inner_map(input, source)).to_json());
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_real_world_chartjs_fixture_chain", |b| {
        b.iter_batched_ref(
            || build_fixture_input("chartjs"),
            |input| {
                black_box(remap(&input.outer, |source| load_inner_map(input, source)).to_json());
            },
            BatchSize::LargeInput,
        );
    });

    criterion.bench_function("remap_json_input_real_world_preact_fixture_chain", |b| {
        b.iter_batched_ref(
            || build_fixture_input("preact"),
            |input| {
                let outer = SourceMap::from_json(&input.outer_json).unwrap();
                black_box(remap(&outer, |source| load_inner_map(input, source)).to_json());
            },
            BatchSize::LargeInput,
        );
    });
}

criterion_group!(benches, bench_chain, bench_bundler, bench_real_world);
criterion_main!(benches);
