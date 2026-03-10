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
            source_map: HashMap::new(),
            name_map: HashMap::new(),
        }
    }

    /// Set the source root prefix.
    pub fn set_source_root(&mut self, root: String) {
        self.source_root = Some(root);
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
        let total_len =
            encoded_lines.iter().map(|l| l.len()).sum::<usize>() + max_line;
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
        for (i, n) in self.names.iter().enumerate() {
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

        json.push('}');
        json
    }

    /// Get the number of mappings.
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
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
                let src = (line as u32 * 10 + col) % 5;
                let name = if col % 3 == 0 {
                    Some((col % 10) as u32)
                } else {
                    None
                };

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

    #[cfg(feature = "parallel")]
    mod parallel_tests {
        use super::*;

        fn build_large_generator(lines: u32, cols_per_line: u32) -> SourceMapGenerator {
            let mut builder =
                SourceMapGenerator::new(Some("bundle.js".to_string()));
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
                        Some(n) => builder.add_named_mapping(
                            line,
                            col * 10,
                            src,
                            line,
                            col * 5,
                            n,
                        ),
                        None => builder.add_mapping(
                            line,
                            col * 10,
                            src,
                            line,
                            col * 5,
                        ),
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
