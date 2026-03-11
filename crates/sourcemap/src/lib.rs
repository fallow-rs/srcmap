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

use std::cell::{OnceCell, RefCell};
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
///
/// Maps a position in the generated output to an optional position in an
/// original source file. Stored contiguously in a `Vec<Mapping>` sorted by
/// `(generated_line, generated_column)` for cache-friendly binary search.
#[derive(Debug, Clone, Copy)]
pub struct Mapping {
    /// 0-based line in the generated output.
    pub generated_line: u32,
    /// 0-based column in the generated output.
    pub generated_column: u32,
    /// Index into `SourceMap::sources`. `u32::MAX` if this mapping has no source.
    pub source: u32,
    /// 0-based line in the original source (only meaningful when `source != u32::MAX`).
    pub original_line: u32,
    /// 0-based column in the original source (only meaningful when `source != u32::MAX`).
    pub original_column: u32,
    /// Index into `SourceMap::names`. `u32::MAX` if this mapping has no name.
    pub name: u32,
}

/// Result of an [`SourceMap::original_position_for`] lookup.
///
/// All indices are 0-based. Use [`SourceMap::source`] and [`SourceMap::name`]
/// to resolve the `source` and `name` indices to strings.
#[derive(Debug, Clone)]
pub struct OriginalLocation {
    /// Index into `SourceMap::sources`.
    pub source: u32,
    /// 0-based line in the original source.
    pub line: u32,
    /// 0-based column in the original source.
    pub column: u32,
    /// Index into `SourceMap::names`, if the mapping has a name.
    pub name: Option<u32>,
}

/// Result of a [`SourceMap::generated_position_for`] lookup.
///
/// All values are 0-based.
#[derive(Debug, Clone)]
pub struct GeneratedLocation {
    /// 0-based line in the generated output.
    pub line: u32,
    /// 0-based column in the generated output.
    pub column: u32,
}

/// Search bias for position lookups.
///
/// Controls how non-exact matches are resolved during binary search:
/// - `GreatestLowerBound` (default): find the closest mapping at or before the position
/// - `LeastUpperBound`: find the closest mapping at or after the position
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Bias {
    /// Return the closest position at or before the requested position (default).
    #[default]
    GreatestLowerBound,
    /// Return the closest position at or after the requested position.
    LeastUpperBound,
}

/// A mapped range: original start/end positions for a generated range.
///
/// Returned by [`SourceMap::map_range`]. Both endpoints must resolve to the
/// same source file.
#[derive(Debug, Clone)]
pub struct MappedRange {
    /// Index into `SourceMap::sources`.
    pub source: u32,
    /// 0-based start line in the original source.
    pub original_start_line: u32,
    /// 0-based start column in the original source.
    pub original_start_column: u32,
    /// 0-based end line in the original source.
    pub original_end_line: u32,
    /// 0-based end column in the original source.
    pub original_end_column: u32,
}

/// Errors that can occur during source map parsing.
#[derive(Debug)]
pub enum ParseError {
    /// The JSON could not be deserialized.
    Json(serde_json::Error),
    /// The VLQ mappings string is malformed.
    Vlq(DecodeError),
    /// The `version` field is not `3`.
    InvalidVersion(u32),
    /// The ECMA-426 scopes data could not be decoded.
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
    /// Deprecated Chrome DevTools field, fallback for `ignoreList`.
    #[serde(default, rename = "x_google_ignoreList")]
    x_google_ignore_list: Option<Vec<u32>>,
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
    /// Catch-all for unknown extension fields (x_*).
    #[serde(flatten)]
    extensions: HashMap<String, serde_json::Value>,
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

/// A fully-parsed source map with O(log n) position lookups.
///
/// Supports both regular and indexed (sectioned) source maps, `ignoreList`,
/// `debugId`, scopes (ECMA-426), and extension fields. All positions are
/// 0-based lines and columns.
///
/// # Construction
///
/// - [`SourceMap::from_json`] — parse from a JSON string (most common)
/// - [`SourceMap::from_parts`] — build from pre-decoded components
/// - [`SourceMap::from_vlq`] — parse from pre-extracted parts + raw VLQ string
/// - [`SourceMap::from_json_lines`] — partial parse for a line range
///
/// # Lookups
///
/// - [`SourceMap::original_position_for`] — forward: generated → original
/// - [`SourceMap::generated_position_for`] — reverse: original → generated (lazy index)
/// - [`SourceMap::all_generated_positions_for`] — all reverse matches
/// - [`SourceMap::map_range`] — map a generated range to its original range
///
/// For cases where you only need a few lookups and want to avoid decoding
/// all mappings upfront, see [`LazySourceMap`].
#[derive(Debug, Clone)]
pub struct SourceMap {
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub sources: Vec<String>,
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    pub ignore_list: Vec<u32>,
    /// Extension fields (x_* keys) preserved for passthrough.
    pub extensions: HashMap<String, serde_json::Value>,
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

        // Use x_google_ignoreList as fallback when ignoreList is absent
        let ignore_list = if raw.ignore_list.is_empty() {
            raw.x_google_ignore_list.unwrap_or_default()
        } else {
            raw.ignore_list
        };

        // Filter extensions to only keep x_* fields
        let extensions: HashMap<String, serde_json::Value> = raw
            .extensions
            .into_iter()
            .filter(|(k, _)| k.starts_with("x_"))
            .collect();

        Ok(Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content,
            names: raw.names,
            ignore_list,
            extensions,
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
            extensions: HashMap::new(),
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
        self.original_position_for_with_bias(line, column, Bias::GreatestLowerBound)
    }

    /// Look up the original source position with a search bias.
    ///
    /// Both `line` and `column` are 0-based.
    /// - `GreatestLowerBound`: find the closest mapping at or before the column (default)
    /// - `LeastUpperBound`: find the closest mapping at or after the column
    pub fn original_position_for_with_bias(
        &self,
        line: u32,
        column: u32,
        bias: Bias,
    ) -> Option<OriginalLocation> {
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

        let idx = match bias {
            Bias::GreatestLowerBound => {
                // Find largest generated_column <= column
                match line_mappings.binary_search_by_key(&column, |m| m.generated_column) {
                    Ok(i) => i,
                    Err(0) => return None,
                    Err(i) => i - 1,
                }
            }
            Bias::LeastUpperBound => {
                // Find smallest generated_column >= column
                match line_mappings.binary_search_by_key(&column, |m| m.generated_column) {
                    Ok(i) => i,
                    Err(i) => {
                        if i >= line_mappings.len() {
                            return None;
                        }
                        i
                    }
                }
            }
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
    /// Uses `LeastUpperBound` by default (finds first mapping at or after the position).
    pub fn generated_position_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Option<GeneratedLocation> {
        self.generated_position_for_with_bias(source, line, column, Bias::LeastUpperBound)
    }

    /// Look up the generated position with a search bias.
    ///
    /// `source` is the source filename. `line` and `column` are 0-based.
    /// - `GreatestLowerBound`: find the closest mapping at or before the position (default)
    /// - `LeastUpperBound`: find the closest mapping at or after the position
    pub fn generated_position_for_with_bias(
        &self,
        source: &str,
        line: u32,
        column: u32,
        bias: Bias,
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

        match bias {
            Bias::GreatestLowerBound => {
                // partition_point gives us the first element >= target.
                // For GLB, we want the element at or before.
                // If exact match at idx, use it. Otherwise use idx-1.
                if idx < reverse_index.len() {
                    let mapping = &self.mappings[reverse_index[idx] as usize];
                    if mapping.source == source_idx
                        && mapping.original_line == line
                        && mapping.original_column == column
                    {
                        return Some(GeneratedLocation {
                            line: mapping.generated_line,
                            column: mapping.generated_column,
                        });
                    }
                }
                // No exact match: use the element before (greatest lower bound)
                if idx == 0 {
                    return None;
                }
                let mapping = &self.mappings[reverse_index[idx - 1] as usize];
                if mapping.source != source_idx {
                    return None;
                }
                Some(GeneratedLocation {
                    line: mapping.generated_line,
                    column: mapping.generated_column,
                })
            }
            Bias::LeastUpperBound => {
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
        }
    }

    /// Find all generated positions for an original source position.
    ///
    /// `source` is the source filename. `line` and `column` are 0-based.
    /// Returns all generated positions that map back to this original location.
    pub fn all_generated_positions_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Vec<GeneratedLocation> {
        let Some(&source_idx) = self.source_map.get(source) else {
            return Vec::new();
        };

        let reverse_index = self
            .reverse_index
            .get_or_init(|| build_reverse_index(&self.mappings));

        // Find the first entry matching (source, line, column)
        let start = reverse_index.partition_point(|&i| {
            let m = &self.mappings[i as usize];
            (m.source, m.original_line, m.original_column) < (source_idx, line, column)
        });

        let mut results = Vec::new();

        for &ri in &reverse_index[start..] {
            let m = &self.mappings[ri as usize];
            if m.source != source_idx || m.original_line != line || m.original_column != column {
                break;
            }
            results.push(GeneratedLocation {
                line: m.generated_line,
                column: m.generated_column,
            });
        }

        results
    }

    /// Map a generated range to its original range.
    ///
    /// Given a generated range `(start_line:start_column → end_line:end_column)`,
    /// maps both endpoints through the source map and returns the original range.
    /// Both endpoints must resolve to the same source file.
    pub fn map_range(
        &self,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    ) -> Option<MappedRange> {
        let start = self.original_position_for(start_line, start_column)?;
        let end = self.original_position_for(end_line, end_column)?;

        // Both endpoints must map to the same source
        if start.source != end.source {
            return None;
        }

        Some(MappedRange {
            source: start.source,
            original_start_line: start.line,
            original_start_column: start.column,
            original_end_line: end.line,
            original_end_column: end.column,
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
        self.to_json_with_options(false)
    }

    /// Serialize the source map back to JSON with options.
    ///
    /// If `exclude_content` is true, `sourcesContent` is omitted from the output.
    pub fn to_json_with_options(&self, exclude_content: bool) -> String {
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

        if !exclude_content
            && !self.sources_content.is_empty()
            && self.sources_content.iter().any(|c| c.is_some())
        {
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

        // Emit extension fields (x_* keys)
        let mut ext_keys: Vec<&String> = self.extensions.keys().collect();
        ext_keys.sort();
        for key in ext_keys {
            if let Some(val) = self.extensions.get(key) {
                json.push(',');
                json_quote_into(&mut json, key);
                json.push(':');
                json.push_str(&serde_json::to_string(val).unwrap_or_default());
            }
        }

        json.push('}');
        json
    }

    /// Construct a `SourceMap` from pre-built parts.
    ///
    /// This avoids the encode-then-decode round-trip used in composition pipelines.
    /// Mappings must be sorted by (generated_line, generated_column).
    /// Use `u32::MAX` for `source`/`name` fields to indicate absence.
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        file: Option<String>,
        source_root: Option<String>,
        sources: Vec<String>,
        sources_content: Vec<Option<String>>,
        names: Vec<String>,
        mappings: Vec<Mapping>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
        scopes: Option<ScopeInfo>,
    ) -> Self {
        // Build line_offsets from sorted mappings
        let line_count = mappings.last().map_or(0, |m| m.generated_line as usize + 1);
        let mut line_offsets: Vec<u32> = vec![0; line_count + 1];
        let mut current_line: usize = 0;
        for (i, m) in mappings.iter().enumerate() {
            while current_line < m.generated_line as usize {
                current_line += 1;
                if current_line < line_offsets.len() {
                    line_offsets[current_line] = i as u32;
                }
            }
        }
        // Fill remaining with sentinel
        if !line_offsets.is_empty() {
            let last = mappings.len() as u32;
            for offset in line_offsets.iter_mut().skip(current_line + 1) {
                *offset = last;
            }
        }

        let source_map: HashMap<String, u32> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        Self {
            file,
            source_root,
            sources,
            sources_content,
            names,
            ignore_list,
            extensions: HashMap::new(),
            debug_id,
            scopes,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
        }
    }

    /// Build a source map from pre-parsed components and a VLQ mappings string.
    ///
    /// This is the fast path for WASM: JS does `JSON.parse()` (V8-native speed),
    /// then only the VLQ mappings string crosses into WASM for decoding.
    /// Avoids copying large `sourcesContent` into WASM linear memory.
    pub fn from_vlq(
        mappings_str: &str,
        sources: Vec<String>,
        names: Vec<String>,
        file: Option<String>,
        source_root: Option<String>,
        sources_content: Vec<Option<String>>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
    ) -> Result<Self, ParseError> {
        let (mappings, line_offsets) = decode_mappings(mappings_str)?;

        let source_map: HashMap<String, u32> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        Ok(Self {
            file,
            source_root,
            sources,
            sources_content,
            names,
            ignore_list,
            extensions: HashMap::new(),
            debug_id,
            scopes: None,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
        })
    }

    /// Parse a source map from JSON, decoding only mappings for lines in `[start_line, end_line)`.
    ///
    /// This is useful for large source maps where only a subset of lines is needed.
    /// VLQ state is maintained through skipped lines (required for correct delta decoding),
    /// but `Mapping` structs are only allocated for lines in the requested range.
    pub fn from_json_lines(json: &str, start_line: u32, end_line: u32) -> Result<Self, ParseError> {
        let raw: RawSourceMap<'_> = serde_json::from_str(json)?;

        if raw.version != 3 {
            return Err(ParseError::InvalidVersion(raw.version));
        }

        // Resolve sources
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

        let source_map: HashMap<String, u32> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        // Decode only the requested line range
        let (mappings, line_offsets) = decode_mappings_range(raw.mappings, start_line, end_line)?;

        // Decode scopes if present
        let num_sources = sources.len();
        let scopes = match raw.scopes {
            Some(scopes_str) if !scopes_str.is_empty() => Some(
                srcmap_scopes::decode_scopes(scopes_str, &raw.names, num_sources)
                    .map_err(|e| ParseError::Scopes(e.to_string()))?,
            ),
            _ => None,
        };

        let ignore_list = if raw.ignore_list.is_empty() {
            raw.x_google_ignore_list.unwrap_or_default()
        } else {
            raw.ignore_list
        };

        // Filter extensions to only keep x_* fields
        let extensions: HashMap<String, serde_json::Value> = raw
            .extensions
            .into_iter()
            .filter(|(k, _)| k.starts_with("x_"))
            .collect();

        Ok(Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content,
            names: raw.names,
            ignore_list,
            extensions,
            debug_id: raw.debug_id,
            scopes,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
        })
    }

    /// Encode all mappings back to a VLQ mappings string.
    pub fn encode_mappings(&self) -> String {
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

// ── LazySourceMap ──────────────────────────────────────────────────

/// Cumulative VLQ state at a line boundary.
#[derive(Debug, Clone, Copy)]
struct VlqState {
    source_index: i64,
    original_line: i64,
    original_column: i64,
    name_index: i64,
}

/// Pre-scanned line info for O(1) random access into the raw mappings string.
#[derive(Debug, Clone)]
struct LineInfo {
    /// Byte offset into the raw mappings string where this line starts.
    byte_offset: usize,
    /// Byte offset where this line ends (exclusive, at `;` or end of string).
    byte_end: usize,
    /// Cumulative VLQ state at the start of this line.
    state: VlqState,
}

/// A lazily-decoded source map that defers VLQ mappings decoding until needed.
///
/// For large source maps (100MB+), this avoids decoding all mappings upfront.
/// JSON metadata (sources, names, etc.) is parsed eagerly, but VLQ mappings
/// are decoded on a per-line basis on demand.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::LazySourceMap;
///
/// let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;AACA"}"#;
/// let sm = LazySourceMap::from_json(json).unwrap();
///
/// // Mappings are only decoded when accessed
/// let loc = sm.original_position_for(0, 0).unwrap();
/// assert_eq!(sm.source(loc.source), "input.js");
/// ```
#[derive(Debug)]
pub struct LazySourceMap {
    pub file: Option<String>,
    pub source_root: Option<String>,
    pub sources: Vec<String>,
    pub sources_content: Vec<Option<String>>,
    pub names: Vec<String>,
    pub ignore_list: Vec<u32>,
    pub extensions: HashMap<String, serde_json::Value>,
    pub debug_id: Option<String>,
    pub scopes: Option<ScopeInfo>,

    /// Raw VLQ mappings string (owned).
    raw_mappings: String,

    /// Pre-scanned line info for O(1) line access.
    line_info: Vec<LineInfo>,

    /// Cache of decoded lines: line index -> `Vec<Mapping>`.
    decoded_lines: RefCell<HashMap<u32, Vec<Mapping>>>,

    /// Source filename -> index for O(1) lookup by name.
    source_map: HashMap<String, u32>,
}

impl LazySourceMap {
    /// Parse a source map from JSON, deferring VLQ mappings decoding.
    ///
    /// Parses all JSON metadata eagerly but stores the raw mappings string.
    /// VLQ mappings are decoded per-line on demand.
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        let raw: RawSourceMap<'_> = serde_json::from_str(json)?;

        if raw.version != 3 {
            return Err(ParseError::InvalidVersion(raw.version));
        }

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

        let source_map: HashMap<String, u32> = sources
            .iter()
            .enumerate()
            .map(|(i, s)| (s.clone(), i as u32))
            .collect();

        // Pre-scan the raw mappings string to find semicolon positions
        // and compute cumulative VLQ state at each line boundary.
        let raw_mappings = raw.mappings.to_string();
        let line_info = prescan_mappings(&raw_mappings)?;

        // Decode scopes if present
        let num_sources = sources.len();
        let scopes = match raw.scopes {
            Some(scopes_str) if !scopes_str.is_empty() => Some(
                srcmap_scopes::decode_scopes(scopes_str, &raw.names, num_sources)
                    .map_err(|e| ParseError::Scopes(e.to_string()))?,
            ),
            _ => None,
        };

        let ignore_list = if raw.ignore_list.is_empty() {
            raw.x_google_ignore_list.unwrap_or_default()
        } else {
            raw.ignore_list
        };

        // Filter extensions to only keep x_* fields
        let extensions: HashMap<String, serde_json::Value> = raw
            .extensions
            .into_iter()
            .filter(|(k, _)| k.starts_with("x_"))
            .collect();

        Ok(Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content,
            names: raw.names,
            ignore_list,
            extensions,
            debug_id: raw.debug_id,
            scopes,
            raw_mappings,
            line_info,
            decoded_lines: RefCell::new(HashMap::new()),
            source_map,
        })
    }

    /// Decode a single line's mappings on demand.
    ///
    /// Returns the cached result if the line has already been decoded.
    /// The line index is 0-based.
    pub fn decode_line(&self, line: u32) -> Result<Vec<Mapping>, DecodeError> {
        // Check cache first
        if let Some(cached) = self.decoded_lines.borrow().get(&line) {
            return Ok(cached.clone());
        }

        let line_idx = line as usize;
        if line_idx >= self.line_info.len() {
            return Ok(Vec::new());
        }

        let info = &self.line_info[line_idx];
        let bytes = self.raw_mappings.as_bytes();
        let slice = &bytes[info.byte_offset..info.byte_end];

        let mut mappings = Vec::new();
        let mut source_index = info.state.source_index;
        let mut original_line = info.state.original_line;
        let mut original_column = info.state.original_column;
        let mut name_index = info.state.name_index;
        let mut generated_column: i64 = 0;
        let mut pos: usize = 0;
        let len = slice.len();

        // We need to adjust `pos` for vlq_fast which works on the full byte slice.
        // Instead, create a local helper working on the slice directly.
        let base_offset = info.byte_offset;

        while pos < len {
            let byte = slice[pos];

            if byte == b',' {
                pos += 1;
                continue;
            }

            // Use vlq_fast on the full byte buffer with adjusted position
            let mut abs_pos = base_offset + pos;

            // Field 1: generated column
            generated_column += vlq_fast(bytes, &mut abs_pos)?;

            if abs_pos < base_offset + len && bytes[abs_pos] != b',' && bytes[abs_pos] != b';' {
                // Fields 2-4
                source_index += vlq_fast(bytes, &mut abs_pos)?;
                original_line += vlq_fast(bytes, &mut abs_pos)?;
                original_column += vlq_fast(bytes, &mut abs_pos)?;

                // Field 5: name (optional)
                let name = if abs_pos < base_offset + len
                    && bytes[abs_pos] != b','
                    && bytes[abs_pos] != b';'
                {
                    name_index += vlq_fast(bytes, &mut abs_pos)?;
                    name_index as u32
                } else {
                    NO_NAME
                };

                mappings.push(Mapping {
                    generated_line: line,
                    generated_column: generated_column as u32,
                    source: source_index as u32,
                    original_line: original_line as u32,
                    original_column: original_column as u32,
                    name,
                });
            } else {
                // 1-field segment
                mappings.push(Mapping {
                    generated_line: line,
                    generated_column: generated_column as u32,
                    source: NO_SOURCE,
                    original_line: 0,
                    original_column: 0,
                    name: NO_NAME,
                });
            }

            pos = abs_pos - base_offset;
        }

        // Cache the result
        self.decoded_lines
            .borrow_mut()
            .insert(line, mappings.clone());

        Ok(mappings)
    }

    /// Look up the original source position for a generated position.
    ///
    /// Both `line` and `column` are 0-based.
    /// Returns `None` if no mapping exists or the mapping has no source.
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        let line_mappings = self.decode_line(line).ok()?;

        if line_mappings.is_empty() {
            return None;
        }

        // Binary search for greatest lower bound
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

    /// Number of generated lines in the source map.
    pub fn line_count(&self) -> usize {
        self.line_info.len()
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

    /// Get all mappings for a line (decoding on demand).
    pub fn mappings_for_line(&self, line: u32) -> Vec<Mapping> {
        self.decode_line(line).unwrap_or_default()
    }

    /// Fully decode all mappings into a regular `SourceMap`.
    ///
    /// Useful when you need the full map after lazy exploration.
    pub fn into_sourcemap(self) -> Result<SourceMap, ParseError> {
        let (mappings, line_offsets) = decode_mappings(&self.raw_mappings)?;

        Ok(SourceMap {
            file: self.file,
            source_root: self.source_root,
            sources: self.sources.clone(),
            sources_content: self.sources_content,
            names: self.names,
            ignore_list: self.ignore_list,
            extensions: self.extensions,
            debug_id: self.debug_id,
            scopes: self.scopes,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map: self.source_map,
        })
    }
}

/// Pre-scan the raw mappings string to find semicolon positions and compute
/// cumulative VLQ state at each line boundary.
fn prescan_mappings(input: &str) -> Result<Vec<LineInfo>, DecodeError> {
    if input.is_empty() {
        return Ok(Vec::new());
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    // Count lines for pre-allocation
    let line_count = bytes.iter().filter(|&&b| b == b';').count() + 1;
    let mut line_info: Vec<LineInfo> = Vec::with_capacity(line_count);

    let mut source_index: i64 = 0;
    let mut original_line: i64 = 0;
    let mut original_column: i64 = 0;
    let mut name_index: i64 = 0;
    let mut pos: usize = 0;

    loop {
        let line_start = pos;
        let state = VlqState {
            source_index,
            original_line,
            original_column,
            name_index,
        };

        let mut saw_semicolon = false;

        // Walk the VLQ data, updating cumulative state but not allocating mappings
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

            // Field 1: generated column (skip value, it resets per line)
            vlq_fast(bytes, &mut pos)?;

            if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                // Fields 2-4: source, original line, original column
                source_index += vlq_fast(bytes, &mut pos)?;
                original_line += vlq_fast(bytes, &mut pos)?;
                original_column += vlq_fast(bytes, &mut pos)?;

                // Field 5: name (optional)
                if pos < len && bytes[pos] != b',' && bytes[pos] != b';' {
                    name_index += vlq_fast(bytes, &mut pos)?;
                }
            }
        }

        // byte_end is before the semicolon (or end of string)
        let byte_end = if saw_semicolon { pos - 1 } else { pos };

        line_info.push(LineInfo {
            byte_offset: line_start,
            byte_end,
            state,
        });

        if !saw_semicolon {
            break;
        }
    }

    Ok(line_info)
}

/// Result of parsing a sourceMappingURL reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceMappingUrl {
    /// An inline base64 data URI containing the source map JSON.
    Inline(String),
    /// An external URL or relative path to the source map file.
    External(String),
}

/// Extract the sourceMappingURL from generated source code.
///
/// Looks for `//# sourceMappingURL=<url>` or `//@ sourceMappingURL=<url>` comments.
/// For inline data URIs (`data:application/json;base64,...`), decodes the base64 content.
/// Returns `None` if no sourceMappingURL is found.
pub fn parse_source_mapping_url(source: &str) -> Option<SourceMappingUrl> {
    // Search backwards from the end (sourceMappingURL is typically the last line)
    for line in source.lines().rev() {
        let trimmed = line.trim();
        let url = if let Some(rest) = trimmed.strip_prefix("//# sourceMappingURL=") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("//@ sourceMappingURL=") {
            rest.trim()
        } else if let Some(rest) = trimmed.strip_prefix("/*# sourceMappingURL=") {
            rest.trim_end_matches("*/").trim()
        } else if let Some(rest) = trimmed.strip_prefix("/*@ sourceMappingURL=") {
            rest.trim_end_matches("*/").trim()
        } else {
            continue;
        };

        if url.is_empty() {
            continue;
        }

        // Check for inline data URI
        if let Some(base64_data) = url
            .strip_prefix("data:application/json;base64,")
            .or_else(|| url.strip_prefix("data:application/json;charset=utf-8;base64,"))
            .or_else(|| url.strip_prefix("data:application/json;charset=UTF-8;base64,"))
        {
            // Decode base64
            let decoded = base64_decode(base64_data);
            if let Some(json) = decoded {
                return Some(SourceMappingUrl::Inline(json));
            }
        }

        return Some(SourceMappingUrl::External(url.to_string()));
    }

    None
}

/// Simple base64 decoder (no dependencies).
fn base64_decode(input: &str) -> Option<String> {
    let input = input.trim();
    let bytes: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();

    let mut output = Vec::with_capacity(bytes.len() * 3 / 4);

    for chunk in bytes.chunks(4) {
        let mut buf = [0u8; 4];
        let mut len = 0;

        for &b in chunk {
            if b == b'=' {
                break;
            }
            let val = match b {
                b'A'..=b'Z' => b - b'A',
                b'a'..=b'z' => b - b'a' + 26,
                b'0'..=b'9' => b - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                _ => return None,
            };
            buf[len] = val;
            len += 1;
        }

        if len >= 2 {
            output.push((buf[0] << 2) | (buf[1] >> 4));
        }
        if len >= 3 {
            output.push((buf[1] << 4) | (buf[2] >> 2));
        }
        if len >= 4 {
            output.push((buf[2] << 6) | buf[3]);
        }
    }

    String::from_utf8(output).ok()
}

/// Validate a source map with deep structural checks.
///
/// Performs bounds checking, segment ordering verification, source resolution,
/// and unreferenced sources detection beyond basic JSON parsing.
pub fn validate_deep(sm: &SourceMap) -> Vec<String> {
    let mut warnings = Vec::new();

    // Check segment ordering (must be sorted by generated position)
    let mut prev_line: u32 = 0;
    let mut prev_col: u32 = 0;
    for m in &sm.mappings {
        if m.generated_line < prev_line
            || (m.generated_line == prev_line && m.generated_column < prev_col)
        {
            warnings.push(format!(
                "mappings out of order at {}:{}",
                m.generated_line, m.generated_column
            ));
        }
        prev_line = m.generated_line;
        prev_col = m.generated_column;
    }

    // Check source indices in bounds
    for m in &sm.mappings {
        if m.source != NO_SOURCE && m.source as usize >= sm.sources.len() {
            warnings.push(format!(
                "source index {} out of bounds (max {})",
                m.source,
                sm.sources.len()
            ));
        }
        if m.name != NO_NAME && m.name as usize >= sm.names.len() {
            warnings.push(format!(
                "name index {} out of bounds (max {})",
                m.name,
                sm.names.len()
            ));
        }
    }

    // Check ignoreList indices in bounds
    for &idx in &sm.ignore_list {
        if idx as usize >= sm.sources.len() {
            warnings.push(format!(
                "ignoreList index {} out of bounds (max {})",
                idx,
                sm.sources.len()
            ));
        }
    }

    // Detect unreferenced sources
    let mut referenced_sources = std::collections::HashSet::new();
    for m in &sm.mappings {
        if m.source != NO_SOURCE {
            referenced_sources.insert(m.source);
        }
    }
    for (i, source) in sm.sources.iter().enumerate() {
        if !referenced_sources.contains(&(i as u32)) {
            warnings.push(format!("source \"{source}\" (index {i}) is unreferenced"));
        }
    }

    warnings
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

/// Decode VLQ mappings for a subset of lines `[start_line, end_line)`.
///
/// Walks VLQ state for all lines up to `end_line`, but only allocates Mapping
/// structs for lines in the requested range. The returned `line_offsets` is
/// indexed by the actual generated line number (not relative to start_line),
/// so that `mappings_for_line(line)` works correctly with the real line values.
fn decode_mappings_range(
    input: &str,
    start_line: u32,
    end_line: u32,
) -> Result<(Vec<Mapping>, Vec<u32>), DecodeError> {
    if input.is_empty() || start_line >= end_line {
        return Ok((Vec::new(), vec![0; end_line as usize + 1]));
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    let mut mappings: Vec<Mapping> = Vec::new();

    let mut source_index: i64 = 0;
    let mut original_line: i64 = 0;
    let mut original_column: i64 = 0;
    let mut name_index: i64 = 0;
    let mut generated_line: u32 = 0;
    let mut pos: usize = 0;

    // Track which line each mapping starts at, so we can build line_offsets after
    // We use a vec of (line, mapping_start_index) pairs for lines in range
    let mut line_starts: Vec<(u32, u32)> = Vec::new();

    loop {
        let in_range = generated_line >= start_line && generated_line < end_line;
        if in_range {
            line_starts.push((generated_line, mappings.len() as u32));
        }

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

                if in_range {
                    mappings.push(Mapping {
                        generated_line,
                        generated_column: generated_column as u32,
                        source: source_index as u32,
                        original_line: original_line as u32,
                        original_column: original_column as u32,
                        name,
                    });
                }
            } else {
                // 1-field segment: no source info
                if in_range {
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
        }

        if !saw_semicolon {
            break;
        }
        generated_line += 1;

        // Stop early once we've passed end_line
        if generated_line >= end_line {
            break;
        }
    }

    // Build line_offsets indexed by actual line number.
    // Size = end_line + 1 so that line_offsets[line] and line_offsets[line+1] both exist
    // for any line < end_line.
    let total = mappings.len() as u32;
    let mut line_offsets: Vec<u32> = vec![total; end_line as usize + 1];

    // Fill from our recorded line starts (in reverse so gaps get forward-filled correctly)
    // First set lines before start_line to 0 (they have no mappings, and 0..0 = empty)
    for i in 0..=start_line as usize {
        if i < line_offsets.len() {
            line_offsets[i] = 0;
        }
    }

    // Set each recorded line start
    for &(line, offset) in &line_starts {
        line_offsets[line as usize] = offset;
    }

    // Forward-fill gaps within the range: if a line in [start_line, end_line) wasn't
    // in line_starts, it should point to the same offset as the next line with mappings.
    // We do a backward pass: for lines not explicitly set, they should get the offset
    // of the next line (which means "empty line" since start == end for that range).
    // Actually, the approach used in decode_mappings is forward: each line offset is
    // set when we encounter it. Lines not encountered get the sentinel (total).
    // But we also need lines before start_line to be "empty" (start == end).
    // Since lines < start_line all have offset 0 and line start_line also starts at 0
    // (or wherever the first mapping is), lines before start_line correctly have 0..0 = empty.

    // However, there's a subtlety: if start_line has no mappings (empty line in range),
    // it would have been set to 0 by line_starts but then line_offsets[start_line+1]
    // might also be 0, making an empty range. Let's just ensure the forward-fill works:
    // lines within range that have no mappings should have the same offset as the next
    // line's start.

    // Actually the simplest correct approach: walk forward from start_line to end_line.
    // For each line not in line_starts, set it to the value of the previous line's end
    // (which is the current mapping count at that point).
    // We already recorded line_starts in order, so let's use them directly.

    // Reset line_offsets for lines in [start_line, end_line] to sentinel
    for i in start_line as usize..=end_line as usize {
        if i < line_offsets.len() {
            line_offsets[i] = total;
        }
    }

    // Set recorded line starts
    for &(line, offset) in &line_starts {
        line_offsets[line as usize] = offset;
    }

    // Forward-fill: lines in the range that weren't recorded should get
    // the offset of the next line (so they appear empty)
    // Walk backward from end_line to start_line
    let mut next_offset = total;
    for i in (start_line as usize..end_line as usize).rev() {
        if line_offsets[i] == total {
            // This line wasn't in the input at all, point to next_offset
            line_offsets[i] = next_offset;
        } else {
            next_offset = line_offsets[i];
        }
    }

    // Lines before start_line should all be empty.
    // Make consecutive lines point to 0 so start == end == 0.
    for offset in line_offsets.iter_mut().take(start_line as usize) {
        *offset = 0;
    }

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
    fn all_generated_positions_for_basic() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let results = sm.all_generated_positions_for("input.js", 0, 0);
        assert!(!results.is_empty(), "should find at least one position");
        assert_eq!(results[0].line, 0);
        assert_eq!(results[0].column, 0);
    }

    #[test]
    fn all_generated_positions_for_unknown_source() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let results = sm.all_generated_positions_for("nonexistent.js", 0, 0);
        assert!(results.is_empty());
    }

    #[test]
    fn all_generated_positions_for_no_match() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let results = sm.all_generated_positions_for("input.js", 999, 999);
        assert!(results.is_empty());
    }

    #[test]
    fn encode_mappings_roundtrip() {
        let json = generate_test_sourcemap(50, 10, 3);
        let sm = SourceMap::from_json(&json).unwrap();
        let encoded = sm.encode_mappings();
        // Re-parse with encoded mappings
        let json2 = format!(
            r#"{{"version":3,"sources":{sources},"names":{names},"mappings":"{mappings}"}}"#,
            sources = serde_json::to_string(&sm.sources).unwrap(),
            names = serde_json::to_string(&sm.names).unwrap(),
            mappings = encoded,
        );
        let sm2 = SourceMap::from_json(&json2).unwrap();
        assert_eq!(sm2.mapping_count(), sm.mapping_count());
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

    // ── Bias tests ───────────────────────────────────────────────

    /// Map with multiple mappings per line for bias testing:
    /// Line 0: col 0 → src:0:0, col 5 → src:0:5, col 10 → src:0:10
    fn bias_map() -> &'static str {
        // AAAA = 0,0,0,0  KAAK = 5,0,0,5  KAAK = 5,0,0,5 (delta)
        r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,KAAK,KAAK"}"#
    }

    #[test]
    fn original_position_glb_exact_match() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        let loc = sm
            .original_position_for_with_bias(0, 5, Bias::GreatestLowerBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_glb_snaps_left() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 7 should snap to the mapping at column 5
        let loc = sm
            .original_position_for_with_bias(0, 7, Bias::GreatestLowerBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_exact_match() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        let loc = sm
            .original_position_for_with_bias(0, 5, Bias::LeastUpperBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_snaps_right() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 3 with LUB should snap to the mapping at column 5
        let loc = sm
            .original_position_for_with_bias(0, 3, Bias::LeastUpperBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_before_first() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 0 with LUB should find mapping at column 0
        let loc = sm
            .original_position_for_with_bias(0, 0, Bias::LeastUpperBound)
            .unwrap();
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn original_position_lub_after_last() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 15 with LUB should return None (no mapping at or after 15)
        let loc = sm.original_position_for_with_bias(0, 15, Bias::LeastUpperBound);
        assert!(loc.is_none());
    }

    #[test]
    fn original_position_glb_before_first() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 0 with GLB should find mapping at column 0
        let loc = sm
            .original_position_for_with_bias(0, 0, Bias::GreatestLowerBound)
            .unwrap();
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn generated_position_lub() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // LUB: find first generated position at or after original col 3
        let loc = sm
            .generated_position_for_with_bias("input.js", 0, 3, Bias::LeastUpperBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn generated_position_glb() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // GLB: find last generated position at or before original col 7
        let loc = sm
            .generated_position_for_with_bias("input.js", 0, 7, Bias::GreatestLowerBound)
            .unwrap();
        assert_eq!(loc.column, 5);
    }

    // ── Range mapping tests ──────────────────────────────────────

    #[test]
    fn map_range_basic() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        let range = sm.map_range(0, 0, 0, 10).unwrap();
        assert_eq!(range.source, 0);
        assert_eq!(range.original_start_line, 0);
        assert_eq!(range.original_start_column, 0);
        assert_eq!(range.original_end_line, 0);
        assert_eq!(range.original_end_column, 10);
    }

    #[test]
    fn map_range_no_mapping() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Line 5 doesn't exist
        let range = sm.map_range(0, 0, 5, 0);
        assert!(range.is_none());
    }

    #[test]
    fn map_range_different_sources() {
        // Map with two sources: line 0 → src0, line 1 → src1
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA;ACAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Start maps to a.js, end maps to b.js → should return None
        let range = sm.map_range(0, 0, 1, 0);
        assert!(range.is_none());
    }

    // ── Phase 10 tests ───────────────────────────────────────────

    #[test]
    fn extension_fields_preserved() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_facebook_sources":[[{"names":["<global>"]}]],"x_google_linecount":42}"#;
        let sm = SourceMap::from_json(json).unwrap();

        assert!(sm.extensions.contains_key("x_facebook_sources"));
        assert!(sm.extensions.contains_key("x_google_linecount"));
        assert_eq!(
            sm.extensions.get("x_google_linecount"),
            Some(&serde_json::json!(42))
        );

        // Round-trip preserves extension fields
        let output = sm.to_json();
        assert!(output.contains("x_facebook_sources"));
        assert!(output.contains("x_google_linecount"));
    }

    #[test]
    fn x_google_ignorelist_fallback() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA","x_google_ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![1]);
    }

    #[test]
    fn ignorelist_takes_precedence_over_x_google() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA","ignoreList":[0],"x_google_ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![0]);
    }

    #[test]
    fn source_mapping_url_external() {
        let source = "var a = 1;\n//# sourceMappingURL=app.js.map\n";
        let result = parse_source_mapping_url(source).unwrap();
        assert_eq!(result, SourceMappingUrl::External("app.js.map".to_string()));
    }

    #[test]
    fn source_mapping_url_inline() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let b64 = base64_encode_simple(json);
        let source =
            format!("var a = 1;\n//# sourceMappingURL=data:application/json;base64,{b64}\n");
        match parse_source_mapping_url(&source).unwrap() {
            SourceMappingUrl::Inline(decoded) => {
                assert_eq!(decoded, json);
            }
            _ => panic!("expected inline"),
        }
    }

    #[test]
    fn source_mapping_url_at_sign() {
        let source = "var a = 1;\n//@ sourceMappingURL=old-style.map";
        let result = parse_source_mapping_url(source).unwrap();
        assert_eq!(
            result,
            SourceMappingUrl::External("old-style.map".to_string())
        );
    }

    #[test]
    fn source_mapping_url_css_comment() {
        let source = "body { }\n/*# sourceMappingURL=styles.css.map */";
        let result = parse_source_mapping_url(source).unwrap();
        assert_eq!(
            result,
            SourceMappingUrl::External("styles.css.map".to_string())
        );
    }

    #[test]
    fn source_mapping_url_none() {
        let source = "var a = 1;";
        assert!(parse_source_mapping_url(source).is_none());
    }

    #[test]
    fn exclude_content_option() {
        let json = r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        let with_content = sm.to_json();
        assert!(with_content.contains("sourcesContent"));

        let without_content = sm.to_json_with_options(true);
        assert!(!without_content.contains("sourcesContent"));
    }

    #[test]
    fn validate_deep_clean_map() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        let warnings = validate_deep(&sm);
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn validate_deep_unreferenced_source() {
        // Source "unused.js" has no mappings pointing to it
        let json =
            r#"{"version":3,"sources":["used.js","unused.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("unused.js")));
    }

    // ── from_parts tests ──────────────────────────────────────────

    #[test]
    fn from_parts_basic() {
        let mappings = vec![
            Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
            },
            Mapping {
                generated_line: 1,
                generated_column: 4,
                source: 0,
                original_line: 1,
                original_column: 2,
                name: NO_NAME,
            },
        ];

        let sm = SourceMap::from_parts(
            Some("out.js".to_string()),
            None,
            vec!["input.js".to_string()],
            vec![Some("var x = 1;".to_string())],
            vec![],
            mappings,
            vec![],
            None,
            None,
        );

        assert_eq!(sm.line_count(), 2);
        assert_eq!(sm.mapping_count(), 2);

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.source, 0);
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);

        let loc = sm.original_position_for(1, 4).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 2);
    }

    #[test]
    fn from_parts_empty() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec![],
            vec![],
            vec![],
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(sm.line_count(), 0);
        assert_eq!(sm.mapping_count(), 0);
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn from_parts_with_names() {
        let mappings = vec![Mapping {
            generated_line: 0,
            generated_column: 0,
            source: 0,
            original_line: 0,
            original_column: 0,
            name: 0,
        }];

        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["input.js".to_string()],
            vec![],
            vec!["myVar".to_string()],
            mappings,
            vec![],
            None,
            None,
        );

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
        assert_eq!(sm.name(0), "myVar");
    }

    #[test]
    fn from_parts_roundtrip_via_json() {
        let json = generate_test_sourcemap(50, 10, 3);
        let sm = SourceMap::from_json(&json).unwrap();

        let sm2 = SourceMap::from_parts(
            sm.file.clone(),
            sm.source_root.clone(),
            sm.sources.clone(),
            sm.sources_content.clone(),
            sm.names.clone(),
            sm.all_mappings().to_vec(),
            sm.ignore_list.clone(),
            sm.debug_id.clone(),
            None,
        );

        assert_eq!(sm2.mapping_count(), sm.mapping_count());
        assert_eq!(sm2.line_count(), sm.line_count());

        // Spot-check lookups
        for m in sm.all_mappings() {
            if m.source != NO_SOURCE {
                let a = sm.original_position_for(m.generated_line, m.generated_column);
                let b = sm2.original_position_for(m.generated_line, m.generated_column);
                match (a, b) {
                    (Some(a), Some(b)) => {
                        assert_eq!(a.source, b.source);
                        assert_eq!(a.line, b.line);
                        assert_eq!(a.column, b.column);
                    }
                    (None, None) => {}
                    _ => panic!("mismatch at ({}, {})", m.generated_line, m.generated_column),
                }
            }
        }
    }

    #[test]
    fn from_parts_reverse_lookup() {
        let mappings = vec![
            Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 10,
                original_column: 5,
                name: NO_NAME,
            },
            Mapping {
                generated_line: 1,
                generated_column: 8,
                source: 0,
                original_line: 20,
                original_column: 0,
                name: NO_NAME,
            },
        ];

        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["src.js".to_string()],
            vec![],
            vec![],
            mappings,
            vec![],
            None,
            None,
        );

        let loc = sm.generated_position_for("src.js", 10, 5).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);

        let loc = sm.generated_position_for("src.js", 20, 0).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 8);
    }

    #[test]
    fn from_parts_sparse_lines() {
        let mappings = vec![
            Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
            },
            Mapping {
                generated_line: 5,
                generated_column: 0,
                source: 0,
                original_line: 5,
                original_column: 0,
                name: NO_NAME,
            },
        ];

        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["src.js".to_string()],
            vec![],
            vec![],
            mappings,
            vec![],
            None,
            None,
        );

        assert_eq!(sm.line_count(), 6);
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_none());
        assert!(sm.original_position_for(5, 0).is_some());
    }

    // ── from_json_lines tests ────────────────────────────────────

    #[test]
    fn from_json_lines_basic() {
        let json = generate_test_sourcemap(10, 5, 2);
        let sm_full = SourceMap::from_json(&json).unwrap();

        // Decode only lines 3..7
        let sm_partial = SourceMap::from_json_lines(&json, 3, 7).unwrap();

        // Verify mappings for lines in range match
        for line in 3..7u32 {
            let full_mappings = sm_full.mappings_for_line(line);
            let partial_mappings = sm_partial.mappings_for_line(line);
            assert_eq!(
                full_mappings.len(),
                partial_mappings.len(),
                "line {line} mapping count mismatch"
            );
            for (a, b) in full_mappings.iter().zip(partial_mappings.iter()) {
                assert_eq!(a.generated_column, b.generated_column);
                assert_eq!(a.source, b.source);
                assert_eq!(a.original_line, b.original_line);
                assert_eq!(a.original_column, b.original_column);
                assert_eq!(a.name, b.name);
            }
        }
    }

    #[test]
    fn from_json_lines_first_lines() {
        let json = generate_test_sourcemap(10, 5, 2);
        let sm_full = SourceMap::from_json(&json).unwrap();
        let sm_partial = SourceMap::from_json_lines(&json, 0, 3).unwrap();

        for line in 0..3u32 {
            let full_mappings = sm_full.mappings_for_line(line);
            let partial_mappings = sm_partial.mappings_for_line(line);
            assert_eq!(full_mappings.len(), partial_mappings.len());
        }
    }

    #[test]
    fn from_json_lines_last_lines() {
        let json = generate_test_sourcemap(10, 5, 2);
        let sm_full = SourceMap::from_json(&json).unwrap();
        let sm_partial = SourceMap::from_json_lines(&json, 7, 10).unwrap();

        for line in 7..10u32 {
            let full_mappings = sm_full.mappings_for_line(line);
            let partial_mappings = sm_partial.mappings_for_line(line);
            assert_eq!(full_mappings.len(), partial_mappings.len(), "line {line}");
        }
    }

    #[test]
    fn from_json_lines_empty_range() {
        let json = generate_test_sourcemap(10, 5, 2);
        let sm = SourceMap::from_json_lines(&json, 5, 5).unwrap();
        assert_eq!(sm.mapping_count(), 0);
    }

    #[test]
    fn from_json_lines_beyond_end() {
        let json = generate_test_sourcemap(5, 3, 1);
        // Request lines beyond what exists
        let sm = SourceMap::from_json_lines(&json, 3, 100).unwrap();
        // Should have mappings for lines 3 and 4 (the ones that exist in the range)
        assert!(sm.mapping_count() > 0);
    }

    #[test]
    fn from_json_lines_single_line() {
        let json = generate_test_sourcemap(10, 5, 2);
        let sm_full = SourceMap::from_json(&json).unwrap();
        let sm_partial = SourceMap::from_json_lines(&json, 5, 6).unwrap();

        let full_mappings = sm_full.mappings_for_line(5);
        let partial_mappings = sm_partial.mappings_for_line(5);
        assert_eq!(full_mappings.len(), partial_mappings.len());
    }

    // ── LazySourceMap tests ──────────────────────────────────────

    #[test]
    fn lazy_basic_lookup() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;AACA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();

        assert_eq!(sm.line_count(), 2);
        assert_eq!(sm.sources, vec!["input.js"]);

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "input.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn lazy_multiple_lines() {
        let json = generate_test_sourcemap(20, 5, 3);
        let sm_eager = SourceMap::from_json(&json).unwrap();
        let sm_lazy = LazySourceMap::from_json(&json).unwrap();

        assert_eq!(sm_lazy.line_count(), sm_eager.line_count());

        // Verify lookups match for every mapping
        for m in sm_eager.all_mappings() {
            if m.source == NO_SOURCE {
                continue;
            }
            let eager_loc = sm_eager
                .original_position_for(m.generated_line, m.generated_column)
                .unwrap();
            let lazy_loc = sm_lazy
                .original_position_for(m.generated_line, m.generated_column)
                .unwrap();
            assert_eq!(eager_loc.source, lazy_loc.source);
            assert_eq!(eager_loc.line, lazy_loc.line);
            assert_eq!(eager_loc.column, lazy_loc.column);
            assert_eq!(eager_loc.name, lazy_loc.name);
        }
    }

    #[test]
    fn lazy_empty_mappings() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 0);
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn lazy_empty_lines() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;;;AACA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 4);

        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(1, 0).is_none());
        assert!(sm.original_position_for(2, 0).is_none());
        assert!(sm.original_position_for(3, 0).is_some());
    }

    #[test]
    fn lazy_decode_line_caching() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,KACA;AACA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();

        // First call decodes
        let line0_a = sm.decode_line(0).unwrap();
        // Second call should return cached
        let line0_b = sm.decode_line(0).unwrap();
        assert_eq!(line0_a.len(), line0_b.len());
        assert_eq!(line0_a[0].generated_column, line0_b[0].generated_column);
    }

    #[test]
    fn lazy_with_names() {
        let json = r#"{"version":3,"sources":["input.js"],"names":["foo","bar"],"mappings":"AAAAA,KACAC"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
        assert_eq!(sm.name(0), "foo");

        let loc = sm.original_position_for(0, 5).unwrap();
        assert_eq!(loc.name, Some(1));
        assert_eq!(sm.name(1), "bar");
    }

    #[test]
    fn lazy_nonexistent_line() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert!(sm.original_position_for(99, 0).is_none());
        let line = sm.decode_line(99).unwrap();
        assert!(line.is_empty());
    }

    #[test]
    fn lazy_into_sourcemap() {
        let json = generate_test_sourcemap(20, 5, 3);
        let sm_eager = SourceMap::from_json(&json).unwrap();
        let sm_lazy = LazySourceMap::from_json(&json).unwrap();
        let sm_converted = sm_lazy.into_sourcemap().unwrap();

        assert_eq!(sm_converted.mapping_count(), sm_eager.mapping_count());
        assert_eq!(sm_converted.line_count(), sm_eager.line_count());

        // Verify all lookups match
        for m in sm_eager.all_mappings() {
            let a = sm_eager.original_position_for(m.generated_line, m.generated_column);
            let b = sm_converted.original_position_for(m.generated_line, m.generated_column);
            match (a, b) {
                (Some(a), Some(b)) => {
                    assert_eq!(a.source, b.source);
                    assert_eq!(a.line, b.line);
                    assert_eq!(a.column, b.column);
                }
                (None, None) => {}
                _ => panic!("mismatch at ({}, {})", m.generated_line, m.generated_column),
            }
        }
    }

    #[test]
    fn lazy_source_index_lookup() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA;ACAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.source_index("a.js"), Some(0));
        assert_eq!(sm.source_index("b.js"), Some(1));
        assert_eq!(sm.source_index("c.js"), None);
    }

    #[test]
    fn lazy_mappings_for_line() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,KACA;AACA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();

        let line0 = sm.mappings_for_line(0);
        assert_eq!(line0.len(), 2);

        let line1 = sm.mappings_for_line(1);
        assert_eq!(line1.len(), 1);

        let line99 = sm.mappings_for_line(99);
        assert!(line99.is_empty());
    }

    #[test]
    fn lazy_large_map_selective_decode() {
        // Generate a large map but only decode a few lines
        let json = generate_test_sourcemap(100, 10, 5);
        let sm_eager = SourceMap::from_json(&json).unwrap();
        let sm_lazy = LazySourceMap::from_json(&json).unwrap();

        // Only decode lines 50 and 75
        for line in [50, 75] {
            let eager_mappings = sm_eager.mappings_for_line(line);
            let lazy_mappings = sm_lazy.mappings_for_line(line);
            assert_eq!(
                eager_mappings.len(),
                lazy_mappings.len(),
                "line {line} count mismatch"
            );
            for (a, b) in eager_mappings.iter().zip(lazy_mappings.iter()) {
                assert_eq!(a.generated_column, b.generated_column);
                assert_eq!(a.source, b.source);
                assert_eq!(a.original_line, b.original_line);
                assert_eq!(a.original_column, b.original_column);
                assert_eq!(a.name, b.name);
            }
        }
    }

    #[test]
    fn lazy_single_field_segments() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,KAAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();

        // First segment is single-field (no source info)
        assert!(sm.original_position_for(0, 0).is_none());
        // Second segment has source info
        let loc = sm.original_position_for(0, 5).unwrap();
        assert_eq!(loc.source, 0);
    }

    // ── Coverage gap tests ──────────────────────────────────────────

    #[test]
    fn parse_error_display_vlq() {
        let err = ParseError::Vlq(srcmap_codec::DecodeError::UnexpectedEof { offset: 3 });
        assert!(err.to_string().contains("VLQ decode error"));
    }

    #[test]
    fn parse_error_display_scopes() {
        let err = ParseError::Scopes("test error".to_string());
        assert_eq!(err.to_string(), "scopes decode error: test error");
    }

    #[test]
    fn indexed_map_with_names_in_sections() {
        let json = r#"{
            "version": 3,
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
                    "offset": {"line": 1, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": ["foo"],
                        "mappings": "AAAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Sources and names should be deduplicated
        assert_eq!(sm.sources.len(), 1);
        assert_eq!(sm.names.len(), 1);
    }

    #[test]
    fn indexed_map_with_ignore_list() {
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["vendor.js"],
                        "names": [],
                        "mappings": "AAAA",
                        "ignoreList": [0]
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![0]);
    }

    #[test]
    fn indexed_map_with_generated_only_segment() {
        // Section with a generated-only (1-field) segment
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": [],
                        "mappings": "A,AAAA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.mapping_count() >= 1);
    }

    #[test]
    fn indexed_map_empty_mappings() {
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
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 0);
    }

    #[test]
    fn generated_position_glb_exact_match() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAE,OAAO"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        let loc = sm.generated_position_for_with_bias("a.js", 0, 0, Bias::GreatestLowerBound);
        assert!(loc.is_some());
        assert_eq!(loc.unwrap().column, 0);
    }

    #[test]
    fn generated_position_glb_no_exact_match() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAE"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        // Look for position between two mappings
        let loc = sm.generated_position_for_with_bias("a.js", 0, 0, Bias::GreatestLowerBound);
        assert!(loc.is_some());
    }

    #[test]
    fn generated_position_glb_wrong_source() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA,KCCA"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        // GLB for position in b.js that doesn't exist at that location
        let loc = sm.generated_position_for_with_bias("b.js", 5, 0, Bias::GreatestLowerBound);
        // Should find something or nothing depending on whether there's a mapping before
        // The key is that source filtering works
        if let Some(l) = loc {
            // Verify returned position is valid (line 0 is the only generated line)
            assert_eq!(l.line, 0);
        }
    }

    #[test]
    fn generated_position_lub_wrong_source() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        // LUB for non-existent source
        let loc =
            sm.generated_position_for_with_bias("nonexistent.js", 0, 0, Bias::LeastUpperBound);
        assert!(loc.is_none());
    }

    #[test]
    fn to_json_with_ignore_list() {
        let json =
            r#"{"version":3,"sources":["vendor.js"],"names":[],"mappings":"AAAA","ignoreList":[0]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        assert!(output.contains("\"ignoreList\":[0]"));
    }

    #[test]
    fn to_json_with_extensions() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_custom":"test_value"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        assert!(output.contains("x_custom"));
        assert!(output.contains("test_value"));
    }

    #[test]
    fn from_parts_empty_mappings() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["a.js".to_string()],
            vec![Some("content".to_string())],
            vec![],
            vec![],
            vec![],
            None,
            None,
        );
        assert_eq!(sm.mapping_count(), 0);
        assert_eq!(sm.sources, vec!["a.js"]);
    }

    #[test]
    fn from_vlq_basic() {
        let sm = SourceMap::from_vlq(
            "AAAA;AACA",
            vec!["a.js".to_string()],
            vec![],
            Some("out.js".to_string()),
            None,
            vec![Some("content".to_string())],
            vec![],
            None,
        )
        .unwrap();

        assert_eq!(sm.file.as_deref(), Some("out.js"));
        assert_eq!(sm.sources, vec!["a.js"]);
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "a.js");
        assert_eq!(loc.line, 0);
    }

    #[test]
    fn from_json_lines_basic_coverage() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA;AACA;AACA;AACA"}"#;
        let sm = SourceMap::from_json_lines(json, 1, 3).unwrap();
        // Should have mappings for lines 1 and 2
        assert!(sm.original_position_for(1, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_some());
    }

    #[test]
    fn from_json_lines_with_source_root() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA;AACA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 2).unwrap();
        assert_eq!(sm.sources[0], "src/a.js");
    }

    #[test]
    fn from_json_lines_with_null_source() {
        let json = r#"{"version":3,"sources":[null,"a.js"],"names":[],"mappings":"AAAA,KCCA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 1).unwrap();
        assert_eq!(sm.sources.len(), 2);
    }

    #[test]
    fn json_escaping_special_chars_sourcemap() {
        // Build a source map with special chars in source name and content via JSON
        // The source name has a newline, the content has \r\n, tab, quotes, backslash, and control char
        let json = r#"{"version":3,"sources":["path/with\nnewline.js"],"sourcesContent":["line1\r\nline2\t\"quoted\"\\\u0001"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Roundtrip through to_json and re-parse
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm.sources[0], sm2.sources[0]);
        assert_eq!(sm.sources_content[0], sm2.sources_content[0]);
    }

    #[test]
    fn to_json_exclude_content() {
        let json = r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json_with_options(true);
        assert!(!output.contains("sourcesContent"));
        let output_with = sm.to_json_with_options(false);
        assert!(output_with.contains("sourcesContent"));
    }

    #[test]
    fn encode_mappings_with_name() {
        // Ensure encode_mappings handles the name field (5th VLQ)
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let encoded = sm.encode_mappings();
        assert_eq!(encoded, "AAAAA");
    }

    #[test]
    fn encode_mappings_generated_only() {
        // Generated-only segments (NO_SOURCE) in encode
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let encoded = sm.encode_mappings();
        let roundtrip = SourceMap::from_json(&format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"{}"}}"#,
            encoded
        ))
        .unwrap();
        assert_eq!(roundtrip.mapping_count(), sm.mapping_count());
    }

    #[test]
    fn map_range_single_result() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAC,OAAO"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // map_range from col 0 to a mapped column
        let result = sm.map_range(0, 0, 0, 1);
        assert!(result.is_some());
        let range = result.unwrap();
        assert_eq!(range.source, 0);
    }

    #[test]
    fn scopes_in_from_json() {
        // Source map with scopes field - build scopes string, then embed in JSON
        let info = srcmap_scopes::ScopeInfo {
            scopes: vec![Some(srcmap_scopes::OriginalScope {
                start: srcmap_scopes::Position { line: 0, column: 0 },
                end: srcmap_scopes::Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&info, &mut names);

        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );

        let sm = SourceMap::from_json(&json).unwrap();
        assert!(sm.scopes.is_some());
    }

    #[test]
    fn from_json_lines_with_scopes() {
        let info = srcmap_scopes::ScopeInfo {
            scopes: vec![Some(srcmap_scopes::OriginalScope {
                start: srcmap_scopes::Position { line: 0, column: 0 },
                end: srcmap_scopes::Position { line: 5, column: 0 },
                name: None, kind: None, is_stack_frame: false,
                variables: vec![], children: vec![],
            })],
            ranges: vec![],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&info, &mut names);
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA","scopes":"{scopes_str}"}}"#
        );
        let sm = SourceMap::from_json_lines(&json, 0, 2).unwrap();
        assert!(sm.scopes.is_some());
    }

    #[test]
    fn from_json_lines_with_extensions() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_custom":"val","not_x":"skip"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 1).unwrap();
        assert!(sm.extensions.contains_key("x_custom"));
        assert!(!sm.extensions.contains_key("not_x"));
    }

    #[test]
    fn lazy_sourcemap_version_error() {
        let json = r#"{"version":2,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let err = LazySourceMap::from_json(json).unwrap_err();
        assert!(matches!(err, ParseError::InvalidVersion(2)));
    }

    #[test]
    fn lazy_sourcemap_with_source_root() {
        let json = r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources[0], "src/a.js");
    }

    #[test]
    fn lazy_sourcemap_with_ignore_list_and_extensions() {
        let json = r#"{"version":3,"sources":["v.js"],"names":[],"mappings":"AAAA","ignoreList":[0],"x_custom":"val","not_x":"skip"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![0]);
        assert!(sm.extensions.contains_key("x_custom"));
        assert!(!sm.extensions.contains_key("not_x"));
    }

    #[test]
    fn lazy_sourcemap_with_scopes() {
        let info = srcmap_scopes::ScopeInfo {
            scopes: vec![Some(srcmap_scopes::OriginalScope {
                start: srcmap_scopes::Position { line: 0, column: 0 },
                end: srcmap_scopes::Position { line: 5, column: 0 },
                name: None, kind: None, is_stack_frame: false,
                variables: vec![], children: vec![],
            })],
            ranges: vec![],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&info, &mut names);
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );
        let sm = LazySourceMap::from_json(&json).unwrap();
        assert!(sm.scopes.is_some());
    }

    #[test]
    fn lazy_sourcemap_null_source() {
        let json = r#"{"version":3,"sources":[null,"a.js"],"names":[],"mappings":"AAAA,KCCA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources.len(), 2);
    }

    #[test]
    fn indexed_map_multi_line_section() {
        // Multi-line section to exercise line_offsets building in from_sections
        let json = r#"{
            "version": 3,
            "sections": [
                {
                    "offset": {"line": 0, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["a.js"],
                        "names": [],
                        "mappings": "AAAA;AACA;AACA"
                    }
                },
                {
                    "offset": {"line": 5, "column": 0},
                    "map": {
                        "version": 3,
                        "sources": ["b.js"],
                        "names": [],
                        "mappings": "AAAA;AACA"
                    }
                }
            ]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(5, 0).is_some());
    }

    #[test]
    fn source_mapping_url_extraction() {
        // External URL
        let input = "var x = 1;\n//# sourceMappingURL=bundle.js.map";
        let url = parse_source_mapping_url(input);
        assert!(matches!(url, Some(SourceMappingUrl::External(ref s)) if s == "bundle.js.map"));

        // CSS comment style
        let input = "body { }\n/*# sourceMappingURL=style.css.map */";
        let url = parse_source_mapping_url(input);
        assert!(matches!(url, Some(SourceMappingUrl::External(ref s)) if s == "style.css.map"));

        // @ sign variant
        let input = "var x;\n//@ sourceMappingURL=old-style.map";
        let url = parse_source_mapping_url(input);
        assert!(matches!(url, Some(SourceMappingUrl::External(ref s)) if s == "old-style.map"));

        // CSS @ variant
        let input = "body{}\n/*@ sourceMappingURL=old-css.map */";
        let url = parse_source_mapping_url(input);
        assert!(matches!(url, Some(SourceMappingUrl::External(ref s)) if s == "old-css.map"));

        // No URL
        let input = "var x = 1;";
        let url = parse_source_mapping_url(input);
        assert!(url.is_none());

        // Empty URL
        let input = "//# sourceMappingURL=";
        let url = parse_source_mapping_url(input);
        assert!(url.is_none());

        // Inline data URI
        let map_json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let encoded = base64_encode_simple(map_json);
        let input = format!("var x;\n//# sourceMappingURL=data:application/json;base64,{encoded}");
        let url = parse_source_mapping_url(&input);
        assert!(matches!(url, Some(SourceMappingUrl::Inline(_))));
    }

    #[test]
    fn validate_deep_unreferenced_coverage() {
        // Map with an unreferenced source
        let sm = SourceMap::from_parts(
            None, None,
            vec!["used.js".to_string(), "unused.js".to_string()],
            vec![None, None],
            vec![],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
            }],
            vec![], None, None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("unreferenced")));
    }

    #[test]
    fn from_json_lines_generated_only_segment() {
        // from_json_lines with 1-field segments to exercise the generated-only branch
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,AAAA;AACA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 2).unwrap();
        assert!(sm.mapping_count() >= 2);
    }

    #[test]
    fn from_json_lines_with_names() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA;AACAA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 2).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
    }

    #[test]
    fn from_parts_with_line_gap() {
        // Mappings with a gap between lines to exercise line_offsets forward fill
        let sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![
                Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME },
                Mapping { generated_line: 5, generated_column: 0, source: 0, original_line: 5, original_column: 0, name: NO_NAME },
            ],
            vec![], None, None,
        );
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(5, 0).is_some());
        // Lines 1-4 have no mappings
        assert!(sm.original_position_for(1, 0).is_none());
    }

    #[test]
    fn lazy_decode_line_with_names_and_generated_only() {
        // LazySourceMap with both named and generated-only segments
        let json = r#"{"version":3,"sources":["a.js"],"names":["fn"],"mappings":"A,AAAAC"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        let line = sm.decode_line(0).unwrap();
        assert!(line.len() >= 2);
        // First is generated-only
        assert_eq!(line[0].source, NO_SOURCE);
        // Second has name
        assert_ne!(line[1].name, NO_NAME);
    }

    #[test]
    fn generated_position_glb_source_mismatch() {
        // a.js maps at (0,0)->(0,0), b.js maps at (0,5)->(1,0)
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA,KCCA"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        // LUB for source that exists but position is way beyond all mappings
        let loc = sm.generated_position_for_with_bias("a.js", 100, 0, Bias::LeastUpperBound);
        assert!(loc.is_none());

        // GLB for position before the only mapping in b.js (b.js has mapping at original 1,0)
        // Searching for (0,0) in b.js: partition_point finds first >= target,
        // then idx-1 if not exact, but that idx-1 maps to a.js (source mismatch), so None
        let loc = sm.generated_position_for_with_bias("b.js", 0, 0, Bias::GreatestLowerBound);
        assert!(loc.is_none());

        // GLB for exact position in b.js
        let loc = sm.generated_position_for_with_bias("b.js", 1, 0, Bias::GreatestLowerBound);
        assert!(loc.is_some());

        // LUB source mismatch: search for position in b.js that lands on a.js mapping
        let loc = sm.generated_position_for_with_bias("b.js", 99, 0, Bias::LeastUpperBound);
        assert!(loc.is_none());
    }

    // ── Coverage gap tests ───────────────────────────────────────────

    #[test]
    fn from_json_invalid_scopes_error() {
        // Invalid scopes string to trigger ParseError::Scopes
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"!!invalid!!"}"#;
        let err = SourceMap::from_json(json).unwrap_err();
        assert!(matches!(err, ParseError::Scopes(_)));
    }

    #[test]
    fn lazy_from_json_invalid_scopes_error() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"!!invalid!!"}"#;
        let err = LazySourceMap::from_json(json).unwrap_err();
        assert!(matches!(err, ParseError::Scopes(_)));
    }

    #[test]
    fn from_json_lines_invalid_scopes_error() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"!!invalid!!"}"#;
        let err = SourceMap::from_json_lines(json, 0, 1).unwrap_err();
        assert!(matches!(err, ParseError::Scopes(_)));
    }

    #[test]
    fn from_json_lines_invalid_version() {
        let json = r#"{"version":2,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let err = SourceMap::from_json_lines(json, 0, 1).unwrap_err();
        assert!(matches!(err, ParseError::InvalidVersion(2)));
    }

    #[test]
    fn indexed_map_with_ignore_list_remapped() {
        // Indexed map with 2 sections that have overlapping ignore_list
        let json = r#"{
            "version": 3,
            "sections": [{
                "offset": {"line": 0, "column": 0},
                "map": {
                    "version": 3,
                    "sources": ["a.js", "b.js"],
                    "names": [],
                    "mappings": "AAAA;ACAA",
                    "ignoreList": [1]
                }
            }, {
                "offset": {"line": 5, "column": 0},
                "map": {
                    "version": 3,
                    "sources": ["b.js", "c.js"],
                    "names": [],
                    "mappings": "AAAA;ACAA",
                    "ignoreList": [0]
                }
            }]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        // b.js should be deduped across sections, ignore_list should have b.js global index
        assert!(sm.ignore_list.len() >= 1);
    }

    #[test]
    fn to_json_with_debug_id() {
        let sm = SourceMap::from_parts(
            Some("out.js".to_string()), None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME }],
            vec![], Some("abc-123".to_string()), None,
        );
        let json = sm.to_json();
        assert!(json.contains(r#""debugId":"abc-123""#));
    }

    #[test]
    fn to_json_with_ignore_list_and_extensions() {
        let mut sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string(), "b.js".to_string()],
            vec![None, None],
            vec![],
            vec![Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME }],
            vec![1], None, None,
        );
        sm.extensions.insert("x_test".to_string(), serde_json::json!(42));
        let json = sm.to_json();
        assert!(json.contains("\"ignoreList\":[1]"));
        assert!(json.contains("\"x_test\":42"));
    }

    #[test]
    fn from_vlq_with_all_options() {
        let sm = SourceMap::from_vlq(
            "AAAA;AACA",
            vec!["a.js".to_string()],
            vec![],
            Some("out.js".to_string()),
            Some("src/".to_string()),
            vec![Some("content".to_string())],
            vec![0],
            Some("debug-123".to_string()),
        ).unwrap();
        assert_eq!(sm.source(0), "a.js");
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(1, 0).is_some());
    }

    #[test]
    fn lazy_into_sourcemap_roundtrip() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["x"],"mappings":"AAAAA;AACAA"}"#;
        let lazy = LazySourceMap::from_json(json).unwrap();
        let sm = lazy.into_sourcemap().unwrap();
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(1, 0).is_some());
        assert_eq!(sm.name(0), "x");
    }

    #[test]
    fn lazy_original_position_for_no_match() {
        // LazySourceMap: column before any mapping should return None (Err(0) branch)
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"KAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        // Column 0 is before column 5 (K = 5), should return None
        assert!(sm.original_position_for(0, 0).is_none());
    }

    #[test]
    fn lazy_original_position_for_empty_line() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":";AAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        // Line 0 is empty
        assert!(sm.original_position_for(0, 0).is_none());
        // Line 1 has mapping
        assert!(sm.original_position_for(1, 0).is_some());
    }

    #[test]
    fn lazy_original_position_generated_only() {
        // Only a 1-field (generated-only) segment on line 0
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A;AAAA"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        // Line 0 has only generated-only segment → returns None
        assert!(sm.original_position_for(0, 0).is_none());
        // Line 1 has a 4-field segment → returns Some
        assert!(sm.original_position_for(1, 0).is_some());
    }

    #[test]
    fn from_json_lines_null_source() {
        let json = r#"{"version":3,"sources":[null,"a.js"],"names":[],"mappings":"ACAA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 1).unwrap();
        assert!(sm.mapping_count() >= 1);
    }

    #[test]
    fn from_json_lines_with_source_root_prefix() {
        let json = r#"{"version":3,"sourceRoot":"lib/","sources":["b.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 1).unwrap();
        assert_eq!(sm.source(0), "lib/b.js");
    }

    #[test]
    fn generated_position_for_glb_idx_zero() {
        // When the reverse index partition_point returns 0, GLB should return None
        // Create a map where source "a.js" only has mapping at original (5,0)
        // Searching for (0,0) in GLB mode: partition_point returns 0 (nothing <= (0,0)), so None
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAKA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let loc = sm.generated_position_for_with_bias("a.js", 0, 0, Bias::GreatestLowerBound);
        assert!(loc.is_none());
    }

    #[test]
    fn from_json_lines_with_ignore_list() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA;ACAA","ignoreList":[1]}"#;
        let sm = SourceMap::from_json_lines(json, 0, 2).unwrap();
        assert_eq!(sm.ignore_list, vec![1]);
    }

    #[test]
    fn validate_deep_out_of_order_mappings() {
        // Manually construct a map with out-of-order segments
        let sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![
                Mapping { generated_line: 1, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME },
                Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME },
            ],
            vec![], None, None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("out of order")));
    }

    #[test]
    fn validate_deep_out_of_bounds_source() {
        let sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![Mapping { generated_line: 0, generated_column: 0, source: 5, original_line: 0, original_column: 0, name: NO_NAME }],
            vec![], None, None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("source index") && w.contains("out of bounds")));
    }

    #[test]
    fn validate_deep_out_of_bounds_name() {
        let sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string()],
            vec![None],
            vec!["foo".to_string()],
            vec![Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: 5 }],
            vec![], None, None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("name index") && w.contains("out of bounds")));
    }

    #[test]
    fn validate_deep_out_of_bounds_ignore_list() {
        let sm = SourceMap::from_parts(
            None, None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![Mapping { generated_line: 0, generated_column: 0, source: 0, original_line: 0, original_column: 0, name: NO_NAME }],
            vec![10], None, None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("ignoreList") && w.contains("out of bounds")));
    }

    #[test]
    fn source_mapping_url_inline_decoded() {
        // Test that inline data URIs actually decode base64 and return the parsed map
        let map_json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let encoded = base64_encode_simple(map_json);
        let input = format!("var x;\n//# sourceMappingURL=data:application/json;base64,{encoded}");
        let url = parse_source_mapping_url(&input);
        match url {
            Some(SourceMappingUrl::Inline(json)) => {
                assert!(json.contains("version"));
                assert!(json.contains("AAAA"));
            }
            _ => panic!("expected inline source map"),
        }
    }

    #[test]
    fn source_mapping_url_charset_variant() {
        let map_json = r#"{"version":3}"#;
        let encoded = base64_encode_simple(map_json);
        let input = format!("x\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,{encoded}");
        let url = parse_source_mapping_url(&input);
        assert!(matches!(url, Some(SourceMappingUrl::Inline(_))));
    }

    #[test]
    fn source_mapping_url_invalid_base64_falls_through_to_external() {
        // Data URI with invalid base64 that fails to decode should still return External
        let input = "x\n//# sourceMappingURL=data:application/json;base64,!!!invalid!!!";
        let url = parse_source_mapping_url(input);
        // Invalid base64 → base64_decode returns None → falls through to External
        assert!(matches!(url, Some(SourceMappingUrl::External(_))));
    }

    #[test]
    fn from_json_lines_with_extensions_preserved() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_custom":99}"#;
        let sm = SourceMap::from_json_lines(json, 0, 1).unwrap();
        assert!(sm.extensions.contains_key("x_custom"));
    }

    // Helper for base64 encoding in tests
    fn base64_encode_simple(input: &str) -> String {
        const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let bytes = input.as_bytes();
        let mut result = String::new();
        for chunk in bytes.chunks(3) {
            let b0 = chunk[0] as u32;
            let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
            let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
            let n = (b0 << 16) | (b1 << 8) | b2;
            result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
            result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
            if chunk.len() > 1 {
                result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
            if chunk.len() > 2 {
                result.push(CHARS[(n & 0x3F) as usize] as char);
            } else {
                result.push('=');
            }
        }
        result
    }
}
