//! Multi-step build pipeline composition with `remap` and `remap_streaming`.
//!
//! Simulates a two-stage transform pipeline:
//!
//!   original.ts  --[tsc]-->  intermediate.js  --[terser]-->  output.min.js
//!
//! Each stage produces its own source map. Composition chains them into a
//! single map that goes directly from output.min.js back to original.ts,
//! so browser devtools can show the original TypeScript source even though
//! two separate transforms ran.
//!
//! This is the **composition** use-case: multiple transforms on the *same*
//! file. For merging source maps from *different* files into a bundle, see
//! the `bundler_concat` example instead.
//!
//! Run with: cargo run -p srcmap-remapping --example composition

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_generator::SourceMapGenerator;
use srcmap_remapping::{remap, remap_streaming};
use srcmap_sourcemap::{MappingsIter, SourceMap};

fn main() {
    println!("=== Source Map Composition ===\n");

    // -----------------------------------------------------------------------
    // 1. Build the inner source map (tsc: original.ts -> intermediate.js)
    // -----------------------------------------------------------------------
    //
    // Simulated TypeScript source (original.ts):
    //
    //   line 0: function greet(name: string): string {
    //   line 1:     const message = `Hello, ${name}!`;
    //   line 2:     return message;
    //   line 3: }
    //
    // After tsc (intermediate.js):
    //
    //   line 0: function greet(name) {
    //   line 1:     var message = "Hello, " + name + "!";
    //   line 2:     return message;
    //   line 3: }
    //
    // All positions are 0-based per ECMA-426.

    println!("--- Step 1: Inner map (tsc: original.ts -> intermediate.js) ---\n");

    let mut inner_gen = SourceMapGenerator::new(Some("intermediate.js".to_string()));
    let inner_src = inner_gen.add_source("original.ts");
    inner_gen.set_source_content(
        inner_src,
        "function greet(name: string): string {\n    const message = `Hello, ${name}!`;\n    return message;\n}\n".to_string(),
    );

    let name_greet = inner_gen.add_name("greet");
    let name_name = inner_gen.add_name("name");
    let name_message = inner_gen.add_name("message");

    // line 0: "function greet(name) {"
    //          ^0       ^9     ^15
    // maps to original.ts line 0
    inner_gen.add_mapping(0, 0, inner_src, 0, 0); // `function`
    inner_gen.add_named_mapping(0, 9, inner_src, 0, 9, name_greet); // `greet`
    inner_gen.add_named_mapping(0, 15, inner_src, 0, 15, name_name); // `name` param

    // line 1: "    var message = "Hello, " + name + "!";"
    //          ^0   ^4          ^18                  ^37
    // maps to original.ts line 1
    inner_gen.add_mapping(1, 0, inner_src, 1, 0); // indent
    inner_gen.add_named_mapping(1, 8, inner_src, 1, 10, name_message); // `message` (var -> const)
    inner_gen.add_mapping(1, 18, inner_src, 1, 20); // `=` assignment

    // line 2: "    return message;"
    //          ^0   ^4     ^11
    // maps to original.ts line 2
    inner_gen.add_mapping(2, 0, inner_src, 2, 0); // indent
    inner_gen.add_mapping(2, 4, inner_src, 2, 4); // `return`
    inner_gen.add_named_mapping(2, 11, inner_src, 2, 11, name_message); // `message`

    // line 3: "}"
    inner_gen.add_mapping(3, 0, inner_src, 3, 0); // closing brace

    let inner_json = inner_gen.to_json();
    let inner_map = SourceMap::from_json(&inner_json).unwrap();

    println!(
        "  Inner map: {} mappings across {} lines",
        inner_map.mapping_count(),
        inner_map.line_count(),
    );
    println!("  Sources: {:?}", inner_map.sources);
    println!("  Names: {:?}\n", inner_map.names);

    // -----------------------------------------------------------------------
    // 2. Build the outer source map (terser: intermediate.js -> output.min.js)
    // -----------------------------------------------------------------------
    //
    // After terser minification (output.min.js):
    //
    //   line 0: function greet(n){var m="Hello, "+n+"!";return m}
    //            ^0       ^9   ^15 ^18 ^22              ^39     ^46
    //
    // Terser renames `name` -> `n` and `message` -> `m`, removes whitespace,
    // and puts everything on a single line.

    println!("--- Step 2: Outer map (terser: intermediate.js -> output.min.js) ---\n");

    let mut outer_gen = SourceMapGenerator::new(Some("output.min.js".to_string()));
    let outer_src = outer_gen.add_source("intermediate.js");

    let name_n = outer_gen.add_name("name");
    let name_m = outer_gen.add_name("message");

    // Everything on line 0 of the minified output:
    // "function greet(n){var m="Hello, "+n+"!";return m}"
    outer_gen.add_mapping(0, 0, outer_src, 0, 0); // `function`
    outer_gen.add_mapping(0, 9, outer_src, 0, 9); // `greet`
    outer_gen.add_named_mapping(0, 15, outer_src, 0, 15, name_n); // `n` (was `name`)
    outer_gen.add_mapping(0, 17, outer_src, 0, 21); // `{`
    outer_gen.add_mapping(0, 18, outer_src, 1, 4); // `var`
    outer_gen.add_named_mapping(0, 22, outer_src, 1, 8, name_m); // `m` (was `message`)
    outer_gen.add_mapping(0, 24, outer_src, 1, 18); // `=` assignment
    outer_gen.add_mapping(0, 39, outer_src, 2, 4); // `return`
    outer_gen.add_named_mapping(0, 46, outer_src, 2, 11, name_m); // `m` (was `message`)
    outer_gen.add_mapping(0, 47, outer_src, 3, 0); // `}`

    let outer_json = outer_gen.to_json();
    let outer_map = SourceMap::from_json(&outer_json).unwrap();

    println!(
        "  Outer map: {} mappings across {} lines",
        outer_map.mapping_count(),
        outer_map.line_count(),
    );
    println!("  Sources: {:?}", outer_map.sources);
    println!("  Names: {:?}\n", outer_map.names);

    // -----------------------------------------------------------------------
    // 3. Compose with remap(): output.min.js -> original.ts
    // -----------------------------------------------------------------------
    //
    // The `remap` function walks every mapping in the outer map and traces it
    // back through the inner map. The result maps directly from the minified
    // output to the original TypeScript source.

    println!("--- Step 3: Compose with remap() ---\n");

    let composed = remap(&outer_map, |source| {
        if source == "intermediate.js" {
            Some(SourceMap::from_json(&inner_json).unwrap())
        } else {
            None
        }
    });

    println!(
        "  Composed map: {} mappings across {} lines",
        composed.mapping_count(),
        composed.line_count(),
    );
    println!("  Sources: {:?}", composed.sources);
    println!("  Names: {:?}", composed.names);

    // The intermediate file is eliminated -- the composed map points directly
    // to original.ts.
    assert_eq!(composed.sources, vec!["original.ts"]);
    assert!(
        !composed.sources.contains(&"intermediate.js".to_string()),
        "intermediate.js should be eliminated by composition",
    );

    // -----------------------------------------------------------------------
    // 4. Verify composed mappings trace back to original.ts
    // -----------------------------------------------------------------------

    println!("\n--- Step 4: Verify composed lookups ---\n");

    // `function` keyword at minified col 0 -> original.ts line 0, col 0
    let loc = composed.original_position_for(0, 0).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 0);
    assert_eq!(loc.column, 0);
    println!("  output.min.js 0:0  -> original.ts {}:{}", loc.line, loc.column);

    // `greet` at minified col 9 -> original.ts line 0, col 9
    let loc = composed.original_position_for(0, 9).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 0);
    assert_eq!(loc.column, 9);
    println!("  output.min.js 0:9  -> original.ts {}:{}", loc.line, loc.column);

    // `n` (was `name`) at minified col 15 -> original.ts line 0, col 15
    let loc = composed.original_position_for(0, 15).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 0);
    assert_eq!(loc.column, 15);
    // Name should trace back through: outer says "name", inner also has "name" at that position
    println!(
        "  output.min.js 0:15 -> original.ts {}:{} (name: {:?})",
        loc.line,
        loc.column,
        loc.name.map(|n| composed.name(n)),
    );

    // `m` (was `message`) at minified col 22 -> original.ts line 1, col 10
    let loc = composed.original_position_for(0, 22).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 10);
    println!(
        "  output.min.js 0:22 -> original.ts {}:{} (name: {:?})",
        loc.line,
        loc.column,
        loc.name.map(|n| composed.name(n)),
    );

    // `return` at minified col 39 -> original.ts line 2, col 4
    let loc = composed.original_position_for(0, 39).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 2);
    assert_eq!(loc.column, 4);
    println!("  output.min.js 0:39 -> original.ts {}:{}", loc.line, loc.column);

    // `}` at minified col 47 -> original.ts line 3, col 0
    let loc = composed.original_position_for(0, 47).unwrap();
    assert_eq!(composed.source(loc.source), "original.ts");
    assert_eq!(loc.line, 3);
    assert_eq!(loc.column, 0);
    println!("  output.min.js 0:47 -> original.ts {}:{}", loc.line, loc.column);

    // sourcesContent should be preserved from the inner map
    assert_eq!(composed.sources_content.len(), 1);
    assert!(
        composed.sources_content[0].is_some(),
        "sourcesContent for original.ts should be preserved through composition",
    );
    println!("\n  sourcesContent preserved: yes");

    // -----------------------------------------------------------------------
    // 5. Same composition with remap_streaming()
    // -----------------------------------------------------------------------
    //
    // `remap_streaming` avoids parsing the outer map into a full SourceMap.
    // Instead, it takes pre-parsed metadata and a MappingsIter that lazily
    // decodes VLQ segments. Uses StreamingGenerator internally for on-the-fly
    // VLQ encoding.
    //
    // The result should be identical to the non-streaming version.

    println!("\n--- Step 5: Compose with remap_streaming() ---\n");

    let composed_streaming = remap_streaming(
        MappingsIter::new(&outer_map.encode_mappings()),
        &outer_map.sources,
        &outer_map.names,
        &outer_map.sources_content,
        &outer_map.ignore_list,
        outer_map.file.clone(),
        |source| {
            if source == "intermediate.js" {
                Some(SourceMap::from_json(&inner_json).unwrap())
            } else {
                None
            }
        },
    );

    println!(
        "  Streaming composed map: {} mappings across {} lines",
        composed_streaming.mapping_count(),
        composed_streaming.line_count(),
    );
    println!("  Sources: {:?}", composed_streaming.sources);

    // Verify sources match
    assert_eq!(
        composed_streaming.sources, composed.sources,
        "streaming and non-streaming should produce the same sources",
    );

    // Verify key lookups produce the same results
    let lookups: &[(u32, u32)] = &[(0, 0), (0, 9), (0, 15), (0, 22), (0, 39), (0, 47)];

    for &(line, col) in lookups {
        let a = composed.original_position_for(line, col).unwrap();
        let b = composed_streaming.original_position_for(line, col).unwrap();

        assert_eq!(composed.source(a.source), composed_streaming.source(b.source));
        assert_eq!(a.line, b.line, "line mismatch for lookup ({line}, {col})");
        assert_eq!(a.column, b.column, "column mismatch for lookup ({line}, {col})");

        println!(
            "  ({line},{col}): remap -> {}:{}  |  streaming -> {}:{}  (match)",
            a.line, a.column, b.line, b.column,
        );
    }

    println!("\nAll assertions passed.");
}
