//! High-performance source map generator (ECMA-426).
//!
//! Builds source maps incrementally by adding mappings one at a time.
//! Outputs standard source map v3 JSON.
//!
//! # Examples
//!
//! ```rust
//! use srcmap_generator::SourceMapGenerator;
//!
//! fn main() {
//!     let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));
//!
//!     let src = builder.add_source("src/app.ts");
//!     builder.set_source_content(src, "const x = 1;".to_string());
//!
//!     let name = builder.add_name("x");
//!     builder.add_named_mapping(0, 0, src, 0, 6, name);
//!     builder.add_mapping(1, 0, src, 1, 0);
//!
//!     let json = builder.to_json();
//!     assert!(json.contains(r#""version":3"#));
//!     assert!(json.contains(r#""sources":["src/app.ts"]"#));
//! }
//! ```

use std::collections::HashMap;

use srcmap_codec::vlq_encode;
use srcmap_scopes::ScopeInfo;

// ── Public types ───────────────────────────────────────────────────

/// A mapping from generated position to original position.
#[derive(Debug, Clone)]
pub struct Mapping {
    pub generated_line: u32,
    pub generated_column: u32,
    pub source: Option<u32>,
    pub original_line: u32,
    pub original_column: u32,
    pub name: Option<u32>,
}

/// Builder for creating source maps incrementally.
#[derive(Debug)]
pub struct SourceMapGenerator {
    file: Option<String>,
    source_root: Option<String>,
    sources: Vec<String>,
    sources_content: Vec<Option<String>>,
    names: Vec<String>,
    mappings: Vec<Mapping>,
    ignore_list: Vec<u32>,
    debug_id: Option<String>,
    scopes: Option<ScopeInfo>,

    // Dedup maps for O(1) lookup
    source_map: HashMap<String, u32>,
    name_map: HashMap<String, u32>,
}

impl SourceMapGenerator {
    /// Create a new empty source map generator.
    pub fn new(file: Option<String>) -> Self {
        Self {
            file,
            source_root: None,
            sources: Vec::new(),
            sources_content: Vec::new(),
            names: Vec::new(),
            mappings: Vec::new(),
            ignore_list: Vec::new(),
            debug_id: None,
            scopes: None,
            source_map: HashMap::new(),
            name_map: HashMap::new(),
        }
    }

    /// Set the source root prefix.
    pub fn set_source_root(&mut self, root: String) {
        self.source_root = Some(root);
    }

    /// Set the debug ID (UUID) for this source map (ECMA-426).
    pub fn set_debug_id(&mut self, id: String) {
        self.debug_id = Some(id);
    }

    /// Set scope and variable information (ECMA-426 scopes proposal).
    pub fn set_scopes(&mut self, scopes: ScopeInfo) {
        self.scopes = Some(scopes);
    }

    /// Register a source file and return its index.
    pub fn add_source(&mut self, source: &str) -> u32 {
        if let Some(&idx) = self.source_map.get(source) {
            return idx;
        }
        let idx = self.sources.len() as u32;
        self.sources.push(source.to_string());
        self.sources_content.push(None);
        self.source_map.insert(source.to_string(), idx);
        idx
    }

    /// Set the content for a source file.
    pub fn set_source_content(&mut self, source_idx: u32, content: String) {
        if (source_idx as usize) < self.sources_content.len() {
            self.sources_content[source_idx as usize] = Some(content);
        }
    }

    /// Register a name and return its index.
    pub fn add_name(&mut self, name: &str) -> u32 {
        if let Some(&idx) = self.name_map.get(name) {
            return idx;
        }
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.name_map.insert(name.to_string(), idx);
        idx
    }

    /// Add a source index to the ignore list.
    pub fn add_to_ignore_list(&mut self, source_idx: u32) {
        if !self.ignore_list.contains(&source_idx) {
            self.ignore_list.push(source_idx);
        }
    }

    /// Add a mapping with no source information (generated-only).
    pub fn add_generated_mapping(&mut self, generated_line: u32, generated_column: u32) {
        self.mappings.push(Mapping {
            generated_line,
            generated_column,
            source: None,
            original_line: 0,
            original_column: 0,
            name: None,
        });
    }

    /// Add a mapping from generated position to original position.
    pub fn add_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
    ) {
        self.mappings.push(Mapping {
            generated_line,
            generated_column,
            source: Some(source),
            original_line,
            original_column,
            name: None,
        });
    }

    /// Add a mapping with a name.
    pub fn add_named_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
        name: u32,
    ) {
        self.mappings.push(Mapping {
            generated_line,
            generated_column,
            source: Some(source),
            original_line,
            original_column,
            name: Some(name),
        });
    }

    /// Add a mapping only if it differs from the previous mapping on the same line.
    ///
    /// This skips redundant mappings where the source position is identical
    /// to the last mapping, which reduces output size without losing information.
    /// Used by bundlers and minifiers to avoid bloating source maps.
    pub fn maybe_add_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
    ) -> bool {
        if let Some(last) = self.mappings.last()
            && last.generated_line == generated_line
            && last.source == Some(source)
            && last.original_line == original_line
            && last.original_column == original_column
        {
            return false;
        }
        self.add_mapping(
            generated_line,
            generated_column,
            source,
            original_line,
            original_column,
        );
        true
    }

    /// Encode all mappings to a VLQ-encoded string.
    fn encode_mappings(&self) -> String {
        if self.mappings.is_empty() {
            return String::new();
        }

        // Sort mappings by (generated_line, generated_column)
        let mut sorted: Vec<&Mapping> = self.mappings.iter().collect();
        sorted.sort_unstable_by(|a, b| {
            a.generated_line
                .cmp(&b.generated_line)
                .then(a.generated_column.cmp(&b.generated_column))
        });

        #[cfg(feature = "parallel")]
        if sorted.len() >= 4096 {
            return Self::encode_parallel_impl(&sorted);
        }

        Self::encode_sequential_impl(&sorted)
    }

    fn encode_sequential_impl(sorted: &[&Mapping]) -> String {
        let mut out: Vec<u8> = Vec::with_capacity(sorted.len() * 6);

        let mut prev_gen_col: i64 = 0;
        let mut prev_source: i64 = 0;
        let mut prev_orig_line: i64 = 0;
        let mut prev_orig_col: i64 = 0;
        let mut prev_name: i64 = 0;
        let mut prev_gen_line: u32 = 0;
        let mut first_in_line = true;

        for m in sorted {
            while prev_gen_line < m.generated_line {
                out.push(b';');
                prev_gen_line += 1;
                prev_gen_col = 0;
                first_in_line = true;
            }

            if !first_in_line {
                out.push(b',');
            }
            first_in_line = false;

            vlq_encode(&mut out, m.generated_column as i64 - prev_gen_col);
            prev_gen_col = m.generated_column as i64;

            if let Some(source) = m.source {
                vlq_encode(&mut out, source as i64 - prev_source);
                prev_source = source as i64;

                vlq_encode(&mut out, m.original_line as i64 - prev_orig_line);
                prev_orig_line = m.original_line as i64;

                vlq_encode(&mut out, m.original_column as i64 - prev_orig_col);
                prev_orig_col = m.original_column as i64;

                if let Some(name) = m.name {
                    vlq_encode(&mut out, name as i64 - prev_name);
                    prev_name = name as i64;
                }
            }
        }

        // SAFETY: VLQ output is always valid ASCII/UTF-8
        unsafe { String::from_utf8_unchecked(out) }
    }

    #[cfg(feature = "parallel")]
    fn encode_parallel_impl(sorted: &[&Mapping]) -> String {
        use rayon::prelude::*;

        let max_line = sorted.last().unwrap().generated_line as usize;

        // Build line ranges: (start_idx, end_idx) into sorted slice
        let mut line_ranges: Vec<(usize, usize)> = vec![(0, 0); max_line + 1];
        let mut i = 0;
        while i < sorted.len() {
            let line = sorted[i].generated_line as usize;
            let start = i;
            while i < sorted.len() && sorted[i].generated_line as usize == line {
                i += 1;
            }
            line_ranges[line] = (start, i);
        }

        // Sequential scan: compute cumulative state at each line boundary
        let mut states: Vec<(i64, i64, i64, i64)> = Vec::with_capacity(max_line + 1);
        let mut prev_source: i64 = 0;
        let mut prev_orig_line: i64 = 0;
        let mut prev_orig_col: i64 = 0;
        let mut prev_name: i64 = 0;

        for &(start, end) in &line_ranges {
            states.push((prev_source, prev_orig_line, prev_orig_col, prev_name));
            for m in &sorted[start..end] {
                if let Some(source) = m.source {
                    prev_source = source as i64;
                    prev_orig_line = m.original_line as i64;
                    prev_orig_col = m.original_column as i64;
                    if let Some(name) = m.name {
                        prev_name = name as i64;
                    }
                }
            }
        }

        // Parallel: encode each line independently
        let encoded_lines: Vec<Vec<u8>> = line_ranges
            .par_iter()
            .zip(states.par_iter())
            .map(|(&(start, end), &(s, ol, oc, n))| {
                if start == end {
                    return Vec::new();
                }
                encode_mapping_slice(&sorted[start..end], s, ol, oc, n)
            })
            .collect();

        // Join with semicolons
        let total_len = encoded_lines.iter().map(|l| l.len()).sum::<usize>() + max_line;
        let mut out: Vec<u8> = Vec::with_capacity(total_len);
        for (i, bytes) in encoded_lines.iter().enumerate() {
            if i > 0 {
                out.push(b';');
            }
            out.extend_from_slice(bytes);
        }

        // SAFETY: VLQ output is always valid ASCII/UTF-8
        unsafe { String::from_utf8_unchecked(out) }
    }

    /// Generate the source map as a JSON string.
    pub fn to_json(&self) -> String {
        let mappings = self.encode_mappings();

        // Encode scopes (may introduce names not yet in self.names)
        let (scopes_str, names_for_json) = if let Some(ref scopes_info) = self.scopes {
            let mut names = self.names.clone();
            let s = srcmap_scopes::encode_scopes(scopes_info, &mut names);
            (Some(s), names)
        } else {
            (None, self.names.clone())
        };

        let mut json = String::with_capacity(256 + mappings.len());
        json.push_str(r#"{"version":3"#);

        if let Some(ref file) = self.file {
            json.push_str(r#","file":"#);
            json.push_str(&json_quote(file));
        }

        if let Some(ref root) = self.source_root {
            json.push_str(r#","sourceRoot":"#);
            json.push_str(&json_quote(root));
        }

        // sources
        json.push_str(r#","sources":["#);
        for (i, s) in self.sources.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&json_quote(s));
        }
        json.push(']');

        // sourcesContent (only if any content is set)
        if self.sources_content.iter().any(|c| c.is_some()) {
            json.push_str(r#","sourcesContent":["#);

            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;

                let total_content: usize = self
                    .sources_content
                    .iter()
                    .map(|c| c.as_ref().map_or(0, |s| s.len()))
                    .sum();

                if self.sources_content.len() >= 8 && total_content >= 8192 {
                    let quoted: Vec<String> = self
                        .sources_content
                        .par_iter()
                        .map(|c| match c {
                            Some(content) => json_quote(content),
                            None => "null".to_string(),
                        })
                        .collect();
                    for (i, q) in quoted.iter().enumerate() {
                        if i > 0 {
                            json.push(',');
                        }
                        json.push_str(q);
                    }
                } else {
                    for (i, c) in self.sources_content.iter().enumerate() {
                        if i > 0 {
                            json.push(',');
                        }
                        match c {
                            Some(content) => json.push_str(&json_quote(content)),
                            None => json.push_str("null"),
                        }
                    }
                }
            }

            #[cfg(not(feature = "parallel"))]
            for (i, c) in self.sources_content.iter().enumerate() {
                if i > 0 {
                    json.push(',');
                }
                match c {
                    Some(content) => json.push_str(&json_quote(content)),
                    None => json.push_str("null"),
                }
            }

            json.push(']');
        }

        // names
        json.push_str(r#","names":["#);
        for (i, n) in names_for_json.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json.push_str(&json_quote(n));
        }
        json.push(']');

        // mappings
        json.push_str(r#","mappings":"#);
        json.push_str(&json_quote(&mappings));

        // ignoreList
        if !self.ignore_list.is_empty() {
            json.push_str(r#","ignoreList":["#);
            for (i, &idx) in self.ignore_list.iter().enumerate() {
                if i > 0 {
                    json.push(',');
                }
                json.push_str(&idx.to_string());
            }
            json.push(']');
        }

        // debugId
        if let Some(ref id) = self.debug_id {
            json.push_str(r#","debugId":"#);
            json.push_str(&json_quote(id));
        }

        // scopes (ECMA-426 scopes proposal)
        if let Some(ref s) = scopes_str {
            json.push_str(r#","scopes":"#);
            json.push_str(&json_quote(s));
        }

        json.push('}');
        json
    }

    /// Get the number of mappings.
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }

    /// Directly construct a `SourceMap` from the generator's internal state.
    ///
    /// This avoids the encode-then-decode round-trip (VLQ encode to JSON string,
    /// then re-parse) that would otherwise be needed in composition pipelines.
    pub fn to_decoded_map(&self) -> srcmap_sourcemap::SourceMap {
        // Sort mappings by (generated_line, generated_column) — same as encode_mappings
        let mut sorted: Vec<&Mapping> = self.mappings.iter().collect();
        sorted.sort_unstable_by(|a, b| {
            a.generated_line
                .cmp(&b.generated_line)
                .then(a.generated_column.cmp(&b.generated_column))
        });

        // Convert generator Mapping → sourcemap Mapping
        let sm_mappings: Vec<srcmap_sourcemap::Mapping> = sorted
            .iter()
            .map(|m| srcmap_sourcemap::Mapping {
                generated_line: m.generated_line,
                generated_column: m.generated_column,
                source: m.source.unwrap_or(u32::MAX),
                original_line: m.original_line,
                original_column: m.original_column,
                name: m.name.unwrap_or(u32::MAX),
            })
            .collect();

        // Build sources_content: convert Vec<Option<String>> → Vec<Option<String>>
        let sources_content: Vec<Option<String>> = self.sources_content.clone();

        // Build the source root-prefixed sources (matching what from_json does)
        let sources: Vec<String> = match &self.source_root {
            Some(root) if !root.is_empty() => {
                self.sources.iter().map(|s| format!("{root}{s}")).collect()
            }
            _ => self.sources.clone(),
        };

        srcmap_sourcemap::SourceMap::from_parts(
            self.file.clone(),
            self.source_root.clone(),
            sources,
            sources_content,
            self.names.clone(),
            sm_mappings,
            self.ignore_list.clone(),
            self.debug_id.clone(),
            None, // scopes are not included in decoded map (would need encoding/decoding)
        )
    }
}

/// Encode a slice of mappings for a single line to VLQ bytes.
///
/// Generated column starts at 0 (reset per line).
/// Cumulative state is passed in from the sequential pre-scan.
#[cfg(feature = "parallel")]
fn encode_mapping_slice(
    mappings: &[&Mapping],
    init_source: i64,
    init_orig_line: i64,
    init_orig_col: i64,
    init_name: i64,
) -> Vec<u8> {
    let mut buf = Vec::with_capacity(mappings.len() * 6);
    let mut prev_gen_col: i64 = 0;
    let mut prev_source = init_source;
    let mut prev_orig_line = init_orig_line;
    let mut prev_orig_col = init_orig_col;
    let mut prev_name = init_name;
    let mut first = true;

    for m in mappings {
        if !first {
            buf.push(b',');
        }
        first = false;

        vlq_encode(&mut buf, m.generated_column as i64 - prev_gen_col);
        prev_gen_col = m.generated_column as i64;

        if let Some(source) = m.source {
            vlq_encode(&mut buf, source as i64 - prev_source);
            prev_source = source as i64;

            vlq_encode(&mut buf, m.original_line as i64 - prev_orig_line);
            prev_orig_line = m.original_line as i64;

            vlq_encode(&mut buf, m.original_column as i64 - prev_orig_col);
            prev_orig_col = m.original_column as i64;

            if let Some(name) = m.name {
                vlq_encode(&mut buf, name as i64 - prev_name);
                prev_name = name as i64;
            }
        }
    }

    buf
}

/// JSON-quote a string (with escape handling).
fn json_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_generator() {
        let builder = SourceMapGenerator::new(None);
        let json = builder.to_json();
        assert!(json.contains(r#""version":3"#));
        assert!(json.contains(r#""mappings":"""#));
    }

    #[test]
    fn simple_mapping() {
        let mut builder = SourceMapGenerator::new(Some("output.js".to_string()));
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);

        let json = builder.to_json();
        assert!(json.contains(r#""file":"output.js""#));
        assert!(json.contains(r#""sources":["input.js"]"#));

        // Verify roundtrip with parser
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "input.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn mapping_with_name() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        let name = builder.add_name("myFunction");
        builder.add_named_mapping(0, 0, src, 0, 0, name);

        let json = builder.to_json();
        assert!(json.contains(r#""names":["myFunction"]"#));

        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
        assert_eq!(sm.name(0), "myFunction");
    }

    #[test]
    fn multiple_lines() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);
        builder.add_mapping(1, 4, src, 1, 2);
        builder.add_mapping(2, 0, src, 2, 0);

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.line_count(), 3);

        let loc = sm.original_position_for(1, 4).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 2);
    }

    #[test]
    fn multiple_sources() {
        let mut builder = SourceMapGenerator::new(None);
        let a = builder.add_source("a.js");
        let b = builder.add_source("b.js");
        builder.add_mapping(0, 0, a, 0, 0);
        builder.add_mapping(1, 0, b, 0, 0);

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();

        let loc0 = sm.original_position_for(0, 0).unwrap();
        let loc1 = sm.original_position_for(1, 0).unwrap();
        assert_eq!(sm.source(loc0.source), "a.js");
        assert_eq!(sm.source(loc1.source), "b.js");
    }

    #[test]
    fn source_content() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.set_source_content(src, "var x = 1;".to_string());
        builder.add_mapping(0, 0, src, 0, 0);

        let json = builder.to_json();
        assert!(json.contains(r#""sourcesContent":["var x = 1;"]"#));

        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.sources_content[0], Some("var x = 1;".to_string()));
    }

    #[test]
    fn source_root() {
        let mut builder = SourceMapGenerator::new(None);
        builder.set_source_root("src/".to_string());
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);

        let json = builder.to_json();
        assert!(json.contains(r#""sourceRoot":"src/""#));
    }

    #[test]
    fn ignore_list() {
        let mut builder = SourceMapGenerator::new(None);
        let _app = builder.add_source("app.js");
        let lib = builder.add_source("node_modules/lib.js");
        builder.add_to_ignore_list(lib);
        builder.add_mapping(0, 0, lib, 0, 0);

        let json = builder.to_json();
        assert!(json.contains(r#""ignoreList":[1]"#));
    }

    #[test]
    fn generated_only_mapping() {
        let mut builder = SourceMapGenerator::new(None);
        builder.add_generated_mapping(0, 0);

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        // Generated-only mapping → no source info
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn dedup_sources_and_names() {
        let mut builder = SourceMapGenerator::new(None);
        let s1 = builder.add_source("input.js");
        let s2 = builder.add_source("input.js"); // duplicate
        assert_eq!(s1, s2);

        let n1 = builder.add_name("foo");
        let n2 = builder.add_name("foo"); // duplicate
        assert_eq!(n1, n2);

        assert_eq!(builder.sources.len(), 1);
        assert_eq!(builder.names.len(), 1);
    }

    #[test]
    fn large_roundtrip() {
        let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));

        for i in 0..5 {
            builder.add_source(&format!("src/file{i}.js"));
        }
        for i in 0..10 {
            builder.add_name(&format!("var{i}"));
        }

        // Add 1000 mappings across 100 lines
        for line in 0..100u32 {
            for col in 0..10u32 {
                let src = (line * 10 + col) % 5;
                let name = if col % 3 == 0 { Some(col % 10) } else { None };

                match name {
                    Some(n) => builder.add_named_mapping(line, col * 10, src, line, col * 5, n),
                    None => builder.add_mapping(line, col * 10, src, line, col * 5),
                }
            }
        }

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();

        assert_eq!(sm.mapping_count(), 1000);
        assert_eq!(sm.line_count(), 100);

        // Verify a few lookups
        let loc = sm.original_position_for(50, 30).unwrap();
        assert_eq!(loc.line, 50);
        assert_eq!(loc.column, 15);
    }

    #[test]
    fn json_escaping() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("path/with\"quotes.js");
        builder.set_source_content(src, "line1\nline2\ttab".to_string());
        builder.add_mapping(0, 0, src, 0, 0);

        let json = builder.to_json();
        // Should be valid JSON
        let _: serde_json::Value = serde_json::from_str(&json).unwrap();
    }

    #[test]
    fn maybe_add_mapping_skips_redundant() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");

        // First mapping — always added
        assert!(builder.maybe_add_mapping(0, 0, src, 10, 0));
        // Same source position, different generated column — redundant, skipped
        assert!(!builder.maybe_add_mapping(0, 5, src, 10, 0));
        // Different source position — added
        assert!(builder.maybe_add_mapping(0, 10, src, 11, 0));
        // Different generated line, same source position as last — added (new line resets)
        assert!(builder.maybe_add_mapping(1, 0, src, 11, 0));

        assert_eq!(builder.mapping_count(), 3);

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.mapping_count(), 3);
    }

    #[test]
    fn maybe_add_mapping_different_source() {
        let mut builder = SourceMapGenerator::new(None);
        let a = builder.add_source("a.js");
        let b = builder.add_source("b.js");

        assert!(builder.maybe_add_mapping(0, 0, a, 0, 0));
        // Same line/col but different source — not redundant
        assert!(builder.maybe_add_mapping(0, 5, b, 0, 0));

        assert_eq!(builder.mapping_count(), 2);
    }

    #[test]
    fn to_decoded_map_basic() {
        let mut builder = SourceMapGenerator::new(Some("output.js".to_string()));
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);
        builder.add_mapping(1, 4, src, 1, 2);

        let sm = builder.to_decoded_map();
        assert_eq!(sm.mapping_count(), 2);
        assert_eq!(sm.line_count(), 2);

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "input.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);

        let loc = sm.original_position_for(1, 4).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 2);
    }

    #[test]
    fn to_decoded_map_with_names() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        let name = builder.add_name("myFunction");
        builder.add_named_mapping(0, 0, src, 0, 0, name);

        let sm = builder.to_decoded_map();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
        assert_eq!(sm.name(0), "myFunction");
    }

    #[test]
    fn to_decoded_map_matches_json_roundtrip() {
        let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));
        for i in 0..5 {
            builder.add_source(&format!("src/file{i}.js"));
        }
        for i in 0..10 {
            builder.add_name(&format!("var{i}"));
        }

        for line in 0..50u32 {
            for col in 0..10u32 {
                let src = (line * 10 + col) % 5;
                let name = if col % 3 == 0 { Some(col % 10) } else { None };
                match name {
                    Some(n) => builder.add_named_mapping(line, col * 10, src, line, col * 5, n),
                    None => builder.add_mapping(line, col * 10, src, line, col * 5),
                }
            }
        }

        // Compare decoded map vs JSON roundtrip
        let sm_decoded = builder.to_decoded_map();
        let json = builder.to_json();
        let sm_json = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();

        assert_eq!(sm_decoded.mapping_count(), sm_json.mapping_count());
        assert_eq!(sm_decoded.line_count(), sm_json.line_count());

        // Verify all lookups match
        for m in sm_json.all_mappings() {
            let a = sm_json.original_position_for(m.generated_line, m.generated_column);
            let b = sm_decoded.original_position_for(m.generated_line, m.generated_column);
            match (a, b) {
                (Some(a), Some(b)) => {
                    assert_eq!(
                        a.source, b.source,
                        "source mismatch at ({}, {})",
                        m.generated_line, m.generated_column
                    );
                    assert_eq!(
                        a.line, b.line,
                        "line mismatch at ({}, {})",
                        m.generated_line, m.generated_column
                    );
                    assert_eq!(
                        a.column, b.column,
                        "column mismatch at ({}, {})",
                        m.generated_line, m.generated_column
                    );
                    assert_eq!(
                        a.name, b.name,
                        "name mismatch at ({}, {})",
                        m.generated_line, m.generated_column
                    );
                }
                (None, None) => {}
                _ => panic!(
                    "lookup mismatch at ({}, {})",
                    m.generated_line, m.generated_column
                ),
            }
        }
    }

    #[test]
    fn to_decoded_map_empty() {
        let builder = SourceMapGenerator::new(None);
        let sm = builder.to_decoded_map();
        assert_eq!(sm.mapping_count(), 0);
        assert_eq!(sm.line_count(), 0);
    }

    #[test]
    fn to_decoded_map_generated_only() {
        let mut builder = SourceMapGenerator::new(None);
        builder.add_generated_mapping(0, 0);

        let sm = builder.to_decoded_map();
        assert_eq!(sm.mapping_count(), 1);
        // Generated-only mapping has no source info
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn to_decoded_map_multiple_sources() {
        let mut builder = SourceMapGenerator::new(None);
        let a = builder.add_source("a.js");
        let b = builder.add_source("b.js");
        builder.add_mapping(0, 0, a, 0, 0);
        builder.add_mapping(1, 0, b, 0, 0);

        let sm = builder.to_decoded_map();
        let loc0 = sm.original_position_for(0, 0).unwrap();
        let loc1 = sm.original_position_for(1, 0).unwrap();
        assert_eq!(sm.source(loc0.source), "a.js");
        assert_eq!(sm.source(loc1.source), "b.js");
    }

    #[test]
    fn to_decoded_map_with_source_content() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.set_source_content(src, "var x = 1;".to_string());
        builder.add_mapping(0, 0, src, 0, 0);

        let sm = builder.to_decoded_map();
        assert_eq!(sm.sources_content[0], Some("var x = 1;".to_string()));
    }

    #[test]
    fn to_decoded_map_reverse_lookup() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 10, 5);

        let sm = builder.to_decoded_map();
        let loc = sm.generated_position_for("input.js", 10, 5).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn to_decoded_map_sparse_lines() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);
        builder.add_mapping(5, 0, src, 5, 0);

        let sm = builder.to_decoded_map();
        assert_eq!(sm.line_count(), 6);
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_none());
        assert!(sm.original_position_for(5, 0).is_some());
    }

    #[test]
    fn empty_lines_between_mappings() {
        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);
        // Skip lines 1-4
        builder.add_mapping(5, 0, src, 5, 0);

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();

        // Line 0 should have a mapping
        assert!(sm.original_position_for(0, 0).is_some());
        // Lines 1-4 should have no mappings
        assert!(sm.original_position_for(2, 0).is_none());
        // Line 5 should have a mapping
        assert!(sm.original_position_for(5, 0).is_some());
    }

    #[test]
    fn debug_id() {
        let mut builder = SourceMapGenerator::new(None);
        builder.set_debug_id("85314830-023f-4cf1-a267-535f4e37bb17".to_string());
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);

        let json = builder.to_json();
        assert!(json.contains(r#""debugId":"85314830-023f-4cf1-a267-535f4e37bb17""#));

        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        assert_eq!(
            sm.debug_id.as_deref(),
            Some("85314830-023f-4cf1-a267-535f4e37bb17")
        );
    }

    #[test]
    fn scopes_roundtrip() {
        use srcmap_scopes::{
            Binding, CallSite, GeneratedRange, OriginalScope, Position, ScopeInfo,
        };

        let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));
        let src = builder.add_source("input.js");
        builder.set_source_content(
            src,
            "function hello(name) {\n  return name;\n}\nhello('world');".to_string(),
        );
        let name_hello = builder.add_name("hello");
        builder.add_named_mapping(0, 0, src, 0, 0, name_hello);
        builder.add_mapping(1, 0, src, 1, 0);

        // Set scopes
        builder.set_scopes(ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 3,
                    column: 14,
                },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["hello".to_string()],
                children: vec![OriginalScope {
                    start: Position { line: 0, column: 9 },
                    end: Position { line: 2, column: 1 },
                    name: Some("hello".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec!["name".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 3,
                    column: 14,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("hello".to_string())],
                children: vec![GeneratedRange {
                    start: Position { line: 0, column: 9 },
                    end: Position { line: 2, column: 1 },
                    is_stack_frame: true,
                    is_hidden: false,
                    definition: Some(1),
                    call_site: None,
                    bindings: vec![Binding::Expression("name".to_string())],
                    children: vec![],
                }],
            }],
        });

        let json = builder.to_json();

        // Verify scopes field is present
        assert!(json.contains(r#""scopes":"#));

        // Parse back and verify
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        assert!(sm.scopes.is_some());

        let scopes_info = sm.scopes.unwrap();

        // Verify original scopes
        assert_eq!(scopes_info.scopes.len(), 1);
        let root_scope = scopes_info.scopes[0].as_ref().unwrap();
        assert_eq!(root_scope.kind.as_deref(), Some("global"));
        assert_eq!(root_scope.variables, vec!["hello"]);
        assert_eq!(root_scope.children.len(), 1);

        let fn_scope = &root_scope.children[0];
        assert_eq!(fn_scope.name.as_deref(), Some("hello"));
        assert_eq!(fn_scope.kind.as_deref(), Some("function"));
        assert!(fn_scope.is_stack_frame);
        assert_eq!(fn_scope.variables, vec!["name"]);

        // Verify generated ranges
        assert_eq!(scopes_info.ranges.len(), 1);
        let outer = &scopes_info.ranges[0];
        assert_eq!(outer.definition, Some(0));
        assert_eq!(
            outer.bindings,
            vec![Binding::Expression("hello".to_string())]
        );
        assert_eq!(outer.children.len(), 1);

        let inner = &outer.children[0];
        assert_eq!(inner.definition, Some(1));
        assert!(inner.is_stack_frame);
        assert_eq!(
            inner.bindings,
            vec![Binding::Expression("name".to_string())]
        );
    }

    #[test]
    fn scopes_with_inlining_roundtrip() {
        use srcmap_scopes::{
            Binding, CallSite, GeneratedRange, OriginalScope, Position, ScopeInfo,
        };

        let mut builder = SourceMapGenerator::new(None);
        let src = builder.add_source("input.js");
        builder.add_mapping(0, 0, src, 0, 0);

        builder.set_scopes(ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![OriginalScope {
                    start: Position { line: 1, column: 0 },
                    end: Position { line: 4, column: 1 },
                    name: Some("greet".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec!["msg".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("_x".to_string())],
                children: vec![GeneratedRange {
                    start: Position { line: 6, column: 0 },
                    end: Position { line: 8, column: 0 },
                    is_stack_frame: true,
                    is_hidden: false,
                    definition: Some(1),
                    call_site: Some(CallSite {
                        source_index: 0,
                        line: 8,
                        column: 0,
                    }),
                    bindings: vec![Binding::Expression("\"Hello\"".to_string())],
                    children: vec![],
                }],
            }],
        });

        let json = builder.to_json();
        let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
        let info = sm.scopes.unwrap();

        // Verify call site on inlined range
        let inlined = &info.ranges[0].children[0];
        assert_eq!(
            inlined.call_site,
            Some(CallSite {
                source_index: 0,
                line: 8,
                column: 0,
            })
        );
        assert_eq!(
            inlined.bindings,
            vec![Binding::Expression("\"Hello\"".to_string())]
        );
    }

    #[cfg(feature = "parallel")]
    mod parallel_tests {
        use super::*;

        fn build_large_generator(lines: u32, cols_per_line: u32) -> SourceMapGenerator {
            let mut builder = SourceMapGenerator::new(Some("bundle.js".to_string()));
            for i in 0..10 {
                let src = builder.add_source(&format!("src/file{i}.js"));
                builder.set_source_content(
                    src,
                    format!("// source file {i}\n{}", "x = 1;\n".repeat(100)),
                );
            }
            for i in 0..20 {
                builder.add_name(&format!("var{i}"));
            }

            for line in 0..lines {
                for col in 0..cols_per_line {
                    let src = (line * cols_per_line + col) % 10;
                    let name = if col % 3 == 0 {
                        Some((col % 20) as u32)
                    } else {
                        None
                    };
                    match name {
                        Some(n) => builder.add_named_mapping(line, col * 10, src, line, col * 5, n),
                        None => builder.add_mapping(line, col * 10, src, line, col * 5),
                    }
                }
            }
            builder
        }

        #[test]
        fn parallel_large_roundtrip() {
            let builder = build_large_generator(500, 20);
            let json = builder.to_json();
            let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
            assert_eq!(sm.mapping_count(), 10000);
            assert_eq!(sm.line_count(), 500);

            // Verify lookups
            let loc = sm.original_position_for(250, 50).unwrap();
            assert_eq!(loc.line, 250);
            assert_eq!(loc.column, 25);
        }

        #[test]
        fn parallel_matches_sequential() {
            let builder = build_large_generator(500, 20);

            // Sort mappings the same way encode_mappings does
            let mut sorted: Vec<&Mapping> = builder.mappings.iter().collect();
            sorted.sort_unstable_by(|a, b| {
                a.generated_line
                    .cmp(&b.generated_line)
                    .then(a.generated_column.cmp(&b.generated_column))
            });

            let sequential = SourceMapGenerator::encode_sequential_impl(&sorted);
            let parallel = SourceMapGenerator::encode_parallel_impl(&sorted);
            assert_eq!(sequential, parallel);
        }

        #[test]
        fn parallel_with_sparse_lines() {
            let mut builder = SourceMapGenerator::new(None);
            let src = builder.add_source("input.js");

            // Add mappings on lines 0, 100, 200, ... (sparse)
            for i in 0..50 {
                let line = i * 100;
                for col in 0..100u32 {
                    builder.add_mapping(line, col * 10, src, line, col * 5);
                }
            }

            let json = builder.to_json();
            let sm = srcmap_sourcemap::SourceMap::from_json(&json).unwrap();
            assert_eq!(sm.mapping_count(), 5000);

            // Verify empty lines have no mappings
            assert!(sm.original_position_for(50, 0).is_none());
            // Verify populated lines work
            let loc = sm.original_position_for(200, 50).unwrap();
            assert_eq!(loc.line, 200);
            assert_eq!(loc.column, 25);
        }
    }
}
