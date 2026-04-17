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
| `resolve_by_debug_id(id, maps)` | Find a source map by its `debugId` proposal field |
| `to_json(stack)` | Serialize a `SymbolicatedStack` to JSON |

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
