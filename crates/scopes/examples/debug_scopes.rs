//! Debug scopes example: model a function inlined into generated code.
//!
//! Demonstrates the ECMA-426 "Scopes" proposal for source maps.
//! This proposal lets debuggers reconstruct original scope trees, variable
//! bindings, and inlined function call sites from minified/bundled output.
//!
//! Scenario: a TypeScript function `add(a, b)` is inlined by the bundler.
//!
//! Original source (math.ts, source index 0):
//! ```text
//! // Line 0: (module-level code)
//! // Line 1: function add(a: number, b: number): number {
//! // Line 2:   return a + b;
//! // Line 3: }
//! // Line 4:
//! // Line 5: const result = add(10, 32);
//! ```
//!
//! Generated code (bundle.js):
//! ```text
//! // Line 0: (module wrapper open)
//! // Line 1: var _a = 10;
//! // Line 2: var _b = 32;
//! // Line 3: var result = _a + _b;
//! // Line 4: (module wrapper close)
//! ```
//!
//! The bundler inlined `add(10, 32)` — lines 1-3 in the output correspond
//! to the body of the original `add` function. A debugger uses the scope
//! info to show that `_a` is really `a` and `_b` is really `b`, and that
//! the code was inlined from a call at line 5, column 14.
//!
//! Run with: cargo run -p srcmap-scopes --example debug_scopes

#![allow(clippy::print_stdout, reason = "Examples are intended to print walkthrough output")]

use srcmap_scopes::{
    Binding, CallSite, GeneratedRange, OriginalScope, Position, ScopeInfo, decode_scopes,
    encode_scopes,
};

fn main() {
    // -----------------------------------------------------------------------
    // 1. Build the original scope tree (what the author wrote)
    // -----------------------------------------------------------------------
    //
    // ECMA-426 original scopes form a tree per source file. Each scope has:
    //   - start/end positions (0-based line and column)
    //   - an optional name (for named functions, classes, etc.)
    //   - an optional kind ("global", "module", "function", "block", "class")
    //   - is_stack_frame: true for function-like scopes that appear in call stacks
    //   - variables: names declared in this scope (parameters, let/const/var)
    //   - children: nested scopes
    //
    // Scopes are indexed by a pre-order traversal counter ("definition index"):
    //   - definition 0: the module scope (root)
    //   - definition 1: the `add` function scope (first child)

    let add_scope = OriginalScope {
        start: Position { line: 1, column: 0 },
        end: Position { line: 3, column: 1 },
        name: Some("add".to_string()),
        kind: Some("function".to_string()),
        is_stack_frame: true,
        variables: vec!["a".to_string(), "b".to_string()],
        children: vec![],
    };

    let module_scope = OriginalScope {
        start: Position { line: 0, column: 0 },
        end: Position { line: 5, column: 27 },
        name: None,
        kind: Some("module".to_string()),
        is_stack_frame: false,
        variables: vec!["result".to_string()],
        children: vec![add_scope],
    };

    // -----------------------------------------------------------------------
    // 2. Build the generated ranges (what the bundler produced)
    // -----------------------------------------------------------------------
    //
    // Generated ranges describe regions of the output code and how they map
    // back to original scopes. Key fields:
    //
    //   - definition: index into the pre-order list of all original scopes,
    //     linking this range to its corresponding original scope
    //   - call_site: if this range is an inlined function body, the location
    //     in original source where the call happened
    //   - bindings: one entry per variable in the referenced original scope,
    //     telling the debugger what JS expression to evaluate for each variable
    //   - is_stack_frame: true if this range should appear in synthetic stacks
    //   - is_hidden: true if the debugger should skip over this range entirely

    let inlined_range = GeneratedRange {
        start: Position { line: 1, column: 0 },
        end: Position { line: 3, column: 22 },
        is_stack_frame: true,
        is_hidden: false,
        // definition=1 points to the `add` function scope (pre-order index 1)
        definition: Some(1),
        // The call site is where `add(10, 32)` was called in the original source.
        // The debugger uses this to reconstruct a synthetic call stack:
        //   add @ math.ts:2:2  (current position in the inlined body)
        //   <module> @ math.ts:5:14  (the call site)
        call_site: Some(CallSite { source_index: 0, line: 5, column: 14 }),
        // Bindings map the original scope's variables to generated expressions.
        // The `add` scope has variables ["a", "b"] (in that order), so:
        //   bindings[0] = Expression("_a")  → original `a` is `_a` in generated code
        //   bindings[1] = Expression("_b")  → original `b` is `_b` in generated code
        bindings: vec![
            Binding::Expression("_a".to_string()),
            Binding::Expression("_b".to_string()),
        ],
        children: vec![],
    };

    let wrapper_range = GeneratedRange {
        start: Position { line: 0, column: 0 },
        end: Position { line: 4, column: 1 },
        is_stack_frame: false,
        is_hidden: false,
        // definition=0 points to the module scope (pre-order index 0)
        definition: Some(0),
        call_site: None,
        // The module scope has variables ["result"], and in the generated code
        // the variable keeps its name, so we bind it to "result".
        bindings: vec![Binding::Expression("result".to_string())],
        children: vec![inlined_range],
    };

    // -----------------------------------------------------------------------
    // 3. Assemble ScopeInfo and encode
    // -----------------------------------------------------------------------
    //
    // ScopeInfo combines original scope trees (one per source file) with the
    // generated ranges. The `scopes` vec is indexed by source index — None
    // means no scope info for that source file.

    let scope_info = ScopeInfo { scopes: vec![Some(module_scope)], ranges: vec![wrapper_range] };

    // Encoding produces a compact VLQ string (stored in the source map's
    // "scopes" field) and populates the names array with any new name strings.
    let mut names: Vec<String> = vec![];
    let encoded = encode_scopes(&scope_info, &mut names);

    println!("=== ECMA-426 Scopes Roundtrip ===\n");
    println!("Encoded scopes: {encoded:?}");
    println!("Names array:    {names:?}\n");

    assert!(!encoded.is_empty(), "encoded string must not be empty");
    assert!(!names.is_empty(), "names array must contain scope/variable names");

    // -----------------------------------------------------------------------
    // 4. Decode back and verify roundtrip
    // -----------------------------------------------------------------------
    //
    // decode_scopes takes the encoded string, the names array, and the number
    // of source files (so it knows how many original scope trees to expect).

    let decoded = decode_scopes(&encoded, &names, 1).expect("decoding must succeed");

    // Verify the original scope tree roundtrips correctly
    assert_eq!(decoded.scopes.len(), 1, "must have exactly one source file's scopes");

    let root_scope = decoded.scopes[0].as_ref().expect("source 0 must have scope info");

    assert_eq!(root_scope.kind.as_deref(), Some("module"));
    assert!(!root_scope.is_stack_frame, "module scope is not a stack frame");
    assert_eq!(root_scope.variables, vec!["result"]);
    assert_eq!(root_scope.start, Position { line: 0, column: 0 });
    assert_eq!(root_scope.end, Position { line: 5, column: 27 });

    println!("Original scope tree (source 0):");
    println!("  Root: kind={:?}, variables={:?}", root_scope.kind, root_scope.variables);

    assert_eq!(root_scope.children.len(), 1, "module has one child scope");

    let func_scope = &root_scope.children[0];
    assert_eq!(func_scope.name.as_deref(), Some("add"));
    assert_eq!(func_scope.kind.as_deref(), Some("function"));
    assert!(func_scope.is_stack_frame, "function scope is a stack frame");
    assert_eq!(func_scope.variables, vec!["a", "b"]);
    assert_eq!(func_scope.start, Position { line: 1, column: 0 });
    assert_eq!(func_scope.end, Position { line: 3, column: 1 });

    println!(
        "  Child: name={:?}, kind={:?}, variables={:?}",
        func_scope.name, func_scope.kind, func_scope.variables
    );

    // Verify the generated ranges roundtrip correctly
    assert_eq!(decoded.ranges.len(), 1, "must have one top-level generated range");

    let wrapper = &decoded.ranges[0];
    assert_eq!(wrapper.definition, Some(0));
    assert!(!wrapper.is_stack_frame);
    assert!(!wrapper.is_hidden);
    assert!(wrapper.call_site.is_none());
    assert_eq!(wrapper.bindings, vec![Binding::Expression("result".to_string())]);

    println!("\nGenerated ranges:");
    println!(
        "  Wrapper: lines {}-{}, definition={:?}, bindings={:?}",
        wrapper.start.line, wrapper.end.line, wrapper.definition, wrapper.bindings
    );

    assert_eq!(wrapper.children.len(), 1, "wrapper has one child range");

    let inlined = &wrapper.children[0];
    assert_eq!(inlined.definition, Some(1));
    assert!(inlined.is_stack_frame, "inlined range is a stack frame");
    assert!(!inlined.is_hidden);
    assert_eq!(inlined.call_site, Some(CallSite { source_index: 0, line: 5, column: 14 }));
    assert_eq!(
        inlined.bindings,
        vec![Binding::Expression("_a".to_string()), Binding::Expression("_b".to_string()),]
    );

    println!(
        "  Inlined: lines {}-{}, definition={:?}, call_site={:?}, bindings={:?}",
        inlined.start.line,
        inlined.end.line,
        inlined.definition,
        inlined.call_site,
        inlined.bindings
    );

    // Full structural equality check
    assert_eq!(decoded, scope_info, "decoded scope info must match the original");

    println!("\nRoundtrip verified: decoded structure matches original.\n");

    // -----------------------------------------------------------------------
    // 5. Look up original scopes by definition index
    // -----------------------------------------------------------------------
    //
    // A debugger hits a breakpoint in generated code and finds a generated
    // range with definition=1. It needs to find the corresponding original
    // scope to know the function name, parameter names, etc.

    println!("--- Definition index lookups ---\n");

    let scope_0 = decoded.original_scope_for_definition(0).expect("definition 0 must exist");
    assert_eq!(scope_0.kind.as_deref(), Some("module"));
    println!("  definition 0: kind={:?}, name={:?}", scope_0.kind, scope_0.name);

    let scope_1 = decoded.original_scope_for_definition(1).expect("definition 1 must exist");
    assert_eq!(scope_1.name.as_deref(), Some("add"));
    assert_eq!(scope_1.variables, vec!["a", "b"]);
    println!("  definition 1: kind={:?}, name={:?}", scope_1.kind, scope_1.name);

    // Out-of-bounds definition index returns None
    assert!(
        decoded.original_scope_for_definition(99).is_none(),
        "non-existent definition must return None"
    );
    println!("  definition 99: None (out of bounds)");

    // -----------------------------------------------------------------------
    // 6. Demonstrate the Unavailable binding variant
    // -----------------------------------------------------------------------
    //
    // Sometimes a variable is optimized out entirely. The debugger should
    // show it as "unavailable" rather than silently omitting it.

    println!("\n--- Unavailable binding ---\n");

    let optimized_info = ScopeInfo {
        scopes: vec![Some(OriginalScope {
            start: Position { line: 0, column: 0 },
            end: Position { line: 3, column: 1 },
            name: Some("compute".to_string()),
            kind: Some("function".to_string()),
            is_stack_frame: true,
            variables: vec!["x".to_string(), "y".to_string()],
            children: vec![],
        })],
        ranges: vec![GeneratedRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: 1, column: 0 },
            is_stack_frame: true,
            is_hidden: false,
            definition: Some(0),
            call_site: None,
            // x is available as "_x", but y was optimized out
            bindings: vec![Binding::Expression("_x".to_string()), Binding::Unavailable],
            children: vec![],
        }],
    };

    let mut opt_names: Vec<String> = vec![];
    let opt_encoded = encode_scopes(&optimized_info, &mut opt_names);
    let opt_decoded = decode_scopes(&opt_encoded, &opt_names, 1).expect("decoding must succeed");

    assert_eq!(opt_decoded, optimized_info);
    println!("  Bindings: {:?}", opt_decoded.ranges[0].bindings);
    println!("  Variable 'x' -> Expression(\"_x\"), variable 'y' -> Unavailable");

    // -----------------------------------------------------------------------
    // 7. Error handling for invalid input
    // -----------------------------------------------------------------------
    //
    // decode_scopes returns ScopesError for malformed input. This is useful
    // for tools that validate source maps.

    println!("\n--- Error handling ---\n");

    // Empty encoded string is valid (no scopes, no ranges)
    let empty_result = decode_scopes("", &[], 0);
    assert!(empty_result.is_ok(), "empty input is valid");
    println!("  Empty input: ok (no scopes, no ranges)");

    // Invalid VLQ data: 'z' is not a valid base64 character for VLQ
    let bad_vlq = decode_scopes("!!!", &[], 1);
    assert!(bad_vlq.is_err(), "invalid VLQ must produce an error");
    println!("  Invalid VLQ (\"!!!\"): {}", bad_vlq.unwrap_err());

    println!("\nAll assertions passed.");
}
