use std::hint::black_box;

use divan::Bencher;
use srcmap_generator::{SourceMapGenerator, StreamingGenerator};

fn main() {
    divan::main();
}

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

#[divan::bench]
fn generate_1000_mappings(bencher: Bencher) {
    bencher
        .with_inputs(|| build_generator(100, 10, false))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_10000_mappings(bencher: Bencher) {
    bencher
        .with_inputs(|| build_generator(500, 20, false))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_100000_mappings(bencher: Bencher) {
    bencher
        .with_inputs(|| build_generator(5000, 20, false))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_100000_mappings_with_sources_content(bencher: Bencher) {
    bencher
        .with_inputs(|| build_generator(5000, 20, true))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_1000_mappings_assume_sorted(bencher: Bencher) {
    bencher
        .with_inputs(|| build_sorted_generator(100, 10))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_10000_mappings_assume_sorted(bencher: Bencher) {
    bencher
        .with_inputs(|| build_sorted_generator(500, 20))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_100000_mappings_assume_sorted(bencher: Bencher) {
    bencher
        .with_inputs(|| build_sorted_generator(5000, 20))
        .bench_refs(|builder| black_box(builder.to_json()));
}

#[divan::bench]
fn generate_100000_mappings_streaming_construct_encode() {
    let mut sg = StreamingGenerator::new(Some("bundle.js".to_string()));
    for i in 0..10 {
        sg.add_source(&format!("src/file{i}.js"));
    }
    for i in 0..20 {
        sg.add_name(&format!("var{i}"));
    }
    let lines = 5000u32;
    let cols_per_line = 20u32;
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
    black_box(sg.to_json());
}
