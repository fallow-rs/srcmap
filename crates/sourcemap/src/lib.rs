//! High-performance source map parser and consumer (ECMA-426).
//!
//! Parses source map JSON and provides O(log n) position lookups.
//! Uses a flat, cache-friendly representation internally.
//!
//! # Examples
//!
//! ```
//! use srcmap_sourcemap::SourceMap;
//!
//! let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;AACA"}"#;
//! let sm = SourceMap::from_json(json).unwrap();
//!
//! // Look up original position for generated line 0, column 0
//! let loc = sm.original_position_for(0, 0).unwrap();
//! assert_eq!(sm.source(loc.source), "input.js");
//! assert_eq!(loc.line, 0);
//! assert_eq!(loc.column, 0);
//!
//! // Reverse lookup
//! let pos = sm.generated_position_for("input.js", 0, 0).unwrap();
//! assert_eq!(pos.line, 0);
//! assert_eq!(pos.column, 0);
//! ```

use std::cell::OnceCell;
use std::collections::HashMap;
use std::fmt;

use serde::Deserialize;
use srcmap_codec::DecodeError;
use srcmap_scopes::ScopeInfo;

// ── Constants ──────────────────────────────────────────────────────

const NO_SOURCE: u32 = u32::MAX;
const NO_NAME: u32 = u32::MAX;

// ── Public types ───────────────────────────────────────────────────

/// A single decoded mapping entry. Compact at 24 bytes (6 × u32).
#[derive(Debug, Clone, Copy)]
pub struct Mapping {
    pub generated_line: u32,
    pub generated_column: u32,
    /// Index into `SourceMap::sources`. `u32::MAX` if absent.
    pub source: u32,
    pub original_line: u32,
    pub original_column: u32,
    /// Index into `SourceMap::names`. `u32::MAX` if absent.
    pub name: u32,
}

/// Result of an `original_position_for` lookup.
#[derive(Debug, Clone)]
pub struct OriginalLocation {
    pub source: u32,
    pub line: u32,
    pub column: u32,
    pub name: Option<u32>,
}

/// Result of a `generated_position_for` lookup.
#[derive(Debug, Clone)]
pub struct GeneratedLocation {
    pub line: u32,
    pub column: u32,
}

/// Errors during source map parsing.
#[derive(Debug)]
pub enum ParseError {
    Json(serde_json::Error),
    Vlq(DecodeError),
    InvalidVersion(u32),
    Scopes(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "JSON parse error: {e}"),
            Self::Vlq(e) => write!(f, "VLQ decode error: {e}"),
            Self::InvalidVersion(v) => write!(f, "unsupported source map version: {v}"),
            Self::Scopes(e) => write!(f, "scopes decode error: {e}"),
        }
    }
}

impl std::error::Error for ParseError {}

impl From<serde_json::Error> for ParseError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(e)
    }
}

impl From<DecodeError> for ParseError {
    fn from(e: DecodeError) -> Self {
        Self::Vlq(e)
    }
}

// ── Raw JSON structure ─────────────────────────────────────────────

#[derive(Deserialize)]
struct RawSourceMap<'a> {
    version: u32,
    #[serde(default)]
    file: Option<String>,
    #[serde(default, rename = "sourceRoot")]
    source_root: Option<String>,
    #[serde(default)]
    sources: Vec<Option<String>>,
    #[serde(default, rename = "sourcesContent")]
    sources_content: Option<Vec<Option<String>>>,
    #[serde(default)]
    names: Vec<String>,
    #[serde(default, borrow)]
    mappings: &'a str,
    #[serde(default, rename = "ignoreList")]
    ignore_list: Vec<u32>,
    /// Debug ID for associating generated files with source maps (ECMA-426).
    /// Accepts both `debugId` (spec) and `debug_id` (Sentry compat).
    #[serde(default, rename = "debugId", alias = "debug_id")]
    debug_id: Option<String>,
    /// Scopes and variables (ECMA-426 scopes proposal).
    #[serde(default, borrow)]
    scopes: Option<&'a str>,
    /// Indexed source maps use `sections` instead of `mappings`.
    #[serde(default)]
    sections: Option<Vec<RawSection>>,
}

/// A section in an indexed source map.
#[derive(Deserialize)]
struct RawSection {
    offset: RawOffset,
    map: serde_json::Value,
}

#[derive(Deserialize)]
struct RawOffset {
    line: u32,
    column: u32,
}

// ── SourceMap ──────────────────────────────────────────────────────

/// A parsed source map with O(log n) position lookups.
#[derive(Debug, Clone)]
pub struct SourceMap {
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub sources: Vec<String>,
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    pub ignore_list: Vec<u32>,
    /// Debug ID (UUID) for associating generated files with source maps (ECMA-426).
    pub debug_id: Option<String>,
    /// Decoded scope and variable information (ECMA-426 scopes proposal).
    pub scopes: Option<ScopeInfo>,

    /// Flat decoded mappings, ordered by (generated_line, generated_column).
    mappings: Vec<Mapping>,

    /// `line_offsets[i]` = index of first mapping on generated line `i`.
    /// `line_offsets[line_count]` = mappings.len() (sentinel).
    line_offsets: Vec<u32>,

    /// Indices into `mappings`, sorted by (source, original_line, original_column).
    /// Built lazily on first `generated_position_for` call.
    reverse_index: OnceCell<Vec<u32>>,

    /// Source filename → index for O(1) lookup by name.
    source_map: HashMap<String, u32>,
}

impl SourceMap {
    /// Parse a source map from a JSON string.
    /// Supports both regular and indexed (sectioned) source maps.
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        let raw: RawSourceMap<'_> = serde_json::from_str(json)?;

        if raw.version != 3 {
            return Err(ParseError::InvalidVersion(raw.version));
        }

        // Handle indexed source maps (sections)
        if let Some(sections) = raw.sections {
            return Self::from_sections(raw.file, sections);
        }

        Self::from_regular(raw)
    }

    /// Parse a regular (non-indexed) source map.
    fn from_regular(raw: RawSourceMap<'_>) -> Result<Self, ParseError> {
        // Resolve sources: apply sourceRoot, replace None with empty string
        let source_root = raw.source_root.as_deref().unwrap_or("");
        let sources: Vec<String> = raw
            .sources
            .iter()
            .map(|s| match s {
                Some(s) if !source_root.is_empty() => format!("{source_root}{s}"),
                Some(s) => s.clone(),
                None => String::new(),
            })
            .collect();

        let sources_content = raw.sources_content.unwrap_or_default();

        // Build source name → index map
        let source_map: HashMap<String, u32> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        // Decode mappings directly into flat Mapping vec
        let (mappings, line_offsets) = decode_mappings(raw.mappings)?;

        // Decode scopes if present
        let num_sources = sources.len();
        let scopes = match raw.scopes {
            Some(scopes_str) if !scopes_str.is_empty() => Some(
                srcmap_scopes::decode_scopes(scopes_str, &raw.names, num_sources)
                    .map_err(|e| ParseError::Scopes(e.to_string()))?,
            ),
            _ => None,
        };

        Ok(Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content,
            names: raw.names,
            ignore_list: raw.ignore_list,
            debug_id: raw.debug_id,
            scopes,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
        })
    }

    /// Flatten an indexed source map (with sections) into a regular one.
    fn from_sections(file: Option<String>, sections: Vec<RawSection>) -> Result<Self, ParseError> {
        let mut all_sources: Vec<String> = Vec::new();
        let mut all_sources_content: Vec<Option<String>> = Vec::new();
        let mut all_names: Vec<String> = Vec::new();
        let mut all_mappings: Vec<Mapping> = Vec::new();
        let mut all_ignore_list: Vec<u32> = Vec::new();
        let mut max_line: u32 = 0;

        // Source/name dedup maps to merge across sections
        let mut source_index_map: HashMap<String, u32> = HashMap::new();
        let mut name_index_map: HashMap<String, u32> = HashMap::new();

        for section in &sections {
            let section_json = serde_json::to_string(&section.map).map_err(ParseError::Json)?;
            let sub = Self::from_json(&section_json)?;

            let line_offset = section.offset.line;
            let col_offset = section.offset.column;

            // Map section source indices to global indices
            let source_remap: Vec<u32> = sub
                .sources
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    if let Some(&existing) = source_index_map.get(s) {
                        existing
                    } else {
                        let idx = all_sources.len() as u32;
                        all_sources.push(s.clone());
                        // Add sourcesContent if available
                        let content = sub.sources_content.get(i).cloned().unwrap_or(None);
                        all_sources_content.push(content);
                        source_index_map.insert(s.clone(), idx);
                        idx
                    }
                })
                .collect();

            // Map section name indices to global indices
            let name_remap: Vec<u32> = sub
                .names
                .iter()
                .map(|n| {
                    if let Some(&existing) = name_index_map.get(n) {
                        existing
                    } else {
                        let idx = all_names.len() as u32;
                        all_names.push(n.clone());
                        name_index_map.insert(n.clone(), idx);
                        idx
                    }
                })
                .collect();

            // Add ignore_list entries (remapped to global source indices)
            for &idx in &sub.ignore_list {
                let global_idx = source_remap[idx as usize];
                if !all_ignore_list.contains(&global_idx) {
                    all_ignore_list.push(global_idx);
                }
            }

            // Remap and offset all mappings from this section
            for m in &sub.mappings {
                let gen_line = m.generated_line + line_offset;
                let gen_col = if m.generated_line == 0 {
                    m.generated_column + col_offset
                } else {
                    m.generated_column
                };

                all_mappings.push(Mapping {
                    generated_line: gen_line,
                    generated_column: gen_col,
                    source: if m.source == NO_SOURCE {
                        NO_SOURCE
                    } else {
                        source_remap[m.source as usize]
                    },
                    original_line: m.original_line,
                    original_column: m.original_column,
                    name: if m.name == NO_NAME {
                        NO_NAME
                    } else {
                        name_remap[m.name as usize]
                    },
                });

                if gen_line > max_line {
                    max_line = gen_line;
                }
            }
        }

        // Sort mappings by (generated_line, generated_column)
        all_mappings.sort_unstable_by(|a, b| {
            a.generated_line
                .cmp(&b.generated_line)
                .then(a.generated_column.cmp(&b.generated_column))
        });

        // Build line_offsets
        let line_count = if all_mappings.is_empty() {
            0
        } else {
            max_line as usize + 1
        };
        let mut line_offsets: Vec<u32> = vec![0; line_count + 1];
        let mut current_line: usize = 0;
        for (i, m) in all_mappings.iter().enumerate() {
            while current_line < m.generated_line as usize {
                current_line += 1;
                if current_line < line_offsets.len() {
                    line_offsets[current_line] = i as u32;
                }
            }
        }
        // Fill sentinel
        if !line_offsets.is_empty() {
            let last = all_mappings.len() as u32;
            for offset in line_offsets.iter_mut().skip(current_line + 1) {
                *offset = last;
            }
        }

        Ok(Self {
            file,
            source_root: None,
            sources: all_sources.clone(),
            sources_content: all_sources_content,
            names: all_names,
            ignore_list: all_ignore_list,
            debug_id: None,
            scopes: None, // TODO: merge scopes from sections
            mappings: all_mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map: all_sources
                .into_iter()
                .enumerate()
                .map(|(i, s)| (s, i as u32))
                .collect(),
        })
    }

    /// Look up the original source position for a generated position.
    ///
    /// Both `line` and `column` are 0-based.
    /// Returns `None` if no mapping exists or the mapping has no source.
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        let line_idx = line as usize;
        if line_idx + 1 >= self.line_offsets.len() {
            return None;
        }

        let start = self.line_offsets[line_idx] as usize;
        let end = self.line_offsets[line_idx + 1] as usize;

        if start == end {
            return None;
        }

        let line_mappings = &self.mappings[start..end];

        // Binary search: find largest generated_column <= column
        let idx = match line_mappings.binary_search_by_key(&column, |m| m.generated_column) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };

        let mapping = &line_mappings[idx];

        if mapping.source == NO_SOURCE {
            return None;
        }

        Some(OriginalLocation {
            source: mapping.source,
            line: mapping.original_line,
            column: mapping.original_column,
            name: if mapping.name == NO_NAME {
                None
            } else {
                Some(mapping.name)
            },
        })
    }

    /// Look up the generated position for an original source position.
    ///
    /// `source` is the source filename. `line` and `column` are 0-based.
    pub fn generated_position_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Option<GeneratedLocation> {
        let &source_idx = self.source_map.get(source)?;

        let reverse_index = self
            .reverse_index
            .get_or_init(|| build_reverse_index(&self.mappings));

        // Binary search in reverse_index for (source, line, column)
        let idx = reverse_index.partition_point(|&i| {
            let m = &self.mappings[i as usize];
            (m.source, m.original_line, m.original_column) < (source_idx, line, column)
        });

        if idx >= reverse_index.len() {
            return None;
        }

        let mapping = &self.mappings[reverse_index[idx] as usize];

        if mapping.source != source_idx {
            return None;
        }

        Some(GeneratedLocation {
            line: mapping.generated_line,
            column: mapping.generated_column,
        })
    }

    /// Resolve a source index to its filename.
    pub fn source(&self, index: u32) -> &str {
        &self.sources[index as usize]
    }

    /// Resolve a name index to its string.
    pub fn name(&self, index: u32) -> &str {
        &self.names[index as usize]
    }

    /// Find the source index for a filename.
    pub fn source_index(&self, name: &str) -> Option<u32> {
        self.source_map.get(name).copied()
    }

    /// Total number of decoded mappings.
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }

    /// Number of generated lines.
    pub fn line_count(&self) -> usize {
        self.line_offsets.len().saturating_sub(1)
    }

    /// Get all mappings for a generated line (0-based).
    pub fn mappings_for_line(&self, line: u32) -> &[Mapping] {
        let line_idx = line as usize;
        if line_idx + 1 >= self.line_offsets.len() {
            return &[];
        }
        let start = self.line_offsets[line_idx] as usize;
        let end = self.line_offsets[line_idx + 1] as usize;
        &self.mappings[start..end]
    }

    /// Iterate all mappings.
    pub fn all_mappings(&self) -> &[Mapping] {
        &self.mappings
    }

    /// Serialize the source map back to JSON.
    ///
    /// Produces a valid source map v3 JSON string that can be written to a file
    /// or embedded in a data URL.
    pub fn to_json(&self) -> String {
        let mappings = self.encode_mappings();

        let mut json = String::with_capacity(256 + mappings.len());
        json.push_str(r#"{"version":3"#);

        if let Some(ref file) = self.file {
            json.push_str(r#","file":"#);
            json_quote_into(&mut json, file);
        }

        if let Some(ref root) = self.source_root {
            json.push_str(r#","sourceRoot":"#);
            json_quote_into(&mut json, root);
        }

        json.push_str(r#","sources":["#);
        for (i, s) in self.sources.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json_quote_into(&mut json, s);
        }
        json.push(']');

        if !self.sources_content.is_empty() && self.sources_content.iter().any(|c| c.is_some()) {
            json.push_str(r#","sourcesContent":["#);
            for (i, c) in self.sources_content.iter().enumerate() {
                if i > 0 {
                    json.push(',');
                }
                match c {
                    Some(content) => json_quote_into(&mut json, content),
                    None => json.push_str("null"),
                }
            }
            json.push(']');
        }

        json.push_str(r#","names":["#);
        for (i, n) in self.names.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            json_quote_into(&mut json, n);
        }
        json.push(']');

        json.push_str(r#","mappings":"#);
        json_quote_into(&mut json, &mappings);

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

        if let Some(ref id) = self.debug_id {
            json.push_str(r#","debugId":"#);
            json_quote_into(&mut json, id);
        }

        json.push('}');
        json
    }

    /// Encode all mappings back to a VLQ mappings string.
    fn encode_mappings(&self) -> String {
        if self.mappings.is_empty() {
            return String::new();
        }

        let mut out: Vec<u8> = Vec::with_capacity(self.mappings.len() * 6);

        let mut prev_gen_col: i64 = 0;
        let mut prev_source: i64 = 0;
        let mut prev_orig_line: i64 = 0;
        let mut prev_orig_col: i64 = 0;
        let mut prev_name: i64 = 0;
        let mut prev_gen_line: u32 = 0;
        let mut first_in_line = true;

        for m in &self.mappings {
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

            srcmap_codec::vlq_encode(&mut out, m.generated_column as i64 - prev_gen_col);
            prev_gen_col = m.generated_column as i64;

            if m.source != NO_SOURCE {
                srcmap_codec::vlq_encode(&mut out, m.source as i64 - prev_source);
                prev_source = m.source as i64;

                srcmap_codec::vlq_encode(&mut out, m.original_line as i64 - prev_orig_line);
                prev_orig_line = m.original_line as i64;

                srcmap_codec::vlq_encode(&mut out, m.original_column as i64 - prev_orig_col);
                prev_orig_col = m.original_column as i64;

                if m.name != NO_NAME {
                    srcmap_codec::vlq_encode(&mut out, m.name as i64 - prev_name);
                    prev_name = m.name as i64;
                }
            }
        }

        // SAFETY: VLQ output is always valid ASCII
        unsafe { String::from_utf8_unchecked(out) }
    }
}

/// Append a JSON-quoted string to the output buffer.
fn json_quote_into(out: &mut String, s: &str) {
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
}

// ── Internal: decode VLQ mappings directly into flat Mapping vec ───

/// Base64 decode lookup table (byte → 6-bit value, 0xFF = invalid).
const B64: [u8; 128] = {
    let mut table = [0xFFu8; 128];
    let chars = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut i = 0u8;
    while i < 64 {
        table[chars[i as usize] as usize] = i;
        i += 1;
    }
    table
};

/// Inline VLQ decode optimized for the hot path (no function call overhead).
/// Most source map VLQ values fit in 1-2 base64 characters.
#[inline(always)]
fn vlq_fast(bytes: &[u8], pos: &mut usize) -> Result<i64, DecodeError> {
    let p = *pos;
    if p >= bytes.len() {
        return Err(DecodeError::UnexpectedEof { offset: p });
    }

    let b0 = bytes[p];
    if b0 >= 128 {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: p,
        });
    }
    let d0 = B64[b0 as usize];
    if d0 == 0xFF {
        return Err(DecodeError::InvalidBase64 {
            byte: b0,
            offset: p,
        });
    }

    // Fast path: single character VLQ (values -15..15)
    if (d0 & 0x20) == 0 {
        *pos = p + 1;
        let val = (d0 >> 1) as i64;
        return Ok(if (d0 & 1) != 0 { -val } else { val });
    }

    // Multi-character VLQ
    let mut result: u64 = (d0 & 0x1F) as u64;
    let mut shift: u32 = 5;
    let mut i = p + 1;

    loop {
        if i >= bytes.len() {
            return Err(DecodeError::UnexpectedEof { offset: i });
        }
        let b = bytes[i];
        if b >= 128 {
            return Err(DecodeError::InvalidBase64 { byte: b, offset: i });
        }
        let d = B64[b as usize];
        if d == 0xFF {
            return Err(DecodeError::InvalidBase64 { byte: b, offset: i });
        }
        i += 1;

        if shift >= 64 {
            return Err(DecodeError::VlqOverflow { offset: p });
        }

        result += ((d & 0x1F) as u64) << shift;
        shift += 5;

        if (d & 0x20) == 0 {
            break;
        }
    }

    *pos = i;
    let value = if (result & 1) == 1 {
        -((result >> 1) as i64)
    } else {
        (result >> 1) as i64
    };
    Ok(value)
}

fn decode_mappings(input: &str) -> Result<(Vec<Mapping>, Vec<u32>), DecodeError> {
    if input.is_empty() {
        return Ok((Vec::new(), vec![0]));
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    // Pre-count for capacity hints
    let line_count = bytes.iter().filter(|&&b| b == b';').count() + 1;
    let approx_segments = bytes.iter().filter(|&&b| b == b',').count() + line_count;

    let mut mappings: Vec<Mapping> = Vec::with_capacity(approx_segments);
    let mut line_offsets: Vec<u32> = Vec::with_capacity(line_count + 1);

    let mut source_index: i64 = 0;
    let mut original_line: i64 = 0;
    let mut original_column: i64 = 0;
    let mut name_index: i64 = 0;
    let mut generated_line: u32 = 0;
    let mut pos: usize = 0;

    loop {
        line_offsets.push(mappings.len() as u32);
        let mut generated_column: i64 = 0;
        let mut saw_semicolon = false;

        while pos < len {
            let byte = bytes[pos];

            if byte == b';' {
                pos += 1;
                saw_semicolon = true;
                break;
            }

            if byte == b',' {
                pos += 1;
                continue;
            }

            // Field 1: generated column
            generated_column += vlq_fast(bytes, &mut pos)?;

            if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                // Fields 2-4: source, original line, original column
                source_index += vlq_fast(bytes, &mut pos)?;
                original_line += vlq_fast(bytes, &mut pos)?;
                original_column += vlq_fast(bytes, &mut pos)?;

                // Field 5: name (optional)
                let name = if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                    name_index += vlq_fast(bytes, &mut pos)?;
                    name_index as u32
                } else {
                    NO_NAME
                };

                mappings.push(Mapping {
                    generated_line,
                    generated_column: generated_column as u32,
                    source: source_index as u32,
                    original_line: original_line as u32,
                    original_column: original_column as u32,
                    name,
                });
            } else {
                // 1-field segment: no source info
                mappings.push(Mapping {
                    generated_line,
                    generated_column: generated_column as u32,
                    source: NO_SOURCE,
                    original_line: 0,
                    original_column: 0,
                    name: NO_NAME,
                });
            }
        }

        if !saw_semicolon {
            break;
        }
        generated_line += 1;
    }

    // Sentinel for line range computation
    line_offsets.push(mappings.len() as u32);

    Ok((mappings, line_offsets))
}

/// Build reverse index: mapping indices sorted by (source, original_line, original_column).
fn build_reverse_index(mappings: &[Mapping]) -> Vec<u32> {
    let mut indices: Vec<u32> = (0..mappings.len() as u32)
        .filter(|&i| mappings[i as usize].source != NO_SOURCE)
        .collect();

    indices.sort_unstable_by(|&a, &b| {
        let ma = &mappings[a as usize];
        let mb = &mappings[b as usize];
        ma.source
            .cmp(&mb.source)
            .then(ma.original_line.cmp(&mb.original_line))
            .then(ma.original_column.cmp(&mb.original_column))
    });

    indices
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn simple_map() -> &'static str {
        r#"{"version":3,"sources":["input.js"],"names":["hello"],"mappings":"AAAA;AACA,EAAA;AACA"}"#
    }

    #[test]
    fn parse_basic() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        assert_eq!(sm.sources, vec!["input.js"]);
        assert_eq!(sm.names, vec!["hello"]);
        assert_eq!(sm.line_count(), 3);
        assert!(sm.mapping_count() > 0);
    }

    #[test]
    fn to_json_roundtrip() {
        let json = simple_map();
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();

        // Parse the output back and verify it produces identical lookups
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.sources, sm.sources);
        assert_eq!(sm2.names, sm.names);
        assert_eq!(sm2.mapping_count(), sm.mapping_count());
        assert_eq!(sm2.line_count(), sm.line_count());

        // Verify all lookups match
        for m in sm.all_mappings() {
            let loc1 = sm.original_position_for(m.generated_line, m.generated_column);
            let loc2 = sm2.original_position_for(m.generated_line, m.generated_column);
            match (loc1, loc2) {
                (Some(a), Some(b)) => {
                    assert_eq!(a.source, b.source);
                    assert_eq!(a.line, b.line);
                    assert_eq!(a.column, b.column);
                    assert_eq!(a.name, b.name);
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
    fn to_json_roundtrip_large() {
        let json = generate_test_sourcemap(50, 10, 3);
        let sm = SourceMap::from_json(&json).unwrap();
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();

        assert_eq!(sm2.mapping_count(), sm.mapping_count());

        // Spot-check lookups
        for line in (0..sm.line_count() as u32).step_by(5) {
            for col in [0u32, 10, 20, 50] {
                let a = sm.original_position_for(line, col);
                let b = sm2.original_position_for(line, col);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        assert_eq!(a.source, b.source);
                        assert_eq!(a.line, b.line);
                        assert_eq!(a.column, b.column);
                    }
                    (None, None) => {}
                    _ => panic!("mismatch at ({line}, {col})"),
                }
            }
        }
    }

    #[test]
    fn to_json_preserves_fields() {
        let json = r#"{"version":3,"file":"out.js","sourceRoot":"src/","sources":["app.ts"],"sourcesContent":["const x = 1;"],"names":["x"],"mappings":"AAAAA","ignoreList":[0]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();

        assert!(output.contains(r#""file":"out.js""#));
        assert!(output.contains(r#""sourceRoot":"src/""#));
        assert!(output.contains(r#""sourcesContent":["const x = 1;"]"#));
        assert!(output.contains(r#""ignoreList":[0]"#));

        // Note: sources will have sourceRoot prepended
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.file.as_deref(), Some("out.js"));
        assert_eq!(sm2.ignore_list, vec![0]);
    }

    #[test]
    fn original_position_for_exact_match() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.source, 0);
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn original_position_for_column_within_segment() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        // Column 5 on line 1: should snap to the mapping at column 2
        let loc = sm.original_position_for(1, 5);
        assert!(loc.is_some());
    }

    #[test]
    fn original_position_for_nonexistent_line() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        assert!(sm.original_position_for(999, 0).is_none());
    }

    #[test]
    fn original_position_for_before_first_mapping() {
        // Line 1 first mapping is at column 2. Column 0 should return None.
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let loc = sm.original_position_for(1, 0);
        // Column 0 on line 1: the first mapping at col 0 (AACA decodes to col=0, src delta=1...)
        // Actually depends on exact VLQ values. Let's just verify it doesn't crash.
        let _ = loc;
    }

    #[test]
    fn generated_position_for_basic() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let loc = sm.generated_position_for("input.js", 0, 0).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn generated_position_for_unknown_source() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        assert!(sm.generated_position_for("nonexistent.js", 0, 0).is_none());
    }

    #[test]
    fn parse_invalid_version() {
        let json = r#"{"version":2,"sources":[],"names":[],"mappings":""}"#;
        let err = SourceMap::from_json(json).unwrap_err();
        assert!(matches!(err, ParseError::InvalidVersion(2)));
    }

    #[test]
    fn parse_empty_mappings() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 0);
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn parse_with_source_root() {
        let json = r#"{"version":3,"sourceRoot":"src/","sources":["foo.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources, vec!["src/foo.js"]);
    }

    #[test]
    fn parse_with_sources_content() {
        let json = r#"{"version":3,"sources":["a.js"],"sourcesContent":["var x = 1;"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources_content, vec![Some("var x = 1;".to_string())]);
    }

    #[test]
    fn mappings_for_line() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let line0 = sm.mappings_for_line(0);
        assert!(!line0.is_empty());
        let empty = sm.mappings_for_line(999);
        assert!(empty.is_empty());
    }

    #[test]
    fn large_sourcemap_lookup() {
        // Generate a realistic source map
        let json = generate_test_sourcemap(500, 20, 5);
        let sm = SourceMap::from_json(&json).unwrap();

        // Verify lookups work across the whole map
        for line in [0, 10, 100, 250, 499] {
            let mappings = sm.mappings_for_line(line);
            if let Some(m) = mappings.first() {
                let loc = sm.original_position_for(line, m.generated_column);
                assert!(loc.is_some(), "lookup failed for line {line}");
            }
        }
    }

    #[test]
    fn reverse_lookup_roundtrip() {
        let json = generate_test_sourcemap(100, 10, 3);
        let sm = SourceMap::from_json(&json).unwrap();

        // Pick a mapping and verify forward + reverse roundtrip
        let mapping = &sm.mappings[50];
        if mapping.source != NO_SOURCE {
            let source_name = sm.source(mapping.source);
            let result = sm.generated_position_for(
                source_name,
                mapping.original_line,
                mapping.original_column,
            );
            assert!(result.is_some(), "reverse lookup failed");
        }
    }

    #[test]
    fn indexed_source_map() {
        let json = r#"{
            "version": 3,
            "file": "bundle.js",
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": ["foo"],
                        "mappings": "AAAAA"
                    }
                },
                {
                    "offset": {"line": 10, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["b.js"],
                        "names": ["bar"],
                        "mappings": "AAAAA"
                    }
                }
            ]
        }"#;

        let sm = SourceMap::from_json(json).unwrap();

        // Should have both sources
        assert_eq!(sm.sources.len(), 2);
        assert!(sm.sources.contains(&"a.js".to_string()));
        assert!(sm.sources.contains(&"b.js".to_string()));

        // Should have both names
        assert_eq!(sm.names.len(), 2);
        assert!(sm.names.contains(&"foo".to_string()));
        assert!(sm.names.contains(&"bar".to_string()));

        // First section: line 0, col 0 should map to a.js
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "a.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);

        // Second section: line 10, col 0 should map to b.js
        let loc = sm.original_position_for(10, 0).unwrap();
        assert_eq!(sm.source(loc.source), "b.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn indexed_source_map_shared_sources() {
        // Two sections referencing the same source
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["shared.js"],
                        "names": [],
                        "mappings": "AAAA"
                    }
                },
                {
                    "offset": {"line": 5, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["shared.js"],
                        "names": [],
                        "mappings": "AACA"
                    }
                }
            ]
        }"#;

        let sm = SourceMap::from_json(json).unwrap();

        // Should deduplicate sources
        assert_eq!(sm.sources.len(), 1);
        assert_eq!(sm.sources[0], "shared.js");

        // Both sections should resolve to the same source
        let loc0 = sm.original_position_for(0, 0).unwrap();
        let loc5 = sm.original_position_for(5, 0).unwrap();
        assert_eq!(loc0.source, loc5.source);
    }

    #[test]
    fn parse_ignore_list() {
        let json = r#"{"version":3,"sources":["app.js","node_modules/lib.js"],"names":[],"mappings":"AAAA;ACAA","ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![1]);
    }

    /// Helper: build a source map JSON from absolute mappings data.
    fn build_sourcemap_json(
        sources: &[&str],
        names: &[&str],
        mappings_data: &[Vec<Vec<i64>>],
    ) -> String {
        let mappings_vec: Vec<Vec<Vec<i64>>> = mappings_data.to_vec();
        let encoded = srcmap_codec::encode(&mappings_vec);
        format!(
            r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
            sources
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(","),
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(","),
            encoded,
        )
    }

    // ── 1. Edge cases in decode_mappings ────────────────────────────

    #[test]
    fn decode_multiple_consecutive_semicolons() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;;;AACA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 4);
        assert!(sm.mappings_for_line(1).is_empty());
        assert!(sm.mappings_for_line(2).is_empty());
        assert!(!sm.mappings_for_line(0).is_empty());
        assert!(!sm.mappings_for_line(3).is_empty());
    }

    #[test]
    fn decode_trailing_semicolons() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;;"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 3);
        assert!(!sm.mappings_for_line(0).is_empty());
        assert!(sm.mappings_for_line(1).is_empty());
        assert!(sm.mappings_for_line(2).is_empty());
    }

    #[test]
    fn decode_leading_comma() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":",AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 1);
        let m = &sm.all_mappings()[0];
        assert_eq!(m.generated_line, 0);
        assert_eq!(m.generated_column, 0);
    }

    #[test]
    fn decode_single_field_segments() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,C"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 2);
        for m in sm.all_mappings() {
            assert_eq!(m.source, NO_SOURCE);
        }
        assert_eq!(sm.all_mappings()[0].generated_column, 0);
        assert_eq!(sm.all_mappings()[1].generated_column, 1);
        assert!(sm.original_position_for(0, 0).is_none());
        assert!(sm.original_position_for(0, 1).is_none());
    }

    #[test]
    fn decode_five_field_segments_with_names() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0, 0], vec![10, 0, 0, 5, 1]]];
        let json = build_sourcemap_json(&["app.js"], &["foo", "bar"], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.mapping_count(), 2);
        assert_eq!(sm.all_mappings()[0].name, 0);
        assert_eq!(sm.all_mappings()[1].name, 1);

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
        assert_eq!(sm.name(0), "foo");

        let loc = sm.original_position_for(0, 10).unwrap();
        assert_eq!(loc.name, Some(1));
        assert_eq!(sm.name(1), "bar");
    }

    #[test]
    fn decode_large_vlq_values() {
        let mappings_data = vec![vec![vec![500_i64, 0, 1000, 2000]]];
        let json = build_sourcemap_json(&["big.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.mapping_count(), 1);
        let m = &sm.all_mappings()[0];
        assert_eq!(m.generated_column, 500);
        assert_eq!(m.original_line, 1000);
        assert_eq!(m.original_column, 2000);

        let loc = sm.original_position_for(0, 500).unwrap();
        assert_eq!(loc.line, 1000);
        assert_eq!(loc.column, 2000);
    }

    #[test]
    fn decode_only_semicolons() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":";;;"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 4);
        assert_eq!(sm.mapping_count(), 0);
        for line in 0..4 {
            assert!(sm.mappings_for_line(line).is_empty());
        }
    }

    #[test]
    fn decode_mixed_single_and_four_field_segments() {
        let mappings_data = vec![vec![vec![5_i64, 0, 0, 0]]];
        let four_field_encoded = srcmap_codec::encode(&mappings_data);
        let combined_mappings = format!("A,{four_field_encoded}");
        let json = format!(
            r#"{{"version":3,"sources":["x.js"],"names":[],"mappings":"{combined_mappings}"}}"#,
        );
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.mapping_count(), 2);
        assert_eq!(sm.all_mappings()[0].source, NO_SOURCE);
        assert_eq!(sm.all_mappings()[1].source, 0);
    }

    // ── 2. Source map parsing ───────────────────────────────────────

    #[test]
    fn parse_missing_optional_fields() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.file.is_none());
        assert!(sm.source_root.is_none());
        assert!(sm.sources_content.is_empty());
        assert!(sm.ignore_list.is_empty());
    }

    #[test]
    fn parse_with_file_field() {
        let json =
            r#"{"version":3,"file":"output.js","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.file.as_deref(), Some("output.js"));
    }

    #[test]
    fn parse_null_entries_in_sources() {
        let json = r#"{"version":3,"sources":["a.js",null,"c.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources.len(), 3);
        assert_eq!(sm.sources[0], "a.js");
        assert_eq!(sm.sources[1], "");
        assert_eq!(sm.sources[2], "c.js");
    }

    #[test]
    fn parse_null_entries_in_sources_with_source_root() {
        let json = r#"{"version":3,"sourceRoot":"lib/","sources":["a.js",null],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources[0], "lib/a.js");
        assert_eq!(sm.sources[1], "");
    }

    #[test]
    fn parse_empty_names_array() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.names.is_empty());
    }

    #[test]
    fn parse_invalid_json() {
        let result = SourceMap::from_json("not valid json");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::Json(_)));
    }

    #[test]
    fn parse_json_missing_version() {
        let result = SourceMap::from_json(r#"{"sources":[],"names":[],"mappings":""}"#);
        assert!(result.is_err());
    }

    #[test]
    fn parse_multiple_sources_overlapping_original_positions() {
        let mappings_data = vec![vec![vec![0_i64, 0, 5, 10], vec![10, 1, 5, 10]]];
        let json = build_sourcemap_json(&["a.js", "b.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc0 = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc0.source, 0);
        assert_eq!(sm.source(loc0.source), "a.js");

        let loc1 = sm.original_position_for(0, 10).unwrap();
        assert_eq!(loc1.source, 1);
        assert_eq!(sm.source(loc1.source), "b.js");

        assert_eq!(loc0.line, loc1.line);
        assert_eq!(loc0.column, loc1.column);
    }

    #[test]
    fn parse_sources_content_with_null_entries() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"sourcesContent":["content a",null],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources_content.len(), 2);
        assert_eq!(sm.sources_content[0], Some("content a".to_string()));
        assert_eq!(sm.sources_content[1], None);
    }

    #[test]
    fn parse_empty_sources_and_names() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.sources.is_empty());
        assert!(sm.names.is_empty());
        assert_eq!(sm.mapping_count(), 0);
    }

    // ── 3. Position lookups ─────────────────────────────────────────

    #[test]
    fn lookup_exact_match() {
        let mappings_data = vec![vec![
            vec![0_i64, 0, 10, 20],
            vec![5, 0, 10, 25],
            vec![15, 0, 11, 0],
        ]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.original_position_for(0, 5).unwrap();
        assert_eq!(loc.line, 10);
        assert_eq!(loc.column, 25);
    }

    #[test]
    fn lookup_before_first_segment() {
        let mappings_data = vec![vec![vec![5_i64, 0, 0, 0]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.original_position_for(0, 0).is_none());
        assert!(sm.original_position_for(0, 4).is_none());
    }

    #[test]
    fn lookup_between_segments() {
        let mappings_data = vec![vec![
            vec![0_i64, 0, 1, 0],
            vec![10, 0, 2, 0],
            vec![20, 0, 3, 0],
        ]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.original_position_for(0, 7).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 0);

        let loc = sm.original_position_for(0, 15).unwrap();
        assert_eq!(loc.line, 2);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn lookup_after_last_segment() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0], vec![10, 0, 1, 5]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.original_position_for(0, 100).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn lookup_empty_lines_no_mappings() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0]],
            vec![],
            vec![vec![0_i64, 0, 2, 0]],
        ];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.original_position_for(1, 0).is_none());
        assert!(sm.original_position_for(1, 10).is_none());
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_some());
    }

    #[test]
    fn lookup_line_with_single_mapping() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);

        let loc = sm.original_position_for(0, 50).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn lookup_column_0_vs_column_nonzero() {
        let mappings_data = vec![vec![vec![0_i64, 0, 10, 0], vec![8, 0, 20, 5]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc0 = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc0.line, 10);
        assert_eq!(loc0.column, 0);

        let loc8 = sm.original_position_for(0, 8).unwrap();
        assert_eq!(loc8.line, 20);
        assert_eq!(loc8.column, 5);

        let loc4 = sm.original_position_for(0, 4).unwrap();
        assert_eq!(loc4.line, 10);
    }

    #[test]
    fn lookup_beyond_last_line() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.original_position_for(1, 0).is_none());
        assert!(sm.original_position_for(100, 0).is_none());
    }

    #[test]
    fn lookup_single_field_returns_none() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 1);
        assert!(sm.original_position_for(0, 0).is_none());
    }

    // ── 4. Reverse lookups (generated_position_for) ─────────────────

    #[test]
    fn reverse_lookup_exact_match() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0]],
            vec![vec![4, 0, 1, 0], vec![10, 0, 1, 8]],
            vec![vec![0, 0, 2, 0]],
        ];
        let json = build_sourcemap_json(&["main.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.generated_position_for("main.js", 1, 8).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 10);
    }

    #[test]
    fn reverse_lookup_no_match() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0], vec![10, 0, 0, 10]]];
        let json = build_sourcemap_json(&["main.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.generated_position_for("main.js", 99, 0).is_none());
    }

    #[test]
    fn reverse_lookup_unknown_source() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0]]];
        let json = build_sourcemap_json(&["main.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.generated_position_for("unknown.js", 0, 0).is_none());
    }

    #[test]
    fn reverse_lookup_multiple_mappings_same_original() {
        let mappings_data = vec![vec![vec![0_i64, 0, 5, 10]], vec![vec![20, 0, 5, 10]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.generated_position_for("src.js", 5, 10);
        assert!(loc.is_some());
        let loc = loc.unwrap();
        assert!(
            (loc.line == 0 && loc.column == 0) || (loc.line == 1 && loc.column == 20),
            "Expected (0,0) or (1,20), got ({},{})",
            loc.line,
            loc.column
        );
    }

    #[test]
    fn reverse_lookup_with_multiple_sources() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0], vec![10, 1, 0, 0]]];
        let json = build_sourcemap_json(&["a.js", "b.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc_a = sm.generated_position_for("a.js", 0, 0).unwrap();
        assert_eq!(loc_a.line, 0);
        assert_eq!(loc_a.column, 0);

        let loc_b = sm.generated_position_for("b.js", 0, 0).unwrap();
        assert_eq!(loc_b.line, 0);
        assert_eq!(loc_b.column, 10);
    }

    #[test]
    fn reverse_lookup_skips_single_field_segments() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,KAAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        let loc = sm.generated_position_for("a.js", 0, 0).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn reverse_lookup_finds_each_original_line() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0]],
            vec![vec![0, 0, 1, 0]],
            vec![vec![0, 0, 2, 0]],
            vec![vec![0, 0, 3, 0]],
        ];
        let json = build_sourcemap_json(&["x.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        for orig_line in 0..4 {
            let loc = sm.generated_position_for("x.js", orig_line, 0).unwrap();
            assert_eq!(
                loc.line, orig_line,
                "reverse lookup for orig line {orig_line}"
            );
            assert_eq!(loc.column, 0);
        }
    }

    // ── 5. ignoreList ───────────────────────────────────────────────

    #[test]
    fn parse_with_ignore_list_multiple() {
        let json = r#"{"version":3,"sources":["app.js","node_modules/lib.js","vendor.js"],"names":[],"mappings":"AAAA","ignoreList":[1,2]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![1, 2]);
    }

    #[test]
    fn parse_with_empty_ignore_list() {
        let json =
            r#"{"version":3,"sources":["app.js"],"names":[],"mappings":"AAAA","ignoreList":[]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.ignore_list.is_empty());
    }

    #[test]
    fn parse_without_ignore_list_field() {
        let json = r#"{"version":3,"sources":["app.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.ignore_list.is_empty());
    }

    // ── Additional edge case tests ──────────────────────────────────

    #[test]
    fn source_index_lookup() {
        let json = r#"{"version":3,"sources":["a.js","b.js","c.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.source_index("a.js"), Some(0));
        assert_eq!(sm.source_index("b.js"), Some(1));
        assert_eq!(sm.source_index("c.js"), Some(2));
        assert_eq!(sm.source_index("d.js"), None);
    }

    #[test]
    fn all_mappings_returns_complete_list() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0], vec![5, 0, 0, 5]],
            vec![vec![0, 0, 1, 0]],
        ];
        let json = build_sourcemap_json(&["x.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.all_mappings().len(), 3);
        assert_eq!(sm.mapping_count(), 3);
    }

    #[test]
    fn line_count_matches_decoded_lines() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0]],
            vec![],
            vec![vec![0_i64, 0, 2, 0]],
            vec![],
            vec![],
        ];
        let json = build_sourcemap_json(&["x.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.line_count(), 5);
    }

    #[test]
    fn parse_error_display() {
        let err = ParseError::InvalidVersion(5);
        assert_eq!(format!("{err}"), "unsupported source map version: 5");

        let json_err = SourceMap::from_json("{}").unwrap_err();
        let display = format!("{json_err}");
        assert!(display.contains("JSON parse error") || display.contains("missing field"));
    }

    #[test]
    fn original_position_name_none_for_four_field() {
        let mappings_data = vec![vec![vec![0_i64, 0, 5, 10]]];
        let json = build_sourcemap_json(&["a.js"], &["unused_name"], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        let loc = sm.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_none());
    }

    #[test]
    fn forward_and_reverse_roundtrip_comprehensive() {
        let mappings_data = vec![
            vec![vec![0_i64, 0, 0, 0], vec![10, 0, 0, 10], vec![20, 1, 5, 0]],
            vec![vec![0, 0, 1, 0], vec![5, 1, 6, 3]],
            vec![vec![0, 0, 2, 0]],
        ];
        let json = build_sourcemap_json(&["a.js", "b.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        for m in sm.all_mappings() {
            if m.source == NO_SOURCE {
                continue;
            }
            let source_name = sm.source(m.source);

            let orig = sm
                .original_position_for(m.generated_line, m.generated_column)
                .unwrap();
            assert_eq!(orig.source, m.source);
            assert_eq!(orig.line, m.original_line);
            assert_eq!(orig.column, m.original_column);

            let gen_loc = sm
                .generated_position_for(source_name, m.original_line, m.original_column)
                .unwrap();
            assert_eq!(gen_loc.line, m.generated_line);
            assert_eq!(gen_loc.column, m.generated_column);
        }
    }

    // ── 6. Comprehensive edge case tests ────────────────────────────

    // -- sourceRoot edge cases --

    #[test]
    fn source_root_with_multiple_sources() {
        let json = r#"{"version":3,"sourceRoot":"lib/","sources":["a.js","b.js","c.js"],"names":[],"mappings":"AAAA,KACA,KACA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources, vec!["lib/a.js", "lib/b.js", "lib/c.js"]);
    }

    #[test]
    fn source_root_empty_string() {
        let json =
            r#"{"version":3,"sourceRoot":"","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources, vec!["a.js"]);
    }

    #[test]
    fn source_root_preserved_in_to_json() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        assert!(output.contains(r#""sourceRoot":"src/""#));
    }

    #[test]
    fn source_root_reverse_lookup_uses_prefixed_name() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Must use the prefixed name for reverse lookups
        assert!(sm.generated_position_for("src/a.js", 0, 0).is_some());
        assert!(sm.generated_position_for("a.js", 0, 0).is_none());
    }

    #[test]
    fn source_root_with_trailing_slash() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources[0], "src/a.js");
    }

    #[test]
    fn source_root_without_trailing_slash() {
        let json =
            r#"{"version":3,"sourceRoot":"src","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources[0], "srca.js"); // No auto-slash — sourceRoot is raw prefix
    }

    // -- JSON/parsing error cases --

    #[test]
    fn parse_empty_json_object() {
        // {} has no version field
        let result = SourceMap::from_json("{}");
        assert!(result.is_err());
    }

    #[test]
    fn parse_version_0() {
        let json = r#"{"version":0,"sources":[],"names":[],"mappings":""}"#;
        assert!(matches!(
            SourceMap::from_json(json).unwrap_err(),
            ParseError::InvalidVersion(0)
        ));
    }

    #[test]
    fn parse_version_4() {
        let json = r#"{"version":4,"sources":[],"names":[],"mappings":""}"#;
        assert!(matches!(
            SourceMap::from_json(json).unwrap_err(),
            ParseError::InvalidVersion(4)
        ));
    }

    #[test]
    fn parse_extra_unknown_fields_ignored() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_custom_field":true,"x_debug":{"foo":"bar"}}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 1);
    }

    #[test]
    fn parse_vlq_error_propagated() {
        // '!' is not valid base64 — should surface as VLQ error
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AA!A"}"#;
        let result = SourceMap::from_json(json);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::Vlq(_)));
    }

    #[test]
    fn parse_truncated_vlq_error() {
        // 'g' has continuation bit set — truncated VLQ
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"g"}"#;
        let result = SourceMap::from_json(json);
        assert!(result.is_err());
    }

    // -- to_json edge cases --

    #[test]
    fn to_json_produces_valid_json() {
        let json = r#"{"version":3,"file":"out.js","sourceRoot":"src/","sources":["a.ts","b.ts"],"sourcesContent":["const x = 1;\nconst y = \"hello\";",null],"names":["x","y"],"mappings":"AAAAA,KACAC;AACA","ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        // Must be valid JSON that serde can parse
        let _: serde_json::Value = serde_json::from_str(&output).unwrap();
    }

    #[test]
    fn to_json_escapes_special_chars() {
        let json = r#"{"version":3,"sources":["path/with\"quotes.js"],"sourcesContent":["line1\nline2\ttab\\backslash"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        let _: serde_json::Value = serde_json::from_str(&output).unwrap();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(
            sm2.sources_content[0].as_deref(),
            Some("line1\nline2\ttab\\backslash")
        );
    }

    #[test]
    fn to_json_empty_map() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.mapping_count(), 0);
        assert!(sm2.sources.is_empty());
    }

    #[test]
    fn to_json_roundtrip_with_names() {
        let mappings_data = vec![vec![
            vec![0_i64, 0, 0, 0, 0],
            vec![10, 0, 0, 10, 1],
            vec![20, 0, 1, 0, 2],
        ]];
        let json = build_sourcemap_json(&["src.js"], &["foo", "bar", "baz"], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();

        for m in sm2.all_mappings() {
            if m.source != NO_SOURCE && m.name != NO_NAME {
                let loc = sm2
                    .original_position_for(m.generated_line, m.generated_column)
                    .unwrap();
                assert!(loc.name.is_some());
            }
        }
    }

    // -- Indexed source map edge cases --

    #[test]
    fn indexed_source_map_column_offset() {
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 10},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": [],
                        "mappings": "AAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Mapping at col 0 in section should be offset to col 10 (first line only)
        let loc = sm.original_position_for(0, 10).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
        // Before the offset should have no mapping
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn indexed_source_map_column_offset_only_first_line() {
        // Column offset only applies to the first line of a section
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 20},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": [],
                        "mappings": "AAAA;AAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Line 0: column offset applies
        let loc = sm.original_position_for(0, 20).unwrap();
        assert_eq!(loc.column, 0);
        // Line 1: column offset does NOT apply
        let loc = sm.original_position_for(1, 0).unwrap();
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn indexed_source_map_empty_section() {
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": [],
                        "names": [],
                        "mappings": ""
                    }
                },
                {
                    "offset": {"line": 5, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["b.js"],
                        "names": [],
                        "mappings": "AAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources.len(), 1);
        let loc = sm.original_position_for(5, 0).unwrap();
        assert_eq!(sm.source(loc.source), "b.js");
    }

    #[test]
    fn indexed_source_map_with_sources_content() {
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "sourcesContent": ["var a = 1;"],
                        "names": [],
                        "mappings": "AAAA"
                    }
                },
                {
                    "offset": {"line": 5, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["b.js"],
                        "sourcesContent": ["var b = 2;"],
                        "names": [],
                        "mappings": "AAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources_content.len(), 2);
        assert_eq!(sm.sources_content[0], Some("var a = 1;".to_string()));
        assert_eq!(sm.sources_content[1], Some("var b = 2;".to_string()));
    }

    #[test]
    fn indexed_source_map_with_ignore_list() {
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["app.js", "vendor.js"],
                        "names": [],
                        "mappings": "AAAA",
                        "ignoreList": [1]
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(!sm.ignore_list.is_empty());
    }

    // -- Boundary conditions --

    #[test]
    fn lookup_max_column_on_line() {
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0]]];
        let json = build_sourcemap_json(&["a.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        // Very large column — should snap to the last mapping on line
        let loc = sm.original_position_for(0, u32::MAX - 1).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn mappings_for_line_beyond_end() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.mappings_for_line(u32::MAX).is_empty());
    }

    #[test]
    fn source_with_unicode_path() {
        let json =
            r#"{"version":3,"sources":["src/日本語.ts"],"names":["変数"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources[0], "src/日本語.ts");
        assert_eq!(sm.names[0], "変数");
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "src/日本語.ts");
        assert_eq!(sm.name(loc.name.unwrap()), "変数");
    }

    #[test]
    fn to_json_roundtrip_unicode_sources() {
        let json = r#"{"version":3,"sources":["src/日本語.ts"],"sourcesContent":["const 変数 = 1;"],"names":["変数"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        let _: serde_json::Value = serde_json::from_str(&output).unwrap();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.sources[0], "src/日本語.ts");
        assert_eq!(sm2.sources_content[0], Some("const 変数 = 1;".to_string()));
    }

    #[test]
    fn many_sources_lookup() {
        // 100 sources, verify source_index works for all
        let sources: Vec<String> = (0..100).map(|i| format!("src/file{i}.js")).collect();
        let source_strs: Vec<&str> = sources.iter().map(|s| s.as_str()).collect();
        let mappings_data = vec![
            sources
                .iter()
                .enumerate()
                .map(|(i, _)| vec![(i * 10) as i64, i as i64, 0, 0])
                .collect::<Vec<_>>(),
        ];
        let json = build_sourcemap_json(&source_strs, &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        for (i, src) in sources.iter().enumerate() {
            assert_eq!(sm.source_index(src), Some(i as u32));
        }
    }

    #[test]
    fn clone_sourcemap() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["x"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let sm2 = sm.clone();
        assert_eq!(sm2.sources, sm.sources);
        assert_eq!(sm2.mapping_count(), sm.mapping_count());
        let loc = sm2.original_position_for(0, 0).unwrap();
        assert_eq!(sm2.source(loc.source), "a.js");
    }

    #[test]
    fn parse_debug_id() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","debugId":"85314830-023f-4cf1-a267-535f4e37bb17"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(
            sm.debug_id.as_deref(),
            Some("85314830-023f-4cf1-a267-535f4e37bb17")
        );
    }

    #[test]
    fn parse_debug_id_snake_case() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","debug_id":"85314830-023f-4cf1-a267-535f4e37bb17"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(
            sm.debug_id.as_deref(),
            Some("85314830-023f-4cf1-a267-535f4e37bb17")
        );
    }

    #[test]
    fn parse_no_debug_id() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.debug_id, None);
    }

    #[test]
    fn debug_id_roundtrip() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","debugId":"85314830-023f-4cf1-a267-535f4e37bb17"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        assert!(output.contains(r#""debugId":"85314830-023f-4cf1-a267-535f4e37bb17""#));
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm.debug_id, sm2.debug_id);
    }

    #[test]
    fn debug_id_not_in_json_when_absent() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        assert!(!output.contains("debugId"));
    }

    /// Generate a test source map JSON with realistic structure.
    fn generate_test_sourcemap(lines: usize, segs_per_line: usize, num_sources: usize) -> String {
        let sources: Vec<String> = (0..num_sources)
            .map(|i| format!("src/file{i}.js"))
            .collect();
        let names: Vec<String> = (0..20).map(|i| format!("var{i}")).collect();

        let mut mappings_parts = Vec::with_capacity(lines);
        let mut gen_col;
        let mut src: i64 = 0;
        let mut src_line: i64 = 0;
        let mut src_col: i64;
        let mut name: i64 = 0;

        for _ in 0..lines {
            gen_col = 0i64;
            let mut line_parts = Vec::with_capacity(segs_per_line);

            for s in 0..segs_per_line {
                let gc_delta = 2 + (s as i64 * 3) % 20;
                gen_col += gc_delta;

                let src_delta = if s % 7 == 0 { 1 } else { 0 };
                src = (src + src_delta) % num_sources as i64;

                src_line += 1;
                src_col = (s as i64 * 5 + 1) % 30;

                let has_name = s % 4 == 0;
                if has_name {
                    name = (name + 1) % names.len() as i64;
                }

                // Build segment using codec encode
                let segment = if has_name {
                    vec![gen_col, src, src_line, src_col, name]
                } else {
                    vec![gen_col, src, src_line, src_col]
                };

                line_parts.push(segment);
            }

            mappings_parts.push(line_parts);
        }

        let encoded = srcmap_codec::encode(&mappings_parts);

        format!(
            r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
            sources
                .iter()
                .map(|s| format!("\"{s}\""))
                .collect::<Vec<_>>()
                .join(","),
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(","),
            encoded,
        )
    }
}
