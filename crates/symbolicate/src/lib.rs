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

use srcmap_scopes::GeneratedRange;
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
    /// Original function name from source map mappings or scopes data.
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

#[derive(Debug, Clone, Copy)]
enum Engine {
    V8,
    /// SpiderMonkey and JavaScriptCore share the `name@file:line:column` format.
    SpiderMonkey,
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

    let Some(first_line) = lines.next() else {
        return ParsedStack { message: None, frames: Vec::new() };
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

/// Detect the stack trace format from its first line. Error message lines
/// default to V8.
fn detect_engine(first_line: &str) -> Engine {
    let trimmed = first_line.trim();
    if trimmed.contains('@') && !trimmed.contains("    at ") {
        Engine::SpiderMonkey
    } else {
        Engine::V8
    }
}

/// Check if a line looks like a stack frame (vs an error message).
fn is_frame_line(line: &str, engine: Engine) -> bool {
    let trimmed = line.trim();
    match engine {
        Engine::V8 => trimmed.starts_with("at "),
        Engine::SpiderMonkey => trimmed.contains('@'),
    }
}

/// Parse a single stack frame line.
fn parse_frame(line: &str, engine: Engine) -> Option<StackFrame> {
    let trimmed = line.trim();

    match engine {
        Engine::V8 => parse_v8_frame(trimmed),
        Engine::SpiderMonkey => parse_spidermonkey_frame(trimmed),
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

/// Parse a SpiderMonkey or JavaScriptCore stack frame: `functionName@file:line:column`
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
    let mut cache: HashMap<String, Option<SourceMap>> = HashMap::new();
    let mut frames = Vec::with_capacity(parsed.frames.len());

    for frame in &parsed.frames {
        let sm = cache.entry(frame.file.clone()).or_insert_with(|| loader(&frame.file));

        // Stack traces are 1-based, source maps are 0-based
        let line = frame.line.saturating_sub(1);
        let column = frame.column.saturating_sub(1);

        let resolved = sm.as_ref().and_then(|sm| {
            let loc = sm.original_position_for(line, column)?;
            Some(SymbolicatedFrame {
                function_name: loc
                    .name
                    .map(|n| sm.name(n).to_string())
                    .or_else(|| find_original_function_name(sm, line, column))
                    .or_else(|| frame.function_name.clone()),
                file: sm.source(loc.source).to_string(),
                line: loc.line + 1,
                column: loc.column + 1,
                symbolicated: true,
            })
        });

        frames.push(resolved.unwrap_or_else(|| SymbolicatedFrame {
            function_name: frame.function_name.clone(),
            file: frame.file.clone(),
            line: frame.line,
            column: frame.column,
            symbolicated: false,
        }));
    }

    SymbolicatedStack { message: parsed.message, frames }
}

fn find_original_function_name(sm: &SourceMap, line: u32, column: u32) -> Option<String> {
    let scopes = sm.scopes.as_ref()?;
    let mut path = Vec::new();
    if !collect_innermost_range_path(&scopes.ranges, line, column, &mut path) {
        return None;
    }

    for range in path.iter().rev() {
        if !range.is_stack_frame || range.is_hidden {
            continue;
        }

        let Some(definition) = range.definition else {
            continue;
        };
        let scope = scopes.original_scope_for_definition(definition)?;
        if let Some(name) = &scope.name {
            return Some(name.clone());
        }
    }

    None
}

fn collect_innermost_range_path<'a>(
    ranges: &'a [GeneratedRange],
    line: u32,
    column: u32,
    path: &mut Vec<&'a GeneratedRange>,
) -> bool {
    for range in ranges {
        if !range_contains_position(range, line, column) {
            continue;
        }

        path.push(range);
        collect_innermost_range_path(&range.children, line, column, path);
        return true;
    }

    false
}

fn range_contains_position(range: &GeneratedRange, line: u32, column: u32) -> bool {
    let pos = (line, column);
    let start = (range.start.line, range.start.column);
    let end = (range.end.line, range.end.column);
    start <= pos && pos <= end
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
    use srcmap_scopes::{GeneratedRange, OriginalScope, Position, ScopeInfo};

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
    fn symbolicate_uses_scopes_function_name_when_mapping_name_is_absent() {
        let scope_info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 10, column: 0 },
                end: Position { line: 20, column: 0 },
                name: Some("originalFunc".to_string()),
                kind: Some("function".to_string()),
                is_stack_frame: true,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 10 },
                is_stack_frame: true,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&scope_info, &mut names);
        let names_json = serde_json::to_string(&names).unwrap();
        let map_json = format!(
            r#"{{"version":3,"sources":["src/app.ts"],"names":{names_json},"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );

        let result = symbolicate("Error: test\n    at foo (bundle.js:1:1)", |file| {
            if file == "bundle.js" { SourceMap::from_json(&map_json).ok() } else { None }
        });

        assert_eq!(result.frames[0].function_name.as_deref(), Some("originalFunc"));
    }

    #[test]
    fn symbolicate_skips_hidden_scopes_and_uses_outer_stack_frame_name() {
        let scope_info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 30, column: 0 },
                name: Some("outerFunc".to_string()),
                kind: Some("function".to_string()),
                is_stack_frame: true,
                variables: vec![],
                children: vec![OriginalScope {
                    start: Position { line: 5, column: 0 },
                    end: Position { line: 10, column: 0 },
                    name: Some("hiddenInner".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec![],
                    children: vec![],
                }],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 20 },
                is_stack_frame: true,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![GeneratedRange {
                    start: Position { line: 0, column: 5 },
                    end: Position { line: 0, column: 10 },
                    is_stack_frame: true,
                    is_hidden: true,
                    definition: Some(1),
                    call_site: None,
                    bindings: vec![],
                    children: vec![],
                }],
            }],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&scope_info, &mut names);
        let names_json = serde_json::to_string(&names).unwrap();
        let map_json = format!(
            r#"{{"version":3,"sources":["src/app.ts"],"names":{names_json},"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );

        let result = symbolicate("Error: test\n    at foo (bundle.js:1:7)", |file| {
            if file == "bundle.js" { SourceMap::from_json(&map_json).ok() } else { None }
        });

        assert_eq!(result.frames[0].function_name.as_deref(), Some("outerFunc"));
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
    fn parse_empty_input() {
        let parsed = parse_stack_trace_full("");
        assert!(parsed.message.is_none());
        assert_eq!(parsed.frames, Vec::<StackFrame>::new());
    }

    #[test]
    fn parse_unparsable_lines() {
        // Lines that don't match any frame format
        let input = "Error: boom\n  this is not a frame\n  neither is this";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error: boom"));
        assert_eq!(parsed.frames, Vec::<StackFrame>::new());
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
    fn parse_location_empty_file() {
        // parse_location returns None when file component is empty
        let input = "Error\n    at (:10:5)";
        let frames = parse_stack_trace(input);
        assert_eq!(frames, Vec::<StackFrame>::new());
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
        assert!(result.frames[0].symbolicated);
        assert!(result.frames[1].symbolicated);
        assert_eq!(call_count.get(), 1);
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
}
