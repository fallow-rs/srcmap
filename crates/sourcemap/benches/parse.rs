use std::hint::black_box;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use serde::Deserialize;
use srcmap_codec::Segment;
use srcmap_sourcemap::{LazySourceMap, SourceMap};

// Prototype: simd-json / sonic-rs parse paths.
//
// These mirror the regular-source-map hot path in `SourceMap::from_json`,
// but swap out the JSON decoder. They omit features that neither simd-json
// nor sonic-rs handle cleanly (sections via RawValue, #[serde(flatten)]
// extensions), so those cases fall through to serde_json in production.
//
// The goal is to measure whether a SIMD JSON parse meaningfully changes
// end-to-end parse time once you add back the VLQ decode work.

#[derive(Deserialize)]
struct MinimalRawSourceMap<'a> {
    version: u32,
    #[serde(default)]
    file: Option<String>,
    #[serde(default, rename = "sourceRoot")]
    source_root: Option<String>,
    #[serde(default)]
    sources: Vec<Option<String>>,
    #[serde(default, rename = "sourcesContent")]
    sources_content: Option<Vec<Option<String>>>,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default, borrow)]
    mappings: &'a str,
    #[serde(default, rename = "ignoreList")]
    ignore_list: Option<Vec<u32>>,
    #[serde(default, rename = "debugId", alias = "debug_id")]
    debug_id: Option<String>,
    #[serde(default, borrow, rename = "rangeMappings")]
    range_mappings: Option<&'a str>,
}

fn build_sourcemap_from_minimal(raw: MinimalRawSourceMap<'_>) -> SourceMap {
    assert_eq!(raw.version, 3, "prototype only handles v3");
    let source_root = raw.source_root.clone().unwrap_or_default();
    let sources: Vec<String> = raw
        .sources
        .into_iter()
        .map(|s| match s {
            Some(s) if !source_root.is_empty() => format!("{source_root}{s}"),
            Some(s) => s,
            None => String::new(),
        })
        .collect();
    let sources_content = raw.sources_content.unwrap_or_default();
    let ignore_list = raw.ignore_list.unwrap_or_default();

    SourceMap::from_vlq_with_range_mappings(
        raw.mappings,
        sources,
        raw.names,
        raw.file,
        raw.source_root,
        sources_content,
        ignore_list,
        raw.debug_id,
        raw.range_mappings,
    )
    .unwrap()
}

fn parse_with_simd_json(json: &str) -> SourceMap {
    let mut bytes = json.as_bytes().to_vec();
    let raw: MinimalRawSourceMap<'_> = simd_json::serde::from_slice(&mut bytes).unwrap();
    build_sourcemap_from_minimal(raw)
}

fn parse_with_sonic_rs(json: &str) -> SourceMap {
    let raw: MinimalRawSourceMap<'_> = sonic_rs::from_str(json).unwrap();
    build_sourcemap_from_minimal(raw)
}

/// Baseline: serde_json parse into the same minimal struct, so the
/// comparison isolates the JSON decoder from all the extra fields the
/// real `RawSourceMap` struct carries (flatten, sections, etc.).
fn parse_with_serde_json_minimal(json: &str) -> SourceMap {
    let raw: MinimalRawSourceMap<'_> = serde_json::from_str(json).unwrap();
    build_sourcemap_from_minimal(raw)
}

fn generate_sourcemap_json(lines: usize, segs_per_line: usize, num_sources: usize) -> String {
    let sources: Vec<String> = (0..num_sources).map(|i| format!("src/file{i}.js")).collect();
    let names: Vec<String> = (0..20).map(|i| format!("var{i}")).collect();
    let sources_content: Vec<String> = (0..num_sources)
        .map(|i| format!("// source file {i}\n{}", "const x = 1;\n".repeat(lines)))
        .collect();

    let mut mappings_parts: Vec<Vec<Segment>> = Vec::with_capacity(lines);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64;
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
                line_parts.push(Segment::five(gen_col, src, src_line, src_col, name));
            } else {
                line_parts.push(Segment::four(gen_col, src, src_line, src_col));
            }
        }
        mappings_parts.push(line_parts);
    }

    let encoded = srcmap_codec::encode(&mappings_parts);

    format!(
        r#"{{"version":3,"sources":[{}],"sourcesContent":[{}],"names":[{}],"mappings":"{}"}}"#,
        sources.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","),
        sources_content
            .iter()
            .map(|s| serde_json::to_string(s).unwrap())
            .collect::<Vec<_>>()
            .join(","),
        names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(","),
        encoded,
    )
}

fn generate_sourcemap_json_no_content(
    lines: usize,
    segs_per_line: usize,
    num_sources: usize,
) -> String {
    let sources: Vec<String> = (0..num_sources).map(|i| format!("src/file{i}.js")).collect();
    let names: Vec<String> = (0..20).map(|i| format!("var{i}")).collect();

    let mut mappings_parts: Vec<Vec<Segment>> = Vec::with_capacity(lines);
    let mut src: i64 = 0;
    let mut src_line: i64 = 0;
    let mut src_col: i64;
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
                line_parts.push(Segment::five(gen_col, src, src_line, src_col, name));
            } else {
                line_parts.push(Segment::four(gen_col, src, src_line, src_col));
            }
        }
        mappings_parts.push(line_parts);
    }

    let encoded = srcmap_codec::encode(&mappings_parts);

    format!(
        r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
        sources.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","),
        names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(","),
        encoded,
    )
}

fn clone_via_from_parts(sm: &SourceMap) -> SourceMap {
    SourceMap::from_parts_with_extensions(
        sm.file.clone(),
        sm.source_root.clone(),
        sm.sources.clone(),
        sm.sources_content.clone(),
        sm.names.clone(),
        sm.all_mappings().to_vec(),
        sm.ignore_list.clone(),
        sm.debug_id.clone(),
        sm.scopes.clone(),
        sm.extensions.clone(),
    )
}

fn json_small() -> String {
    generate_sourcemap_json(50, 10, 3)
}

fn json_medium() -> String {
    generate_sourcemap_json(500, 20, 5)
}

fn json_large() -> String {
    generate_sourcemap_json(2000, 50, 10)
}

fn json_large_no_content() -> String {
    generate_sourcemap_json_no_content(2000, 50, 10)
}

fn json_indexed_sections() -> String {
    let mut sections = Vec::with_capacity(8);
    let mut offset_line = 0;

    for section_idx in 0..8 {
        let map = generate_sourcemap_json_no_content(250, 20, 5);
        sections.push(format!(r#"{{"offset":{{"line":{offset_line},"column":0}},"map":{map}}}"#));
        offset_line += 250 + section_idx % 2;
    }

    format!(r#"{{"version":3,"sections":[{}]}}"#, sections.join(","))
}

fn bench_with_input<I, O, Setup, Routine>(
    criterion: &mut Criterion,
    name: &'static str,
    mut setup: Setup,
    mut routine: Routine,
) where
    Setup: FnMut() -> I,
    Routine: FnMut(&mut I) -> O,
{
    criterion.bench_function(name, |b| {
        b.iter_batched_ref(&mut setup, &mut routine, BatchSize::LargeInput);
    });
}

fn bench_parse_sizes(criterion: &mut Criterion) {
    bench_with_input(criterion, "parse_small_500_segments", json_small, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "parse_medium_10k_segments", json_medium, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "parse_large_100k_segments", json_large, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "parse_large_no_sources_content", json_large_no_content, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(
        criterion,
        "parse_indexed_8_sections_40k_segments",
        json_indexed_sections,
        |json| SourceMap::from_json(black_box(json)).unwrap(),
    );
}

fn sourcemap_from_json_input(lines: usize, segs_per_line: usize, num_sources: usize) -> SourceMap {
    SourceMap::from_json(&generate_sourcemap_json(lines, segs_per_line, num_sources)).unwrap()
}

fn sourcemap_no_content_from_json_input(
    lines: usize,
    segs_per_line: usize,
    num_sources: usize,
) -> SourceMap {
    SourceMap::from_json(&generate_sourcemap_json_no_content(lines, segs_per_line, num_sources))
        .unwrap()
}

fn json_roundtrip(sm: &mut SourceMap) -> SourceMap {
    let json = black_box(sm).to_json();
    SourceMap::from_json(black_box(&json)).unwrap()
}

fn bench_from_parts_interop(criterion: &mut Criterion) {
    bench_with_input(
        criterion,
        "from_parts_interop_small_json_roundtrip",
        || sourcemap_from_json_input(50, 10, 3),
        json_roundtrip,
    );
    bench_with_input(
        criterion,
        "from_parts_interop_small_from_parts",
        || sourcemap_from_json_input(50, 10, 3),
        |sm| clone_via_from_parts(black_box(sm)),
    );
    bench_with_input(
        criterion,
        "from_parts_interop_medium_json_roundtrip",
        || sourcemap_from_json_input(500, 20, 5),
        json_roundtrip,
    );
    bench_with_input(
        criterion,
        "from_parts_interop_medium_from_parts",
        || sourcemap_from_json_input(500, 20, 5),
        |sm| clone_via_from_parts(black_box(sm)),
    );
    bench_with_input(
        criterion,
        "from_parts_interop_large_json_roundtrip",
        || sourcemap_from_json_input(2000, 50, 10),
        json_roundtrip,
    );
    bench_with_input(
        criterion,
        "from_parts_interop_large_from_parts",
        || sourcemap_from_json_input(2000, 50, 10),
        |sm| clone_via_from_parts(black_box(sm)),
    );
}

fn bench_serialize(criterion: &mut Criterion) {
    bench_with_input(
        criterion,
        "serialize_medium_10k_segments_to_json",
        || sourcemap_from_json_input(500, 20, 5),
        |sm| black_box(sm).to_json(),
    );
    bench_with_input(
        criterion,
        "serialize_large_100k_segments_no_content_to_json",
        || sourcemap_no_content_from_json_input(2000, 50, 10),
        |sm| black_box(sm).to_json(),
    );
    bench_with_input(
        criterion,
        "serialize_large_100k_segments_no_content_to_writer",
        || sourcemap_no_content_from_json_input(2000, 50, 10),
        |sm| {
            let mut out = Vec::new();
            black_box(sm).to_writer(&mut out).unwrap();
            black_box(out);
        },
    );
    bench_with_input(
        criterion,
        "serialize_large_100k_segments_no_content_to_vlq",
        || sourcemap_no_content_from_json_input(2000, 50, 10),
        |sm| black_box(sm).encode_mappings(),
    );
}

fn bench_parse_backends(criterion: &mut Criterion) {
    bench_with_input(criterion, "parse_backends_medium_current", json_medium, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "parse_backends_medium_serde_json_minimal", json_medium, |json| {
        parse_with_serde_json_minimal(black_box(json))
    });
    bench_with_input(criterion, "parse_backends_medium_simd_json", json_medium, |json| {
        parse_with_simd_json(black_box(json))
    });
    bench_with_input(criterion, "parse_backends_medium_sonic_rs", json_medium, |json| {
        parse_with_sonic_rs(black_box(json))
    });
    bench_with_input(criterion, "parse_backends_large_current", json_large, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "parse_backends_large_serde_json_minimal", json_large, |json| {
        parse_with_serde_json_minimal(black_box(json))
    });
    bench_with_input(criterion, "parse_backends_large_simd_json", json_large, |json| {
        parse_with_simd_json(black_box(json))
    });
    bench_with_input(criterion, "parse_backends_large_sonic_rs", json_large, |json| {
        parse_with_sonic_rs(black_box(json))
    });
    bench_with_input(
        criterion,
        "parse_backends_large_no_content_current",
        json_large_no_content,
        |json| SourceMap::from_json(black_box(json)).unwrap(),
    );
    bench_with_input(
        criterion,
        "parse_backends_large_no_content_serde_json_minimal",
        json_large_no_content,
        |json| parse_with_serde_json_minimal(black_box(json)),
    );
    bench_with_input(
        criterion,
        "parse_backends_large_no_content_simd_json",
        json_large_no_content,
        |json| parse_with_simd_json(black_box(json)),
    );
    bench_with_input(
        criterion,
        "parse_backends_large_no_content_sonic_rs",
        json_large_no_content,
        |json| parse_with_sonic_rs(black_box(json)),
    );
}

/// Real-world source map fixtures. Falls back to synthetic maps if
/// `benchmarks/fixtures/*.js.map` isn't available.
fn load_fixture(name: &str) -> Option<String> {
    let path = format!("../../benchmarks/fixtures/{name}.js.map");
    std::fs::read_to_string(&path).ok()
}

fn fixture_or_synthetic(name: &str) -> String {
    load_fixture(name).unwrap_or_else(json_large_no_content)
}

fn fixture_preact() -> String {
    fixture_or_synthetic("preact")
}

fn fixture_chartjs() -> String {
    fixture_or_synthetic("chartjs")
}

fn fixture_pdfjs() -> String {
    fixture_or_synthetic("pdfjs")
}

fn bench_real_world(criterion: &mut Criterion) {
    bench_with_input(criterion, "real_world_preact_current", fixture_preact, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "real_world_preact_sonic_rs", fixture_preact, |json| {
        parse_with_sonic_rs(black_box(json))
    });
    bench_with_input(criterion, "real_world_preact_simd_json", fixture_preact, |json| {
        parse_with_simd_json(black_box(json))
    });
    bench_with_input(criterion, "real_world_chartjs_current", fixture_chartjs, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "real_world_chartjs_sonic_rs", fixture_chartjs, |json| {
        parse_with_sonic_rs(black_box(json))
    });
    bench_with_input(criterion, "real_world_chartjs_simd_json", fixture_chartjs, |json| {
        parse_with_simd_json(black_box(json))
    });
    bench_with_input(criterion, "real_world_pdfjs_current", fixture_pdfjs, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "real_world_pdfjs_sonic_rs", fixture_pdfjs, |json| {
        parse_with_sonic_rs(black_box(json))
    });
    bench_with_input(criterion, "real_world_pdfjs_simd_json", fixture_pdfjs, |json| {
        parse_with_simd_json(black_box(json))
    });
}

fn bench_lite_paths(criterion: &mut Criterion) {
    bench_with_input(criterion, "lite_paths_preact_no_content", fixture_preact, |json| {
        SourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_preact_lazy_no_content", fixture_preact, |json| {
        LazySourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_preact_lazy_fast", fixture_preact, |json| {
        LazySourceMap::from_json_fast(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_chartjs_no_content", fixture_chartjs, |json| {
        SourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_chartjs_lazy_no_content", fixture_chartjs, |json| {
        LazySourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_chartjs_lazy_fast", fixture_chartjs, |json| {
        LazySourceMap::from_json_fast(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_pdfjs_no_content", fixture_pdfjs, |json| {
        SourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_pdfjs_lazy_no_content", fixture_pdfjs, |json| {
        LazySourceMap::from_json_no_content(black_box(json)).unwrap()
    });
    bench_with_input(criterion, "lite_paths_pdfjs_lazy_fast", fixture_pdfjs, |json| {
        LazySourceMap::from_json_fast(black_box(json)).unwrap()
    });
}

struct VlqFixture {
    mappings: String,
    sources: Vec<String>,
    names: Vec<String>,
}

fn vlq_fixture(name: &str) -> VlqFixture {
    #[derive(Deserialize)]
    struct JustMappings {
        #[serde(default)]
        mappings: String,
        #[serde(default)]
        sources: Vec<Option<String>>,
        #[serde(default)]
        names: Vec<String>,
    }

    let json = fixture_or_synthetic(name);
    let jm: JustMappings = serde_json::from_str(&json).unwrap();
    let sources = jm.sources.into_iter().map(|s| s.unwrap_or_default()).collect();

    VlqFixture { mappings: jm.mappings, sources, names: jm.names }
}

fn bench_codec_decode(input: &mut VlqFixture) {
    srcmap_codec::decode(black_box(&input.mappings)).unwrap();
}

fn bench_source_map_from_vlq(input: &mut VlqFixture) {
    SourceMap::from_vlq(
        black_box(&input.mappings),
        input.sources.clone(),
        input.names.clone(),
        None,
        None,
        Vec::new(),
        Vec::new(),
        None,
    )
    .unwrap();
}

fn bench_vlq_isolation(criterion: &mut Criterion) {
    bench_with_input(
        criterion,
        "vlq_isolation_preact_codec_decode",
        || vlq_fixture("preact"),
        bench_codec_decode,
    );
    bench_with_input(
        criterion,
        "vlq_isolation_preact_source_map_from_vlq",
        || vlq_fixture("preact"),
        bench_source_map_from_vlq,
    );
    bench_with_input(
        criterion,
        "vlq_isolation_chartjs_codec_decode",
        || vlq_fixture("chartjs"),
        bench_codec_decode,
    );
    bench_with_input(
        criterion,
        "vlq_isolation_chartjs_source_map_from_vlq",
        || vlq_fixture("chartjs"),
        bench_source_map_from_vlq,
    );
    bench_with_input(
        criterion,
        "vlq_isolation_pdfjs_codec_decode",
        || vlq_fixture("pdfjs"),
        bench_codec_decode,
    );
    bench_with_input(
        criterion,
        "vlq_isolation_pdfjs_source_map_from_vlq",
        || vlq_fixture("pdfjs"),
        bench_source_map_from_vlq,
    );
}

fn bench_json_only(criterion: &mut Criterion) {
    bench_with_input(criterion, "json_only_serde_json_value", json_large_no_content, |json| {
        let _: serde_json::Value = serde_json::from_str(black_box(json)).unwrap();
    });
    bench_with_input(criterion, "json_only_serde_json_minimal", json_large_no_content, |json| {
        let _: MinimalRawSourceMap<'_> = serde_json::from_str(black_box(json)).unwrap();
    });
    bench_with_input(criterion, "json_only_simd_json_minimal", json_large_no_content, |json| {
        let mut bytes = json.as_bytes().to_vec();
        let _: MinimalRawSourceMap<'_> = simd_json::serde::from_slice(&mut bytes).unwrap();
    });
    bench_with_input(criterion, "json_only_sonic_rs_minimal", json_large_no_content, |json| {
        let _: MinimalRawSourceMap<'_> = sonic_rs::from_str(black_box(json)).unwrap();
    });
}

fn lookup_input() -> SourceMap {
    SourceMap::from_json(&json_medium()).unwrap()
}

struct BatchLookupInput {
    sm: SourceMap,
    lookups: Vec<(u32, u32)>,
}

fn batch_lookup_input() -> BatchLookupInput {
    let sm = lookup_input();
    let lookups = (0..1000).map(|i| ((i * 7) % 500, (i * 13) % 200)).collect();

    BatchLookupInput { sm, lookups }
}

fn bench_lookup(criterion: &mut Criterion) {
    bench_with_input(criterion, "lookup_single_original_position_for", lookup_input, |sm| {
        sm.original_position_for(black_box(250), black_box(30))
    });
    bench_with_input(
        criterion,
        "lookup_1000x_original_position_for",
        batch_lookup_input,
        |input| {
            for &(line, col) in &input.lookups {
                black_box(input.sm.original_position_for(line, col));
            }
        },
    );
}

fn bench_vlq_decode(criterion: &mut Criterion) {
    bench_with_input(criterion, "vlq_decode_large_mappings_only", json_large_no_content, |json| {
        SourceMap::from_json(black_box(json)).unwrap()
    });
    bench_with_input(
        criterion,
        "vlq_decode_serde_json_parse_only",
        json_large_no_content,
        |json| {
            let _: serde_json::Value = serde_json::from_str(black_box(json)).unwrap();
        },
    );
}

criterion_group!(
    benches,
    bench_parse_sizes,
    bench_from_parts_interop,
    bench_serialize,
    bench_parse_backends,
    bench_real_world,
    bench_lite_paths,
    bench_vlq_isolation,
    bench_json_only,
    bench_lookup,
    bench_vlq_decode,
);
criterion_main!(benches);
