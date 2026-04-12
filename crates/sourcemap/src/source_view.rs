//! Efficient source view with lazy line caching and UTF-16 column support.
//!
//! [`SourceView`] provides indexed access to lines in a JavaScript source file,
//! with support for UTF-16 column offsets (as used by source maps) and
//! heuristic function name inference for error grouping.

use std::sync::{Arc, OnceLock};

use crate::js_identifiers::is_valid_javascript_identifier;
use crate::{OriginalLocation, SourceMap};

/// Pre-computed byte offset pair for a single line.
///
/// `start` is inclusive, `end` is exclusive, and excludes the line terminator.
#[derive(Debug, Clone, Copy)]
struct LineRange {
    start: usize,
    end: usize,
}

/// Efficient view into a JavaScript source string with lazy line indexing.
///
/// Stores the source as `Arc<str>` for cheap cloning. Line boundaries are
/// computed on first access and cached with [`OnceLock`] for lock-free
/// concurrent reads.
///
/// # UTF-16 column support
///
/// JavaScript source maps use UTF-16 code-unit offsets for columns. Methods
/// like [`get_line_slice`](SourceView::get_line_slice) accept UTF-16 columns
/// and convert them to byte offsets internally.
#[derive(Debug, Clone)]
pub struct SourceView {
    source: Arc<str>,
    line_cache: OnceLock<Vec<LineRange>>,
}

impl SourceView {
    /// Create a new `SourceView` from a shared source string.
    pub fn new(source: Arc<str>) -> Self {
        Self { source, line_cache: OnceLock::new() }
    }

    /// Create a new `SourceView` from an owned `String`.
    pub fn from_string(source: String) -> Self {
        Self::new(Arc::from(source))
    }

    /// Return the full source string.
    #[inline]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Return the number of lines in the source.
    ///
    /// A trailing newline does NOT produce an extra empty line
    /// (e.g. `"a\n"` has 1 line, `"a\nb"` has 2 lines).
    pub fn line_count(&self) -> usize {
        self.lines().len()
    }

    /// Get a specific line by 0-based index.
    ///
    /// Returns `None` if `idx` is out of bounds. The returned slice does NOT
    /// include the line terminator.
    pub fn get_line(&self, idx: u32) -> Option<&str> {
        let lines = self.lines();
        let range = lines.get(idx as usize)?;
        Some(&self.source[range.start..range.end])
    }

    /// Get a substring of a line using UTF-16 column offsets.
    ///
    /// `line` and `col` are 0-based. `span` is the number of UTF-16 code units
    /// to include. Returns `None` if the line index is out of bounds or the
    /// column/span extends past the line.
    ///
    /// This is necessary because JavaScript source maps encode columns as
    /// UTF-16 code unit offsets, but Rust strings are UTF-8.
    pub fn get_line_slice(&self, line: u32, col: u32, span: u32) -> Option<&str> {
        let line_str = self.get_line(line)?;
        let start_byte = utf16_col_to_byte_offset(line_str, col)?;
        let end_byte = utf16_offset_from(line_str, start_byte, span)?;
        Some(&line_str[start_byte..end_byte])
    }

    /// Attempt to infer the original function name for a token.
    ///
    /// This is a best-effort heuristic used for error grouping. Given a token's
    /// position in the generated (minified) source and the minified name at
    /// that position, it looks at surrounding code patterns to identify the
    /// function context.
    ///
    /// # Algorithm
    ///
    /// 1. Find the mapping at the token's generated position
    /// 2. Look backwards from that position in the generated line to find
    ///    patterns like `name(`, `name:`, `name =`, `.name`
    /// 3. If the identified name matches `minified_name`, look it up in the
    ///    source map for the original name
    pub fn get_original_function_name<'a>(
        &self,
        token: &OriginalLocation,
        minified_name: &str,
        sm: &'a SourceMap,
    ) -> Option<&'a str> {
        // We need to find where this original location maps to in the generated source
        let source_name = sm.get_source(token.source)?;
        let gen_loc = sm.generated_position_for(source_name, token.line, token.column)?;

        let line_str = self.get_line(gen_loc.line)?;
        let col_byte = utf16_col_to_byte_offset(line_str, gen_loc.column)?;

        // Look at code before the token position to find function name patterns
        let prefix = &line_str[..col_byte];

        // Try to find what precedes this position
        let candidate = extract_function_name_candidate(prefix)?;

        if !is_valid_javascript_identifier(candidate) {
            return None;
        }

        // If the candidate matches the minified name, look for the original
        // name by finding a mapping at the candidate's position
        if candidate != minified_name {
            return None;
        }

        // Find the byte offset where the candidate starts in the line
        let candidate_start_byte = prefix.len() - candidate.len();
        let candidate_col = byte_offset_to_utf16_col(line_str, candidate_start_byte);

        // Look up the original location for this generated position
        let original = sm.original_position_for(gen_loc.line, candidate_col)?;
        let name_idx = original.name?;
        sm.get_name(name_idx)
    }

    /// Compute or retrieve the cached line ranges.
    fn lines(&self) -> &[LineRange] {
        self.line_cache.get_or_init(|| compute_line_ranges(&self.source))
    }
}

/// Compute `(start, end)` byte-offset pairs for every line.
///
/// Handles LF (`\n`), CR (`\r`), and CRLF (`\r\n`) terminators.
/// A trailing newline does NOT produce an extra empty line.
fn compute_line_ranges(source: &str) -> Vec<LineRange> {
    let bytes = source.as_bytes();
    let len = bytes.len();

    if len == 0 {
        return vec![];
    }

    let mut ranges = Vec::new();
    let mut start = 0;
    let mut i = 0;

    while i < len {
        match bytes[i] {
            b'\n' => {
                ranges.push(LineRange { start, end: i });
                start = i + 1;
                i += 1;
            }
            b'\r' => {
                ranges.push(LineRange { start, end: i });
                // Skip \n in \r\n
                if i + 1 < len && bytes[i + 1] == b'\n' {
                    i += 2;
                } else {
                    i += 1;
                }
                start = i;
            }
            _ => {
                i += 1;
            }
        }
    }

    // If the source doesn't end with a newline, add the last line
    if start < len {
        ranges.push(LineRange { start, end: len });
    }

    ranges
}

/// Convert a UTF-16 column offset to a byte offset within a UTF-8 string.
///
/// Returns `None` if the column is past the end of the string.
fn utf16_col_to_byte_offset(s: &str, col: u32) -> Option<usize> {
    if col == 0 {
        return Some(0);
    }

    let mut utf16_offset = 0u32;
    for (byte_idx, ch) in s.char_indices() {
        if utf16_offset == col {
            return Some(byte_idx);
        }
        utf16_offset += ch.len_utf16() as u32;
        if utf16_offset > col {
            // Column points into the middle of a surrogate pair
            return None;
        }
    }

    // Column exactly at the end of the string
    if utf16_offset == col {
        return Some(s.len());
    }

    None
}

/// Advance `span` UTF-16 code units from `start_byte` and return the resulting byte offset.
///
/// Returns `None` if the span extends past the end of the string.
fn utf16_offset_from(s: &str, start_byte: usize, span: u32) -> Option<usize> {
    if span == 0 {
        return Some(start_byte);
    }

    let tail = s.get(start_byte..)?;
    let mut utf16_offset = 0u32;
    for (byte_idx, ch) in tail.char_indices() {
        if utf16_offset == span {
            return Some(start_byte + byte_idx);
        }
        utf16_offset += ch.len_utf16() as u32;
        if utf16_offset > span {
            return None;
        }
    }

    if utf16_offset == span {
        return Some(start_byte + tail.len());
    }

    None
}

/// Convert a byte offset to a UTF-16 column offset.
fn byte_offset_to_utf16_col(s: &str, byte_offset: usize) -> u32 {
    let prefix = &s[..byte_offset];
    prefix.chars().map(|c| c.len_utf16() as u32).sum()
}

/// Extract a function name candidate from the text preceding a token.
///
/// Looks for patterns like:
/// - `name(` — function call or declaration
/// - `name:` — object property
/// - `name =` / `name=` — assignment
/// - `.name` — member access
/// - `var name` / `let name` / `const name` — variable declaration
fn extract_function_name_candidate(prefix: &str) -> Option<&str> {
    let trimmed = prefix.trim_end();
    if trimmed.is_empty() {
        return None;
    }

    let last_char = trimmed.chars().next_back()?;

    match last_char {
        // `name(` pattern — the prefix already has the `(` trimmed, look for ident before it
        '(' | ',' => {
            let before_paren = trimmed[..trimmed.len() - last_char.len_utf8()].trim_end();
            extract_trailing_identifier(before_paren)
        }
        // `name:` pattern
        ':' => {
            let before_colon = trimmed[..trimmed.len() - last_char.len_utf8()].trim_end();
            extract_trailing_identifier(before_colon)
        }
        // `name =` pattern
        '=' => {
            // Make sure it's not `==`, `!=`, `>=`, `<=`, `+=`, `-=`, etc.
            let before_eq_str = &trimmed[..trimmed.len() - 1];
            if let Some(prev) = before_eq_str.chars().next_back()
                && matches!(
                    prev,
                    '=' | '!' | '>' | '<' | '+' | '-' | '*' | '/' | '%' | '|' | '&' | '^' | '?'
                )
            {
                return None;
            }
            let before_eq = before_eq_str.trim_end();
            extract_trailing_identifier(before_eq)
        }
        // `.name` or identifier at end — try to get the trailing identifier
        _ if last_char.is_ascii_alphanumeric()
            || last_char == '_'
            || last_char == '$'
            || (!last_char.is_ascii() && last_char.is_alphanumeric()) =>
        {
            let ident = extract_trailing_identifier(trimmed)?;
            let before = trimmed[..trimmed.len() - ident.len()].trim_end();
            if before.ends_with('.') {
                return Some(ident);
            }
            // Keyword patterns: var/let/const
            if before.ends_with("var ")
                || before.ends_with("let ")
                || before.ends_with("const ")
                || before.ends_with("function ")
            {
                return Some(ident);
            }
            Some(ident)
        }
        _ => None,
    }
}

/// Extract the trailing JavaScript identifier from a string.
///
/// Scans backwards from the end to find the start of the identifier.
fn extract_trailing_identifier(s: &str) -> Option<&str> {
    if s.is_empty() {
        return None;
    }

    let end = s.len();
    let mut chars = s.char_indices().rev().peekable();

    // Find identifier characters from the end
    let mut start = end;
    while let Some((idx, ch)) = chars.peek() {
        if ch.is_ascii_alphanumeric()
            || *ch == '_'
            || *ch == '$'
            || *ch == '\u{200c}'
            || *ch == '\u{200d}'
            || (!ch.is_ascii() && ch.is_alphanumeric())
        {
            start = *idx;
            chars.next();
        } else {
            break;
        }
    }

    if start == end {
        return None;
    }

    let ident = &s[start..end];

    // Verify it starts with a valid identifier start character
    let first = ident.chars().next()?;
    if first.is_ascii_digit() {
        return None;
    }

    if is_valid_javascript_identifier(ident) { Some(ident) } else { None }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::SourceMap;

    use super::*;

    // ── Line cache tests ─────────────────────────────────────────

    #[test]
    fn test_empty_source() {
        let view = SourceView::from_string(String::new());
        assert_eq!(view.line_count(), 0);
        assert_eq!(view.get_line(0), None);
    }

    #[test]
    fn test_single_line_no_newline() {
        let view = SourceView::from_string("hello world".into());
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.get_line(0), Some("hello world"));
        assert_eq!(view.get_line(1), None);
    }

    #[test]
    fn test_single_line_with_trailing_lf() {
        let view = SourceView::from_string("hello\n".into());
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.get_line(0), Some("hello"));
    }

    #[test]
    fn test_multiple_lines_lf() {
        let view = SourceView::from_string("line1\nline2\nline3".into());
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.get_line(0), Some("line1"));
        assert_eq!(view.get_line(1), Some("line2"));
        assert_eq!(view.get_line(2), Some("line3"));
    }

    #[test]
    fn test_multiple_lines_cr() {
        let view = SourceView::from_string("line1\rline2\rline3".into());
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.get_line(0), Some("line1"));
        assert_eq!(view.get_line(1), Some("line2"));
        assert_eq!(view.get_line(2), Some("line3"));
    }

    #[test]
    fn test_multiple_lines_crlf() {
        let view = SourceView::from_string("line1\r\nline2\r\nline3".into());
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.get_line(0), Some("line1"));
        assert_eq!(view.get_line(1), Some("line2"));
        assert_eq!(view.get_line(2), Some("line3"));
    }

    #[test]
    fn test_mixed_line_endings() {
        let view = SourceView::from_string("a\nb\rc\r\nd".into());
        assert_eq!(view.line_count(), 4);
        assert_eq!(view.get_line(0), Some("a"));
        assert_eq!(view.get_line(1), Some("b"));
        assert_eq!(view.get_line(2), Some("c"));
        assert_eq!(view.get_line(3), Some("d"));
    }

    #[test]
    fn test_empty_lines() {
        let view = SourceView::from_string("\n\n\n".into());
        assert_eq!(view.line_count(), 3);
        assert_eq!(view.get_line(0), Some(""));
        assert_eq!(view.get_line(1), Some(""));
        assert_eq!(view.get_line(2), Some(""));
    }

    #[test]
    fn test_crlf_trailing() {
        let view = SourceView::from_string("a\r\n".into());
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.get_line(0), Some("a"));
    }

    #[test]
    fn test_cr_trailing() {
        let view = SourceView::from_string("a\r".into());
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.get_line(0), Some("a"));
    }

    // ── UTF-16 column tests ─────────────────────────────────────

    #[test]
    fn test_get_line_slice_ascii() {
        let view = SourceView::from_string("abcdefgh".into());
        assert_eq!(view.get_line_slice(0, 2, 3), Some("cde"));
        assert_eq!(view.get_line_slice(0, 0, 8), Some("abcdefgh"));
        assert_eq!(view.get_line_slice(0, 0, 0), Some(""));
    }

    #[test]
    fn test_get_line_slice_multibyte() {
        // Each char here is 3 bytes in UTF-8 but 1 UTF-16 code unit
        let view = SourceView::from_string("\u{00e9}\u{00e8}\u{00ea}abc".into());
        // UTF-16 col 0 = \u{00e9}, col 1 = \u{00e8}, col 2 = \u{00ea}, col 3 = a, etc.
        assert_eq!(view.get_line_slice(0, 0, 3), Some("\u{00e9}\u{00e8}\u{00ea}"));
        assert_eq!(view.get_line_slice(0, 3, 3), Some("abc"));
    }

    #[test]
    fn test_get_line_slice_emoji_surrogate_pair() {
        // Emoji: U+1F600 (GRINNING FACE) is 4 bytes in UTF-8, 2 UTF-16 code units
        let view = SourceView::from_string("a\u{1F600}b".into());
        // UTF-16: col 0 = 'a' (1 unit), col 1-2 = emoji (2 units), col 3 = 'b' (1 unit)
        assert_eq!(view.get_line_slice(0, 0, 1), Some("a"));
        assert_eq!(view.get_line_slice(0, 1, 2), Some("\u{1F600}"));
        assert_eq!(view.get_line_slice(0, 3, 1), Some("b"));
        assert_eq!(view.get_line_slice(0, 0, 4), Some("a\u{1F600}b"));
    }

    #[test]
    fn test_get_line_slice_surrogate_pair_middle() {
        // Pointing into the middle of a surrogate pair should return None
        let view = SourceView::from_string("\u{1F600}".into());
        assert_eq!(view.get_line_slice(0, 1, 1), None); // middle of surrogate pair
    }

    #[test]
    fn test_get_line_slice_out_of_bounds() {
        let view = SourceView::from_string("abc".into());
        assert_eq!(view.get_line_slice(0, 0, 10), None); // span too long
        assert_eq!(view.get_line_slice(0, 5, 1), None); // col past end
        assert_eq!(view.get_line_slice(1, 0, 1), None); // line doesn't exist
    }

    #[test]
    fn test_get_line_slice_cjk() {
        // CJK characters: U+4E16 and U+754C are 3 bytes in UTF-8, 1 UTF-16 code unit each
        let view = SourceView::from_string("x\u{4e16}\u{754c}y".into());
        assert_eq!(view.get_line_slice(0, 1, 2), Some("\u{4e16}\u{754c}"));
    }

    #[test]
    fn test_get_line_slice_multiline() {
        let view = SourceView::from_string("abc\ndef\nghi".into());
        assert_eq!(view.get_line_slice(0, 1, 2), Some("bc"));
        assert_eq!(view.get_line_slice(1, 0, 3), Some("def"));
        assert_eq!(view.get_line_slice(2, 2, 1), Some("i"));
    }

    // ── UTF-16 conversion tests ─────────────────────────────────

    #[test]
    fn test_utf16_col_to_byte_offset_ascii() {
        assert_eq!(utf16_col_to_byte_offset("abcd", 0), Some(0));
        assert_eq!(utf16_col_to_byte_offset("abcd", 2), Some(2));
        assert_eq!(utf16_col_to_byte_offset("abcd", 4), Some(4));
    }

    #[test]
    fn test_utf16_col_to_byte_offset_multibyte() {
        // \u{00e9} is 2 bytes in UTF-8, 1 UTF-16 code unit
        let s = "\u{00e9}a";
        assert_eq!(utf16_col_to_byte_offset(s, 0), Some(0));
        assert_eq!(utf16_col_to_byte_offset(s, 1), Some(2)); // after \u{00e9}
        assert_eq!(utf16_col_to_byte_offset(s, 2), Some(3)); // after 'a'
    }

    #[test]
    fn test_utf16_col_to_byte_offset_surrogate_pair() {
        // U+1F600 is 4 bytes in UTF-8, 2 UTF-16 code units
        let s = "\u{1F600}a";
        assert_eq!(utf16_col_to_byte_offset(s, 0), Some(0));
        assert_eq!(utf16_col_to_byte_offset(s, 1), None); // middle of surrogate pair
        assert_eq!(utf16_col_to_byte_offset(s, 2), Some(4)); // after emoji
        assert_eq!(utf16_col_to_byte_offset(s, 3), Some(5)); // after 'a'
    }

    #[test]
    fn test_byte_offset_to_utf16_col() {
        assert_eq!(byte_offset_to_utf16_col("abcd", 0), 0);
        assert_eq!(byte_offset_to_utf16_col("abcd", 2), 2);
        // \u{1F600} is 4 bytes, 2 UTF-16 units
        let s = "a\u{1F600}b";
        assert_eq!(byte_offset_to_utf16_col(s, 0), 0);
        assert_eq!(byte_offset_to_utf16_col(s, 1), 1); // after 'a'
        assert_eq!(byte_offset_to_utf16_col(s, 5), 3); // after emoji (1 + 2)
        assert_eq!(byte_offset_to_utf16_col(s, 6), 4); // after 'b'
    }

    // ── Function name candidate extraction tests ────────────────

    #[test]
    fn test_extract_function_call() {
        assert_eq!(extract_function_name_candidate("foo("), Some("foo"));
        assert_eq!(extract_function_name_candidate("  bar("), Some("bar"));
        assert_eq!(extract_function_name_candidate("obj.method("), Some("method"));
    }

    #[test]
    fn test_extract_assignment() {
        assert_eq!(extract_function_name_candidate("x ="), Some("x"));
        assert_eq!(extract_function_name_candidate("myVar ="), Some("myVar"));
        assert_eq!(extract_function_name_candidate("x =  "), Some("x"));
    }

    #[test]
    fn test_extract_colon() {
        assert_eq!(extract_function_name_candidate("key:"), Some("key"));
        assert_eq!(extract_function_name_candidate("  prop:"), Some("prop"));
    }

    #[test]
    fn test_extract_comparison_operators() {
        // These should NOT be treated as assignments
        assert_eq!(extract_function_name_candidate("x =="), None);
        assert_eq!(extract_function_name_candidate("x !="), None);
        assert_eq!(extract_function_name_candidate("x >="), None);
        assert_eq!(extract_function_name_candidate("x <="), None);
    }

    #[test]
    fn test_extract_member_access() {
        assert_eq!(extract_function_name_candidate("obj.prop"), Some("prop"));
        assert_eq!(
            extract_function_name_candidate("window.addEventListener"),
            Some("addEventListener")
        );
    }

    #[test]
    fn test_extract_variable_declaration() {
        assert_eq!(extract_function_name_candidate("var x"), Some("x"));
        assert_eq!(extract_function_name_candidate("let myVar"), Some("myVar"));
        assert_eq!(extract_function_name_candidate("const CONSTANT"), Some("CONSTANT"));
    }

    #[test]
    fn test_extract_none() {
        assert_eq!(extract_function_name_candidate(""), None);
        assert_eq!(extract_function_name_candidate("  "), None);
        assert_eq!(extract_function_name_candidate("123"), None);
    }

    #[test]
    fn test_extract_comma_separated() {
        assert_eq!(extract_function_name_candidate("foo(a,"), Some("a"));
    }

    // ── Arc / Send / Sync tests ──────────────────────────────────

    #[test]
    fn test_arc_construction() {
        let source: Arc<str> = Arc::from("test source");
        let view = SourceView::new(Arc::clone(&source));
        assert_eq!(view.source(), "test source");
    }

    #[test]
    fn test_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<SourceView>();
    }

    #[test]
    fn test_clone() {
        let view = SourceView::from_string("line1\nline2".into());
        // Prime the cache
        assert_eq!(view.line_count(), 2);
        let view2 = view.clone();
        assert_eq!(view.get_line(1), Some("line2"));
        assert_eq!(view2.line_count(), 2);
        assert_eq!(view2.get_line(0), Some("line1"));
    }

    // ── Integration test with SourceMap ──────────────────────────

    #[test]
    fn test_get_original_function_name() {
        // Build a source map that maps generated positions to original positions
        // Generated: "a(b)" where `a` was originally `originalFunc` and `b` was `originalArg`
        let json = r#"{
            "version": 3,
            "sources": ["input.js"],
            "names": ["originalFunc", "originalArg"],
            "mappings": "AAAA,CAAC"
        }"#;

        let sm = SourceMap::from_json(json).unwrap();

        // The generated source is "a(b)"
        // Mapping: gen 0:0 -> orig 0:0 (source 0), gen 0:2 -> orig 0:1 (source 0)
        // But we need names in the mappings for this to work.
        // Let's create a more realistic test with the builder.

        // For now, verify that the function works without crashing when there's no match
        let view = SourceView::from_string("a(b)".into());
        let token = OriginalLocation { source: 0, line: 0, column: 0, name: Some(0) };
        // This should return None since the heuristic won't find a matching minified name
        let result = view.get_original_function_name(&token, "nonexistent", &sm);
        assert_eq!(result, None);
    }

    #[test]
    fn test_get_original_function_name_with_match() {
        // Create a source map where:
        // Generated line 0: "a(b)"
        //   col 0 -> source 0, line 0, col 0, name "originalFunc" (names[0])
        //   col 2 -> source 0, line 0, col 5, name "originalArg" (names[1])
        //
        // We want to test: given a token at orig 0:5 (which maps to gen 0:2),
        // the code before gen 0:2 is "a(" — so the candidate is "a".
        // If minified_name is "a", we look up gen 0:0 to find name "originalFunc".

        // AAAAA = gen_col 0, source 0, orig_line 0, orig_col 0, name 0
        // EAAKC = gen_col +2, source +0, orig_line +0, orig_col +5, name +1
        let json = r#"{
            "version": 3,
            "sources": ["input.js"],
            "names": ["originalFunc", "originalArg"],
            "mappings": "AAAAA,EAAKC"
        }"#;

        let sm = SourceMap::from_json(json).unwrap();

        // Verify the mappings are correct
        let loc0 = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc0.source, 0);
        assert_eq!(loc0.line, 0);
        assert_eq!(loc0.column, 0);
        assert_eq!(loc0.name, Some(0));

        let loc2 = sm.original_position_for(0, 2).unwrap();
        assert_eq!(loc2.source, 0);
        assert_eq!(loc2.line, 0);
        assert_eq!(loc2.column, 5);
        assert_eq!(loc2.name, Some(1));

        // Generated source: "a(b)"
        let view = SourceView::from_string("a(b)".into());

        // Token is at original 0:5 with name "originalArg"
        // We want to find the function name "originalFunc" for this token
        let token = OriginalLocation { source: 0, line: 0, column: 5, name: Some(1) };

        // The minified name "a" should match the candidate extracted from "a("
        let result = view.get_original_function_name(&token, "a", &sm);
        assert_eq!(result, Some("originalFunc"));
    }

    #[test]
    fn test_line_cache_consistency() {
        let view = SourceView::from_string("a\nb\nc".into());
        // Access lines in different orders to ensure cache is consistent
        assert_eq!(view.get_line(2), Some("c"));
        assert_eq!(view.get_line(0), Some("a"));
        assert_eq!(view.get_line(1), Some("b"));
        assert_eq!(view.line_count(), 3);
    }

    #[test]
    fn test_only_newlines() {
        let view = SourceView::from_string("\n".into());
        assert_eq!(view.line_count(), 1);
        assert_eq!(view.get_line(0), Some(""));
    }

    #[test]
    fn test_consecutive_crlf() {
        let view = SourceView::from_string("\r\n\r\n".into());
        assert_eq!(view.line_count(), 2);
        assert_eq!(view.get_line(0), Some(""));
        assert_eq!(view.get_line(1), Some(""));
    }

    #[test]
    fn test_unicode_line_content() {
        let view = SourceView::from_string("Hello \u{4e16}\u{754c}\n\u{1F600} smile".into());
        assert_eq!(view.line_count(), 2);
        assert_eq!(view.get_line(0), Some("Hello \u{4e16}\u{754c}"));
        assert_eq!(view.get_line(1), Some("\u{1F600} smile"));
    }

    #[test]
    fn test_get_line_slice_at_line_end() {
        let view = SourceView::from_string("abc".into());
        // Slice at the very end with 0 span
        assert_eq!(view.get_line_slice(0, 3, 0), Some(""));
    }

    #[test]
    fn test_get_line_slice_full_line() {
        let view = SourceView::from_string("abc\ndef".into());
        assert_eq!(view.get_line_slice(0, 0, 3), Some("abc"));
        assert_eq!(view.get_line_slice(1, 0, 3), Some("def"));
    }
}
