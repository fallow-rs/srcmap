//! Simulates a TypeScript-to-JavaScript compiler emitting a source map.
//!
//! Demonstrates the full `SourceMapGenerator` workflow: registering sources,
//! setting source content, adding names, mapping tokens, deduplication via
//! `maybe_add_mapping`, ignore lists, and round-trip verification.
//!
//! Run with: `cargo run -p srcmap-generator --example compiler_output`

use srcmap_generator::SourceMapGenerator;

fn main() {
    // -- Original TypeScript source (src/app.ts) --
    //
    //  Line 0: import { log } from "./utils";
    //  Line 1: export function greet(name: string): string {
    //  Line 2:   const message = `Hello, ${name}!`;
    //  Line 3:   log(message);
    //  Line 4:   return message;
    //  Line 5: }
    let ts_source = r#"import { log } from "./utils";
export function greet(name: string): string {
  const message = `Hello, ${name}!`;
  log(message);
  return message;
}"#;

    // -- Generated JavaScript (dist/app.js) --
    //
    //  Line 0: import { log } from "./utils";
    //  Line 1: function greet(name) {
    //  Line 2:   const message = `Hello, ${name}!`;
    //  Line 3:   log(message);
    //  Line 4:   return message;
    //  Line 5: }
    //  Line 6: export { greet };

    // ---------------------------------------------------------------
    // 1. Create generator
    // ---------------------------------------------------------------
    let mut builder = SourceMapGenerator::new(Some("dist/app.js".to_string()));

    // ---------------------------------------------------------------
    // 2. Register sources and set source content
    // ---------------------------------------------------------------
    let src_app = builder.add_source("src/app.ts");
    builder.set_source_content(src_app, ts_source);

    // Verify deduplication: adding the same source returns the same index.
    assert_eq!(builder.add_source("src/app.ts"), src_app);

    // ---------------------------------------------------------------
    // 3. Register a node_modules helper and mark it as ignored
    // ---------------------------------------------------------------
    let src_helper = builder.add_source("node_modules/tslib/tslib.es6.js");
    builder.add_to_ignore_list(src_helper);

    // ---------------------------------------------------------------
    // 4. Register names
    // ---------------------------------------------------------------
    let name_greet = builder.add_name("greet");
    let name_name = builder.add_name("name");
    let name_message = builder.add_name("message");
    let name_log = builder.add_name("log");

    // ---------------------------------------------------------------
    // 5. Add mappings (all positions are 0-based per ECMA-426)
    // ---------------------------------------------------------------

    // Generated line 0: `import { log } from "./utils";`
    // Maps 1:1 to original line 0.
    builder.add_mapping(0, 0, src_app, 0, 0); // `import`
    builder.add_mapping(0, 7, src_app, 0, 7); // `{`
    builder.add_named_mapping(0, 9, src_app, 0, 9, name_log); // `log`
    builder.add_mapping(0, 13, src_app, 0, 13); // `}`
    builder.add_mapping(0, 15, src_app, 0, 15); // `from`
    builder.add_mapping(0, 20, src_app, 0, 20); // `"./utils"`

    // Generated line 1: `function greet(name) {`
    // Original line 1: `export function greet(name: string): string {`
    // The `export` keyword is removed, so the function starts at column 0.
    builder.add_mapping(1, 0, src_app, 1, 7); // `function`
    builder.add_named_mapping(1, 9, src_app, 1, 16, name_greet); // `greet`
    builder.add_mapping(1, 14, src_app, 1, 21); // `(`
    builder.add_named_mapping(1, 15, src_app, 1, 22, name_name); // `name`
    builder.add_mapping(1, 19, src_app, 1, 35); // `)` — skipping `: string): string`
    builder.add_mapping(1, 21, src_app, 1, 45); // `{`

    // Generated line 2: `  const message = \`Hello, ${name}!\`;`
    // Original line 2: `  const message = \`Hello, ${name}!\`;`
    builder.add_mapping(2, 2, src_app, 2, 2); // `const`
    builder.add_named_mapping(2, 8, src_app, 2, 8, name_message); // `message`
    builder.add_mapping(2, 16, src_app, 2, 16); // `=`
    builder.add_mapping(2, 18, src_app, 2, 18); // template literal start
    builder.add_named_mapping(2, 30, src_app, 2, 30, name_name); // `name` inside template

    // Generated line 3: `  log(message);`
    // Original line 3: `  log(message);`
    builder.add_named_mapping(3, 2, src_app, 3, 2, name_log); // `log`
    builder.add_mapping(3, 5, src_app, 3, 5); // `(`
    builder.add_named_mapping(3, 6, src_app, 3, 6, name_message); // `message`
    builder.add_mapping(3, 13, src_app, 3, 13); // `)`

    // Generated line 4: `  return message;`
    // Original line 4: `  return message;`
    builder.add_mapping(4, 2, src_app, 4, 2); // `return`
    builder.add_named_mapping(4, 9, src_app, 4, 9, name_message); // `message`

    // Generated line 5: `}`
    // Original line 5: `}`
    builder.add_mapping(5, 0, src_app, 5, 0);

    // Generated line 6: `export { greet };`
    // This is a synthetic re-export — maps back to the original export on line 1.
    builder.add_mapping(6, 0, src_app, 1, 0); // `export`
    builder.add_named_mapping(6, 9, src_app, 1, 16, name_greet); // `greet`

    // ---------------------------------------------------------------
    // 6. Demonstrate maybe_add_mapping (deduplication)
    // ---------------------------------------------------------------
    let count_before = builder.mapping_count();

    // This mapping is identical to the last mapping on line 6 (same source position),
    // so maybe_add_mapping should skip it and return false.
    let added = builder.maybe_add_mapping(6, 15, src_app, 1, 16);
    assert!(!added, "Duplicate mapping should have been skipped");
    assert_eq!(builder.mapping_count(), count_before);

    // This mapping differs in original column, so it should be added.
    let added = builder.maybe_add_mapping(6, 15, src_app, 1, 0);
    assert!(added, "Distinct mapping should have been added");
    assert_eq!(builder.mapping_count(), count_before + 1);

    // ---------------------------------------------------------------
    // 7. Set source root and debug ID
    // ---------------------------------------------------------------
    builder.set_source_root("../");
    builder.set_debug_id("a1b2c3d4-e5f6-7890-abcd-ef1234567890");

    // ---------------------------------------------------------------
    // 8. Serialize to JSON
    // ---------------------------------------------------------------
    let json = builder.to_json();

    println!("Generated source map ({} bytes):\n", json.len());
    println!("{json}\n");

    // Basic JSON structure checks
    assert!(json.contains(r#""version":3"#));
    assert!(json.contains(r#""file":"dist/app.js""#));
    assert!(json.contains(r#""sourceRoot":"../""#));
    assert!(json.contains(r#""sources":["src/app.ts""#));
    assert!(json.contains(r#""sourcesContent":["#));
    assert!(json.contains(r#""names":["greet","name","message","log"]"#));
    assert!(json.contains(r#""ignoreList":[1]"#));
    assert!(json.contains(r#""debugId":"a1b2c3d4-e5f6-7890-abcd-ef1234567890""#));

    println!("Total mappings: {}\n", builder.mapping_count());

    // ---------------------------------------------------------------
    // 9. Verify by parsing and doing lookups
    // ---------------------------------------------------------------
    let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();

    // Look up generated line 1, column 9 — should map to `greet` in src/app.ts
    // (source_root "../" is prepended to the source path during parsing)
    let loc = sm.original_position_for(1, 9).unwrap();
    assert_eq!(sm.source(loc.source), "../src/app.ts");
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 16);
    assert_eq!(loc.name.map(|n| sm.name(n)), Some("greet"));
    println!(
        "Lookup gen(1,9) -> original({},{}) source={} name={:?}",
        loc.line,
        loc.column,
        sm.source(loc.source),
        loc.name.map(|n| sm.name(n))
    );

    // Look up generated line 3, column 2 — should map to `log` in src/app.ts
    let loc = sm.original_position_for(3, 2).unwrap();
    assert_eq!(sm.source(loc.source), "../src/app.ts");
    assert_eq!(loc.line, 3);
    assert_eq!(loc.column, 2);
    assert_eq!(loc.name.map(|n| sm.name(n)), Some("log"));
    println!(
        "Lookup gen(3,2) -> original({},{}) source={} name={:?}",
        loc.line,
        loc.column,
        sm.source(loc.source),
        loc.name.map(|n| sm.name(n))
    );

    // Look up generated line 4, column 9 — should map to `message` in src/app.ts
    let loc = sm.original_position_for(4, 9).unwrap();
    assert_eq!(sm.source(loc.source), "../src/app.ts");
    assert_eq!(loc.line, 4);
    assert_eq!(loc.column, 9);
    assert_eq!(loc.name.map(|n| sm.name(n)), Some("message"));
    println!(
        "Lookup gen(4,9) -> original({},{}) source={} name={:?}",
        loc.line,
        loc.column,
        sm.source(loc.source),
        loc.name.map(|n| sm.name(n))
    );

    // ---------------------------------------------------------------
    // 10. Also verify via to_decoded_map (skip JSON round-trip)
    // ---------------------------------------------------------------
    let decoded = builder.to_decoded_map();
    let loc = decoded.original_position_for(1, 15).unwrap();
    assert_eq!(loc.line, 1);
    assert_eq!(loc.column, 22);
    assert_eq!(loc.name.map(|n| decoded.name(n)), Some("name"));
    println!(
        "Decoded map lookup gen(1,15) -> original({},{}) name={:?}",
        loc.line,
        loc.column,
        loc.name.map(|n| decoded.name(n))
    );

    println!("\nAll assertions passed.");
}
