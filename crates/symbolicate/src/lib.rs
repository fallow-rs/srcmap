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

// ── Parser ──────────────────────────────────────────────────────

/// Parse a stack trace string into individual frames.
///
/// Supports V8 (Chrome, Node.js), SpiderMonkey (Firefox), and
/// JavaScriptCore (Safari) stack trace formats.
pub fn parse_stack_trace(input: &str) -> Vec<StackFrame> {
    parse_stack_trace_full(input).frames
}

/// Parse a stack trace string into message + frames.
///
/// Each line selects its own frame format. The first line is the message
/// when it does not parse as a frame.
pub fn parse_stack_trace_full(input: &str) -> ParsedStack {
    let mut lines = input.lines();
    let mut message = None;
    let mut frames = Vec::new();

    let Some(first_line) = lines.next() else {
        return ParsedStack { message: None, frames: Vec::new() };
    };

    match parse_frame(first_line) {
        Some(frame) => frames.push(frame),
        None => message = Some(first_line.to_string()),
    }

    frames.extend(lines.filter_map(parse_frame));

    ParsedStack { message, frames }
}

/// Parse a single stack frame line. A line that starts with `at ` is a V8
/// frame. Any other line with `@` is a SpiderMonkey or JavaScriptCore frame.
fn parse_frame(line: &str) -> Option<StackFrame> {
    let trimmed = line.trim();
    if trimmed.starts_with("at ") {
        parse_v8_frame(trimmed)
    } else if trimmed.contains('@') {
        parse_spidermonkey_frame(trimmed)
    } else {
        None
    }
}

/// Parse a V8 stack frame: `at functionName (file:line:column)` or `at file:line:column`
fn parse_v8_frame(line: &str) -> Option<StackFrame> {
    let rest = line.strip_prefix("at ")?;

    // `functionName (file:line:column)`: the location starts at the first
    // ` (`, so parentheses inside the path stay part of the file.
    if let Some(inner) = rest.strip_suffix(')') {
        let open = if inner.starts_with('(') { Some(0) } else { inner.find(" (").map(|i| i + 1) };
        if let Some(open) = open {
            let func = inner[..open].trim();
            let location = v8_eval_location(&inner[open + 1..]);
            let (file, line_num, col) = parse_location(location.trim())?;

            return Some(StackFrame {
                function_name: if func.is_empty() { None } else { Some(func.to_string()) },
                file,
                line: line_num,
                column: col,
            });
        }
    }

    // Bare `file:line:column` format
    let (file, line_num, col) = parse_location(rest)?;
    Some(StackFrame { function_name: None, file, line: line_num, column: col })
}

/// Select the script location of a V8 eval frame.
///
/// For `eval at f (a.js:1:2), <anonymous>:1:5` this returns `a.js:1:2`, the
/// innermost parenthesized location. Each `eval at name (` prefix is
/// removed, and the location ends at the first `), `, so parentheses inside
/// the path stay part of the file. Other locations return unchanged.
fn v8_eval_location(location: &str) -> &str {
    let mut inner = location;
    while let Some(after) = inner.strip_prefix("eval at ") {
        let Some(open) = after.find(" (") else {
            return location;
        };
        inner = &after[open + 2..];
    }
    if inner.len() == location.len() {
        return location;
    }
    inner.split_once("), ").map_or(inner, |(script, _)| script)
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
    symbolicate_frames(stack, loader)
}

/// Symbolicate a stack trace, and find each source map by debug ID first.
///
/// For each unique generated file in the stack trace, the function does these
/// steps:
///
/// 1. It calls `debug_id_for_file` with the file of the frame. The caller
///    returns the debug ID of that file, for example from
///    [`srcmap_sourcemap::extract_debug_id`] or from a table that the runtime
///    sent with the error.
/// 2. If there is a debug ID, it calls `map_for_debug_id` with that ID. A
///    [`DebugIdIndex`] can answer this call.
/// 3. If there is no debug ID, or no source map has it, it calls
///    `map_for_file` with the file of the frame, as [`symbolicate`] does.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::{SourceMap, extract_debug_id};
/// use srcmap_symbolicate::{DebugIdIndex, symbolicate_with_debug_ids};
///
/// let generated = "f();\n//# debugId=85314830-023f-4cf1-a267-535f4e37bb17\n";
/// let map = SourceMap::from_json(
///     r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA",
///         "debugId":"85314830-023f-4cf1-a267-535f4e37bb17"}"#,
/// )
/// .unwrap();
/// let mut index = DebugIdIndex::new();
/// index.insert(map);
///
/// let stack = "Error: oops\n    at f (https://cdn.example.com/app.min.js:1:1)";
/// let result = symbolicate_with_debug_ids(
///     stack,
///     |_file| extract_debug_id(generated),
///     |debug_id| index.get(debug_id).cloned(),
///     |_file| None,
/// );
///
/// assert_eq!(result.frames[0].file, "src/app.ts");
/// ```
pub fn symbolicate_with_debug_ids<D, M, F>(
    stack: &str,
    debug_id_for_file: D,
    map_for_debug_id: M,
    map_for_file: F,
) -> SymbolicatedStack
where
    D: Fn(&str) -> Option<String>,
    M: Fn(&str) -> Option<SourceMap>,
    F: Fn(&str) -> Option<SourceMap>,
{
    symbolicate_frames(stack, |file| {
        debug_id_for_file(file)
            .and_then(|debug_id| map_for_debug_id(&debug_id))
            .or_else(|| map_for_file(file))
    })
}

fn symbolicate_frames<F>(stack: &str, loader: F) -> SymbolicatedStack
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
///
/// The function examines each map, and it ignores the case of the hex digits.
/// For many lookups, use a [`DebugIdIndex`].
pub fn resolve_by_debug_id<'a>(
    debug_id: &str,
    maps: &'a HashMap<String, SourceMap>,
) -> Option<&'a SourceMap> {
    maps.values()
        .find(|sm| sm.debug_id.as_deref().is_some_and(|id| id.eq_ignore_ascii_case(debug_id)))
}

/// A set of source maps that a caller finds by debug ID.
///
/// The index ignores the case of the hex digits in a debug ID, because the
/// debug ID proposal permits upper-case and lower-case digits.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::SourceMap;
/// use srcmap_symbolicate::DebugIdIndex;
///
/// let map = SourceMap::from_json(
///     r#"{"version":3,"sources":["src/app.ts"],"names":[],"mappings":"AAAA",
///         "debugId":"85314830-023f-4cf1-a267-535f4e37bb17"}"#,
/// )
/// .unwrap();
///
/// let mut index = DebugIdIndex::new();
/// assert!(index.insert(map));
/// assert!(index.get("85314830-023F-4CF1-A267-535F4E37BB17").is_some());
/// assert!(index.get("00000000-0000-4000-8000-000000000000").is_none());
/// ```
#[derive(Debug, Clone, Default)]
pub struct DebugIdIndex {
    maps: HashMap<String, SourceMap>,
}

impl DebugIdIndex {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a source map under its own `debugId` field.
    ///
    /// Returns `false`, and does not add the map, when the map has no
    /// `debugId` field. A map replaces an earlier map with the same debug ID.
    pub fn insert(&mut self, map: SourceMap) -> bool {
        let Some(debug_id) = map.debug_id.clone() else {
            return false;
        };
        self.insert_with_id(&debug_id, map);
        true
    }

    /// Add a source map under a debug ID that the caller gives.
    ///
    /// Use this function when the debug ID comes from a different place than
    /// the map, for example from the name of an uploaded file. A map replaces
    /// an earlier map with the same debug ID.
    pub fn insert_with_id(&mut self, debug_id: &str, map: SourceMap) {
        self.maps.insert(debug_id.to_ascii_lowercase(), map);
    }

    /// Find the source map for a debug ID.
    pub fn get(&self, debug_id: &str) -> Option<&SourceMap> {
        self.maps.get(&debug_id.to_ascii_lowercase())
    }

    /// The number of source maps in the index.
    pub fn len(&self) -> usize {
        self.maps.len()
    }

    /// Returns `true` when the index has no source maps.
    pub fn is_empty(&self) -> bool {
        self.maps.is_empty()
    }
}

impl Extend<SourceMap> for DebugIdIndex {
    /// Add each source map that has a `debugId` field.
    fn extend<I: IntoIterator<Item = SourceMap>>(&mut self, maps: I) {
        for map in maps {
            self.insert(map);
        }
    }
}

impl FromIterator<SourceMap> for DebugIdIndex {
    /// Collect each source map that has a `debugId` field.
    fn from_iter<I: IntoIterator<Item = SourceMap>>(maps: I) -> Self {
        let mut index = Self::new();
        index.extend(maps);
        index
    }
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
    fn parse_v8_message_with_at_sign() {
        let input = "Error: user@example.com not found\n    at foo (bundle.js:1:1)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error: user@example.com not found"));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(parsed.frames[0].file, "bundle.js");
        assert_eq!(parsed.frames[0].line, 1);
        assert_eq!(parsed.frames[0].column, 1);
    }

    #[test]
    fn parse_spidermonkey_after_message() {
        let input = "TypeError: x is undefined\nfoo@bundle.js:1:2";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("TypeError: x is undefined"));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(parsed.frames[0].file, "bundle.js");
        assert_eq!(parsed.frames[0].line, 1);
        assert_eq!(parsed.frames[0].column, 2);
    }

    #[test]
    fn parse_v8_path_with_parentheses() {
        let input = "Error\n    at foo (/app/(shop)/bundle.js:1:1)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error"));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("foo"));
        assert_eq!(parsed.frames[0].file, "/app/(shop)/bundle.js");
        assert_eq!(parsed.frames[0].line, 1);
        assert_eq!(parsed.frames[0].column, 1);
    }

    #[test]
    fn parse_v8_bare_path_with_parentheses() {
        let input = "Error\n    at /app/(shop)/bundle.js:1:1";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error"));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name, None);
        assert_eq!(parsed.frames[0].file, "/app/(shop)/bundle.js");
        assert_eq!(parsed.frames[0].line, 1);
        assert_eq!(parsed.frames[0].column, 1);
    }

    #[test]
    fn parse_v8_eval_frame_uses_inner_location() {
        let input = "Error\n    at eval (eval at f (a.js:1:2), <anonymous>:1:5)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error"));
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("eval"));
        assert_eq!(parsed.frames[0].file, "a.js");
        assert_eq!(parsed.frames[0].line, 1);
        assert_eq!(parsed.frames[0].column, 2);
    }

    #[test]
    fn parse_v8_eval_frame_with_parentheses_in_path() {
        let input =
            "Error\n    at eval (eval at runEval (/app/(shop)/bundle.js:20:9), <anonymous>:1:1)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("eval"));
        assert_eq!(parsed.frames[0].file, "/app/(shop)/bundle.js");
        assert_eq!(parsed.frames[0].line, 20);
        assert_eq!(parsed.frames[0].column, 9);
    }

    #[test]
    fn parse_v8_nested_eval_frame_uses_innermost_location() {
        let input = "Error\n    at eval (eval at f (eval at g (a.js:3:4), <anonymous>:2:2), <anonymous>:1:5)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.frames.len(), 1);
        assert_eq!(parsed.frames[0].function_name.as_deref(), Some("eval"));
        assert_eq!(parsed.frames[0].file, "a.js");
        assert_eq!(parsed.frames[0].line, 3);
        assert_eq!(parsed.frames[0].column, 4);
    }

    #[test]
    fn parse_v8_frame_without_location_is_skipped() {
        let input = "Error\n    at async Promise.all (index 0)";
        let parsed = parse_stack_trace_full(input);
        assert_eq!(parsed.message.as_deref(), Some("Error"));
        assert_eq!(parsed.frames.len(), 0);
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
