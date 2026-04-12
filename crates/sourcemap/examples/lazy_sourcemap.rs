//! Lazy source map example — on-demand parsing for large source maps.
//!
//! `LazySourceMap` defers VLQ decoding until a specific line is accessed.
//! This is ideal for error monitoring services (like Sentry) that ingest
//! millions of source maps but only need to resolve a handful of positions
//! per map. Decoding the full mappings string would waste CPU and memory.
//!
//! Run with: `cargo run -p srcmap-sourcemap --example lazy_sourcemap`

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_sourcemap::{LazySourceMap, SourceMap};

/// Same source map as the consumer example — a two-file TypeScript bundle.
///
/// In production this would be a multi-megabyte mappings string from a
/// webpack/esbuild/Rollup build. The lazy parser pre-scans semicolons
/// to index line boundaries but skips VLQ decoding entirely.
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
    // ── 1. Parse lazily ─────────────────────────────────────────
    // from_json parses JSON metadata eagerly (sources, names, etc.)
    // but only pre-scans the mappings string for semicolons — no VLQ
    // decoding happens yet.
    let lazy = LazySourceMap::from_json(SOURCE_MAP_JSON).expect("valid source map");

    println!("Lazy-parsed source map for {:?}", lazy.file.as_deref().unwrap_or("<unknown>"));
    println!("  Sources: {:?}", lazy.sources);
    println!("  Names:   {:?}", lazy.names);
    println!("  Lines:   {}", lazy.line_count());
    println!("  (mappings NOT decoded yet — only line boundaries scanned)");
    println!();

    assert_eq!(lazy.sources.len(), 2);
    assert_eq!(lazy.names.len(), 4);
    assert!(lazy.line_count() >= 9);

    // ── 2. Forward lookup (triggers lazy decode) ────────────────
    // Only the requested line's VLQ segment is decoded.
    // Scenario: resolve a stack frame at bundle.js line 1, col 9.
    println!("Lazy lookup: generated(1, 9) → original");
    let loc = lazy.original_position_for(1, 9).expect("mapping should exist");

    println!(
        "  → {}:{}:{} (name: {:?})",
        lazy.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| lazy.name(n))
    );
    assert_eq!(lazy.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 9);
    assert_eq!(loc.name.map(|n| lazy.name(n)), Some("greet"));
    println!();

    // A second lookup on the same line hits the cache — no re-decoding.
    println!("Cached lookup: generated(1, 0) → original");
    let loc2 = lazy.original_position_for(1, 0).expect("mapping should exist");

    println!("  → {}:{}:{}", lazy.source(loc2.source), loc2.line, loc2.column,);
    assert_eq!(lazy.source(loc2.source), "src/app.ts");
    assert_eq!(loc2.line, 1);
    assert_eq!(loc2.column, 0);
    println!();

    // Lookup on a different line — decodes only that line.
    println!("Lazy lookup: generated(3, 2) → original");
    let loc3 = lazy.original_position_for(3, 2).expect("mapping should exist");

    println!(
        "  → {}:{}:{} (name: {:?})",
        lazy.source(loc3.source),
        loc3.line,
        loc3.column,
        loc3.name.map(|n| lazy.name(n))
    );
    assert_eq!(lazy.source(loc3.source), "src/app.ts");
    assert_eq!(loc3.line, 3);
    assert_eq!(loc3.column, 4);
    assert_eq!(loc3.name.map(|n| lazy.name(n)), Some("console"));
    println!();

    // ── 3. Decode a specific line ───────────────────────────────
    // decode_line returns all mappings for a line as a Vec<Mapping>.
    println!("Explicit decode_line(2):");
    let line2_mappings = lazy.decode_line(2).expect("line 2 should decode");
    for m in &line2_mappings {
        if m.source != u32::MAX {
            let name_str = if m.name != u32::MAX { lazy.name(m.name) } else { "(none)" };
            println!(
                "  gen_col={} → {}:{}:{} name={}",
                m.generated_column,
                lazy.source(m.source),
                m.original_line,
                m.original_column,
                name_str,
            );
        }
    }
    assert!(!line2_mappings.is_empty());
    println!();

    // ── 4. mappings_for_line ────────────────────────────────────
    // Convenience method that returns an empty Vec if the line is out of bounds.
    println!("mappings_for_line(0):");
    let line0 = lazy.mappings_for_line(0);
    for m in &line0 {
        println!(
            "  gen_col={} → {}:{}:{}",
            m.generated_column,
            lazy.source(m.source),
            m.original_line,
            m.original_column,
        );
    }
    assert!(!line0.is_empty());
    println!();

    // Out-of-bounds line returns empty
    let empty = lazy.mappings_for_line(9999);
    assert!(empty.is_empty());

    // ── 5. Metadata accessors ───────────────────────────────────
    assert_eq!(lazy.source(0), "src/app.ts");
    assert_eq!(lazy.source(1), "src/utils.ts");
    assert_eq!(lazy.name(0), "greet");
    assert_eq!(lazy.name(1), "formatName");

    assert_eq!(lazy.source_index("src/app.ts"), Some(0));
    assert_eq!(lazy.source_index("src/utils.ts"), Some(1));
    assert_eq!(lazy.source_index("nonexistent.js"), None);

    println!("Metadata assertions passed.");
    println!();

    // ── 6. Convert to full SourceMap ────────────────────────────
    // When you need the complete picture (e.g., re-serialization or
    // bulk iteration), convert with into_sourcemap(). This decodes
    // all remaining lines.
    println!("Converting LazySourceMap → SourceMap (full decode)...");
    let sm: SourceMap = lazy.into_sourcemap().expect("full decode should succeed");

    println!("  Total mappings: {}", sm.mapping_count());
    println!("  Line count: {}", sm.line_count());
    assert!(sm.mapping_count() > 0);
    println!();

    // ── 7. Verify the full SourceMap matches lazy results ───────
    // The same lookups should produce identical results.
    let full_loc = sm.original_position_for(1, 9).expect("full map lookup should work");

    assert_eq!(sm.source(full_loc.source), "src/app.ts");
    assert_eq!(full_loc.line, 1);
    assert_eq!(full_loc.column, 9);
    assert_eq!(full_loc.name.map(|n| sm.name(n)), Some("greet"));

    // Reverse lookup is only available on the full SourceMap
    let gen_loc =
        sm.generated_position_for("src/app.ts", 3, 4).expect("reverse lookup should work");
    assert_eq!(gen_loc.line, 3);
    assert_eq!(gen_loc.column, 2);
    println!("Full SourceMap lookups verified — results match lazy lookups.");

    // Serialize the full map
    let json_out = sm.to_json();
    assert!(!json_out.is_empty());
    println!("Serialized to {} bytes.", json_out.len());

    println!();
    println!("All assertions passed.");
}
