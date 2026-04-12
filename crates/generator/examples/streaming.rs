//! Demonstrates `StreamingGenerator` — optimal for bundlers and minifiers
//! where mappings are emitted in sorted order.
//!
//! `StreamingGenerator` encodes VLQ on-the-fly as mappings are added, avoiding
//! the intermediate `Vec<Mapping>` that `SourceMapGenerator` uses. This means:
//!
//! - No sorting step at finalization (mappings must arrive pre-sorted)
//! - No per-mapping heap allocation for `Mapping` structs
//! - VLQ bytes are written directly into a contiguous buffer
//! - Ideal for single-pass code transformations (minifiers, bundlers)
//!
//! Run with: `cargo run -p srcmap-generator --example streaming`

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_generator::{SourceMapGenerator, StreamingGenerator};

fn main() {
    // ---------------------------------------------------------------
    // Scenario: a minifier processing two source files into one bundle
    // ---------------------------------------------------------------
    //
    // Source file 1 — src/math.ts (original):
    //   Line 0: export function add(a: number, b: number): number {
    //   Line 1:   return a + b;
    //   Line 2: }
    //
    // Source file 2 — src/main.ts (original):
    //   Line 0: import { add } from "./math";
    //   Line 1: const result = add(1, 2);
    //   Line 2: console.log(result);
    //
    // Minified output — dist/bundle.js:
    //   Line 0: function add(a,b){return a+b}const result=add(1,2);console.log(result);

    let math_content = "export function add(a: number, b: number): number {\n  return a + b;\n}";
    let main_content =
        "import { add } from \"./math\";\nconst result = add(1, 2);\nconsole.log(result);";

    // ---------------------------------------------------------------
    // 1. Build source map with StreamingGenerator
    // ---------------------------------------------------------------
    let mut sg = StreamingGenerator::new(Some("dist/bundle.js".to_string()));

    let src_math = sg.add_source("src/math.ts");
    sg.set_source_content(src_math, math_content);

    let src_main = sg.add_source("src/main.ts");
    sg.set_source_content(src_main, main_content);

    let name_add = sg.add_name("add");
    let name_a = sg.add_name("a");
    let name_b = sg.add_name("b");
    let name_result = sg.add_name("result");

    sg.set_source_root("../");
    sg.set_debug_id("deadbeef-1234-5678-9abc-def012345678");

    // All mappings on line 0 of the minified output, in column order.
    // Streaming requires mappings in sorted (generated_line, generated_column) order.

    // `function` — from math.ts line 0 col 7 (skipping `export `)
    sg.add_mapping(0, 0, src_math, 0, 7);
    // `add` — named mapping
    sg.add_named_mapping(0, 9, src_math, 0, 16, name_add);
    // `(` — start of params
    sg.add_mapping(0, 12, src_math, 0, 19);
    // `a` — named param
    sg.add_named_mapping(0, 13, src_math, 0, 20, name_a);
    // `,` — param separator
    sg.add_mapping(0, 14, src_math, 0, 30);
    // `b` — named param
    sg.add_named_mapping(0, 15, src_math, 0, 31, name_b);
    // `)` — end of params (skipping type annotations)
    sg.add_mapping(0, 16, src_math, 0, 41);
    // `{` — function body open
    sg.add_mapping(0, 17, src_math, 0, 51);
    // `return` — from math.ts line 1 col 2
    sg.add_mapping(0, 18, src_math, 1, 2);
    // `a` — return expression
    sg.add_named_mapping(0, 25, src_math, 1, 9, name_a);
    // `+` — operator
    sg.add_mapping(0, 26, src_math, 1, 11);
    // `b` — second operand
    sg.add_named_mapping(0, 27, src_math, 1, 13, name_b);
    // `}` — function body close, maps to math.ts line 2 col 0
    sg.add_mapping(0, 28, src_math, 2, 0);

    // Now the main.ts portion of the minified output (same line 0, higher columns).
    // `const` — from main.ts line 1 col 0
    sg.add_mapping(0, 29, src_main, 1, 0);
    // `result` — named
    sg.add_named_mapping(0, 35, src_main, 1, 6, name_result);
    // `=` — assignment
    sg.add_mapping(0, 41, src_main, 1, 13);
    // `add` — function call, named
    sg.add_named_mapping(0, 42, src_main, 1, 15, name_add);
    // `(` — call open
    sg.add_mapping(0, 45, src_main, 1, 18);
    // `1` — first arg
    sg.add_mapping(0, 46, src_main, 1, 19);
    // `,` — separator
    sg.add_mapping(0, 47, src_main, 1, 20);
    // `2` — second arg
    sg.add_mapping(0, 49, src_main, 1, 22);
    // `)` — call close
    sg.add_mapping(0, 50, src_main, 1, 23);
    // `;` — semicolon
    sg.add_mapping(0, 51, src_main, 1, 24);

    // `console.log(result)` — from main.ts line 2
    sg.add_mapping(0, 52, src_main, 2, 0); // `console`
    sg.add_mapping(0, 59, src_main, 2, 7); // `.`
    sg.add_mapping(0, 60, src_main, 2, 8); // `log`
    sg.add_mapping(0, 63, src_main, 2, 11); // `(`
    sg.add_named_mapping(0, 64, src_main, 2, 12, name_result); // `result`
    sg.add_mapping(0, 70, src_main, 2, 18); // `)`

    // ---------------------------------------------------------------
    // 2. Add a range mapping (ECMA-426)
    // ---------------------------------------------------------------
    // Range mappings indicate that the entire range from this position to the
    // next mapping preserves a 1:1 column correspondence with the original.
    // Useful for sections of code that are copied verbatim (no minification).
    //
    // Here we mark the template literal in a hypothetical second output line
    // as a range mapping — every column in the generated range maps directly
    // to the corresponding column in the original.
    sg.add_range_mapping(1, 0, src_math, 0, 7);

    let streaming_count = sg.mapping_count();
    println!("StreamingGenerator: {} mappings", streaming_count);

    // ---------------------------------------------------------------
    // 3. Get JSON output
    // ---------------------------------------------------------------
    let streaming_json = sg.to_json();
    println!("Streaming JSON ({} bytes):\n{}\n", streaming_json.len(), streaming_json);

    assert!(streaming_json.contains(r#""version":3"#));
    assert!(streaming_json.contains(r#""file":"dist/bundle.js""#));
    assert!(streaming_json.contains(r#""sourceRoot":"../""#));
    assert!(streaming_json.contains(r#""rangeMappings":"#));
    assert!(streaming_json.contains(r#""debugId":"deadbeef-1234-5678-9abc-def012345678""#));

    // ---------------------------------------------------------------
    // 4. Build the same map with SourceMapGenerator for comparison
    // ---------------------------------------------------------------
    let mut smg = SourceMapGenerator::new(Some("dist/bundle.js".to_string()));

    let smg_math = smg.add_source("src/math.ts");
    smg.set_source_content(smg_math, math_content);
    let smg_main = smg.add_source("src/main.ts");
    smg.set_source_content(smg_main, main_content);

    let smg_add = smg.add_name("add");
    let smg_a = smg.add_name("a");
    let smg_b = smg.add_name("b");
    let smg_result = smg.add_name("result");

    smg.set_source_root("../");
    smg.set_debug_id("deadbeef-1234-5678-9abc-def012345678");

    // Same mappings as above (SourceMapGenerator does not require sorted order,
    // but we add them in order anyway for clarity).
    smg.add_mapping(0, 0, smg_math, 0, 7);
    smg.add_named_mapping(0, 9, smg_math, 0, 16, smg_add);
    smg.add_mapping(0, 12, smg_math, 0, 19);
    smg.add_named_mapping(0, 13, smg_math, 0, 20, smg_a);
    smg.add_mapping(0, 14, smg_math, 0, 30);
    smg.add_named_mapping(0, 15, smg_math, 0, 31, smg_b);
    smg.add_mapping(0, 16, smg_math, 0, 41);
    smg.add_mapping(0, 17, smg_math, 0, 51);
    smg.add_mapping(0, 18, smg_math, 1, 2);
    smg.add_named_mapping(0, 25, smg_math, 1, 9, smg_a);
    smg.add_mapping(0, 26, smg_math, 1, 11);
    smg.add_named_mapping(0, 27, smg_math, 1, 13, smg_b);
    smg.add_mapping(0, 28, smg_math, 2, 0);
    smg.add_mapping(0, 29, smg_main, 1, 0);
    smg.add_named_mapping(0, 35, smg_main, 1, 6, smg_result);
    smg.add_mapping(0, 41, smg_main, 1, 13);
    smg.add_named_mapping(0, 42, smg_main, 1, 15, smg_add);
    smg.add_mapping(0, 45, smg_main, 1, 18);
    smg.add_mapping(0, 46, smg_main, 1, 19);
    smg.add_mapping(0, 47, smg_main, 1, 20);
    smg.add_mapping(0, 49, smg_main, 1, 22);
    smg.add_mapping(0, 50, smg_main, 1, 23);
    smg.add_mapping(0, 51, smg_main, 1, 24);
    smg.add_mapping(0, 52, smg_main, 2, 0);
    smg.add_mapping(0, 59, smg_main, 2, 7);
    smg.add_mapping(0, 60, smg_main, 2, 8);
    smg.add_mapping(0, 63, smg_main, 2, 11);
    smg.add_named_mapping(0, 64, smg_main, 2, 12, smg_result);
    smg.add_mapping(0, 70, smg_main, 2, 18);
    smg.add_range_mapping(1, 0, smg_math, 0, 7);

    let generator_json = smg.to_json();
    println!("SourceMapGenerator JSON ({} bytes):\n{}\n", generator_json.len(), generator_json);

    // ---------------------------------------------------------------
    // 5. Parse both and verify they produce the same results
    // ---------------------------------------------------------------
    let sm_streaming = srcmap_sourcemap::SourceMap::from_json(&streaming_json).unwrap();
    let sm_generator = srcmap_sourcemap::SourceMap::from_json(&generator_json).unwrap();

    // Same mapping count (excluding the range mapping which is encoded separately)
    assert_eq!(smg.mapping_count(), streaming_count);
    println!(
        "Mapping counts match: streaming={}, generator={}",
        streaming_count,
        smg.mapping_count(),
    );

    // Verify identical lookups at several positions.
    let test_positions: &[(u32, u32)] = &[
        (0, 0),  // `function`
        (0, 9),  // `add`
        (0, 13), // `a` param
        (0, 18), // `return`
        (0, 35), // `result`
        (0, 42), // `add` call
        (0, 64), // `result` in console.log
    ];

    for &(line, col) in test_positions {
        let loc_s = sm_streaming.original_position_for(line, col);
        let loc_g = sm_generator.original_position_for(line, col);

        match (&loc_s, &loc_g) {
            (Some(s), Some(g)) => {
                assert_eq!(
                    sm_streaming.source(s.source),
                    sm_generator.source(g.source),
                    "Source mismatch at gen({line},{col})"
                );
                assert_eq!(s.line, g.line, "Line mismatch at gen({line},{col})");
                assert_eq!(s.column, g.column, "Column mismatch at gen({line},{col})");
                assert_eq!(
                    s.name.map(|n| sm_streaming.name(n)),
                    g.name.map(|n| sm_generator.name(n)),
                    "Name mismatch at gen({line},{col})"
                );
                println!(
                    "  gen({line},{col}) -> original({},{}) source={} name={:?}  OK",
                    s.line,
                    s.column,
                    sm_streaming.source(s.source),
                    s.name.map(|n| sm_streaming.name(n)),
                );
            }
            (None, None) => {
                println!("  gen({line},{col}) -> None  OK");
            }
            _ => {
                panic!("Mismatch at gen({line},{col}): streaming={loc_s:?}, generator={loc_g:?}");
            }
        }
    }

    // ---------------------------------------------------------------
    // 6. Also verify via to_decoded_map (avoids JSON round-trip)
    // ---------------------------------------------------------------
    let decoded_streaming = sg.to_decoded_map().expect("StreamingGenerator::to_decoded_map failed");
    let decoded_generator = smg.to_decoded_map();

    // Spot-check a lookup on the decoded maps
    let loc_s = decoded_streaming.original_position_for(0, 42).unwrap();
    let loc_g = decoded_generator.original_position_for(0, 42).unwrap();
    assert_eq!(loc_s.line, loc_g.line);
    assert_eq!(loc_s.column, loc_g.column);
    assert_eq!(
        loc_s.name.map(|n| decoded_streaming.name(n)),
        loc_g.name.map(|n| decoded_generator.name(n)),
    );
    println!(
        "\nDecoded map spot-check at gen(0,42): original({},{}) name={:?}  OK",
        loc_s.line,
        loc_s.column,
        loc_s.name.map(|n| decoded_streaming.name(n)),
    );

    // ---------------------------------------------------------------
    // Why StreamingGenerator is faster:
    //
    // SourceMapGenerator stores every mapping in a Vec<Mapping> (28 bytes each),
    // then sorts the entire vector and VLQ-encodes in a separate pass at
    // finalization. For N mappings this is O(N log N) + O(N) allocation.
    //
    // StreamingGenerator skips the intermediate storage entirely — each
    // add_mapping call immediately VLQ-encodes the delta into a byte buffer.
    // No Mapping structs are allocated, no sorting is needed (the caller
    // guarantees order), and the VLQ buffer is written sequentially, which
    // is cache-friendly. Total cost: O(N) with minimal allocation.
    //
    // The tradeoff: the caller must provide mappings in sorted order
    // (generated_line, generated_column). This is natural for single-pass
    // code generators and minifiers that process the output left-to-right.
    // ---------------------------------------------------------------

    println!("\nAll assertions passed.");
}
