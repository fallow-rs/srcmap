//! End-to-end error monitoring scenario.
//!
//! Demonstrates parsing stack traces from multiple browser engines,
//! symbolicating them against source maps, and producing output in
//! both human-readable and JSON formats.
//!
//! # Position conventions
//!
//! Stack traces use **1-based** line and column numbers (browser convention).
//! Source maps use **0-based** positions internally (ECMA-426 spec). The
//! symbolicate functions handle this conversion: they subtract 1 before the
//! source map lookup and add 1 back to the resolved original positions.
//!
//! Run with: cargo run -p srcmap-symbolicate --example error_symbolication

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use std::collections::HashMap;

use srcmap_sourcemap::SourceMap;
use srcmap_symbolicate::{
    parse_stack_trace, parse_stack_trace_full, resolve_by_debug_id, symbolicate, symbolicate_batch,
    to_json,
};

/// Build a source map JSON string by hand.
///
/// This maps a fictitious `bundle.js` back to two original TypeScript sources.
/// The VLQ mappings string is constructed manually — no generator crate needed.
///
/// ## Mapping table
///
/// | Generated (0-based)  | Original file   | Original (0-based) | Name         |
/// |----------------------|-----------------|--------------------|--------------|
/// | line 0, col 0        | src/app.ts      | line 4, col 0      | handleClick  |
/// | line 2, col 9        | src/app.ts      | line 9, col 4      | processData  |
/// | line 4, col 5        | src/utils.ts    | line 2, col 0      | validateInput|
///
/// ## VLQ breakdown
///
/// Each segment encodes [genCol, sourceIdx, origLine, origCol, nameIdx] as deltas.
///
/// Segment 1 (line 0): genCol=0, src=0, origLine=4, origCol=0, name=0
///   -> VLQ: 0=A, 0=A, 8=I (4<<1), 0=A, 0=A => `AAIAA`
///
/// Segment 2 (line 2): genCol=9, src=+0, origLine=+5, origCol=+4, name=+1
///   -> VLQ: 18=S, 0=A, 10=K, 8=I, 2=C => `SAKIC`
///
/// Segment 3 (line 4): genCol=5, src=+1, origLine=-7, origCol=-4, name=+1
///   -> VLQ: 10=K, 2=C, 15=P (-7: 7<<1|1), 9=J (-4: 4<<1|1), 2=C => `KCPJC`
///
/// Lines are separated by `;`. Empty lines (no mappings) produce consecutive semicolons.
fn build_source_map_json() -> String {
    r#"{
  "version": 3,
  "file": "bundle.js",
  "sources": ["src/app.ts", "src/utils.ts"],
  "sourcesContent": [
    "import { validateInput } from './utils';\n\ninterface ClickEvent { target: HTMLElement }\n\nexport const handleClick = (event: ClickEvent): void => {\n  const data = event.target.dataset;\n  processData(data);\n};\n\nconst processData = (data: DOMStringMap): void => {\n  const value = data['key'];\n  if (!value) throw new TypeError(\"Cannot read property 'x' of null\");\n  console.log(value);\n};",
    "export const validateInput = (input: string): boolean => {\n  if (!input) return false;\n  return input.trim().length > 0;\n};"
  ],
  "names": ["handleClick", "processData", "validateInput"],
  "mappings": "AAIAA;;SAKIC;;KCPJC",
  "debugId": "a1b2c3d4-e5f6-7890-abcd-ef1234567890"
}"#
    .to_string()
}

fn main() {
    println!("=== Error Monitoring Symbolication Demo ===\n");

    let source_map_json = build_source_map_json();

    // -----------------------------------------------------------------------
    // 1. Parse a V8-format stack trace (Chrome / Node.js)
    // -----------------------------------------------------------------------
    //
    // V8 format: `at functionName (file:line:column)` with 1-based positions.
    // The error message appears on the first line.

    println!("--- Step 1: Parse V8 stack trace ---\n");

    let v8_stack = "\
TypeError: Cannot read property 'x' of null
    at processData (bundle.js:3:10)
    at handleClick (bundle.js:1:1)";

    let v8_parsed = parse_stack_trace_full(v8_stack);

    println!("  Message: {:?}", v8_parsed.message);
    println!("  Frames:  {}", v8_parsed.frames.len());
    for (i, frame) in v8_parsed.frames.iter().enumerate() {
        println!(
            "    [{i}] {}  {}:{}:{}",
            frame.function_name.as_deref().unwrap_or("<anonymous>"),
            frame.file,
            frame.line,
            frame.column,
        );
    }

    assert_eq!(v8_parsed.message.as_deref(), Some("TypeError: Cannot read property 'x' of null"));
    assert_eq!(v8_parsed.frames.len(), 2);
    assert_eq!(v8_parsed.frames[0].function_name.as_deref(), Some("processData"));
    assert_eq!(v8_parsed.frames[0].file, "bundle.js");
    assert_eq!(v8_parsed.frames[0].line, 3);
    assert_eq!(v8_parsed.frames[0].column, 10);
    assert_eq!(v8_parsed.frames[1].function_name.as_deref(), Some("handleClick"));
    assert_eq!(v8_parsed.frames[1].line, 1);
    assert_eq!(v8_parsed.frames[1].column, 1);

    // -----------------------------------------------------------------------
    // 2. Parse a SpiderMonkey-format stack trace (Firefox)
    // -----------------------------------------------------------------------
    //
    // SpiderMonkey format: `functionName@file:line:column` (no message line).

    println!("\n--- Step 2: Parse SpiderMonkey stack trace ---\n");

    let sm_stack = "\
processData@bundle.js:3:10
handleClick@bundle.js:1:1";

    let sm_frames = parse_stack_trace(sm_stack);

    println!("  Frames: {}", sm_frames.len());
    for (i, frame) in sm_frames.iter().enumerate() {
        println!(
            "    [{i}] {}  {}:{}:{}",
            frame.function_name.as_deref().unwrap_or("<anonymous>"),
            frame.file,
            frame.line,
            frame.column,
        );
    }

    assert_eq!(sm_frames.len(), 2);
    assert_eq!(sm_frames[0].function_name.as_deref(), Some("processData"));
    assert_eq!(sm_frames[0].file, "bundle.js");
    assert_eq!(sm_frames[0].line, 3);
    assert_eq!(sm_frames[0].column, 10);

    // -----------------------------------------------------------------------
    // 3. Verify both formats produce the same parsed frames
    // -----------------------------------------------------------------------

    println!("\n--- Step 3: Compare parsed frames ---\n");

    assert_eq!(v8_parsed.frames.len(), sm_frames.len());
    for (v8_frame, sm_frame) in v8_parsed.frames.iter().zip(sm_frames.iter()) {
        assert_eq!(v8_frame.function_name, sm_frame.function_name);
        assert_eq!(v8_frame.file, sm_frame.file);
        assert_eq!(v8_frame.line, sm_frame.line);
        assert_eq!(v8_frame.column, sm_frame.column);
    }
    println!("  V8 and SpiderMonkey frames match.");

    // -----------------------------------------------------------------------
    // 4. Symbolicate the V8 stack trace
    // -----------------------------------------------------------------------
    //
    // The loader closure receives a filename ("bundle.js") and returns the
    // parsed SourceMap. Internally, the symbolicate function:
    //   1. Subtracts 1 from the 1-based stack trace positions to get 0-based
    //   2. Calls SourceMap::original_position_for(line, column)
    //   3. Adds 1 back to the resolved 0-based original positions

    println!("\n--- Step 4: Symbolicate V8 stack trace ---\n");

    let v8_result = symbolicate(v8_stack, |file| {
        if file == "bundle.js" { SourceMap::from_json(&source_map_json).ok() } else { None }
    });

    println!("  Symbolicated stack (Display):\n");
    // SymbolicatedStack implements Display, printing a V8-style resolved trace
    print!("{v8_result}");

    assert_eq!(v8_result.message.as_deref(), Some("TypeError: Cannot read property 'x' of null"));
    assert_eq!(v8_result.frames.len(), 2);

    // Frame 0: bundle.js:3:10 (1-based) -> 0-based (2, 9) -> src/app.ts (9, 4) -> 1-based (10, 5)
    assert!(v8_result.frames[0].symbolicated);
    assert_eq!(v8_result.frames[0].file, "src/app.ts");
    assert_eq!(v8_result.frames[0].line, 10);
    assert_eq!(v8_result.frames[0].column, 5);
    assert_eq!(v8_result.frames[0].function_name.as_deref(), Some("processData"));

    // Frame 1: bundle.js:1:1 (1-based) -> 0-based (0, 0) -> src/app.ts (4, 0) -> 1-based (5, 1)
    assert!(v8_result.frames[1].symbolicated);
    assert_eq!(v8_result.frames[1].file, "src/app.ts");
    assert_eq!(v8_result.frames[1].line, 5);
    assert_eq!(v8_result.frames[1].column, 1);
    assert_eq!(v8_result.frames[1].function_name.as_deref(), Some("handleClick"));

    // -----------------------------------------------------------------------
    // 5. Symbolicate the SpiderMonkey stack trace
    // -----------------------------------------------------------------------

    println!("\n--- Step 5: Symbolicate SpiderMonkey stack trace ---\n");

    let sm_result = symbolicate(sm_stack, |file| {
        if file == "bundle.js" { SourceMap::from_json(&source_map_json).ok() } else { None }
    });

    print!("{sm_result}");

    // Both engine formats should resolve to the same original positions
    assert_eq!(v8_result.frames.len(), sm_result.frames.len());
    for (v8_frame, sm_frame) in v8_result.frames.iter().zip(sm_result.frames.iter()) {
        assert_eq!(v8_frame.file, sm_frame.file);
        assert_eq!(v8_frame.line, sm_frame.line);
        assert_eq!(v8_frame.column, sm_frame.column);
        assert_eq!(v8_frame.function_name, sm_frame.function_name);
        assert_eq!(v8_frame.symbolicated, sm_frame.symbolicated);
    }
    println!("\n  V8 and SpiderMonkey symbolicated frames match.");

    // -----------------------------------------------------------------------
    // 6. JSON output
    // -----------------------------------------------------------------------

    println!("\n--- Step 6: JSON output ---\n");

    let json_output = to_json(&v8_result);
    println!("{json_output}");

    assert!(json_output.contains("\"symbolicated\": true"));
    assert!(json_output.contains("src/app.ts"));
    assert!(json_output.contains("processData"));
    assert!(json_output.contains("handleClick"));
    assert!(json_output.contains("Cannot read property"));

    // -----------------------------------------------------------------------
    // 7. Batch symbolication
    // -----------------------------------------------------------------------
    //
    // In production error monitoring, you often need to symbolicate many stack
    // traces against a pre-loaded set of source maps. symbolicate_batch avoids
    // re-parsing source maps for each trace.

    println!("\n--- Step 7: Batch symbolication ---\n");

    let sm = SourceMap::from_json(&source_map_json).expect("valid source map");
    let mut maps: HashMap<String, SourceMap> = HashMap::new();
    maps.insert("bundle.js".to_string(), sm);

    let error_stack_1 = "\
TypeError: Cannot read property 'x' of null
    at processData (bundle.js:3:10)
    at handleClick (bundle.js:1:1)";

    let error_stack_2 = "\
ReferenceError: foo is not defined
    at handleClick (bundle.js:1:1)";

    let stacks = vec![error_stack_1, error_stack_2];
    let batch_results = symbolicate_batch(&stacks, &maps);

    assert_eq!(batch_results.len(), 2);
    println!("  Batch result 1 ({} frames):", batch_results[0].frames.len());
    print!("{}", batch_results[0]);

    println!("  Batch result 2 ({} frames):", batch_results[1].frames.len());
    print!("{}", batch_results[1]);

    // First stack: same as individual symbolication
    assert_eq!(batch_results[0].frames.len(), 2);
    assert!(batch_results[0].frames[0].symbolicated);
    assert_eq!(batch_results[0].frames[0].file, "src/app.ts");

    // Second stack: single frame
    assert_eq!(batch_results[1].frames.len(), 1);
    assert!(batch_results[1].frames[0].symbolicated);
    assert_eq!(batch_results[1].message.as_deref(), Some("ReferenceError: foo is not defined"));

    // -----------------------------------------------------------------------
    // 8. Debug ID resolution
    // -----------------------------------------------------------------------
    //
    // In production, source maps are often stored by their debug ID (a UUID
    // embedded in both the generated file and the source map). This allows
    // matching without relying on filename conventions.

    println!("\n--- Step 8: Debug ID resolution ---\n");

    let debug_id = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";

    let resolved = resolve_by_debug_id(debug_id, &maps);
    assert!(resolved.is_some(), "should find source map by debug ID");

    let resolved_map = resolved.unwrap();
    assert_eq!(resolved_map.debug_id.as_deref(), Some(debug_id));
    println!("  Found source map for debug ID: {debug_id}");
    println!("  Sources: {:?}", resolved_map.sources);
    println!("  Names:   {:?}", resolved_map.names);

    // Non-existent debug ID returns None
    let missing = resolve_by_debug_id("00000000-0000-0000-0000-000000000000", &maps);
    assert!(missing.is_none(), "unknown debug ID should return None");
    println!("  Unknown debug ID correctly returns None.");

    println!("\n=== All assertions passed. ===");
}
