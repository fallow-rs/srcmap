use criterion::{Criterion, black_box, criterion_group, criterion_main};
use serde::Deserialize;
use srcmap_codec::Segment;
use srcmap_sourcemap::{LazySourceMap, SourceMap};

// ── Prototype: simd-json / sonic-rs parse paths ────────────────────
//
// These mirror the regular-source-map hot path in `SourceMap::from_json`,
// but swap out the JSON decoder. They omit features that neither simd-json
// nor sonic-rs handle cleanly (sections via RawValue, #[serde(flatten)]
// extensions) — those cases fall through to serde_json in production.
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

/// End-to-end parse comparison: current `SourceMap::from_json` (serde_json with
/// the full `RawSourceMap` struct including `#[serde(flatten)]` extensions) vs
/// the three minimal-struct paths backed by serde_json, simd-json, and sonic-rs.
fn bench_parse_backends(c: &mut Criterion) {
    let medium = generate_sourcemap_json(500, 20, 5);
    let large = generate_sourcemap_json(2000, 50, 10);
    let large_no_content = generate_sourcemap_json_no_content(2000, 50, 10);

    let mut group = c.benchmark_group("parse_backends");

    for (label, json) in [
        ("medium (10K segs)", &medium),
        ("large (100K segs)", &large),
        ("large no sourcesContent", &large_no_content),
    ] {
        group.bench_function(format!("{label} / serde_json (current)"), |b| {
            b.iter(|| SourceMap::from_json(black_box(json)).unwrap())
        });
        group.bench_function(format!("{label} / serde_json (minimal struct)"), |b| {
            b.iter(|| parse_with_serde_json_minimal(black_box(json)))
        });
        group.bench_function(format!("{label} / simd-json"), |b| {
            b.iter(|| parse_with_simd_json(black_box(json)))
        });
        group.bench_function(format!("{label} / sonic-rs"), |b| {
            b.iter(|| parse_with_sonic_rs(black_box(json)))
        });
    }

    group.finish();
}

/// Real-world source map fixtures. Falls back to synthetic maps if
/// `benchmarks/fixtures/*.js.map` isn't available.
fn load_fixture(name: &str) -> Option<String> {
    let path = format!("../../benchmarks/fixtures/{name}.js.map");
    std::fs::read_to_string(&path).ok()
}

/// Real-world parse: chartjs / preact / pdfjs. Isolates the concrete workload
/// so we know whether synthetic generator results translate to real data.
fn bench_real_world(c: &mut Criterion) {
    let fixtures: Vec<(&str, String)> = ["preact", "chartjs", "pdfjs"]
        .iter()
        .filter_map(|name| load_fixture(name).map(|s| (*name, s)))
        .collect();

    if fixtures.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("real_world");

    for (name, json) in &fixtures {
        let kb = json.len() / 1024;
        let label = format!("{name} ({kb} KB)");

        group.bench_function(format!("{label} / serde_json (current)"), |b| {
            b.iter(|| SourceMap::from_json(black_box(json)).unwrap())
        });
        group.bench_function(format!("{label} / sonic-rs"), |b| {
            b.iter(|| parse_with_sonic_rs(black_box(json)))
        });
        group.bench_function(format!("{label} / simd-json"), |b| {
            b.iter(|| parse_with_simd_json(black_box(json)))
        });
    }

    group.finish();
}

/// Lite-path parses (skip sourcesContent allocation). These hit the
/// `RawSourceMapLite` codepath used by the WASM bindings — the round-3
/// sonic-rs migration target.
fn bench_lite_paths(c: &mut Criterion) {
    let fixtures: Vec<(&str, String)> = ["preact", "chartjs", "pdfjs"]
        .iter()
        .filter_map(|name| load_fixture(name).map(|s| (*name, s)))
        .collect();

    if fixtures.is_empty() {
        return;
    }

    let mut group = c.benchmark_group("lite_paths");

    for (name, json) in &fixtures {
        let kb = json.len() / 1024;
        let label = format!("{name} ({kb} KB)");

        group.bench_function(format!("{label} / SourceMap::from_json_no_content"), |b| {
            b.iter(|| SourceMap::from_json_no_content(black_box(json)).unwrap())
        });
        group.bench_function(format!("{label} / LazySourceMap::from_json_no_content"), |b| {
            b.iter(|| LazySourceMap::from_json_no_content(black_box(json)).unwrap())
        });
        group.bench_function(format!("{label} / LazySourceMap::from_json_fast"), |b| {
            b.iter(|| LazySourceMap::from_json_fast(black_box(json)).unwrap())
        });
    }

    group.finish();
}

/// Pure VLQ decode isolation: how much of the total parse time is the
/// VLQ pass, separate from JSON decode and struct construction?
fn bench_vlq_isolation(c: &mut Criterion) {
    let fixtures: Vec<(&str, String)> = ["preact", "chartjs", "pdfjs"]
        .iter()
        .filter_map(|name| load_fixture(name).map(|s| (*name, s)))
        .collect();

    if fixtures.is_empty() {
        return;
    }

    // Pre-extract the mappings string (and sources, names) from each fixture
    // so the VLQ bench doesn't include JSON parse overhead.
    #[derive(Deserialize)]
    struct JustMappings {
        #[serde(default)]
        mappings: String,
        #[serde(default)]
        sources: Vec<Option<String>>,
        #[serde(default)]
        names: Vec<String>,
    }

    let extracted: Vec<(String, String, Vec<String>, Vec<String>)> = fixtures
        .iter()
        .map(|(name, json)| {
            let jm: JustMappings = serde_json::from_str(json).unwrap();
            let sources: Vec<String> =
                jm.sources.into_iter().map(|s| s.unwrap_or_default()).collect();
            (name.to_string(), jm.mappings, sources, jm.names)
        })
        .collect();

    let mut group = c.benchmark_group("vlq_isolation");

    for (name, mappings_str, sources, names) in &extracted {
        let kb = mappings_str.len() / 1024;
        let label = format!("{name} mappings ({kb} KB)");

        group.bench_function(format!("{label} / codec::decode (segments)"), |b| {
            b.iter(|| srcmap_codec::decode(black_box(mappings_str)).unwrap())
        });

        // End-to-end: mappings string -> full SourceMap via the sourcemap-crate
        // decoder (measures the specialized `decode_mappings` + struct assembly).
        group.bench_function(format!("{label} / SourceMap::from_vlq"), |b| {
            b.iter(|| {
                SourceMap::from_vlq(
                    black_box(mappings_str),
                    sources.clone(),
                    names.clone(),
                    None,
                    None,
                    Vec::new(),
                    Vec::new(),
                    None,
                )
                .unwrap()
            })
        });
    }

    group.finish();
}

/// JSON-only parse comparison: isolate the JSON decoder from VLQ decoding.
fn bench_json_only(c: &mut Criterion) {
    let large = generate_sourcemap_json_no_content(2000, 50, 10);

    let mut group = c.benchmark_group("json_only");

    group.bench_function("serde_json -> Value", |b| {
        b.iter(|| {
            let _: serde_json::Value = serde_json::from_str(black_box(&large)).unwrap();
        })
    });
    group.bench_function("serde_json -> MinimalRawSourceMap", |b| {
        b.iter(|| {
            let _: MinimalRawSourceMap<'_> = serde_json::from_str(black_box(&large)).unwrap();
        })
    });
    group.bench_function("simd-json -> MinimalRawSourceMap", |b| {
        b.iter(|| {
            let mut bytes = large.as_bytes().to_vec();
            let _: MinimalRawSourceMap<'_> = simd_json::serde::from_slice(&mut bytes).unwrap();
        })
    });
    group.bench_function("sonic-rs -> MinimalRawSourceMap", |b| {
        b.iter(|| {
            let _: MinimalRawSourceMap<'_> = sonic_rs::from_str(black_box(&large)).unwrap();
        })
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
    let json = generate_sourcemap_json_no_content(2000, 50, 10);

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

criterion_group!(
    benches,
    bench_parse,
    bench_lookup,
    bench_vlq_only,
    bench_parse_backends,
    bench_json_only,
    bench_real_world,
    bench_lite_paths,
    bench_vlq_isolation
);
criterion_main!(benches);
