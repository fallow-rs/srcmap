//! Source map consumer example — dev tools error position resolution.
//!
//! Demonstrates the full `SourceMap` API: parsing, forward/reverse lookups,
//! bias-controlled search, range mapping, iteration, and serialization.
//!
//! Run with: `cargo run -p srcmap-sourcemap --example consumer`

use srcmap_sourcemap::{Bias, SourceMap};

/// Source map JSON representing a two-file TypeScript bundle.
///
/// Generated output (`bundle.js`):
/// ```text
///   0: "use strict";
///   1: function greet(name) {
///   2:   var msg = formatName(name);
///   3:   console.log(msg);
///   4: }
///   5:
///   6: function formatName(n) {
///   7:   return "Hello, " + n;
///   8: }
/// ```
///
/// Original sources:
///   - `src/app.ts`   — greet function (lines 0-4)
///   - `src/utils.ts` — formatName utility (lines 0-4)
const SOURCE_MAP_JSON: &str = r#"{
    "version": 3,
    "file": "bundle.js",
    "sources": ["src/app.ts", "src/utils.ts"],
    "sourcesContent": [
        "\"use strict\";\nfunction greet(name) {\n    const msg = formatName(name);\n    console.log(msg);\n}\n",
        "// Utility functions\n\nexport function formatName(n: string): string {\n    return \"Hello, \" + n;\n}\n"
    ],
    "names": ["greet", "formatName", "console", "log"],
    "mappings": "AAAA;AACAA,SAASA,MAAM;EACX,UCFJC,WAAW;EDGPC,QAAQC,IAAI;AAChB;;ACFAF,SAAgBA;EACZ,oBAAoB;AACxB",
    "ignoreList": [],
    "debugId": "12345678-1234-1234-1234-123456789abc"
}"#;

fn main() {
    // ── 1. Parse ──────────────────────────────────────────────────
    let sm = SourceMap::from_json(SOURCE_MAP_JSON).expect("valid source map");

    println!(
        "Parsed source map for {:?}",
        sm.file.as_deref().unwrap_or("<unknown>")
    );
    println!("  Sources: {:?}", sm.sources);
    println!("  Names:   {:?}", sm.names);
    println!("  Lines:   {}", sm.line_count());
    println!("  Mappings: {}", sm.mapping_count());
    println!("  Debug ID: {:?}", sm.debug_id);
    println!();

    assert_eq!(sm.sources.len(), 2);
    assert_eq!(sm.names.len(), 4);
    assert!(sm.line_count() >= 9);
    assert!(sm.mapping_count() > 0);

    // ── 2. Forward lookup: generated → original ──────────────────
    // Scenario: a stack trace says "bundle.js line 1, col 9".
    // We want to find the original position.
    println!("Forward lookup: generated(1, 9) → original");
    let loc = sm
        .original_position_for(1, 9)
        .expect("mapping should exist");

    println!(
        "  → {}:{}:{} (name: {:?})",
        sm.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| sm.name(n))
    );
    assert_eq!(sm.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 9);
    assert_eq!(loc.name.map(|n| sm.name(n)), Some("greet"));
    println!();

    // Another forward lookup: "console" at generated line 3, col 2
    println!("Forward lookup: generated(3, 2) → original");
    let loc = sm
        .original_position_for(3, 2)
        .expect("mapping should exist");

    println!(
        "  → {}:{}:{} (name: {:?})",
        sm.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| sm.name(n))
    );
    assert_eq!(sm.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 3);
    assert_eq!(loc.column, 4);
    assert_eq!(loc.name.map(|n| sm.name(n)), Some("console"));
    println!();

    // ── 3. Reverse lookup: original → generated ─────────────────
    // Scenario: set a breakpoint in src/app.ts at line 3, col 4.
    // Find the generated position for it.
    println!("Reverse lookup: src/app.ts:3:4 → generated");
    let gen_loc = sm
        .generated_position_for("src/app.ts", 3, 4)
        .expect("reverse mapping should exist");

    println!("  → bundle.js:{}:{}", gen_loc.line, gen_loc.column);
    assert_eq!(gen_loc.line, 3);
    assert_eq!(gen_loc.column, 2);
    println!();

    // ── 4. Bias-controlled lookups ──────────────────────────────
    // On line 1, mappings exist at columns 0, 9, and 15.
    // Query column 12 — between "greet" (col 9) and "name" (col 15).

    // GreatestLowerBound: snap to the closest mapping at or before col 12 → col 9
    println!("Bias lookup: generated(1, 12) with GreatestLowerBound");
    let glb = sm
        .original_position_for_with_bias(1, 12, Bias::GreatestLowerBound)
        .expect("GLB should find mapping");
    println!("  → original col {} (snapped left to col 9)", glb.column);
    assert_eq!(glb.column, 9);

    // LeastUpperBound: snap to the closest mapping at or after col 12 → col 15
    println!("Bias lookup: generated(1, 12) with LeastUpperBound");
    let lub = sm
        .original_position_for_with_bias(1, 12, Bias::LeastUpperBound)
        .expect("LUB should find mapping");
    println!("  → original col {} (snapped right to col 15)", lub.column);
    assert_eq!(lub.column, 15);
    println!();

    // ── 5. All generated positions ──────────────────────────────
    // Find every generated position that maps to src/app.ts:1:0
    // (the "greet" function declaration)
    println!("All generated positions for src/app.ts:1:0");
    let all_positions = sm.all_generated_positions_for("src/app.ts", 1, 0);
    for pos in &all_positions {
        println!("  → generated({}:{})", pos.line, pos.column);
    }
    assert!(!all_positions.is_empty());
    assert_eq!(all_positions[0].line, 1);
    assert_eq!(all_positions[0].column, 0);
    println!();

    // ── 6. Range mapping ────────────────────────────────────────
    // Map a generated range back to the original range.
    // Generated line 3, cols 2..14 covers "console.log(" in the output.
    println!("Range mapping: generated(3:2 → 3:14)");
    if let Some(range) = sm.map_range(3, 2, 3, 14) {
        println!(
            "  → {}:{}:{} → {}:{}",
            sm.source(range.source),
            range.original_start_line,
            range.original_start_column,
            range.original_end_line,
            range.original_end_column,
        );
        assert_eq!(sm.source(range.source), "src/app.ts");
    } else {
        println!("  (endpoints mapped to different sources — not a valid range)");
    }
    println!();

    // ── 7. Iterate mappings for a specific line ─────────────────
    println!("Mappings on generated line 2:");
    let line_mappings = sm.mappings_for_line(2);
    for m in line_mappings {
        if m.source != u32::MAX {
            let name_str = if m.name != u32::MAX {
                sm.name(m.name)
            } else {
                "(none)"
            };
            println!(
                "  gen_col={} → {}:{}:{} name={}",
                m.generated_column,
                sm.source(m.source),
                m.original_line,
                m.original_column,
                name_str,
            );
        }
    }
    assert!(!sm.mappings_for_line(2).is_empty());
    println!();

    // ── 8. Metadata accessors ───────────────────────────────────
    // source() and name() resolve indices to strings
    assert_eq!(sm.source(0), "src/app.ts");
    assert_eq!(sm.source(1), "src/utils.ts");
    assert_eq!(sm.name(0), "greet");
    assert_eq!(sm.name(3), "log");

    // source_index() looks up the index by filename
    assert_eq!(sm.source_index("src/app.ts"), Some(0));
    assert_eq!(sm.source_index("src/utils.ts"), Some(1));
    assert_eq!(sm.source_index("nonexistent.js"), None);

    // Sources content
    assert!(sm.sources_content[0].is_some());
    assert!(sm.sources_content[1].is_some());
    println!("Source content for src/app.ts (first 40 chars):");
    println!(
        "  {:?}...",
        &sm.sources_content[0].as_deref().unwrap_or("")[..40]
    );
    println!();

    // ── 9. All mappings iteration ───────────────────────────────
    println!("Total mappings: {}", sm.all_mappings().len());
    assert_eq!(sm.all_mappings().len(), sm.mapping_count());
    println!();

    // ── 10. Serialize back to JSON ──────────────────────────────
    let output_json = sm.to_json();
    println!("Serialized JSON length: {} bytes", output_json.len());

    // Verify round-trip: parse the serialized output and check a lookup
    let sm2 = SourceMap::from_json(&output_json).expect("round-trip should parse");
    let loc2 = sm2
        .original_position_for(1, 9)
        .expect("round-trip lookup should work");
    assert_eq!(loc2.line, 1);
    assert_eq!(loc2.column, 9);
    assert_eq!(sm2.source(loc2.source), "src/app.ts");

    println!("Round-trip verification passed.");
    println!();
    println!("All assertions passed.");
}
