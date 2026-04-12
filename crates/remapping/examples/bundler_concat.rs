//! Bundler concatenation scenario with `ConcatBuilder`.
//!
//! Simulates a bundler that concatenates three separately-compiled modules
//! into a single output file:
//!
//!   bundle.js:
//!     lines 0-2:  header.js   (banner/license comment)
//!     lines 3-7:  app.js      (main application logic)
//!     lines 8-9:  footer.js   (module wrapper close)
//!
//! Each module has its own source map. `ConcatBuilder` merges them with
//! the appropriate line offsets, preserving sourcesContent and ignoreList.
//!
//! This is the **concatenation** use-case: multiple *different* files placed
//! sequentially in a bundle. For chaining transforms on the *same* file
//! (e.g. TS -> JS -> minified), see the `composition` example instead.
//!
//! Run with: cargo run -p srcmap-remapping --example bundler_concat

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_generator::SourceMapGenerator;
use srcmap_remapping::ConcatBuilder;
use srcmap_sourcemap::SourceMap;

fn main() {
    println!("=== Bundler Concatenation ===\n");

    // -----------------------------------------------------------------------
    // 1. Build source maps for three modules
    // -----------------------------------------------------------------------

    // --- header.js: banner/license comment (3 lines) ---
    //
    //   line 0: /**
    //   line 1:  * MIT License - MyApp v1.0.0
    //   line 2:  */
    //
    // This is generated boilerplate, so we mark it as ignored.

    println!("--- Building individual source maps ---\n");

    let mut header_gen = SourceMapGenerator::new(Some("header.js".to_string()));
    let header_src = header_gen.add_source("header.js");

    // One mapping per line to track the banner
    header_gen.add_mapping(0, 0, header_src, 0, 0); // `/**`
    header_gen.add_mapping(1, 0, header_src, 1, 0); // ` * MIT License...`
    header_gen.add_mapping(2, 0, header_src, 2, 0); // ` */`

    // Mark header.js as ignored (it's boilerplate, not authored source)
    header_gen.add_to_ignore_list(header_src);

    let header_json = header_gen.to_json();
    let header_map = SourceMap::from_json(&header_json).unwrap();

    println!(
        "  header.js: {} mappings, {} lines, ignore_list: {:?}",
        header_map.mapping_count(),
        header_map.line_count(),
        header_map.ignore_list,
    );

    // --- app.js: main application (5 lines, multiple mappings) ---
    //
    //   line 0: import { utils } from './utils';
    //   line 1: function main() {
    //   line 2:     const result = utils.process();
    //   line 3:     console.log(result);
    //   line 4: }
    //
    // App has sourcesContent so devtools can show the original code.

    let mut app_gen = SourceMapGenerator::new(Some("app.js".to_string()));
    let app_src = app_gen.add_source("src/app.ts");
    app_gen.set_source_content(
        app_src,
        "import { utils } from './utils';\nfunction main() {\n    const result = utils.process();\n    console.log(result);\n}\n".to_string(),
    );

    let name_main = app_gen.add_name("main");
    let name_result = app_gen.add_name("result");
    let name_utils = app_gen.add_name("utils");

    // line 0: "import { utils } from './utils';"
    app_gen.add_mapping(0, 0, app_src, 0, 0); // `import`
    app_gen.add_named_mapping(0, 9, app_src, 0, 9, name_utils); // `utils`

    // line 1: "function main() {"
    app_gen.add_mapping(1, 0, app_src, 1, 0); // `function`
    app_gen.add_named_mapping(1, 9, app_src, 1, 9, name_main); // `main`

    // line 2: "    const result = utils.process();"
    app_gen.add_mapping(2, 0, app_src, 2, 0); // indent
    app_gen.add_named_mapping(2, 10, app_src, 2, 10, name_result); // `result`
    app_gen.add_named_mapping(2, 19, app_src, 2, 19, name_utils); // `utils`
    app_gen.add_mapping(2, 24, app_src, 2, 24); // `.process()`

    // line 3: "    console.log(result);"
    app_gen.add_mapping(3, 0, app_src, 3, 0); // indent
    app_gen.add_mapping(3, 4, app_src, 3, 4); // `console`
    app_gen.add_named_mapping(3, 16, app_src, 3, 16, name_result); // `result`

    // line 4: "}"
    app_gen.add_mapping(4, 0, app_src, 4, 0); // `}`

    let app_json = app_gen.to_json();
    let app_map = SourceMap::from_json(&app_json).unwrap();

    println!(
        "  app.js:    {} mappings, {} lines, sourcesContent: {}",
        app_map.mapping_count(),
        app_map.line_count(),
        if app_map.sources_content.iter().any(|c| c.is_some()) { "yes" } else { "no" },
    );

    // --- footer.js: module wrapper close (2 lines) ---
    //
    //   line 0: main();
    //   line 1: export default main;

    let mut footer_gen = SourceMapGenerator::new(Some("footer.js".to_string()));
    let footer_src = footer_gen.add_source("footer.js");

    let name_main_f = footer_gen.add_name("main");

    footer_gen.add_named_mapping(0, 0, footer_src, 0, 0, name_main_f); // `main()`
    footer_gen.add_mapping(1, 0, footer_src, 1, 0); // `export`
    footer_gen.add_named_mapping(1, 15, footer_src, 1, 15, name_main_f); // `main`

    let footer_json = footer_gen.to_json();
    let footer_map = SourceMap::from_json(&footer_json).unwrap();

    println!(
        "  footer.js: {} mappings, {} lines",
        footer_map.mapping_count(),
        footer_map.line_count(),
    );

    // -----------------------------------------------------------------------
    // 2. Concatenate with ConcatBuilder
    // -----------------------------------------------------------------------
    //
    // Bundle layout:
    //   lines 0-2:  header.js  (3 lines, starts at offset 0)
    //   lines 3-7:  app.js     (5 lines, starts at offset 3)
    //   lines 8-9:  footer.js  (2 lines, starts at offset 8)

    println!("\n--- Concatenating with ConcatBuilder ---\n");

    let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
    builder.add_map(&header_map, 0); // header starts at line 0
    builder.add_map(&app_map, 3); // app starts at line 3
    builder.add_map(&footer_map, 8); // footer starts at line 8

    let combined = builder.build();
    let combined_json = builder.to_json();

    println!(
        "  Combined map: {} mappings across {} lines",
        combined.mapping_count(),
        combined.line_count(),
    );
    println!("  Sources: {:?}", combined.sources);
    println!("  Names: {:?}", combined.names);
    println!("  Ignore list: {:?}", combined.ignore_list);

    // -----------------------------------------------------------------------
    // 3. Verify the combined map
    // -----------------------------------------------------------------------

    println!("\n--- Verifying combined map ---\n");

    // Total line count: header(3) + app(5) + footer(2) = 10 lines (0-9)
    assert!(
        combined.line_count() >= 10,
        "combined map should cover at least 10 lines, got {}",
        combined.line_count(),
    );
    println!("  Line count: {} (>= 10)", combined.line_count());

    // Total mappings: header(3) + app(12) + footer(3) = 18
    let expected_mappings =
        header_map.mapping_count() + app_map.mapping_count() + footer_map.mapping_count();
    assert_eq!(
        combined.mapping_count(),
        expected_mappings,
        "total mapping count should be sum of individual maps",
    );
    println!(
        "  Mapping count: {} (= {}+{}+{})",
        combined.mapping_count(),
        header_map.mapping_count(),
        app_map.mapping_count(),
        footer_map.mapping_count()
    );

    // Header lookups (lines 0-2) -> header.js
    let loc = combined.original_position_for(0, 0).unwrap();
    assert_eq!(combined.source(loc.source), "header.js");
    assert_eq!(loc.line, 0);
    println!("  bundle.js 0:0  -> {} {}:{}", combined.source(loc.source), loc.line, loc.column);

    let loc = combined.original_position_for(1, 0).unwrap();
    assert_eq!(combined.source(loc.source), "header.js");
    assert_eq!(loc.line, 1);
    println!("  bundle.js 1:0  -> {} {}:{}", combined.source(loc.source), loc.line, loc.column);

    // App lookups (lines 3-7) -> src/app.ts
    let loc = combined.original_position_for(3, 0).unwrap();
    assert_eq!(combined.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 0);
    println!("  bundle.js 3:0  -> {} {}:{}", combined.source(loc.source), loc.line, loc.column);

    let loc = combined.original_position_for(4, 9).unwrap();
    assert_eq!(combined.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 9);
    assert_eq!(loc.name.map(|n| combined.name(n)), Some("main"));
    println!(
        "  bundle.js 4:9  -> {} {}:{} (name: {:?})",
        combined.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| combined.name(n)),
    );

    // App line 5 (bundle) = app line 2 (original), col 10 -> `result`
    let loc = combined.original_position_for(5, 10).unwrap();
    assert_eq!(combined.source(loc.source), "src/app.ts");
    assert_eq!(loc.line, 2);
    assert_eq!(loc.column, 10);
    assert_eq!(loc.name.map(|n| combined.name(n)), Some("result"));
    println!(
        "  bundle.js 5:10 -> {} {}:{} (name: {:?})",
        combined.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| combined.name(n)),
    );

    // Footer lookups (lines 8-9) -> footer.js
    let loc = combined.original_position_for(8, 0).unwrap();
    assert_eq!(combined.source(loc.source), "footer.js");
    assert_eq!(loc.line, 0);
    println!("  bundle.js 8:0  -> {} {}:{}", combined.source(loc.source), loc.line, loc.column);

    let loc = combined.original_position_for(9, 15).unwrap();
    assert_eq!(combined.source(loc.source), "footer.js");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 15);
    println!(
        "  bundle.js 9:15 -> {} {}:{} (name: {:?})",
        combined.source(loc.source),
        loc.line,
        loc.column,
        loc.name.map(|n| combined.name(n)),
    );

    // -----------------------------------------------------------------------
    // 4. Verify sourcesContent is preserved
    // -----------------------------------------------------------------------

    println!("\n--- sourcesContent ---\n");

    // Find the index of src/app.ts in the combined map
    let app_idx = combined
        .sources
        .iter()
        .position(|s| s == "src/app.ts")
        .expect("src/app.ts should be in combined sources");

    assert!(
        combined.sources_content[app_idx].is_some(),
        "sourcesContent for src/app.ts should be preserved",
    );
    println!(
        "  src/app.ts sourcesContent: {} bytes",
        combined.sources_content[app_idx].as_ref().unwrap().len(),
    );

    // Other sources should not have sourcesContent (we didn't set any)
    for (i, source) in combined.sources.iter().enumerate() {
        if i != app_idx {
            assert!(
                combined.sources_content.get(i).is_none() || combined.sources_content[i].is_none(),
                "only src/app.ts should have sourcesContent, but {source} also has it",
            );
        }
    }
    println!("  Other sources have no sourcesContent: correct");

    // -----------------------------------------------------------------------
    // 5. Verify ignoreList propagates
    // -----------------------------------------------------------------------

    println!("\n--- ignoreList ---\n");

    let header_idx = combined
        .sources
        .iter()
        .position(|s| s == "header.js")
        .expect("header.js should be in combined sources") as u32;

    assert!(
        combined.ignore_list.contains(&header_idx),
        "header.js (index {header_idx}) should be in the ignoreList",
    );
    println!("  header.js (index {}) is in ignoreList: yes", header_idx,);

    // app.ts and footer.js should NOT be in the ignore list
    let other_sources: Vec<_> =
        combined.sources.iter().enumerate().filter(|(_, s)| *s != "header.js").collect();
    for (i, source) in &other_sources {
        assert!(
            !combined.ignore_list.contains(&(*i as u32)),
            "{source} should not be in ignoreList",
        );
    }
    println!("  Non-boilerplate sources excluded from ignoreList: correct");

    // -----------------------------------------------------------------------
    // 6. Show JSON output
    // -----------------------------------------------------------------------

    println!("\n--- JSON output ---\n");

    // Pretty-print the JSON to show the structure
    let parsed: serde_json::Value =
        serde_json::from_str(&combined_json).expect("generated JSON should be valid");
    let pretty = serde_json::to_string_pretty(&parsed).unwrap();

    println!("{pretty}");

    println!("\nAll assertions passed.");
}
