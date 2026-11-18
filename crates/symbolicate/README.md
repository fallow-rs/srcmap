# srcmap-symbolicate

[![crates.io](https://img.shields.io/crates/v/srcmap-symbolicate.svg)](https://crates.io/crates/srcmap-symbolicate)
[![docs.rs](https://docs.rs/srcmap-symbolicate/badge.svg)](https://docs.rs/srcmap-symbolicate)

Stack trace symbolication using source maps.

Parses stack traces from V8 (Chrome, Node.js), SpiderMonkey (Firefox), and JavaScriptCore (Safari), resolves each frame through source maps, and produces readable output. Built for error monitoring, crash reporting, and debugging services.

## Usage

```rust
use srcmap_symbolicate::{symbolicate, parse_stack_trace};
use srcmap_sourcemap::SourceMap;

let stack = "Error: oops\n    at foo (bundle.js:10:5)\n    at bar (bundle.js:20:10)";

// Parse without symbolication
let frames = parse_stack_trace(stack);
assert_eq!(frames[0].function_name.as_deref(), Some("foo"));
assert_eq!(frames[0].file, "bundle.js");

// Symbolicate with a source map loader
let result = symbolicate(stack, |file| {
    let json = std::fs::read_to_string(format!("{file}.map")).ok()?;
    SourceMap::from_json(&json).ok()
});

println!("{result}"); // Pretty-printed symbolicated stack
```

## API

| Function | Description |
|----------|-------------|
| `parse_stack_trace(input)` | Parse into `Vec<StackFrame>` |
| `parse_stack_trace_full(input)` | Parse into `ParsedStack` (message + frames) |
| `symbolicate(stack, loader)` | Parse and resolve through source maps |
| `symbolicate_batch(stacks, maps)` | Batch symbolication with pre-loaded maps |
| `symbolicate_with_debug_ids(stack, debug_id_for_file, map_for_debug_id, map_for_file)` | Parse and resolve. Find each source map by debug ID first, then by file |
| `DebugIdIndex` | A set of source maps that a caller finds by debug ID (`insert`, `insert_with_id`, `get`) |
| `resolve_by_debug_id(id, maps)` | Find a source map by its `debugId` proposal field in a map that has file keys |
| `to_json(stack)` | Serialize a `SymbolicatedStack` to JSON |

## Source maps by debug ID

The [debug ID proposal](https://github.com/tc39/ecma426/blob/main/proposals/debug-id.md) puts the same UUID in a generated file (`//# debugId=<UUID>`) and in its source map (`debugId`). A debug ID stays correct when the URL of a file changes.

```rust
use srcmap_sourcemap::{SourceMap, extract_debug_id};
use srcmap_symbolicate::{DebugIdIndex, symbolicate_with_debug_ids};

// Put each source map that has a `debugId` field in the index.
let index: DebugIdIndex = std::fs::read_dir("maps")
    .unwrap()
    .filter_map(|entry| std::fs::read_to_string(entry.ok()?.path()).ok())
    .filter_map(|json| SourceMap::from_json(&json).ok())
    .collect();

let stack = "Error: oops\n    at foo (dist/bundle.js:10:5)";
let result = symbolicate_with_debug_ids(
    stack,
    // The debug ID of the generated file of a frame.
    |file| extract_debug_id(&std::fs::read_to_string(file).ok()?),
    // The source map for that debug ID.
    |debug_id| index.get(debug_id),
    // The source map for a file that has no debug ID.
    |file| SourceMap::from_json(&std::fs::read_to_string(format!("{file}.map")).ok()?).ok(),
);

println!("{result}");
```

The index ignores the case of the hex digits in a debug ID.

## Supported stack trace formats

| Engine | Example format |
|--------|---------------|
| V8 (Chrome, Node.js) | `at functionName (file:line:column)` |
| SpiderMonkey (Firefox) | `functionName@file:line:column` |
| JavaScriptCore (Safari) | `functionName@file:line:column` |

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See the [main repo](https://github.com/fallow-rs/srcmap) for the full source map SDK.

## License

MIT
