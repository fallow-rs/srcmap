//! Compares srcmap with `oxc_sourcemap` and the Sentry `sourcemap` crate on
//! the real-world fixtures: parse, lookup, JSON output and composition.
//!
//! Usage: `cargo run --release -- [fixtures directory]`

use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Instant;

use srcmap_sourcemap::{Mapping, SourceMap};

const FIXTURES: [&str; 3] = ["preact", "chartjs", "pdfjs"];
/// Upper limit for the positions of one lookup measurement.
const MAX_LOOKUPS: usize = 100_000;
/// Rounds for a map below `LARGE_MAP_BYTES`, and for a map at or above it.
const ROUNDS: usize = 40;
const ROUNDS_LARGE_MAP: usize = 15;
const LARGE_MAP_BYTES: usize = 2_000_000;
const NO_INDEX: u32 = u32::MAX;
const INTERMEDIATE_SOURCE: &str = "intermediate.js";

type Candidate<'a> = &'a mut dyn FnMut();

/// Runs the candidates in turn for `rounds` rounds and returns the best time
/// of each one, in microseconds.
///
/// The candidates share each round, so a change in machine load has the same
/// effect on all of them. The best time is the value with the least noise.
fn race(rounds: usize, candidates: &mut [Candidate]) -> Vec<f64> {
    let mut best = vec![f64::MAX; candidates.len()];
    for candidate in candidates.iter_mut() {
        candidate();
    }
    for _ in 0..rounds {
        for (candidate, best) in candidates.iter_mut().zip(&mut best) {
            let start = Instant::now();
            candidate();
            *best = best.min(start.elapsed().as_secs_f64() * 1e6);
        }
    }
    best
}

fn print_header(title: &str) {
    println!("\n### {title}\n");
    println!("| Operation | srcmap | oxc_sourcemap | sourcemap (Sentry) | srcmap / oxc |");
    println!("|---|---:|---:|---:|---:|");
}

/// Prints one row. `times` has srcmap, then `oxc_sourcemap`, then optionally
/// the Sentry crate. `unit_scale` converts microseconds to the printed unit.
fn print_row(operation: &str, times: &[f64], unit_scale: f64, unit: &str) {
    let cell = |time: f64| format!("{:.1} {unit}", time * unit_scale);
    let sentry = times.get(2).map_or_else(|| "n/a".to_string(), |&time| cell(time));
    println!(
        "| {operation} | {} | {} | {sentry} | {:.2} |",
        cell(times[0]),
        cell(times[1]),
        times[0] / times[1]
    );
}

/// The generated positions for the lookup measurements: the start of a
/// segment, and one column before the next segment on the same line.
struct Positions {
    at_segment_start: Vec<(u32, u32)>,
    between_segments: Vec<(u32, u32)>,
}

fn lookup_positions(mappings: &[Mapping]) -> Positions {
    let step = (mappings.len() / MAX_LOOKUPS).max(1);
    let at_segment_start =
        mappings.iter().step_by(step).map(|m| (m.generated_line, m.generated_column)).collect();
    let between_segments = mappings
        .windows(2)
        .step_by(step)
        .filter(|pair| {
            pair[0].generated_line == pair[1].generated_line
                && pair[1].generated_column > pair[0].generated_column + 1
        })
        .map(|pair| (pair[0].generated_line, pair[1].generated_column - 1))
        .collect();
    Positions { at_segment_start, between_segments }
}

/// Builds the map of a second transform over the generated file of `inner`.
///
/// Each segment of `inner` gets one segment in the result. Every second
/// segment points one column after the start of a segment of `inner`, when
/// that column is free, so the trace also has to search between segments.
fn second_transform_map(inner: &SourceMap) -> SourceMap {
    let inner_mappings = inner.all_mappings();
    let mappings = inner_mappings
        .iter()
        .enumerate()
        .map(|(index, m)| {
            let next_column = inner_mappings
                .get(index + 1)
                .filter(|next| next.generated_line == m.generated_line)
                .map(|next| next.generated_column);
            let has_free_column = next_column.is_none_or(|next| next > m.generated_column + 1);
            let offset = u32::from(index % 2 == 1 && has_free_column);
            Mapping {
                generated_line: m.generated_line,
                generated_column: m.generated_column,
                source: 0,
                original_line: m.generated_line,
                original_column: m.generated_column + offset,
                name: NO_INDEX,
                is_range_mapping: false,
            }
        })
        .collect();
    SourceMap::from_parts(
        None,
        None,
        vec![INTERMEDIATE_SOURCE.to_string()],
        vec![None],
        Vec::new(),
        mappings,
        Vec::new(),
        None,
        None,
    )
}

fn measure_parse_and_output(json: &str, rounds: usize) {
    let srcmap = SourceMap::from_json(json).unwrap();
    let oxc = oxc_sourcemap::SourceMap::from_json_string(json).unwrap();
    let sentry = sourcemap::SourceMap::from_slice(json.as_bytes()).unwrap();

    let times = race(
        rounds,
        &mut [
            &mut || {
                black_box(SourceMap::from_json(black_box(json)).unwrap());
            },
            &mut || {
                black_box(oxc_sourcemap::SourceMap::from_json_string(black_box(json)).unwrap());
            },
            &mut || {
                black_box(sourcemap::SourceMap::from_slice(black_box(json.as_bytes())).unwrap());
            },
        ],
    );
    print_row("Parse", &times, 1.0, "µs");

    let times = race(
        rounds,
        &mut [
            &mut || {
                black_box(srcmap.to_json());
            },
            &mut || {
                black_box(oxc.to_json_string());
            },
            &mut || {
                let mut out = Vec::new();
                sentry.to_writer(&mut out).unwrap();
                black_box(out);
            },
        ],
    );
    print_row("Write JSON", &times, 1.0, "µs");
}

fn measure_lookups(json: &str, rounds: usize) {
    let srcmap = SourceMap::from_json(json).unwrap();
    let oxc = oxc_sourcemap::SourceMap::from_json_string(json).unwrap();
    let oxc_table = oxc.generate_lookup_table();
    let sentry = sourcemap::SourceMap::from_slice(json.as_bytes()).unwrap();
    let positions = lookup_positions(srcmap.all_mappings());

    for (operation, positions) in [
        ("Lookup at a segment start", &positions.at_segment_start),
        ("Lookup between two segments", &positions.between_segments),
    ] {
        if positions.is_empty() {
            continue;
        }
        let times = race(
            rounds,
            &mut [
                &mut || {
                    for &(line, column) in positions {
                        black_box(srcmap.original_position_for(line, column));
                    }
                },
                &mut || {
                    for &(line, column) in positions {
                        black_box(oxc.lookup_token(&oxc_table, line, column));
                    }
                },
                &mut || {
                    for &(line, column) in positions {
                        black_box(sentry.lookup_token(line, column));
                    }
                },
            ],
        );
        // One lookup takes nanoseconds, so print the time for each lookup.
        let ns_per_lookup = 1000.0 / positions.len() as f64;
        print_row(operation, &times, ns_per_lookup, "ns");
    }
}

fn measure_composition(inner_json: &str, rounds: usize) {
    let inner = SourceMap::from_json(inner_json).unwrap();
    let outer = second_transform_map(&inner);
    let outer_json = outer.to_json();
    let oxc_inner = oxc_sourcemap::SourceMap::from_json_string(inner_json).unwrap();
    let oxc_outer = oxc_sourcemap::SourceMap::from_json_string(&outer_json).unwrap();

    let times = race(
        rounds,
        &mut [
            &mut || {
                black_box(srcmap_remapping::remap_ref(&outer, |_| Some(&inner)));
            },
            &mut || {
                black_box(oxc_outer.clone().compose(oxc_inner.clone()));
            },
            // The clones are not part of `compose`. The next step subtracts them.
            &mut || {
                black_box((oxc_outer.clone(), oxc_inner.clone()));
            },
        ],
    );
    print_row("Compose two parsed maps", &[times[0], times[1] - times[2]], 1.0, "µs");

    let times = race(
        rounds,
        &mut [
            &mut || {
                let outer = SourceMap::from_json(&outer_json).unwrap();
                let inner = SourceMap::from_json(inner_json).unwrap();
                black_box(srcmap_remapping::remap_ref(&outer, |_| Some(&inner)).to_json());
            },
            &mut || {
                let outer = oxc_sourcemap::SourceMap::from_json_string(&outer_json).unwrap();
                let inner = oxc_sourcemap::SourceMap::from_json_string(inner_json).unwrap();
                black_box(outer.compose(inner).to_json_string());
            },
        ],
    );
    print_row("Parse two maps, compose, write JSON", &times, 1.0, "µs");

    let composed = srcmap_remapping::remap_ref(&outer, |_| Some(&inner));
    let oxc_composed = oxc_outer.compose(oxc_inner);
    println!(
        "\nComposed segments: srcmap {}, oxc_sourcemap {}. srcmap applies the rules of \
         `@jridgewell/remapping` to each segment, so the two results can differ.",
        composed.mapping_count(),
        oxc_composed.get_tokens().len()
    );
}

fn fixtures_directory() -> PathBuf {
    std::env::args()
        .nth(1)
        .map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures"), PathBuf::from)
}

fn main() {
    let directory = fixtures_directory();
    println!("Best time of each operation. A ratio below 1.00 means that srcmap is faster.");

    for name in FIXTURES {
        let path = directory.join(format!("{name}.js.map"));
        let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "cannot read {}: {error}. Run `node benchmarks/download-fixtures.mjs` first.",
                path.display()
            )
        });
        let rounds = if json.len() >= LARGE_MAP_BYTES { ROUNDS_LARGE_MAP } else { ROUNDS };
        let segments = SourceMap::from_json(&json).unwrap().mapping_count();

        print_header(&format!("{name} ({} KB, {segments} segments)", json.len() / 1024));
        measure_parse_and_output(&json, rounds);
        measure_lookups(&json, rounds);
        measure_composition(&json, rounds);
    }
}
