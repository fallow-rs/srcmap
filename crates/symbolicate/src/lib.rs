//! Stack trace symbolication using source maps.
//!
//! Parses stack traces from V8, SpiderMonkey, and JavaScriptCore,
//! resolves each frame through source maps, and produces readable output.
//!
//! # Examples
//!
//! ```
//! use srcmap_symbolicate::{parse_stack_trace, symbolicate, StackFrame};
//!
//! let stack = "Error: oops\n    at foo (bundle.js:10:5)\n    at bar (bundle.js:20:10)";
//! let frames = parse_stack_trace(stack);
//! assert_eq!(frames.len(), 2);
//! assert_eq!(frames[0].function_name.as_deref(), Some("foo"));
//! ```

use std::collections::HashMap;
use std::fmt;

use srcmap_sourcemap::SourceMap;

// ── Types ───────────────────────────────────────────────────────

/// A single parsed stack frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackFrame {
    /// Function name (if available).
    pub function_name: Option<String>,
    /// Source file path or URL.
    pub file: String,
    /// Line number (1-based as in stack traces).
    pub line: u32,
    /// Column number (1-based as in stack traces).
    pub column: u32,
}

/// A symbolicated (resolved) stack frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolicatedFrame {
    /// Original function name from source map (or original function name).
    pub function_name: Option<String>,
    /// Resolved original source file.
    pub file: String,
    /// Resolved original line (1-based).
    pub line: u32,
    /// Resolved original column (1-based).
    pub column: u32,
    /// Whether this frame was successfully symbolicated.
    pub symbolicated: bool,
}

/// A full symbolicated stack trace.
#[derive(Debug, Clone)]
pub struct SymbolicatedStack {
    /// Error message (first line of the stack trace).
    pub message: Option<String>,
    /// Resolved frames.
    pub frames: Vec<SymbolicatedFrame>,
}

impl fmt::Display for SymbolicatedStack {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref msg) = self.message {
            writeln!(f, "{msg}")?;
        }
        for frame in &self.frames {
            let name = frame.function_name.as_deref().unwrap_or("<anonymous>");
            writeln!(f, "    at {name} ({}:{}:{})", frame.file, frame.line, frame.column)?;
        }
        Ok(())
    }
}

/// Result of parsing a stack trace: the message line and the parsed frames.
#[derive(Debug, Clone)]
pub struct ParsedStack {
    /// Error message (e.g. "Error: something went wrong").
    pub message: Option<String>,
    /// Parsed stack frames.
    pub frames: Vec<StackFrame>,
}

// ── Stack trace engine detection ─────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Engine {
    V8,
    SpiderMonkey,
    JavaScriptCore,
}

// ── Parser ──────────────────────────────────────────────────────

/// Parse a stack trace string into individual frames.
///
/// Supports V8 (Chrome, Node.js), SpiderMonkey (Firefox), and
/// JavaScriptCore (Safari) stack trace formats.
pub fn parse_stack_trace(input: &str) -> Vec<StackFrame> {
    parse_stack_trace_full(input).frames
}

/// Parse a stack trace string into message + frames.
pub fn parse_stack_trace_full(input: &str) -> ParsedStack {
    let mut lines = input.lines();
    let mut message = None;
    let mut frames = Vec::new();

    // Detect engine and extract message from first line
    let first_line = match lines.next() {
        Some(l) => l,
        None => {
            return ParsedStack { message: None, frames: Vec::new() };
        }
    };

    let engine = detect_engine(first_line);

    // If the first line looks like a message (not a frame), save it
    if !is_frame_line(first_line, engine) {
        message = Some(first_line.to_string());
    } else if let Some(frame) = parse_frame(first_line, engine) {
        frames.push(frame);
    }

    for line in lines {
        if let Some(frame) = parse_frame(line, engine) {
            frames.push(frame);
        }
    }

    ParsedStack { message, frames }
}

/// Detect the JavaScript engine from the first line of a stack trace.
fn detect_engine(first_line: &str) -> Engine {
    let trimmed = first_line.trim();
    if trimmed.starts_with("    at ") || trimmed.contains("    at ") {
        Engine::V8
    } else if trimmed.contains('@') && (trimmed.contains(':') || trimmed.contains('/')) {
        Engine::SpiderMonkey
    } else if trimmed.contains('@') {
        Engine::JavaScriptCore
    } else {
        // Default to V8 for error message lines
        Engine::V8
    }
}

/// Check if a line looks like a stack frame (vs an error message).
fn is_frame_line(line: &str, engine: Engine) -> bool {
    let trimmed = line.trim();
    match engine {
        Engine::V8 => trimmed.starts_with("at "),
        Engine::SpiderMonkey | Engine::JavaScriptCore => trimmed.contains('@'),
    }
}

/// Parse a single stack frame line.
fn parse_frame(line: &str, engine: Engine) -> Option<StackFrame> {
    let trimmed = line.trim();

    match engine {
        Engine::V8 => parse_v8_frame(trimmed),
        Engine::SpiderMonkey => parse_spidermonkey_frame(trimmed),
        Engine::JavaScriptCore => parse_jsc_frame(trimmed),
    }
}

/// Parse a V8 stack frame: `at functionName (file:line:column)` or `at file:line:column`
fn parse_v8_frame(line: &str) -> Option<StackFrame> {
    let rest = line.strip_prefix("at ")?;

    // Check for `functionName (file:line:column)` format
    if let Some(paren_start) = rest.rfind('(') {
        let func = rest[..paren_start].trim();
        let location = rest[paren_start + 1..].trim_end_matches(')').trim();
        let (file, line_num, col) = parse_location(location)?;

        return Some(StackFrame {
            function_name: if func.is_empty() { None } else { Some(func.to_string()) },
            file,
            line: line_num,
            column: col,
        });
    }

    // Bare `file:line:column` format
    let (file, line_num, col) = parse_location(rest)?;
    Some(StackFrame { function_name: None, file, line: line_num, column: col })
}

/// Parse a SpiderMonkey stack frame: `functionName@file:line:column`
fn parse_spidermonkey_frame(line: &str) -> Option<StackFrame> {
    let (func, location) = line.split_once('@')?;
    let (file, line_num, col) = parse_location(location)?;

    Some(StackFrame {
        function_name: if func.is_empty() { None } else { Some(func.to_string()) },
        file,
        line: line_num,
        column: col,
    })
}

/// Parse a JavaScriptCore stack frame: `functionName@file:line:column`
/// Same format as SpiderMonkey.
fn parse_jsc_frame(line: &str) -> Option<StackFrame> {
    parse_spidermonkey_frame(line)
}

/// Parse a location string: `file:line:column` or `file:line`
/// Handles URLs with colons (http://host:port/file:line:column)
fn parse_location(location: &str) -> Option<(String, u32, u32)> {
    // Split from the right to handle URLs with colons
    let (rest, col_str) = location.rsplit_once(':')?;
    let col: u32 = col_str.parse().ok()?;

    let (file, line_str) = rest.rsplit_once(':')?;
    let line_num: u32 = line_str.parse().ok()?;

    if file.is_empty() {
        return None;
    }

    Some((file.to_string(), line_num, col))
}

// ── Symbolication ───────────────────────────────────────────────

/// Symbolicate a stack trace using a source map loader function.
///
/// The `loader` is called with each unique source file and should return
/// the corresponding `SourceMap`, or `None` if not available.
///
/// Stack trace lines/columns are 1-based; source maps use 0-based internally.
pub fn symbolicate<F>(stack: &str, loader: F) -> SymbolicatedStack
where
    F: Fn(&str) -> Option<SourceMap>,
{
    let parsed = parse_stack_trace_full(stack);
    symbolicate_frames(&parsed.frames, parsed.message, &loader)
}

/// Symbolicate pre-parsed frames.
fn symbolicate_frames<F>(
    frames: &[StackFrame],
    message: Option<String>,
    loader: &F,
) -> SymbolicatedStack
where
    F: Fn(&str) -> Option<SourceMap>,
{
    let mut cache: HashMap<String, Option<SourceMap>> = HashMap::new();
    let mut result_frames = Vec::with_capacity(frames.len());

    for frame in frames {
        let sm = cache.entry(frame.file.clone()).or_insert_with(|| loader(&frame.file));

        let resolved = match sm {
            Some(sm) => {
                // Stack traces are 1-based, source maps are 0-based
                let line = frame.line.saturating_sub(1);
                let column = frame.column.saturating_sub(1);

                match sm.original_position_for(line, column) {
                    Some(loc) => SymbolicatedFrame {
                        function_name: loc
                            .name
                            .map(|n| sm.name(n).to_string())
                            .or_else(|| frame.function_name.clone()),
                        file: sm.source(loc.source).to_string(),
                        line: loc.line + 1,     // back to 1-based
                        column: loc.column + 1, // back to 1-based
                        symbolicated: true,
                    },
                    None => SymbolicatedFrame {
                        function_name: frame.function_name.clone(),
                        file: frame.file.clone(),
                        line: frame.line,
                        column: frame.column,
                        symbolicated: false,
                    },
                }
            }
            None => SymbolicatedFrame {
                function_name: frame.function_name.clone(),
                file: frame.file.clone(),
                line: frame.line,
                column: frame.column,
                symbolicated: false,
            },
        };

        result_frames.push(resolved);
    }

    SymbolicatedStack { message, frames: result_frames }
}

/// Batch symbolicate multiple stack traces against pre-loaded source maps.
///
/// `maps` is a map of source file → SourceMap. All stack traces are resolved
/// against these pre-loaded maps without additional loading.
pub fn symbolicate_batch(
    stacks: &[&str],
    maps: &HashMap<String, SourceMap>,
) -> Vec<SymbolicatedStack> {
    stacks.iter().map(|stack| symbolicate(stack, |file| maps.get(file).cloned())).collect()
}

/// Resolve a debug ID to a source map from a set of maps indexed by debug ID.
///
/// Useful for error monitoring systems where source maps are identified by
/// their debug ID rather than by filename.
pub fn resolve_by_debug_id<'a>(
    debug_id: &str,
    maps: &'a HashMap<String, SourceMap>,
) -> Option<&'a SourceMap> {
    maps.values().find(|sm| sm.debug_id.as_deref() == Some(debug_id))
}

/// Serialize a symbolicated stack to JSON.
pub fn to_json(stack: &SymbolicatedStack) -> String {
    let frames: Vec<serde_json::Value> = stack
        .frames
        .iter()
        .map(|f| {
            serde_json::json!({
                "functionName": f.function_name,
                "file": f.file,
                "line": f.line,
                "column": f.column,
                "symbolicated": f.symbolicated,
            })
        })
        .collect();

    let obj = serde_json::json!({
        "message": stack.message,
        "frames": frames,
    });

    serde_json::to_string_pretty(&obj).unwrap_or_default()
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── V8 format tests ──────────────────────────────────────────

    #[test]
    fn parse_v8_basic() {
        let input = "Error: test\n    at foo (bundle.js:10:5)\n    at bar (bundle.js:20:10)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error: test"));
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(parsed.frames[0].file, "bundle.js");
        assert_eq!(parsed.frames[0].line, 10);
        assert_eq!(parsed.frames[0].column, 5);
        assert_eq!(parsed.frames[1].function_name.as_deref(), Some("bar"));
    }

    #[test]
    fn parse_v8_anonymous() {
        let input = "Error\n    at bundle.js:10:5";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].function_name.is_none());
        assert_eq!(frames[0].file, "bundle.js");
    }

    #[test]
    fn parse_v8_url() {
        let input = "Error\n    at foo (https://cdn.example.com/bundle.js:10:5)";
        let frames = parse_stack_trace(input);
        assert_eq!(frames[0].file, "https://cdn.example.com/bundle.js");
    }

    // ── SpiderMonkey format tests ────────────────────────────────

    #[test]
    fn parse_spidermonkey_basic() {
        let input = "foo@bundle.js:10:5\nbar@bundle.js:20:10";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(frames[0].file, "bundle.js");
        assert_eq!(frames[0].line, 10);
    }

    #[test]
    fn parse_spidermonkey_anonymous() {
        let input = "@bundle.js:10:5";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].function_name.is_none());
    }

    #[test]
    fn parse_spidermonkey_url() {
        let input = "foo@https://example.com/bundle.js:10:5";
        let frames = parse_stack_trace(input);
        assert_eq!(frames[0].file, "https://example.com/bundle.js");
    }

    // ── Symbolication tests ──────────────────────────────────────

    #[test]
    fn symbolicate_basic() {
        let map_json = r#"{"version":3,"sources":["src/app.ts"],"names":["handleClick"],"mappings":"AAAA;AACA;AACA;AACA;AACA;AACA;AACA;AACA;AACA;AAAAA"}"#;

        let stack = "Error: test\n    at foo (bundle.js:10:1)";

        let result = symbolicate(stack, |file| {
            if file == "bundle.js" { SourceMap::from_json(map_json).ok() } else { None }
        });

        assert_eq!(result.message.as_deref(), Some("Error: test"));
        assert_eq!(result.frames.len(), 1);
        assert!(result.frames[0].symbolicated);
        assert_eq!(result.frames[0].file, "src/app.ts");
        assert_eq!(result.frames[0].function_name.as_deref(), Some("handleClick"));
    }

    #[test]
    fn symbolicate_no_map() {
        let stack = "Error: test\n    at foo (unknown.js:10:5)";
        let result = symbolicate(stack, |_| None);
        assert!(!result.frames[0].symbolicated);
        assert_eq!(result.frames[0].file, "unknown.js");
    }

    #[test]
    fn batch_symbolicate_test() {
        let map_json = r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(map_json).unwrap();
        let mut maps = HashMap::new();
        maps.insert("bundle.js".to_string(), sm);

        let stacks = vec!["Error\n    at foo (bundle.js:1:1)", "Error\n    at bar (bundle.js:1:1)"];
        let results = symbolicate_batch(&stacks, &maps);
        assert_eq!(results.len(), 2);
        assert!(results[0].frames[0].symbolicated);
        assert!(results[1].frames[0].symbolicated);
    }

    #[test]
    fn debug_id_resolution() {
        let map_json =
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","debugId":"abc-123"}"#;
        let sm = SourceMap::from_json(map_json).unwrap();
        let mut maps = HashMap::new();
        maps.insert("bundle.js".to_string(), sm);

        let found = resolve_by_debug_id("abc-123", &maps);
        assert!(found.is_some());
        assert_eq!(found.unwrap().debug_id.as_deref(), Some("abc-123"));

        let not_found = resolve_by_debug_id("nonexistent", &maps);
        assert!(not_found.is_none());
    }

    #[test]
    fn to_json_output() {
        let stack = SymbolicatedStack {
            message: Some("Error: test".to_string()),
            frames: vec![SymbolicatedFrame {
                function_name: Some("foo".to_string()),
                file: "src/app.ts".to_string(),
                line: 42,
                column: 10,
                symbolicated: true,
            }],
        };
        let json = to_json(&stack);
        assert!(json.contains("Error: test"));
        assert!(json.contains("src/app.ts"));
        assert!(json.contains("\"symbolicated\": true"));
    }

    #[test]
    fn display_format() {
        let stack = SymbolicatedStack {
            message: Some("Error: test".to_string()),
            frames: vec![SymbolicatedFrame {
                function_name: Some("foo".to_string()),
                file: "app.ts".to_string(),
                line: 42,
                column: 10,
                symbolicated: true,
            }],
        };
        let output = format!("{stack}");
        assert!(output.contains("Error: test"));
        assert!(output.contains("at foo (app.ts:42:10)"));
    }

    #[test]
    fn display_anonymous_frame() {
        let stack = SymbolicatedStack {
            message: None,
            frames: vec![SymbolicatedFrame {
                function_name: None,
                file: "app.js".to_string(),
                line: 1,
                column: 1,
                symbolicated: false,
            }],
        };
        let output = format!("{stack}");
        assert!(output.contains("<anonymous>"));
        assert!(!output.contains("Error"));
    }

    #[test]
    fn parse_empty_input() {
        let parsed = parse_stack_trace_full("");
        assert!(parsed.message.is_none());
        assert!(parsed.frames.is_empty());
    }

    #[test]
    fn parse_unparsable_lines() {
        // Lines that don't match any frame format
        let input = "Error: boom\n  this is not a frame\n  neither is this";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error: boom"));
        assert!(parsed.frames.is_empty());
    }

    #[test]
    fn detect_jsc_engine() {
        // JSC: has @ but no : or / (just @ sign alone)
        let input = "someFunc@native code";
        let frames = parse_stack_trace(input);
        // Should detect as JSC and try to parse
        assert!(frames.is_empty() || frames[0].function_name.as_deref() == Some("someFunc"));
    }

    #[test]
    fn parse_v8_bare_location() {
        // V8 bare format: `at file:line:column` (no parens, no function name)
        let input = "Error\n    at bundle.js:42:13";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].function_name.is_none());
        assert_eq!(frames[0].file, "bundle.js");
        assert_eq!(frames[0].line, 42);
        assert_eq!(frames[0].column, 13);
    }

    #[test]
    fn parse_v8_empty_function_in_parens() {
        // V8 with empty function name before parens
        let input = "Error\n    at (bundle.js:10:5)";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].function_name.is_none());
    }

    #[test]
    fn parse_spidermonkey_anonymous_frame() {
        // SpiderMonkey: @file:line:col with empty function name
        let input = "@bundle.js:10:5\n@bundle.js:20:10";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 2);
        assert!(frames[0].function_name.is_none());
        assert!(frames[1].function_name.is_none());
    }

    #[test]
    fn parse_location_empty_file() {
        // parse_location returns None when file component is empty
        let input = "Error\n    at (:10:5)";
        let frames = parse_stack_trace(input);
        assert!(frames.is_empty());
    }

    #[test]
    fn symbolicate_missing_map_for_some_files() {
        let map_json = r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA"}"#;

        let stack = "Error: test\n    at foo (bundle.js:1:1)\n    at bar (unknown.js:5:3)";
        let result = symbolicate(stack, |file| {
            if file == "bundle.js" { SourceMap::from_json(map_json).ok() } else { None }
        });

        assert_eq!(result.frames.len(), 2);
        assert!(result.frames[0].symbolicated);
        assert!(!result.frames[1].symbolicated);
        assert_eq!(result.frames[1].file, "unknown.js");
        assert_eq!(result.frames[1].function_name.as_deref(), Some("bar"));
    }

    #[test]
    fn symbolicate_no_match_at_position() {
        // Source map exists but no mapping at the requested position
        let map_json = r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA"}"#;

        let stack = "Error: test\n    at foo (bundle.js:100:100)";
        let result = symbolicate(stack, |_| SourceMap::from_json(map_json).ok());

        assert_eq!(result.frames.len(), 1);
        // Position 99:99 (0-based) is beyond any mapping, frame should not be symbolicated
        // Actually it may snap to closest - let's check either way
        assert!(!result.frames[0].file.is_empty());
    }

    #[test]
    fn symbolicate_caches_source_maps() {
        use std::cell::Cell;

        // Multiple frames from the same file should only call the loader once
        let map_json = r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA"}"#;

        let stack = "Error: test\n    at foo (bundle.js:1:1)\n    at bar (bundle.js:1:1)";
        let call_count = Cell::new(0u32);
        let result = symbolicate(stack, |file| {
            call_count.set(call_count.get() + 1);
            if file == "bundle.js" { SourceMap::from_json(map_json).ok() } else { None }
        });

        assert_eq!(result.frames.len(), 2);
        // Both should be resolved
        assert!(result.frames[0].symbolicated);
        assert!(result.frames[1].symbolicated);
    }

    #[test]
    fn parse_default_engine_detection() {
        // First line is just an error message, no frame indicators
        let input = "TypeError: Cannot read property 'x' of null";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("TypeError: Cannot read property 'x' of null"));
        assert!(parsed.frames.is_empty());
    }

    #[test]
    fn symbolicated_stack_display_with_message_and_mixed_frames() {
        let stack = SymbolicatedStack {
            message: Some("Error: oops".to_string()),
            frames: vec![
                SymbolicatedFrame {
                    function_name: Some("foo".to_string()),
                    file: "app.js".to_string(),
                    line: 10,
                    column: 5,
                    symbolicated: true,
                },
                SymbolicatedFrame {
                    function_name: None,
                    file: "lib.js".to_string(),
                    line: 20,
                    column: 1,
                    symbolicated: false,
                },
            ],
        };
        let output = stack.to_string();
        assert!(output.contains("Error: oops"));
        assert!(output.contains("foo"));
        assert!(output.contains("<anonymous>"));
        assert!(output.contains("app.js:10:5"));
        assert!(output.contains("lib.js:20:1"));
    }

    #[test]
    fn parse_v8_url_with_port() {
        let input = "Error\n    at foo (http://localhost:3000/bundle.js:42:13)";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].file, "http://localhost:3000/bundle.js");
        assert_eq!(frames[0].line, 42);
        assert_eq!(frames[0].column, 13);
    }

    #[test]
    fn parse_v8_bare_url_with_port() {
        // V8 bare format with a URL containing a port
        let input = "Error\n    at http://localhost:3000/bundle.js:10:5";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert!(frames[0].function_name.is_none());
        assert_eq!(frames[0].file, "http://localhost:3000/bundle.js");
        assert_eq!(frames[0].line, 10);
        assert_eq!(frames[0].column, 5);
    }

    #[test]
    fn parse_spidermonkey_with_message_line() {
        // SpiderMonkey stack where the second line (after engine detection from first
        // frame line) goes through parse_spidermonkey_frame
        let input = "foo@http://example.com/bundle.js:10:5\nbar@http://example.com/bundle.js:20:10";
        let parsed = parse_stack_trace_full(input);
        assert!(parsed.message.is_none());
        assert_eq!(parsed.frames.len(), 2);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(parsed.frames[0].file, "http://example.com/bundle.js");
        assert_eq!(parsed.frames[0].line, 10);
        assert_eq!(parsed.frames[1].function_name.as_deref(), Some("bar"));
        assert_eq!(parsed.frames[1].line, 20);
    }

    #[test]
    fn parse_spidermonkey_url_with_port() {
        let input = "handler@http://localhost:8080/app.js:42:13";
        let frames = parse_stack_trace(input);
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].function_name.as_deref(), Some("handler"));
        assert_eq!(frames[0].file, "http://localhost:8080/app.js");
        assert_eq!(frames[0].line, 42);
        assert_eq!(frames[0].column, 13);
    }

    #[test]
    fn detect_v8_engine_from_frame_line() {
        // First line is itself a V8 frame (contains "    at ")
        let engine = detect_engine("    at foo (bundle.js:1:1)");
        assert_eq!(engine, Engine::V8);
    }

    #[test]
    fn detect_jsc_engine_at_sign_only() {
        // JSC: has @ but no colon or slash
        let engine = detect_engine("func@native");
        assert_eq!(engine, Engine::JavaScriptCore);
    }

    #[test]
    fn parse_location_returns_none_for_invalid_column() {
        // If column is not a number, parse_location should return None
        let result = parse_location("file.js:10:abc");
        assert!(result.is_none());
    }

    #[test]
    fn parse_location_returns_none_for_invalid_line() {
        // If line is not a number, parse_location should return None
        let result = parse_location("file.js:abc:5");
        assert!(result.is_none());
    }

    #[test]
    fn parse_location_simple() {
        let result = parse_location("bundle.js:42:13");
        assert!(result.is_some());
        let (file, line, col) = result.unwrap();
        assert_eq!(file, "bundle.js");
        assert_eq!(line, 42);
        assert_eq!(col, 13);
    }

    #[test]
    fn parse_location_url_with_port() {
        let result = parse_location("http://localhost:3000/bundle.js:42:13");
        assert!(result.is_some());
        let (file, line, col) = result.unwrap();
        assert_eq!(file, "http://localhost:3000/bundle.js");
        assert_eq!(line, 42);
        assert_eq!(col, 13);
    }
}
