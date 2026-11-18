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

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;

use serde::Deserialize;
use srcmap_codec::{DecodeError, vlq_encode_unsigned};
use srcmap_scopes::{Binding, CallSite, GeneratedRange, OriginalScope, Position, ScopeInfo};

mod debug_id;
pub mod js_identifiers;
pub mod offset_lookup;
pub mod source_view;
pub mod utils;

pub use debug_id::{extract_css_debug_id, extract_debug_id};
pub use offset_lookup::{GeneratedOffsetLookup, OriginalPositionLookup};
pub use source_view::SourceView;

// ── Constants ──────────────────────────────────────────────────────

const NO_SOURCE: u32 = u32::MAX;
const NO_NAME: u32 = u32::MAX;

/// The largest encoded segment: one separator and 5 VLQ values.
const MAX_SEGMENT_BYTES: usize = 1 + 5 * srcmap_codec::MAX_VLQ_BYTES;
/// A typical encoded segment, with its separator, takes about 6 bytes.
const ESTIMATED_BYTES_PER_MAPPING: usize = 6;
/// The fixed keys and punctuation of the JSON output.
const JSON_FIXED_OVERHEAD: usize = 256;
/// The quotes and the comma of one JSON string in an array.
const JSON_STRING_OVERHEAD: usize = 3;
/// Source text grows by about one sixteenth when its line breaks and quotes get an escape.
const ESCAPE_SHARE_DIVISOR: usize = 16;

// ── Public types ───────────────────────────────────────────────────

/// A single decoded mapping entry. Compact at 28 bytes (6 × u32 + bool with padding).
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
    /// Whether this mapping is a range mapping (ECMA-426).
    pub is_range_mapping: bool,
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
    Scopes(srcmap_scopes::ScopesError),
    /// A section map in an indexed source map is itself an indexed map (not allowed per ECMA-426).
    NestedIndexMap,
    /// Sections in an indexed source map are not in ascending (line, column) order.
    SectionsNotOrdered,
    /// The data URL is malformed (not a valid `data:application/json` URL).
    InvalidDataUrl,
    /// A section offset, added to a section mapping, exceeds the supported generated line or column range.
    SectionOffsetOutOfRange,
    /// A mapping, `ignoreList` entry, or scopes reference points past the end of its array.
    IndexOutOfRange {
        /// The array that the index refers to.
        kind: IndexKind,
        /// The out-of-range index.
        index: u32,
        /// The length of the array.
        len: usize,
    },
}

/// The array that an out-of-range index refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexKind {
    /// A mapping source index, checked against `sources`.
    Source,
    /// A mapping name index, checked against `names`.
    Name,
    /// An `ignoreList` or `x_google_ignoreList` entry, checked against `sources`.
    IgnoreList,
    /// A generated range definition, checked against the original scope count.
    ScopeDefinition,
    /// A generated range call-site source index, checked against `sources`.
    CallSiteSource,
}

/// Options for [`SourceMap::from_json_with_options`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParseOptions {
    /// What the parser does with a mapping name index outside `names`.
    pub out_of_range_names: OutOfRangeNames,
    /// Accept the `names` values that JavaScript reads as no name.
    ///
    /// When this is true, an entry of `names` that is `null`, `false` or `0`
    /// becomes `""`. A top-level `names` value that is `null`, a boolean, a
    /// number or `""` becomes `[]`. Other entries that are not strings stay
    /// invalid. The parser does this only after the first parse fails, so a
    /// valid map parses once. Section maps do not get this check.
    pub lenient_names: bool,
}

/// What the parser does with a mapping name index outside `names`.
///
/// A negative name index is also outside `names`. Source indices, `ignoreList`
/// entries and scopes references stay strict in each mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum OutOfRangeNames {
    /// Fail with [`ParseError::IndexOutOfRange`]. [`SourceMap::from_json`] does this.
    #[default]
    Reject,
    /// Give the mapping no name, as `@jridgewell/trace-mapping` does when
    /// `names[index]` is `undefined`.
    Unnamed,
    /// Give the mapping the name at index `names.len()` of the input, and add
    /// this string to the end of `names`. The parser adds the string only when
    /// a mapping needs it.
    ///
    /// A caller that composes maps can use this mode to tell a 5-field segment
    /// with an invalid name from a 4-field segment, and remove the string later.
    Placeholder(String),
}

impl IndexKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::Name => "name",
            Self::IgnoreList => "ignoreList",
            Self::ScopeDefinition => "scopes definition",
            Self::CallSiteSource => "call-site source",
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "JSON parse error: {e}"),
            Self::Vlq(e) => write!(f, "VLQ decode error: {e}"),
            Self::InvalidVersion(v) => write!(f, "unsupported source map version: {v}"),
            Self::Scopes(e) => write!(f, "scopes decode error: {e}"),
            Self::NestedIndexMap => write!(f, "section map must not be an indexed source map"),
            Self::SectionsNotOrdered => {
                write!(f, "sections must be in ascending (line, column) order")
            }
            Self::InvalidDataUrl => write!(f, "malformed data URL"),
            Self::SectionOffsetOutOfRange => {
                write!(f, "section offset moves a mapping outside the supported generated range")
            }
            Self::IndexOutOfRange { kind, index, len } => {
                write!(f, "{} index {index} out of range (length {len})", kind.label())
            }
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(e) => Some(e),
            Self::Vlq(e) => Some(e),
            Self::Scopes(e) => Some(e),
            Self::InvalidVersion(_)
            | Self::NestedIndexMap
            | Self::SectionsNotOrdered
            | Self::InvalidDataUrl
            | Self::SectionOffsetOutOfRange
            | Self::IndexOutOfRange { .. } => None,
        }
    }
}

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

impl From<srcmap_scopes::ScopesError> for ParseError {
    fn from(e: srcmap_scopes::ScopesError) -> Self {
        Self::Scopes(e)
    }
}

// ── Helpers ────────────────────────────────────────────────────────

/// Resolve source filenames by applying `source_root` prefix and replacing `None` with empty string.
pub fn resolve_sources(raw_sources: &[Option<String>], source_root: &str) -> Vec<String> {
    raw_sources
        .iter()
        .map(|s| match s {
            Some(s) if !source_root.is_empty() => format!("{source_root}{s}"),
            Some(s) => s.clone(),
            None => String::new(),
        })
        .collect()
}

/// Build a source filename -> index lookup map.
///
/// A duplicate filename resolves to its first index, as `sources.indexOf` does
/// in JavaScript source map consumers.
fn build_source_map(sources: &[String]) -> HashMap<String, u32> {
    let mut map = HashMap::with_capacity(sources.len());
    for (i, s) in sources.iter().enumerate() {
        map.entry(s.clone()).or_insert(i as u32);
    }
    map
}

fn check_version(version: u32) -> Result<(), ParseError> {
    if version == 3 { Ok(()) } else { Err(ParseError::InvalidVersion(version)) }
}

/// `x_google_ignoreList` is only a fallback when `ignoreList` is absent; an
/// explicit empty `ignoreList` wins.
fn resolve_ignore_list(ignore_list: Option<Vec<u32>>, legacy: Option<Vec<u32>>) -> Vec<u32> {
    ignore_list.or(legacy).unwrap_or_default()
}

const fn index_error(kind: IndexKind, index: u32, len: usize) -> ParseError {
    ParseError::IndexOutOfRange { kind, index, len }
}

/// Limits and findings of one mappings decode, so that later checks need no extra pass.
#[derive(Debug, Clone, Copy)]
struct DecodeChecks {
    /// Source indices from this value up set `index_out_of_range`.
    source_limit: u64,
    /// Name indices from this value up set `index_out_of_range`.
    name_limit: u64,
    /// True when a mapping holds a source or name index at or above its limit.
    index_out_of_range: bool,
    /// True when a line has a segment with a lower column than the segment before it.
    unsorted: bool,
    /// The name index for a name index outside `names`, or `None` to report it
    /// with `index_out_of_range` (see [`OutOfRangeNames`]).
    out_of_range_name: Option<u32>,
    /// True when a mapping got `out_of_range_name`.
    used_out_of_range_name: bool,
}

impl DecodeChecks {
    /// Checks for a map with `sources_len` sources and `names_len` names.
    fn new(sources_len: usize, names_len: usize) -> Self {
        Self {
            source_limit: (sources_len as u64).min(POSITION_END),
            name_limit: (names_len as u64).min(POSITION_END),
            index_out_of_range: false,
            unsorted: false,
            out_of_range_name: None,
            used_out_of_range_name: false,
        }
    }

    /// Checks like [`DecodeChecks::new`] that apply `mode` to name indices outside `names`.
    fn with_names_mode(sources_len: usize, names_len: usize, mode: &OutOfRangeNames) -> Self {
        let out_of_range_name = match mode {
            OutOfRangeNames::Reject => None,
            OutOfRangeNames::Unnamed => Some(NO_NAME),
            // A placeholder index at or above `POSITION_END` cannot be a name index.
            OutOfRangeNames::Placeholder(_) => Some(
                u32::try_from(names_len)
                    .ok()
                    .filter(|&i| u64::from(i) < POSITION_END)
                    .unwrap_or(NO_NAME),
            ),
        };
        Self { out_of_range_name, ..Self::new(sources_len, names_len) }
    }

    /// Checks that only find unsorted lines, for a decode that does not check indices.
    const fn without_index_limits() -> Self {
        Self {
            source_limit: POSITION_END,
            name_limit: POSITION_END,
            index_out_of_range: false,
            unsorted: false,
            out_of_range_name: None,
            used_out_of_range_name: false,
        }
    }
}

/// Return the error for the first mapping with an out-of-range source or name index.
#[cold]
fn check_mapping_indices(
    mappings: &[Mapping],
    sources_len: usize,
    names_len: usize,
) -> Result<(), ParseError> {
    for m in mappings {
        if m.source != NO_SOURCE && m.source as usize >= sources_len {
            return Err(index_error(IndexKind::Source, m.source, sources_len));
        }
        if m.name != NO_NAME && m.name as usize >= names_len {
            return Err(index_error(IndexKind::Name, m.name, names_len));
        }
    }
    Ok(())
}

/// Check that every index in decoded map data points inside its array.
///
/// The walks over original scopes and generated ranges use explicit stacks, so
/// deep nesting from untrusted input cannot overflow the call stack.
///
/// `checks` comes from the decode of `mappings`. The mappings are scanned only
/// when the decode found an out-of-range index, to report the first one.
fn check_indices(
    mappings: &[Mapping],
    checks: &DecodeChecks,
    sources_len: usize,
    names_len: usize,
    ignore_list: &[u32],
    scopes: Option<&ScopeInfo>,
) -> Result<(), ParseError> {
    if checks.index_out_of_range {
        check_mapping_indices(mappings, sources_len, names_len)?;
    }

    if let Some(&idx) = ignore_list.iter().find(|&&idx| idx as usize >= sources_len) {
        return Err(index_error(IndexKind::IgnoreList, idx, sources_len));
    }

    let Some(scopes) = scopes else {
        return Ok(());
    };
    let total = scopes.scopes.iter().flatten().map(count_scope_tree).sum::<u32>() as usize;
    let mut stack: Vec<&GeneratedRange> = scopes.ranges.iter().collect();
    while let Some(range) = stack.pop() {
        if let Some(def) = range.definition
            && def as usize >= total
        {
            return Err(index_error(IndexKind::ScopeDefinition, def, total));
        }
        if let Some(call_site) = &range.call_site
            && call_site.source_index as usize >= sources_len
        {
            return Err(index_error(
                IndexKind::CallSiteSource,
                call_site.source_index,
                sources_len,
            ));
        }
        stack.extend(range.children.iter());
    }

    Ok(())
}

fn decode_scopes_field(
    scopes: Option<&str>,
    names: &[String],
    num_sources: usize,
) -> Result<Option<ScopeInfo>, ParseError> {
    match scopes {
        Some(s) if !s.is_empty() => Ok(Some(srcmap_scopes::decode_scopes(s, names, num_sources)?)),
        _ => Ok(None),
    }
}

/// Mark range mappings in place and report whether any were set.
fn decode_range_mappings_field(
    range_mappings: Option<&str>,
    mappings: &mut [Mapping],
    line_offsets: &[u32],
) -> Result<bool, DecodeError> {
    match range_mappings {
        Some(s) if !s.is_empty() => {
            decode_range_mappings(s, mappings, line_offsets)?;
            Ok(mappings.iter().any(|m| m.is_range_mapping))
        }
        _ => Ok(false),
    }
}

fn name_index(mapping: &Mapping) -> Option<u32> {
    if mapping.name == NO_NAME { None } else { Some(mapping.name) }
}

/// Largest generated line that an indexed source map may address.
///
/// The merged line table costs 4 bytes per generated line, so this bounds it at 64 MiB.
const MAX_GENERATED_LINE: u32 = 1 << 24;

fn build_mapping_line_offsets(mappings: &[Mapping], line_count: usize) -> Vec<u32> {
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

    if !line_offsets.is_empty() {
        let last = mappings.len() as u32;
        for offset in line_offsets.iter_mut().skip(current_line + 1) {
            *offset = last;
        }
    }

    line_offsets
}

fn finish_section_mappings(mappings: &mut [Mapping], max_line: u32) -> Vec<u32> {
    mappings.sort_unstable_by(|a, b| {
        a.generated_line.cmp(&b.generated_line).then(a.generated_column.cmp(&b.generated_column))
    });

    let line_count = if mappings.is_empty() { 0 } else { max_line as usize + 1 };
    build_mapping_line_offsets(mappings, line_count)
}

fn build_section_scope_info(
    scopes: Vec<Option<OriginalScope>>,
    ranges: Vec<GeneratedRange>,
) -> Option<ScopeInfo> {
    if ranges.is_empty() && scopes.iter().all(Option::is_none) {
        return None;
    }

    Some(ScopeInfo { scopes, ranges })
}

struct SectionMergeState {
    all_sources: Vec<String>,
    all_sources_content: Vec<Option<String>>,
    all_names: Vec<String>,
    all_mappings: Vec<Mapping>,
    all_ignore_list: Vec<u32>,
    ignore_set: HashSet<u32>,
    all_scopes: Vec<Option<OriginalScope>>,
    all_ranges: Vec<GeneratedRange>,
    pending_scopes: Vec<(ScopeInfo, Vec<u32>, u32, u32)>,
    max_line: u32,
    source_index_map: HashMap<String, u32>,
    name_index_map: HashMap<String, u32>,
}

impl SectionMergeState {
    fn new() -> Self {
        Self {
            all_sources: Vec::new(),
            all_sources_content: Vec::new(),
            all_names: Vec::new(),
            all_mappings: Vec::new(),
            all_ignore_list: Vec::new(),
            ignore_set: HashSet::new(),
            all_scopes: Vec::new(),
            all_ranges: Vec::new(),
            pending_scopes: Vec::new(),
            max_line: 0,
            source_index_map: HashMap::new(),
            name_index_map: HashMap::new(),
        }
    }

    fn append_section(
        &mut self,
        section: &RawSection,
        options: &ParseOptions,
    ) -> Result<(), ParseError> {
        // Section maps must not be indexed maps themselves (ECMA-426)
        let sub = SourceMap::from_json_inner(section.map.get(), false, options)?;

        let line_offset = section.offset.line;
        let col_offset = section.offset.column;

        let source_remap = remap_section_sources(
            &sub.sources,
            &sub.sources_content,
            &mut self.all_sources,
            &mut self.all_sources_content,
            &mut self.all_scopes,
            &mut self.source_index_map,
        );

        let name_remap =
            remap_section_names(&sub.names, &mut self.all_names, &mut self.name_index_map);

        merge_section_ignore_list(
            &sub.ignore_list,
            &source_remap,
            &mut self.ignore_set,
            &mut self.all_ignore_list,
        );

        if let Some(section_scopes) = &sub.scopes {
            self.pending_scopes.push((
                section_scopes.clone(),
                source_remap.clone(),
                line_offset,
                col_offset,
            ));
        }

        append_section_mappings(
            &sub.mappings,
            &source_remap,
            &name_remap,
            line_offset,
            col_offset,
            &mut self.all_mappings,
            &mut self.max_line,
        )?;

        Ok(())
    }
}

fn filter_section_source_root(source_root: Option<String>, sources: &[String]) -> Option<String> {
    source_root.filter(|root| {
        root.is_empty()
            || sources
                .iter()
                .filter(|source| !source.is_empty())
                .all(|source| source.starts_with(root))
    })
}

/// Build the original location for a lookup hit on `mapping` at `column`.
///
/// A range mapping shifts the original column by the distance from its generated column.
#[inline]
fn hit_location(mapping: &Mapping, column: u32) -> Option<OriginalLocation> {
    // Only a range mapping moves the original column, so the common mapping
    // needs no overflow check.
    let original_column = if mapping.is_range_mapping {
        mapping.original_column.checked_add(column.saturating_sub(mapping.generated_column))?
    } else {
        mapping.original_column
    };

    Some(OriginalLocation {
        source: mapping.source,
        line: mapping.original_line,
        column: original_column,
        name: name_index(mapping),
    })
}

/// Resolve `(line, column)` through `last`, the last mapping before that position.
///
/// Returns `None` unless `last` is a range mapping with a source. Uses
/// `saturating_sub` for the column delta, so a query column before the range
/// start maps to the range start.
#[inline]
fn range_fallback_location(last: &Mapping, line: u32, column: u32) -> Option<OriginalLocation> {
    if !last.is_range_mapping || last.source == NO_SOURCE {
        return None;
    }
    let line_delta = line - last.generated_line;
    let column_delta =
        if line_delta == 0 { column.saturating_sub(last.generated_column) } else { 0 };
    Some(OriginalLocation {
        source: last.source,
        line: last.original_line.checked_add(line_delta)?,
        column: last.original_column.checked_add(column_delta)?,
        name: name_index(last),
    })
}

fn original_position_glb_index(line_mappings: &[Mapping], column: u32) -> Option<usize> {
    // A binary search with no early exit: each step is one compare and one
    // conditional move, which the CPU does not have to predict.
    let mut base = 0;
    let mut size = line_mappings.len();
    while size > 1 {
        let half = size / 2;
        let mid = base + half;
        if line_mappings[mid].generated_column <= column {
            base = mid;
        }
        size -= half;
    }
    if line_mappings.get(base)?.generated_column > column {
        return None;
    }
    // `base` is the last mapping at or before `column`. On an exact match,
    // `@jridgewell/trace-mapping` takes the earliest segment with that column.
    while base > 0 && line_mappings[base - 1].generated_column == column {
        base -= 1;
    }
    Some(base)
}

fn original_position_lub_index(line_mappings: &[Mapping], column: u32) -> Option<usize> {
    match line_mappings.binary_search_by_key(&column, |m| m.generated_column) {
        // Exact match: walk forward to the latest segment sharing this column.
        // Mirrors `@jridgewell/trace-mapping`'s LUB = latest-equal tie-break.
        Ok(i) => {
            let mut idx = i;
            while idx + 1 < line_mappings.len() && line_mappings[idx + 1].generated_column == column
            {
                idx += 1;
            }
            Some(idx)
        }
        Err(i) if i >= line_mappings.len() => None,
        Err(i) => Some(i),
    }
}

fn owned_range_mappings(range_mappings: Option<&str>) -> Option<String> {
    range_mappings.filter(|s| !s.is_empty()).map(str::to_string)
}

const fn generated_only_mapping(line: u32, generated_column: u32) -> Mapping {
    Mapping {
        generated_line: line,
        generated_column,
        source: NO_SOURCE,
        original_line: 0,
        original_column: 0,
        name: NO_NAME,
        is_range_mapping: false,
    }
}

fn validate_section_order(sections: &[RawSection]) -> Result<(), ParseError> {
    for i in 1..sections.len() {
        let prev = &sections[i - 1].offset;
        let curr = &sections[i].offset;
        if (curr.line, curr.column) <= (prev.line, prev.column) {
            return Err(ParseError::SectionsNotOrdered);
        }
    }

    Ok(())
}

fn remap_section_sources(
    sources: &[String],
    sources_content: &[Option<String>],
    all_sources: &mut Vec<String>,
    all_sources_content: &mut Vec<Option<String>>,
    all_scopes: &mut Vec<Option<OriginalScope>>,
    source_index_map: &mut HashMap<String, u32>,
) -> Vec<u32> {
    sources
        .iter()
        .enumerate()
        .map(|(i, source)| {
            if let Some(&existing) = source_index_map.get(source) {
                existing
            } else {
                let idx = all_sources.len() as u32;
                all_sources.push(source.clone());
                all_sources_content.push(sources_content.get(i).cloned().unwrap_or(None));
                all_scopes.push(None);
                source_index_map.insert(source.clone(), idx);
                idx
            }
        })
        .collect()
}

fn remap_section_names(
    names: &[String],
    all_names: &mut Vec<String>,
    name_index_map: &mut HashMap<String, u32>,
) -> Vec<u32> {
    names
        .iter()
        .map(|name| {
            if let Some(&existing) = name_index_map.get(name) {
                existing
            } else {
                let idx = all_names.len() as u32;
                all_names.push(name.clone());
                name_index_map.insert(name.clone(), idx);
                idx
            }
        })
        .collect()
}

fn merge_section_ignore_list(
    ignore_list: &[u32],
    source_remap: &[u32],
    ignore_set: &mut HashSet<u32>,
    all_ignore_list: &mut Vec<u32>,
) {
    for &idx in ignore_list {
        let Some(&global_idx) = source_remap.get(idx as usize) else {
            continue;
        };
        if ignore_set.insert(global_idx) {
            all_ignore_list.push(global_idx);
        }
    }
}

fn append_section_mappings(
    mappings: &[Mapping],
    source_remap: &[u32],
    name_remap: &[u32],
    line_offset: u32,
    col_offset: u32,
    all_mappings: &mut Vec<Mapping>,
    max_line: &mut u32,
) -> Result<(), ParseError> {
    for m in mappings {
        let gen_line = m
            .generated_line
            .checked_add(line_offset)
            .filter(|&line| line <= MAX_GENERATED_LINE)
            .ok_or(ParseError::SectionOffsetOutOfRange)?;
        let gen_col = if m.generated_line == 0 {
            m.generated_column.checked_add(col_offset).ok_or(ParseError::SectionOffsetOutOfRange)?
        } else {
            m.generated_column
        };

        all_mappings.push(Mapping {
            generated_line: gen_line,
            generated_column: gen_col,
            source: source_remap.get(m.source as usize).copied().unwrap_or(NO_SOURCE),
            original_line: m.original_line,
            original_column: m.original_column,
            name: name_remap.get(m.name as usize).copied().unwrap_or(NO_NAME),
            is_range_mapping: m.is_range_mapping,
        });

        if gen_line > *max_line {
            *max_line = gen_line;
        }
    }
    Ok(())
}

fn merge_section_original_scopes(
    pending_scopes: &[(ScopeInfo, Vec<u32>, u32, u32)],
    all_scopes: &mut [Option<OriginalScope>],
) {
    for (section_scopes, source_remap, _, _) in pending_scopes {
        for (local_idx, local_scope) in section_scopes.scopes.iter().enumerate() {
            let global_idx = source_remap[local_idx] as usize;
            if all_scopes[global_idx].is_none() {
                all_scopes[global_idx] = local_scope.clone();
            }
        }
    }
}

fn build_section_definition_remap(
    section_scopes: &ScopeInfo,
    source_remap: &[u32],
    global_bases: &[u32],
) -> Vec<u32> {
    let local_bases = definition_bases(&section_scopes.scopes);
    let total_local_definitions =
        section_scopes.scopes.iter().flatten().map(count_scope_tree).sum::<u32>() as usize;
    let mut definition_remap = vec![0; total_local_definitions];

    for (local_idx, local_scope) in section_scopes.scopes.iter().enumerate() {
        let Some(local_scope) = local_scope else {
            continue;
        };
        let local_base = local_bases[local_idx];
        let global_base = global_bases[source_remap[local_idx] as usize];
        for offset in 0..count_scope_tree(local_scope) {
            definition_remap[(local_base + offset) as usize] = global_base + offset;
        }
    }

    definition_remap
}

fn merge_section_scopes_and_ranges(
    pending_scopes: Vec<(ScopeInfo, Vec<u32>, u32, u32)>,
    all_scopes: &mut [Option<OriginalScope>],
    all_ranges: &mut Vec<GeneratedRange>,
) {
    merge_section_original_scopes(&pending_scopes, all_scopes);

    let global_bases = definition_bases(all_scopes);
    for (section_scopes, source_remap, line_offset, col_offset) in pending_scopes {
        let definition_remap =
            build_section_definition_remap(&section_scopes, &source_remap, &global_bases);
        all_ranges.extend(section_scopes.ranges.iter().map(|range| {
            remap_generated_range(range, line_offset, col_offset, &definition_remap, &source_remap)
        }));
    }
}

/// Retain only extension fields that use an `x_*` or `x-*` prefix.
fn filter_extensions(
    extensions: HashMap<String, serde_json::Value>,
) -> HashMap<String, serde_json::Value> {
    extensions.into_iter().filter(|(k, _)| k.starts_with("x_") || k.starts_with("x-")).collect()
}

/// Count `scope` and all scopes nested in it.
///
/// The walk uses an explicit stack, so deep nesting from untrusted input
/// cannot overflow the call stack.
fn count_scope_tree(scope: &OriginalScope) -> u32 {
    let mut count: u32 = 0;
    let mut stack = vec![scope];
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend(&node.children);
    }
    count
}

fn definition_bases(scopes: &[Option<OriginalScope>]) -> Vec<u32> {
    let mut bases = Vec::with_capacity(scopes.len());
    let mut next = 0;
    for scope in scopes {
        bases.push(next);
        if let Some(scope) = scope {
            next += count_scope_tree(scope);
        }
    }
    bases
}

fn offset_generated_position(pos: Position, line_offset: u32, col_offset: u32) -> Position {
    Position {
        line: pos.line.saturating_add(line_offset),
        column: if pos.line == 0 { pos.column.saturating_add(col_offset) } else { pos.column },
    }
}

fn remap_binding(binding: &Binding, line_offset: u32, col_offset: u32) -> Binding {
    match binding {
        Binding::Expression(expr) => Binding::Expression(expr.clone()),
        Binding::Unavailable => Binding::Unavailable,
        Binding::SubRanges(sub_ranges) => Binding::SubRanges(
            sub_ranges
                .iter()
                .map(|sub| srcmap_scopes::SubRangeBinding {
                    expression: sub.expression.clone(),
                    from: offset_generated_position(sub.from, line_offset, col_offset),
                })
                .collect(),
        ),
    }
}

fn remap_generated_range(
    range: &GeneratedRange,
    line_offset: u32,
    col_offset: u32,
    definition_remap: &[u32],
    source_remap: &[u32],
) -> GeneratedRange {
    GeneratedRange {
        start: offset_generated_position(range.start, line_offset, col_offset),
        end: offset_generated_position(range.end, line_offset, col_offset),
        is_stack_frame: range.is_stack_frame,
        is_hidden: range.is_hidden,
        definition: range.definition.and_then(|idx| definition_remap.get(idx as usize).copied()),
        call_site: range.call_site.and_then(|call_site| {
            source_remap.get(call_site.source_index as usize).map(|&source_index| CallSite {
                source_index,
                line: call_site.line,
                column: call_site.column,
            })
        }),
        bindings: range
            .bindings
            .iter()
            .map(|binding| remap_binding(binding, line_offset, col_offset))
            .collect(),
        children: range
            .children
            .iter()
            .map(|child| {
                remap_generated_range(
                    child,
                    line_offset,
                    col_offset,
                    definition_remap,
                    source_remap,
                )
            })
            .collect(),
    }
}

// ── Raw JSON structure ─────────────────────────────────────────────

/// Return a copy of `json` with the changes of [`ParseOptions::lenient_names`],
/// or `None` when `json` is not an object or has no `names` value to change.
fn lenient_names_json(json: &str) -> Option<String> {
    use serde_json::Value;

    let mut map: Value = serde_json::from_str(json).ok()?;
    let names = map.as_object_mut()?.get_mut("names")?;
    match names {
        Value::Array(entries) => {
            let mut changed = false;
            for entry in entries.iter_mut().filter(|entry| is_falsy_non_string(entry)) {
                *entry = Value::String(String::new());
                changed = true;
            }
            if !changed {
                return None;
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => *names = Value::Array(Vec::new()),
        Value::String(s) if s.is_empty() => *names = Value::Array(Vec::new()),
        Value::String(_) | Value::Object(_) => return None,
    }
    serde_json::to_string(&map).ok()
}

/// True when `value` is not a string and JavaScript reads it as falsy.
fn is_falsy_non_string(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Null => true,
        serde_json::Value::Bool(b) => !b,
        serde_json::Value::Number(n) => n.as_f64() == Some(0.0),
        serde_json::Value::String(_)
        | serde_json::Value::Array(_)
        | serde_json::Value::Object(_) => false,
    }
}

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
    ignore_list: Option<Vec<u32>>,
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
    /// Range mappings (ECMA-426).
    #[serde(default, borrow, rename = "rangeMappings")]
    range_mappings: Option<&'a str>,
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
    map: Box<serde_json::value::RawValue>,
}

#[derive(Deserialize)]
struct RawOffset {
    line: u32,
    column: u32,
}

/// Lightweight version that skips sourcesContent allocation.
/// Used by WASM bindings where sourcesContent is kept JS-side.
///
/// Note: Indexed/sectioned source maps are detected via the `sections` field
/// and must be rejected by callers (LazySourceMap does not support them).
#[derive(Deserialize)]
pub struct RawSourceMapLite<'a> {
    pub version: u32,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default, rename = "sourceRoot")]
    pub source_root: Option<String>,
    #[serde(default)]
    pub sources: Vec<Option<String>>,
    #[serde(default)]
    pub names: Vec<String>,
    #[serde(default, borrow)]
    pub mappings: &'a str,
    #[serde(default, rename = "ignoreList")]
    pub ignore_list: Option<Vec<u32>>,
    #[serde(default, rename = "x_google_ignoreList")]
    pub x_google_ignore_list: Option<Vec<u32>>,
    #[serde(default, rename = "debugId", alias = "debug_id")]
    pub debug_id: Option<String>,
    #[serde(default, borrow)]
    pub scopes: Option<&'a str>,
    #[serde(default, borrow, rename = "rangeMappings")]
    pub range_mappings: Option<&'a str>,
    /// Indexed source maps use `sections` instead of `mappings`.
    /// Presence is checked to reject indexed maps in lazy parse paths.
    #[serde(default)]
    pub sections: Option<Vec<serde_json::Value>>,
}

/// The fields of a regular (non-indexed) source map, before the decode of
/// `mappings`. The eager parsers build a [`SourceMap`] from it.
struct RegularFields<'a> {
    file: Option<String>,
    source_root: Option<String>,
    sources: Vec<String>,
    sources_content: Vec<Option<String>>,
    names: Vec<String>,
    mappings: &'a str,
    ignore_list: Option<Vec<u32>>,
    x_google_ignore_list: Option<Vec<u32>>,
    debug_id: Option<String>,
    scopes: Option<&'a str>,
    range_mappings: Option<&'a str>,
    extensions: HashMap<String, serde_json::Value>,
}

impl<'a> RegularFields<'a> {
    /// Parse the JSON of a regular source map. Reject an indexed map.
    fn parse(json: &'a str) -> Result<Self, ParseError> {
        let raw: RawSourceMap<'a> = serde_json::from_str(json)?;
        check_version(raw.version)?;
        if raw.sections.is_some() {
            return Err(ParseError::NestedIndexMap);
        }
        Ok(Self::from_raw(raw))
    }

    /// Parse the JSON of a regular source map, but skip `sourcesContent`
    /// and the extension fields. Reject an indexed map.
    fn parse_lite(json: &'a str) -> Result<Self, ParseError> {
        let raw: RawSourceMapLite<'a> = serde_json::from_str(json)?;
        check_version(raw.version)?;
        if raw.sections.is_some() {
            return Err(ParseError::NestedIndexMap);
        }
        Ok(Self::from_lite(raw))
    }

    fn from_raw(raw: RawSourceMap<'a>) -> Self {
        let sources = resolve_sources(&raw.sources, raw.source_root.as_deref().unwrap_or(""));
        Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content: raw.sources_content.unwrap_or_default(),
            names: raw.names,
            mappings: raw.mappings,
            ignore_list: raw.ignore_list,
            x_google_ignore_list: raw.x_google_ignore_list,
            debug_id: raw.debug_id,
            scopes: raw.scopes,
            range_mappings: raw.range_mappings,
            extensions: filter_extensions(raw.extensions),
        }
    }

    fn from_lite(raw: RawSourceMapLite<'a>) -> Self {
        let sources = resolve_sources(&raw.sources, raw.source_root.as_deref().unwrap_or(""));
        Self {
            file: raw.file,
            source_root: raw.source_root,
            sources,
            sources_content: Vec::new(),
            names: raw.names,
            mappings: raw.mappings,
            ignore_list: raw.ignore_list,
            x_google_ignore_list: raw.x_google_ignore_list,
            debug_id: raw.debug_id,
            scopes: raw.scopes,
            range_mappings: raw.range_mappings,
            extensions: HashMap::new(),
        }
    }
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

    /// Cached flag: true if any mapping has `is_range_mapping == true`.
    has_range_mappings: bool,
}

impl SourceMap {
    /// Parse a source map from a JSON string.
    /// Supports both regular and indexed (sectioned) source maps.
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        Self::from_json_inner(json, true, &ParseOptions::default())
    }

    /// Parse a source map from a JSON string like [`SourceMap::from_json`], with `options`.
    ///
    /// With [`OutOfRangeNames::Unnamed`], a mapping name index outside `names`
    /// gives a mapping without a name. The default options give the result of
    /// [`SourceMap::from_json`].
    ///
    /// # Examples
    ///
    /// ```
    /// use srcmap_sourcemap::{OutOfRangeNames, ParseOptions, SourceMap};
    ///
    /// let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAAA"}"#;
    /// assert!(SourceMap::from_json(json).is_err());
    ///
    /// let options =
    ///     ParseOptions { out_of_range_names: OutOfRangeNames::Unnamed, ..Default::default() };
    /// let sm = SourceMap::from_json_with_options(json, &options).unwrap();
    /// assert_eq!(sm.original_position_for(0, 0).unwrap().name, None);
    /// ```
    pub fn from_json_with_options(json: &str, options: &ParseOptions) -> Result<Self, ParseError> {
        match Self::from_json_inner(json, true, options) {
            Err(error @ ParseError::Json(_)) if options.lenient_names => {
                match lenient_names_json(json) {
                    Some(fixed) => Self::from_json_inner(&fixed, true, options),
                    None => Err(error),
                }
            }
            result => result,
        }
    }

    /// Parse a source map from JSON, skipping sourcesContent allocation.
    /// Useful for WASM bindings where sourcesContent is kept on the JS side.
    /// The resulting SourceMap has an empty `sources_content` vec.
    /// Indexed source maps are not supported; use [`SourceMap::from_json`] instead.
    pub fn from_json_no_content(json: &str) -> Result<Self, ParseError> {
        let fields = RegularFields::parse_lite(json)?;
        let decoded = decode_mappings(fields.mappings, fields.sources.len(), fields.names.len())?;
        Self::from_decoded(fields, decoded, &OutOfRangeNames::Reject)
    }

    /// Internal parser with control over whether indexed maps (sections) are allowed.
    fn from_json_inner(
        json: &str,
        allow_sections: bool,
        options: &ParseOptions,
    ) -> Result<Self, ParseError> {
        let raw: RawSourceMap<'_> = serde_json::from_str(json)?;

        check_version(raw.version)?;

        // Handle indexed source maps (sections)
        if let Some(sections) = raw.sections {
            if !allow_sections {
                return Err(ParseError::NestedIndexMap);
            }
            return Self::from_sections(
                raw.file,
                raw.source_root,
                raw.debug_id,
                filter_extensions(raw.extensions),
                sections,
                options,
            );
        }

        Self::from_regular(raw, options)
    }

    /// Parse a regular (non-indexed) source map.
    fn from_regular(raw: RawSourceMap<'_>, options: &ParseOptions) -> Result<Self, ParseError> {
        Self::from_fields(RegularFields::from_raw(raw), options)
    }

    /// Decode the `mappings` of `fields` with the name mode of `options`, and build the map.
    fn from_fields(fields: RegularFields<'_>, options: &ParseOptions) -> Result<Self, ParseError> {
        let checks = DecodeChecks::with_names_mode(
            fields.sources.len(),
            fields.names.len(),
            &options.out_of_range_names,
        );
        let decoded = decode_mappings_with(fields.mappings, checks)?;
        Self::from_decoded(fields, decoded, &options.out_of_range_names)
    }

    /// Run the steps that follow the decode of `mappings`, and build the map.
    ///
    /// The steps decode `rangeMappings` and `scopes`, sort out-of-order
    /// segments, and check every source, name and ignore-list index.
    /// `names_mode` must be the mode that the decode of `mappings` used.
    fn from_decoded(
        mut fields: RegularFields<'_>,
        (mut mappings, line_offsets, checks): (Vec<Mapping>, Vec<u32>, DecodeChecks),
        names_mode: &OutOfRangeNames,
    ) -> Result<Self, ParseError> {
        let has_range_mappings =
            decode_range_mappings_field(fields.range_mappings, &mut mappings, &line_offsets)?;
        sort_unsorted_lines(&mut mappings, &line_offsets, &checks);
        let scopes = decode_scopes_field(fields.scopes, &fields.names, fields.sources.len())?;
        // Add the placeholder after the scopes decode, so that scopes cannot refer to it.
        if checks.used_out_of_range_name
            && let OutOfRangeNames::Placeholder(placeholder) = names_mode
        {
            fields.names.push(placeholder.clone());
        }
        let ignore_list = resolve_ignore_list(fields.ignore_list, fields.x_google_ignore_list);
        check_indices(
            &mappings,
            &checks,
            fields.sources.len(),
            fields.names.len(),
            &ignore_list,
            scopes.as_ref(),
        )?;
        let source_map = build_source_map(&fields.sources);

        Ok(Self {
            file: fields.file,
            source_root: fields.source_root,
            sources: fields.sources,
            sources_content: fields.sources_content,
            names: fields.names,
            ignore_list,
            extensions: fields.extensions,
            debug_id: fields.debug_id,
            scopes,
            mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
            has_range_mappings,
        })
    }

    /// Flatten an indexed source map (with sections) into a regular one.
    fn from_sections(
        file: Option<String>,
        source_root: Option<String>,
        debug_id: Option<String>,
        extensions: HashMap<String, serde_json::Value>,
        sections: Vec<RawSection>,
        options: &ParseOptions,
    ) -> Result<Self, ParseError> {
        validate_section_order(&sections)?;

        let mut state = SectionMergeState::new();
        for section in &sections {
            state.append_section(section, options)?;
        }

        merge_section_scopes_and_ranges(
            std::mem::take(&mut state.pending_scopes),
            &mut state.all_scopes,
            &mut state.all_ranges,
        );

        let line_offsets = finish_section_mappings(&mut state.all_mappings, state.max_line);

        let source_map = build_source_map(&state.all_sources);
        let has_range_mappings = state.all_mappings.iter().any(|m| m.is_range_mapping);
        let scopes = build_section_scope_info(state.all_scopes, state.all_ranges);
        let source_root = filter_section_source_root(source_root, &state.all_sources);

        Ok(Self {
            file,
            source_root,
            sources: state.all_sources,
            sources_content: state.all_sources_content,
            names: state.all_names,
            ignore_list: state.all_ignore_list,
            extensions,
            debug_id,
            scopes,
            mappings: state.all_mappings,
            line_offsets,
            reverse_index: OnceCell::new(),
            source_map,
            has_range_mappings,
        })
    }

    /// Look up the original source position for a generated position.
    ///
    /// Both `line` and `column` are 0-based.
    /// Returns `None` if no mapping exists or the mapping has no source.
    #[inline]
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        let line_mappings = self.mappings_for_line(line);
        let Some(idx) = original_position_glb_index(line_mappings, column) else {
            return self.range_mapping_fallback(line, column);
        };
        let mapping = &line_mappings[idx];
        if mapping.source == NO_SOURCE {
            return None;
        }
        hit_location(mapping, column)
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
        match bias {
            Bias::GreatestLowerBound => self.original_position_for(line, column),
            Bias::LeastUpperBound => {
                let line_mappings = self.mappings_for_line(line);
                if line_mappings.is_empty() {
                    return self.range_mapping_fallback(line, column);
                }
                let mapping = &line_mappings[original_position_lub_index(line_mappings, column)?];
                if mapping.source == NO_SOURCE {
                    return None;
                }
                hit_location(mapping, column)
            }
        }
    }

    /// Fall back to range mappings when no exact mapping is found for the position.
    ///
    /// Uses `saturating_sub` for column delta to prevent underflow when the
    /// query column is before the range mapping's generated column.
    fn range_mapping_fallback(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        let line_idx = line as usize;
        let search_end = if line_idx + 1 < self.line_offsets.len() {
            self.line_offsets[line_idx] as usize
        } else {
            self.mappings.len()
        };
        if search_end == 0 {
            return None;
        }
        range_fallback_location(&self.mappings[search_end - 1], line, column)
    }

    /// Look up the generated position for an original source position.
    ///
    /// `source` is the source filename. `line` and `column` are 0-based.
    /// Uses `GreatestLowerBound` by default (finds closest mapping at or before the position),
    /// matching `@jridgewell/trace-mapping`'s `generatedPositionFor` semantics.
    pub fn generated_position_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Option<GeneratedLocation> {
        self.generated_position_for_with_bias(source, line, column, Bias::GreatestLowerBound)
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

        let reverse_index = self.reverse_index.get_or_init(|| build_reverse_index(&self.mappings));

        // Binary search in reverse_index for (source, line, column)
        let idx = reverse_index.partition_point(|&i| {
            let m = &self.mappings[i as usize];
            (m.source, m.original_line, m.original_column) < (source_idx, line, column)
        });

        // jridgewell's generatedPositionFor searches within a single original
        // line only, so both GLB and LUB must be constrained to the same line.
        match bias {
            Bias::GreatestLowerBound => {
                self.generated_position_glb(reverse_index, idx, source_idx, line, column)
            }
            Bias::LeastUpperBound => {
                if idx >= reverse_index.len() {
                    return None;
                }
                let mapping = &self.mappings[reverse_index[idx] as usize];
                if mapping.source != source_idx || mapping.original_line != line {
                    return None;
                }
                // On exact match, scan forward to find the last mapping with the
                // same (source, origLine, origCol). This matches jridgewell's
                // upperBound behavior: when multiple generated positions map to
                // the same original position, return the last one.
                // On non-exact match, return the first element > target as-is.
                if mapping.original_column == column {
                    let mut last_idx = idx;
                    while last_idx + 1 < reverse_index.len() {
                        let next = &self.mappings[reverse_index[last_idx + 1] as usize];
                        if next.source != source_idx
                            || next.original_line != line
                            || next.original_column != column
                        {
                            break;
                        }
                        last_idx += 1;
                    }
                    let last_mapping = &self.mappings[reverse_index[last_idx] as usize];
                    return Some(GeneratedLocation {
                        line: last_mapping.generated_line,
                        column: last_mapping.generated_column,
                    });
                }
                Some(GeneratedLocation {
                    line: mapping.generated_line,
                    column: mapping.generated_column,
                })
            }
        }
    }

    fn generated_position_glb(
        &self,
        reverse_index: &[u32],
        idx: usize,
        source_idx: u32,
        line: u32,
        column: u32,
    ) -> Option<GeneratedLocation> {
        // partition_point gives us the first element >= target.
        // For GLB, we want the element at or before on the SAME line.
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

        if idx == 0 {
            return None;
        }

        let mapping = &self.mappings[reverse_index[idx - 1] as usize];
        if mapping.source != source_idx || mapping.original_line != line {
            return None;
        }
        Some(GeneratedLocation { line: mapping.generated_line, column: mapping.generated_column })
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

        let reverse_index = self.reverse_index.get_or_init(|| build_reverse_index(&self.mappings));

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
            results.push(GeneratedLocation { line: m.generated_line, column: m.generated_column });
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
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds. Use [`get_source`](Self::get_source)
    /// for a non-panicking alternative.
    #[inline]
    pub fn source(&self, index: u32) -> &str {
        &self.sources[index as usize]
    }

    /// Resolve a source index to its filename, returning `None` if out of bounds.
    #[inline]
    pub fn get_source(&self, index: u32) -> Option<&str> {
        self.sources.get(index as usize).map(|s| s.as_str())
    }

    /// Resolve a name index to its string.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds. Use [`get_name`](Self::get_name)
    /// for a non-panicking alternative.
    #[inline]
    pub fn name(&self, index: u32) -> &str {
        &self.names[index as usize]
    }

    /// Resolve a name index to its string, returning `None` if out of bounds.
    #[inline]
    pub fn get_name(&self, index: u32) -> Option<&str> {
        self.names.get(index as usize).map(|s| s.as_str())
    }

    /// Find the source index for a filename.
    #[inline]
    pub fn source_index(&self, name: &str) -> Option<u32> {
        self.source_map.get(name).copied()
    }

    /// Total number of decoded mappings.
    #[inline]
    pub fn mapping_count(&self) -> usize {
        self.mappings.len()
    }

    /// Number of generated lines.
    #[inline]
    pub fn line_count(&self) -> usize {
        self.line_offsets.len().saturating_sub(1)
    }

    /// Get all mappings for a generated line (0-based).
    #[inline]
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
    #[inline]
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
        let scopes_encoded = self.encode_scopes_for_json();
        let names_for_json = match &scopes_encoded {
            Some((_, expanded_names)) => expanded_names,
            None => &self.names,
        };

        let source_root_prefix = self.source_root.as_deref().unwrap_or("");

        let mut json = String::with_capacity(self.json_capacity_estimate(exclude_content));
        json.push_str(r#"{"version":3"#);

        if let Some(ref file) = self.file {
            json.push_str(r#","file":"#);
            json_quote_into(&mut json, file);
        }

        if let Some(ref root) = self.source_root {
            json.push_str(r#","sourceRoot":"#);
            json_quote_into(&mut json, root);
        }

        self.write_sources_json(&mut json, source_root_prefix);
        self.write_sources_content_json(&mut json, exclude_content);
        write_string_array_field(&mut json, "names", names_for_json);

        // VLQ mappings are pure base64/,/; — no escaping needed
        json.push_str(r#","mappings":""#);
        // SAFETY: `encode_mappings_into` appends ASCII bytes only.
        self.encode_mappings_into(unsafe { json.as_mut_vec() });
        json.push('"');

        if let Some(range_mappings) = self.encode_range_mappings() {
            // Range mappings are also pure VLQ — no escaping needed
            json.push_str(r#","rangeMappings":""#);
            json.push_str(&range_mappings);
            json.push('"');
        }

        self.write_ignore_list_json(&mut json);

        if let Some(ref id) = self.debug_id {
            json.push_str(r#","debugId":"#);
            json_quote_into(&mut json, id);
        }

        // scopes (ECMA-426 scopes proposal)
        if let Some((ref s, _)) = scopes_encoded {
            json.push_str(r#","scopes":"#);
            json_quote_into(&mut json, s);
        }

        // Emit extension fields (x_* and x-* keys)
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

    /// Estimates the size of the JSON output, so that `to_json` allocates one time.
    fn json_capacity_estimate(&self, exclude_content: bool) -> usize {
        let quoted_len = |text: &String| text.len() + JSON_STRING_OVERHEAD;
        let content: usize = if exclude_content {
            0
        } else {
            self.sources_content.iter().flatten().map(String::len).sum()
        };
        JSON_FIXED_OVERHEAD
            + self.mappings.len() * ESTIMATED_BYTES_PER_MAPPING
            + content
            + content / ESCAPE_SHARE_DIVISOR
            + self.sources_content.len() * JSON_STRING_OVERHEAD
            + self.sources.iter().map(quoted_len).sum::<usize>()
            + self.names.iter().map(quoted_len).sum::<usize>()
    }

    fn encode_scopes_for_json(&self) -> Option<(String, Vec<String>)> {
        let scopes_info = self.scopes.as_ref()?;
        let mut names = self.names.clone();
        let scopes = srcmap_scopes::encode_scopes(scopes_info, &mut names);
        Some((scopes, names))
    }

    fn write_sources_json(&self, json: &mut String, source_root_prefix: &str) {
        json.push_str(r#","sources":["#);
        for (i, s) in self.sources.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            let source_name = if !source_root_prefix.is_empty() {
                s.strip_prefix(source_root_prefix).unwrap_or(s)
            } else {
                s
            };
            json_quote_into(json, source_name);
        }
        json.push(']');
    }

    fn write_sources_content_json(&self, json: &mut String, exclude_content: bool) {
        if exclude_content
            || self.sources_content.is_empty()
            || !self.sources_content.iter().any(|c| c.is_some())
        {
            return;
        }

        json.push_str(r#","sourcesContent":["#);
        for (i, c) in self.sources_content.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            match c {
                Some(content) => json_quote_into(json, content),
                None => json.push_str("null"),
            }
        }
        json.push(']');
    }

    fn write_ignore_list_json(&self, json: &mut String) {
        if self.ignore_list.is_empty() {
            return;
        }

        use std::fmt::Write;
        json.push_str(r#","ignoreList":["#);
        for (i, &idx) in self.ignore_list.iter().enumerate() {
            if i > 0 {
                json.push(',');
            }
            let _ = write!(json, "{idx}");
        }
        json.push(']');
    }

    /// Construct a `SourceMap` from pre-built parts.
    ///
    /// This avoids the encode-then-decode round-trip used in composition pipelines.
    /// Mappings must be sorted by (generated_line, generated_column).
    /// Use `u32::MAX` for `source`/`name` fields to indicate absence.
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor-style API keeps the hot path allocation-free"
    )]
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
        let line_count = mappings.last().map_or(0, |m| m.generated_line as usize + 1);
        let line_offsets = build_mapping_line_offsets(&mappings, line_count);

        let source_map = build_source_map(&sources);
        let has_range_mappings = mappings.iter().any(|m| m.is_range_mapping);

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
            has_range_mappings,
        }
    }

    /// Construct a `SourceMap` from pre-built parts and extension fields.
    ///
    /// This is the structured interop path for compilers and instrumenters that
    /// already have decoded source-map-v3 data in memory and should not need to
    /// serialize to JSON before using lookup or remapping APIs.
    ///
    /// Mappings must be sorted by (generated_line, generated_column). Use
    /// `u32::MAX` for `source` and `name` fields to indicate absence. Extension
    /// fields are filtered the same way as JSON parsing: only keys with `x_` or
    /// `x-` prefixes are retained.
    #[allow(
        clippy::too_many_arguments,
        reason = "constructor-style API mirrors source-map-v3 fields"
    )]
    pub fn from_parts_with_extensions(
        file: Option<String>,
        source_root: Option<String>,
        sources: Vec<String>,
        sources_content: Vec<Option<String>>,
        names: Vec<String>,
        mappings: Vec<Mapping>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
        scopes: Option<ScopeInfo>,
        extensions: HashMap<String, serde_json::Value>,
    ) -> Self {
        let mut sm = Self::from_parts(
            file,
            source_root,
            sources,
            sources_content,
            names,
            mappings,
            ignore_list,
            debug_id,
            scopes,
        );
        sm.extensions = filter_extensions(extensions);
        sm
    }

    /// Build a source map from pre-parsed components and a VLQ mappings string.
    ///
    /// This is the fast path for WASM: JS does `JSON.parse()` (V8-native speed),
    /// then only the VLQ mappings string crosses into WASM for decoding.
    /// Avoids copying large `sourcesContent` into WASM linear memory.
    #[allow(clippy::too_many_arguments, reason = "WASM bindings pass parsed map parts directly")]
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
        Self::from_vlq_with_range_mappings(
            mappings_str,
            sources,
            names,
            file,
            source_root,
            sources_content,
            ignore_list,
            debug_id,
            None,
        )
    }

    /// Build a source map from pre-parsed components, a VLQ mappings string,
    /// and an optional range mappings string.
    #[allow(
        clippy::too_many_arguments,
        reason = "range mappings are optional but share the same low-level constructor shape"
    )]
    pub fn from_vlq_with_range_mappings(
        mappings_str: &str,
        sources: Vec<String>,
        names: Vec<String>,
        file: Option<String>,
        source_root: Option<String>,
        sources_content: Vec<Option<String>>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
        range_mappings_str: Option<&str>,
    ) -> Result<Self, ParseError> {
        let (mut mappings, line_offsets, checks) =
            decode_mappings(mappings_str, sources.len(), names.len())?;
        let has_range_mappings =
            decode_range_mappings_field(range_mappings_str, &mut mappings, &line_offsets)?;
        sort_unsorted_lines(&mut mappings, &line_offsets, &checks);
        check_indices(&mappings, &checks, sources.len(), names.len(), &ignore_list, None)?;
        let source_map = build_source_map(&sources);
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
            has_range_mappings,
        })
    }

    /// Build a source map from pre-parsed parts and a VLQ mappings string, with
    /// the parse options of [`SourceMap::from_json_with_options`].
    ///
    /// The result and the errors are the same as when `from_json_with_options`
    /// parses a JSON map with these fields and no `sourceRoot`. The parts are
    /// already values, so [`ParseOptions::lenient_names`] has no effect.
    ///
    /// # Examples
    ///
    /// ```
    /// use srcmap_sourcemap::{OutOfRangeNames, ParseOptions, SourceMap};
    ///
    /// let options = ParseOptions {
    ///     out_of_range_names: OutOfRangeNames::Placeholder("?".to_string()),
    ///     ..Default::default()
    /// };
    /// let sm = SourceMap::from_vlq_with_options(
    ///     "AAAAC",
    ///     vec!["a.js".to_string()],
    ///     Vec::new(),
    ///     None,
    ///     Vec::new(),
    ///     Vec::new(),
    ///     &options,
    /// )
    /// .unwrap();
    /// assert_eq!(sm.names, vec!["?"]);
    /// ```
    pub fn from_vlq_with_options(
        mappings_str: &str,
        sources: Vec<String>,
        names: Vec<String>,
        file: Option<String>,
        sources_content: Vec<Option<String>>,
        ignore_list: Vec<u32>,
        options: &ParseOptions,
    ) -> Result<Self, ParseError> {
        let fields = RegularFields {
            file,
            source_root: None,
            sources,
            sources_content,
            names,
            mappings: mappings_str,
            ignore_list: Some(ignore_list),
            x_google_ignore_list: None,
            debug_id: None,
            scopes: None,
            range_mappings: None,
            extensions: HashMap::new(),
        };
        Self::from_fields(fields, options)
    }

    /// Create a builder for incrementally constructing a `SourceMap`.
    ///
    /// The builder accepts iterators for sources, names, and mappings,
    /// avoiding the need to pre-collect into `Vec`s.
    ///
    /// ```
    /// use srcmap_sourcemap::{SourceMap, Mapping};
    ///
    /// let sm = SourceMap::builder()
    ///     .file("output.js")
    ///     .sources(["input.ts"])
    ///     .sources_content([Some("let x = 1;")])
    ///     .names(["x"])
    ///     .mappings([Mapping {
    ///         generated_line: 0,
    ///         generated_column: 0,
    ///         source: 0,
    ///         original_line: 0,
    ///         original_column: 0,
    ///         name: 0,
    ///         is_range_mapping: false,
    ///     }])
    ///     .build();
    ///
    /// assert_eq!(sm.mapping_count(), 1);
    /// ```
    pub fn builder() -> SourceMapBuilder {
        SourceMapBuilder::new()
    }

    /// Parse a source map from JSON, decoding only mappings for lines in `[start_line, end_line)`.
    ///
    /// This is useful for large source maps where only a subset of lines is needed.
    /// VLQ state is maintained through skipped lines (required for correct delta decoding),
    /// but `Mapping` structs are only allocated for lines in the requested range.
    /// Indexed source maps are not supported; use [`SourceMap::from_json`] instead.
    pub fn from_json_lines(json: &str, start_line: u32, end_line: u32) -> Result<Self, ParseError> {
        let fields = RegularFields::parse(json)?;
        let checks = DecodeChecks::new(fields.sources.len(), fields.names.len());
        let decoded = decode_mappings_range(fields.mappings, start_line, end_line, checks)?;
        Self::from_decoded(fields, decoded, &OutOfRangeNames::Reject)
    }

    /// Encode all mappings back to a VLQ mappings string.
    pub fn encode_mappings(&self) -> String {
        let mut encoded = String::new();
        // SAFETY: `encode_mappings_into` appends ASCII bytes only.
        self.encode_mappings_into(unsafe { encoded.as_mut_vec() });
        encoded
    }

    /// Appends the VLQ `mappings` string to `out`. The bytes are ASCII.
    fn encode_mappings_into(&self, out: &mut Vec<u8>) {
        out.reserve(self.mappings.len() * ESTIMATED_BYTES_PER_MAPPING);

        let mut prev_gen_col: u32 = 0;
        let mut prev_source: u32 = 0;
        let mut prev_orig_line: u32 = 0;
        let mut prev_orig_col: u32 = 0;
        let mut prev_name: u32 = 0;
        let mut prev_gen_line: u32 = 0;
        let mut first_in_line = true;

        for m in &self.mappings {
            while prev_gen_line < m.generated_line {
                out.push(b';');
                prev_gen_line += 1;
                prev_gen_col = 0;
                first_in_line = true;
            }

            // One capacity check covers the separator and the 5 values of a segment.
            out.reserve(MAX_SEGMENT_BYTES);

            // SAFETY: the `reserve` above gives room for a separator and 5 VLQ
            // values, and this block writes at most that much.
            unsafe {
                if !first_in_line {
                    let len = out.len();
                    out.as_mut_ptr().add(len).write(b',');
                    out.set_len(len + 1);
                }
                first_in_line = false;

                encode_delta_unchecked(out, m.generated_column, prev_gen_col);
                prev_gen_col = m.generated_column;

                if m.source != NO_SOURCE {
                    encode_delta_unchecked(out, m.source, prev_source);
                    prev_source = m.source;

                    encode_delta_unchecked(out, m.original_line, prev_orig_line);
                    prev_orig_line = m.original_line;

                    encode_delta_unchecked(out, m.original_column, prev_orig_col);
                    prev_orig_col = m.original_column;

                    if m.name != NO_NAME {
                        encode_delta_unchecked(out, m.name, prev_name);
                        prev_name = m.name;
                    }
                }
            }
        }
    }

    pub fn encode_range_mappings(&self) -> Option<String> {
        if !self.has_range_mappings {
            return None;
        }
        let line_count = self.line_offsets.len().saturating_sub(1);
        let mut out: Vec<u8> = Vec::new();
        for line_idx in 0..line_count {
            if line_idx > 0 {
                out.push(b';');
            }
            let start = self.line_offsets[line_idx] as usize;
            let end = self.line_offsets[line_idx + 1] as usize;
            let mut prev_offset: u64 = 0;
            let mut first_on_line = true;
            for (i, mapping) in self.mappings[start..end].iter().enumerate() {
                if mapping.is_range_mapping {
                    if !first_on_line {
                        out.push(b',');
                    }
                    first_on_line = false;
                    vlq_encode_unsigned(&mut out, i as u64 - prev_offset);
                    prev_offset = i as u64;
                }
            }
        }
        while out.last() == Some(&b';') {
            out.pop();
        }
        if out.is_empty() {
            return None;
        }
        debug_assert!(out.is_ascii());
        // SAFETY: vlq_encode_unsigned only pushes ASCII base64 chars,
        // and we only add b';' and b',' — all valid UTF-8.
        Some(unsafe { String::from_utf8_unchecked(out) })
    }

    #[inline]
    pub fn has_range_mappings(&self) -> bool {
        self.has_range_mappings
    }

    #[inline]
    pub fn range_mapping_count(&self) -> usize {
        self.mappings.iter().filter(|m| m.is_range_mapping).count()
    }

    /// Parse a source map from a `data:` URL.
    ///
    /// Supports both base64-encoded and plain JSON data URLs:
    /// - `data:application/json;base64,<base64-encoded-json>`
    /// - `data:application/json;charset=utf-8;base64,<base64-encoded-json>`
    /// - `data:application/json,<json>`
    ///
    /// Returns [`ParseError::InvalidDataUrl`] if the URL format is not recognized
    /// or base64 decoding fails.
    pub fn from_data_url(url: &str) -> Result<Self, ParseError> {
        let rest = url.strip_prefix("data:application/json").ok_or(ParseError::InvalidDataUrl)?;

        // Try base64-encoded variants first
        let json = if let Some(data) = rest
            .strip_prefix(";base64,")
            .or_else(|| rest.strip_prefix(";charset=utf-8;base64,"))
            .or_else(|| rest.strip_prefix(";charset=UTF-8;base64,"))
        {
            base64_decode(data).ok_or(ParseError::InvalidDataUrl)?
        } else if let Some(data) = rest.strip_prefix(',') {
            // Plain JSON — percent-decode if needed
            if data.contains('%') { percent_decode(data) } else { data.to_string() }
        } else {
            return Err(ParseError::InvalidDataUrl);
        };

        Self::from_json(&json)
    }

    /// Serialize the source map JSON to a writer.
    ///
    /// Equivalent to calling [`to_json`](Self::to_json) and writing the result.
    /// The full JSON string is built in memory before writing.
    pub fn to_writer(&self, mut writer: impl io::Write) -> io::Result<()> {
        let json = self.to_json();
        writer.write_all(json.as_bytes())
    }

    /// Serialize the source map JSON to a writer with options.
    ///
    /// If `exclude_content` is true, `sourcesContent` is omitted from the output.
    pub fn to_writer_with_options(
        &self,
        mut writer: impl io::Write,
        exclude_content: bool,
    ) -> io::Result<()> {
        let json = self.to_json_with_options(exclude_content);
        writer.write_all(json.as_bytes())
    }

    /// Serialize the source map to a `data:` URL.
    ///
    /// Format: `data:application/json;base64,<base64-encoded-json>`
    pub fn to_data_url(&self) -> String {
        utils::to_data_url(&self.to_json())
    }

    // ── Mutable setters ─────────────────────────────────────────

    /// Set or clear the `file` property.
    pub fn set_file(&mut self, file: Option<String>) {
        self.file = file;
    }

    /// Set or clear the `sourceRoot` property.
    pub fn set_source_root(&mut self, source_root: Option<String>) {
        self.source_root = source_root;
    }

    /// Set or clear the `debugId` property.
    pub fn set_debug_id(&mut self, debug_id: Option<String>) {
        self.debug_id = debug_id;
    }

    /// Set the `ignoreList` property.
    pub fn set_ignore_list(&mut self, ignore_list: Vec<u32>) {
        self.ignore_list = ignore_list;
    }

    /// Replace the sources array and rebuild the source index lookup map.
    pub fn set_sources(&mut self, sources: Vec<Option<String>>) {
        let source_root = self.source_root.as_deref().unwrap_or("");
        self.sources = resolve_sources(&sources, source_root);
        self.source_map = build_source_map(&self.sources);
        // Invalidate the reverse index since source indices may have changed
        self.reverse_index = OnceCell::new();
    }
}

// ── LazySourceMap ──────────────────────────────────────────────────

/// Cumulative VLQ state at a line boundary.
#[derive(Debug, Clone, Copy, Default)]
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
/// Not thread-safe (`!Sync`). Uses `RefCell`/`Cell` for internal caching.
/// Intended for single-threaded use (WASM) or with external synchronization.
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

    /// Raw `rangeMappings` string (owned), decoded per line on demand.
    raw_range_mappings: Option<String>,

    /// Byte offset of each line's group in `raw_range_mappings`. Empty when
    /// the map has no range mappings.
    range_line_starts: Vec<usize>,

    /// Pre-scanned line info for O(1) line access.
    /// In fast-scan mode, VlqState is zeroed and decoded progressively.
    line_info: Vec<LineInfo>,

    /// Cache of decoded lines: line index -> `Vec<Mapping>`.
    decoded_lines: RefCell<HashMap<u32, Vec<Mapping>>>,

    /// Source filename -> index for O(1) lookup by name.
    source_map: HashMap<String, u32>,

    /// Whether line_info was built with fast-scan (no VLQ state tracking).
    /// If true, decode_line must decode sequentially from the start.
    fast_scan: bool,

    /// Highest line fully decoded so far (for progressive decode in fast-scan mode).
    /// VLQ state at the end of this line is stored in `decode_state`.
    decode_watermark: Cell<u32>,
    decode_state: Cell<VlqState>,
}

impl LazySourceMap {
    #[allow(
        clippy::too_many_arguments,
        reason = "private constructor centralizes shared LazySourceMap setup"
    )]
    fn new_inner(
        file: Option<String>,
        source_root: Option<String>,
        sources: Vec<String>,
        sources_content: Vec<Option<String>>,
        names: Vec<String>,
        ignore_list: Vec<u32>,
        extensions: HashMap<String, serde_json::Value>,
        debug_id: Option<String>,
        scopes: Option<ScopeInfo>,
        raw_mappings: String,
        raw_range_mappings: Option<String>,
        line_info: Vec<LineInfo>,
        source_map: HashMap<String, u32>,
        fast_scan: bool,
    ) -> Self {
        let range_line_starts = raw_range_mappings.as_deref().map_or_else(Vec::new, |s| {
            std::iter::once(0)
                .chain(memchr::memchr_iter(b';', s.as_bytes()).map(|i| i + 1))
                .collect()
        });
        Self {
            file,
            source_root,
            sources,
            sources_content,
            names,
            ignore_list,
            extensions,
            debug_id,
            scopes,
            raw_mappings,
            raw_range_mappings,
            range_line_starts,
            line_info,
            decoded_lines: RefCell::new(HashMap::new()),
            source_map,
            fast_scan,
            decode_watermark: Cell::new(0),
            decode_state: Cell::new(VlqState::default()),
        }
    }

    /// Parse a source map from JSON, deferring VLQ mappings decoding.
    ///
    /// Parses all JSON metadata eagerly but stores the raw mappings string.
    /// VLQ mappings are decoded per-line on demand.
    /// Indexed source maps are not supported; use [`SourceMap::from_json`] instead.
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        let fields = RegularFields::parse(json)?;
        let line_info = prescan_mappings(fields.mappings)?;
        Self::from_fields(fields, line_info, false)
    }

    /// Decode `scopes`, resolve the ignore list, and build the lazy map.
    fn from_fields(
        fields: RegularFields<'_>,
        line_info: Vec<LineInfo>,
        fast_scan: bool,
    ) -> Result<Self, ParseError> {
        let scopes = decode_scopes_field(fields.scopes, &fields.names, fields.sources.len())?;
        let ignore_list = resolve_ignore_list(fields.ignore_list, fields.x_google_ignore_list);
        let source_map = build_source_map(&fields.sources);

        Ok(Self::new_inner(
            fields.file,
            fields.source_root,
            fields.sources,
            fields.sources_content,
            fields.names,
            ignore_list,
            fields.extensions,
            fields.debug_id,
            scopes,
            fields.mappings.to_string(),
            owned_range_mappings(fields.range_mappings),
            line_info,
            source_map,
            fast_scan,
        ))
    }

    /// Parse a source map from JSON, skipping sourcesContent allocation
    /// and deferring VLQ mappings decoding.
    ///
    /// Useful for WASM bindings where sourcesContent is kept on the JS side.
    ///
    /// Returns `ParseError::NestedIndexMap` if the JSON contains `sections`
    /// (indexed source maps are not supported by `LazySourceMap`).
    pub fn from_json_no_content(json: &str) -> Result<Self, ParseError> {
        let fields = RegularFields::parse_lite(json)?;
        let line_info = prescan_mappings(fields.mappings)?;
        Self::from_fields(fields, line_info, false)
    }

    /// Build a lazy source map from pre-parsed components.
    ///
    /// The raw VLQ mappings string is prescanned but not decoded.
    /// sourcesContent is NOT included. Does not support indexed source maps.
    pub fn from_vlq(
        mappings: &str,
        sources: Vec<String>,
        names: Vec<String>,
        file: Option<String>,
        source_root: Option<String>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
    ) -> Result<Self, ParseError> {
        let source_map = build_source_map(&sources);
        let raw_mappings = mappings.to_string();
        let line_info = prescan_mappings(&raw_mappings)?;

        Ok(Self::new_inner(
            file,
            source_root,
            sources,
            Vec::new(),
            names,
            ignore_list,
            HashMap::new(),
            debug_id,
            None,
            raw_mappings,
            None,
            line_info,
            source_map,
            false,
        ))
    }

    /// Parse a source map from JSON using fast-scan mode.
    ///
    /// Only scans for semicolons at construction (no VLQ decode at all).
    /// VLQ state is computed progressively on demand. This gives the fastest
    /// possible parse time at the cost of first-lookup needing sequential decode.
    /// sourcesContent is skipped. Scopes and range mappings are kept.
    ///
    /// Returns `ParseError::NestedIndexMap` if the JSON contains `sections`
    /// (indexed source maps are not supported by `LazySourceMap`).
    pub fn from_json_fast(json: &str) -> Result<Self, ParseError> {
        let fields = RegularFields::parse_lite(json)?;
        let line_info = fast_scan_lines(fields.mappings);
        Self::from_fields(fields, line_info, true)
    }

    /// Decode one line's VLQ segment given the VLQ state at its start; returns
    /// the mappings and the state after the line.
    fn decode_line_with_state(
        &self,
        line: u32,
        mut state: VlqState,
    ) -> Result<(Vec<Mapping>, VlqState), DecodeError> {
        let line_idx = line as usize;
        if line_idx >= self.line_info.len() {
            return Ok((Vec::new(), state));
        }

        let info = &self.line_info[line_idx];
        let bytes = self.raw_mappings.as_bytes();
        let end = info.byte_end;

        let mut mappings = Vec::new();
        let mut generated_column: i64 = 0;
        let mut pos = info.byte_offset;
        let mut checks = DecodeChecks::without_index_limits();

        while pos < end {
            if bytes[pos] == b',' {
                pos += 1;
                continue;
            }
            mappings.push(decode_mapping_segment(
                bytes,
                &mut pos,
                line,
                &mut generated_column,
                &mut state,
                &mut checks,
            )?);
        }
        if let (Some(range_mappings), Some(&start)) =
            (self.raw_range_mappings.as_deref(), self.range_line_starts.get(line_idx))
        {
            let mut pos = start;
            mark_range_line(range_mappings.as_bytes(), &mut pos, &mut mappings)?;
        }
        if checks.unsorted {
            sort_line_by_column(&mut mappings);
        }

        Ok((mappings, state))
    }

    fn ensure_line_decoded(&self, line: u32) -> Result<(), DecodeError> {
        if self.decoded_lines.borrow().contains_key(&line) {
            return Ok(());
        }

        let line_idx = line as usize;
        if line_idx >= self.line_info.len() {
            return Ok(());
        }

        if self.fast_scan {
            let watermark = self.decode_watermark.get();
            let start = if line >= watermark { watermark } else { 0 };
            let mut state =
                if line >= watermark { self.decode_state.get() } else { VlqState::default() };

            for current_line in start..=line {
                let info = &self.line_info[current_line as usize];
                if self.decoded_lines.borrow().contains_key(&current_line) {
                    // Already cached — just walk VLQ bytes to compute end-state
                    let bytes = self.raw_mappings.as_bytes();
                    state = walk_vlq_state(bytes, info.byte_offset, info.byte_end, state)?;
                } else {
                    let (mappings, new_state) = self.decode_line_with_state(current_line, state)?;
                    state = new_state;
                    self.decoded_lines.borrow_mut().insert(current_line, mappings);
                }
            }

            if line + 1 > self.decode_watermark.get() {
                self.decode_watermark.set(line + 1);
                self.decode_state.set(state);
            }

            return Ok(());
        }

        let state = self.line_info[line_idx].state;
        let (mappings, _) = self.decode_line_with_state(line, state)?;
        self.decoded_lines.borrow_mut().insert(line, mappings);
        Ok(())
    }

    /// Decode a single line's mappings on demand.
    ///
    /// Returns the cached result if the line has already been decoded.
    /// The line index is 0-based.
    pub fn decode_line(&self, line: u32) -> Result<Vec<Mapping>, DecodeError> {
        self.ensure_line_decoded(line)?;
        Ok(self.decoded_lines.borrow().get(&line).cloned().unwrap_or_default())
    }

    /// Look up the original source position for a generated position.
    ///
    /// Both `line` and `column` are 0-based.
    /// Returns `None` if no mapping exists or the mapping has no source.
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        let glb = |m: &[Mapping]| original_position_glb_index(m, column).map(|idx| m[idx]);
        // A decoded line needs one map lookup. Only a line that is not decoded yet needs more.
        let found = match self.with_decoded_line(line, glb) {
            Some(found) => found,
            None => {
                self.ensure_line_decoded(line).ok()?;
                self.with_decoded_line(line, glb).flatten()
            }
        };

        let Some(mapping) = found else {
            return self.range_mapping_fallback(line, column);
        };
        if mapping.source == NO_SOURCE {
            return None;
        }
        hit_location(&mapping, column)
    }

    /// Call `f` with the decoded mappings of `line`, or return `None` when `line` is not decoded.
    fn with_decoded_line<R>(&self, line: u32, f: impl FnOnce(&[Mapping]) -> R) -> Option<R> {
        self.decoded_lines.borrow().get(&line).map(|mappings| f(mappings))
    }

    /// Resolve a position with no mapping at or before it on its line through
    /// the last mapping on an earlier line, as [`SourceMap`] does.
    ///
    /// Maps without range mappings return `None` at once and decode nothing.
    fn range_mapping_fallback(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        self.raw_range_mappings.as_ref()?;
        let end = line.min(u32::try_from(self.line_info.len()).unwrap_or(u32::MAX));
        for previous in (0..end).rev() {
            self.ensure_line_decoded(previous).ok()?;
            let decoded_lines = self.decoded_lines.borrow();
            if let Some(last) = decoded_lines.get(&previous).and_then(|m| m.last()) {
                return range_fallback_location(last, line, column);
            }
        }
        None
    }

    /// Number of generated lines in the source map.
    #[inline]
    pub fn line_count(&self) -> usize {
        self.line_info.len()
    }

    /// Resolve a source index to its filename.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds. Use [`get_source`](Self::get_source)
    /// for a non-panicking alternative.
    #[inline]
    pub fn source(&self, index: u32) -> &str {
        &self.sources[index as usize]
    }

    /// Resolve a source index to its filename, returning `None` if out of bounds.
    #[inline]
    pub fn get_source(&self, index: u32) -> Option<&str> {
        self.sources.get(index as usize).map(|s| s.as_str())
    }

    /// Resolve a name index to its string.
    ///
    /// # Panics
    ///
    /// Panics if `index` is out of bounds. Use [`get_name`](Self::get_name)
    /// for a non-panicking alternative.
    #[inline]
    pub fn name(&self, index: u32) -> &str {
        &self.names[index as usize]
    }

    /// Resolve a name index to its string, returning `None` if out of bounds.
    #[inline]
    pub fn get_name(&self, index: u32) -> Option<&str> {
        self.names.get(index as usize).map(|s| s.as_str())
    }

    /// Find the source index for a filename.
    #[inline]
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
        let (mut mappings, line_offsets, checks) =
            decode_mappings(&self.raw_mappings, self.sources.len(), self.names.len())?;
        let has_range_mappings = decode_range_mappings_field(
            self.raw_range_mappings.as_deref(),
            &mut mappings,
            &line_offsets,
        )?;
        sort_unsorted_lines(&mut mappings, &line_offsets, &checks);
        check_indices(
            &mappings,
            &checks,
            self.sources.len(),
            self.names.len(),
            &self.ignore_list,
            self.scopes.as_ref(),
        )?;

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
            has_range_mappings,
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

    let mut state = VlqState::default();
    let mut pos: usize = 0;

    loop {
        let line_start = pos;
        let line_state = state;
        while pos < len && bytes[pos] != b';' {
            pos += 1;
        }
        let byte_end = pos;
        state = walk_vlq_state(bytes, line_start, byte_end, state)?;

        line_info.push(LineInfo { byte_offset: line_start, byte_end, state: line_state });

        if pos >= len {
            break;
        }
        pos += 1;
    }

    Ok(line_info)
}

/// Advance the VLQ state across one line, discarding the decoded mappings.
fn walk_vlq_state(
    bytes: &[u8],
    start: usize,
    end: usize,
    mut state: VlqState,
) -> Result<VlqState, DecodeError> {
    let mut pos = start;
    let mut generated_column: i64 = 0;
    while pos < end {
        if bytes[pos] == b',' {
            pos += 1;
            continue;
        }
        decode_mapping_segment(
            bytes,
            &mut pos,
            0,
            &mut generated_column,
            &mut state,
            &mut DecodeChecks::without_index_limits(),
        )?;
    }
    Ok(state)
}

/// Record line byte offsets by scanning for semicolons only. The VLQ state is
/// left zeroed and computed progressively on demand.
fn fast_scan_lines(input: &str) -> Vec<LineInfo> {
    if input.is_empty() {
        return Vec::new();
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    let mut line_info = Vec::new();
    let mut pos = 0;
    loop {
        let line_start = pos;
        while pos < len && bytes[pos] != b';' {
            pos += 1;
        }

        line_info.push(LineInfo {
            byte_offset: line_start,
            byte_end: pos,
            state: VlqState::default(),
        });

        if pos >= len {
            break;
        }
        pos += 1;
    }

    line_info
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

/// Decode percent-encoded strings (e.g. `%7B` to `{`).
fn percent_decode(input: &str) -> String {
    let mut output = Vec::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            output.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(output).unwrap_or_else(|_| input.to_string())
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

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
    let mappings = sm.all_mappings();
    for m in mappings {
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
    for m in mappings {
        if m.source != NO_SOURCE && m.source as usize >= sm.sources.len() {
            warnings.push(format!(
                "source index {} out of bounds (max {})",
                m.source,
                sm.sources.len()
            ));
        }
        if m.name != NO_NAME && m.name as usize >= sm.names.len() {
            warnings.push(format!("name index {} out of bounds (max {})", m.name, sm.names.len()));
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
    for m in mappings {
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

fn write_string_array_field(out: &mut String, field: &str, values: &[String]) {
    out.push_str(r#",""#);
    out.push_str(field);
    out.push_str(r#"":["#);
    for (i, value) in values.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        json_quote_into(out, value);
    }
    out.push(']');
}

/// Appends the signed VLQ of `value - previous` to `out`. The sign is the low bit.
///
/// # Safety
/// `out` must have at least [`srcmap_codec::MAX_VLQ_BYTES`] bytes of spare capacity.
#[inline(always)]
unsafe fn encode_delta_unchecked(out: &mut Vec<u8>, value: u32, previous: u32) {
    let vlq = (u64::from(value.abs_diff(previous)) << 1) | u64::from(value < previous);
    // SAFETY: the caller guarantees the spare capacity.
    unsafe { srcmap_codec::vlq_encode_unsigned_unchecked(out, vlq) };
}

/// Append a JSON-quoted string to the output buffer.
fn json_quote_into(out: &mut String, s: &str) {
    // SAFETY: `srcmap_codec::json_quote_into` appends the bytes of `s` and
    // ASCII escapes, so the buffer stays valid UTF-8.
    srcmap_codec::json_quote_into(unsafe { out.as_mut_vec() }, s);
}

// ── Internal: decode VLQ mappings directly into flat Mapping vec ───

/// Base64 decode lookup table (byte → 6-bit value, 0xFF = invalid).
///
/// The table covers every byte value, so a lookup needs no separate check for
/// bytes from 128 up.
const B64: [u8; 256] = {
    let mut table = [0xFFu8; 256];
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
    let raw = vlq_unsigned_fast(bytes, pos)?;
    let magnitude = (raw >> 1) as i64;
    Ok(if (raw & 1) != 0 { -magnitude } else { magnitude })
}

#[inline(always)]
fn vlq_unsigned_fast(bytes: &[u8], pos: &mut usize) -> Result<u64, DecodeError> {
    let p = *pos;
    if p >= bytes.len() {
        return Err(DecodeError::UnexpectedEof { offset: p });
    }
    let b0 = bytes[p];
    let d0 = B64[b0 as usize];
    if d0 == 0xFF {
        return Err(DecodeError::InvalidBase64 { byte: b0, offset: p });
    }
    if (d0 & 0x20) == 0 {
        *pos = p + 1;
        return Ok(d0 as u64);
    }
    let mut result: u64 = (d0 & 0x1F) as u64;
    let mut shift: u32 = 5;
    let mut i = p + 1;
    loop {
        if i >= bytes.len() {
            return Err(DecodeError::UnexpectedEof { offset: i });
        }
        let b = bytes[i];
        let d = B64[b as usize];
        if d == 0xFF {
            return Err(DecodeError::InvalidBase64 { byte: b, offset: i });
        }
        i += 1;
        if shift >= 60 {
            return Err(DecodeError::VlqOverflow { offset: p });
        }
        result |= ((d & 0x1F) as u64) << shift;
        shift += 5;
        if (d & 0x20) == 0 {
            break;
        }
    }
    *pos = i;
    Ok(result)
}

/// End of the valid range of a decoded position or index.
///
/// Valid values are `0..u32::MAX`. `u32::MAX` itself is excluded, because
/// `NO_SOURCE` and `NO_NAME` use it as a sentinel.
const POSITION_END: u64 = u32::MAX as u64;

#[cold]
const fn value_out_of_range(offset: usize) -> DecodeError {
    DecodeError::ValueOutOfRange { offset }
}

/// Read one VLQ value at `pos`, add it to `acc`, and return the new value as a position.
///
/// Returns [`DecodeError::ValueOutOfRange`] with the offset of the value when
/// the sum is outside `0..POSITION_END`.
///
/// The decoders stop at the first error, so `acc` is in `0..POSITION_END` on
/// entry. [`vlq_fast`] returns values below 2^60 in magnitude, so the sum
/// cannot overflow `i64`.
#[inline(always)]
fn add_vlq(acc: &mut i64, bytes: &[u8], pos: &mut usize) -> Result<u32, DecodeError> {
    let start = *pos;
    let delta = vlq_fast(bytes, pos)?;
    *acc = acc.wrapping_add(delta);
    // One unsigned compare rejects both negative values and values from `u32::MAX` up.
    if (*acc as u64) < POSITION_END { Ok(*acc as u32) } else { Err(value_out_of_range(start)) }
}

/// Read one generated column like [`add_vlq`], and set `checks.unsorted` when
/// the column is lower than the previous column in `acc`.
#[inline(always)]
fn add_vlq_column(
    acc: &mut i64,
    bytes: &[u8],
    pos: &mut usize,
    checks: &mut DecodeChecks,
) -> Result<u32, DecodeError> {
    let start = *pos;
    let prev = *acc;
    let delta = vlq_fast(bytes, pos)?;
    *acc = prev.wrapping_add(delta);
    // `prev` is in `0..POSITION_END`, so one unsigned compare checks both
    // `delta >= 0` and `prev + delta < POSITION_END`.
    if (delta as u64) < POSITION_END - prev as u64 {
        Ok(*acc as u32)
    } else {
        column_outside_order(*acc, start, checks)
    }
}

/// Handle a column that [`add_vlq_column`] found below the previous column or out of range.
#[cold]
fn column_outside_order(
    value: i64,
    offset: usize,
    checks: &mut DecodeChecks,
) -> Result<u32, DecodeError> {
    if (value as u64) < POSITION_END {
        checks.unsorted = true;
        Ok(value as u32)
    } else {
        Err(value_out_of_range(offset))
    }
}

/// Read one source or name index like [`add_vlq`], and compare it with the
/// `checks` limit for `kind` ([`IndexKind::Source`] or [`IndexKind::Name`]).
///
/// The limit is at most `POSITION_END`. An index from the limit up that is still
/// a valid position sets `checks.index_out_of_range` and decodes as usual, so a
/// later decode error still takes precedence. When `checks.out_of_range_name`
/// is set, a name index outside `names` gives that value. The accumulated name
/// index can then leave `0..POSITION_END`.
#[inline(always)]
fn add_vlq_index(
    acc: &mut i64,
    bytes: &[u8],
    pos: &mut usize,
    kind: IndexKind,
    checks: &mut DecodeChecks,
) -> Result<u32, DecodeError> {
    let limit =
        if matches!(kind, IndexKind::Name) { checks.name_limit } else { checks.source_limit };
    let start = *pos;
    let delta = vlq_fast(bytes, pos)?;
    *acc = acc.wrapping_add(delta);
    // The common case needs one unsigned compare for both checks.
    if (*acc as u64) < limit {
        Ok(*acc as u32)
    } else {
        index_at_or_above_limit(*acc, start, kind, checks)
    }
}

/// Handle an index that [`add_vlq_index`] found at or above its limit.
#[cold]
fn index_at_or_above_limit(
    value: i64,
    offset: usize,
    kind: IndexKind,
    checks: &mut DecodeChecks,
) -> Result<u32, DecodeError> {
    if matches!(kind, IndexKind::Name)
        && let Some(name) = checks.out_of_range_name
    {
        checks.used_out_of_range_name = true;
        return Ok(name);
    }
    if (value as u64) < POSITION_END {
        checks.index_out_of_range = true;
        Ok(value as u32)
    } else {
        Err(value_out_of_range(offset))
    }
}

#[inline(always)]
const fn at_segment_delimiter(bytes: &[u8], pos: usize) -> bool {
    pos >= bytes.len() || bytes[pos] == b',' || bytes[pos] == b';'
}

/// Mark the range mappings of one line, from one `;`-separated `rangeMappings` group.
///
/// Reads from `pos` up to and including the next `;`. Each value adds to an
/// index into `line`, in input segment order. Indices past the end of the line
/// mark nothing.
fn mark_range_line(bytes: &[u8], pos: &mut usize, line: &mut [Mapping]) -> Result<(), DecodeError> {
    let line_len = line.len() as u64;
    let mut mapping_index: u64 = 0;
    while *pos < bytes.len() {
        match bytes[*pos] {
            b';' => {
                *pos += 1;
                break;
            }
            b',' => *pos += 1,
            _ => {
                let offset = vlq_unsigned_fast(bytes, pos)?;
                mapping_index = mapping_index.saturating_add(offset);
                if mapping_index < line_len {
                    line[mapping_index as usize].is_range_mapping = true;
                }
            }
        }
    }
    Ok(())
}

fn decode_range_mappings(
    input: &str,
    mappings: &mut [Mapping],
    line_offsets: &[u32],
) -> Result<(), DecodeError> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut pos: usize = 0;
    let mut generated_line: usize = 0;
    while pos < len {
        let line_start = if generated_line + 1 < line_offsets.len() {
            line_offsets[generated_line] as usize
        } else {
            break;
        };
        // Bound range marking to this line's mappings only
        let line_end = if generated_line + 2 < line_offsets.len() {
            line_offsets[generated_line + 1] as usize
        } else {
            mappings.len()
        };
        let line = mappings.get_mut(line_start..line_end).unwrap_or_default();
        mark_range_line(bytes, &mut pos, line)?;
        generated_line += 1;
    }
    Ok(())
}

/// Decode one segment at `pos`, and record unsorted columns and out-of-range
/// indices in `checks`.
fn decode_mapping_segment(
    bytes: &[u8],
    pos: &mut usize,
    generated_line: u32,
    generated_column: &mut i64,
    state: &mut VlqState,
    checks: &mut DecodeChecks,
) -> Result<Mapping, DecodeError> {
    let column = add_vlq_column(generated_column, bytes, pos, checks)?;

    if at_segment_delimiter(bytes, *pos) {
        return Ok(generated_only_mapping(generated_line, column));
    }

    let source = add_vlq_index(&mut state.source_index, bytes, pos, IndexKind::Source, checks)?;

    // Reject 2-field segments (only 1, 4, or 5 are valid per ECMA-426)
    if at_segment_delimiter(bytes, *pos) {
        return Err(DecodeError::InvalidSegmentLength { fields: 2, offset: *pos });
    }

    let original_line = add_vlq(&mut state.original_line, bytes, pos)?;

    // Reject 3-field segments (only 1, 4, or 5 are valid per ECMA-426)
    if at_segment_delimiter(bytes, *pos) {
        return Err(DecodeError::InvalidSegmentLength { fields: 3, offset: *pos });
    }

    let original_column = add_vlq(&mut state.original_column, bytes, pos)?;

    let name = if at_segment_delimiter(bytes, *pos) {
        NO_NAME
    } else {
        let name = add_vlq_index(&mut state.name_index, bytes, pos, IndexKind::Name, checks)?;
        // Reject 6-field segments (only 1, 4, or 5 are valid per ECMA-426)
        if !at_segment_delimiter(bytes, *pos) {
            return Err(DecodeError::InvalidSegmentLength { fields: 6, offset: *pos });
        }
        name
    };

    Ok(Mapping {
        generated_line,
        generated_column: column,
        source,
        original_line,
        original_column,
        name,
        is_range_mapping: false,
    })
}

fn build_range_line_offsets(
    start_line: u32,
    end_line: u32,
    line_starts: &[(u32, u32)],
    total: u32,
) -> Vec<u32> {
    let mut line_offsets: Vec<u32> = vec![total; end_line as usize + 1];

    for offset in line_offsets.iter_mut().take(start_line as usize + 1) {
        *offset = 0;
    }

    for &(line, offset) in line_starts {
        line_offsets[line as usize] = offset;
    }

    let mut next_offset = total;
    for i in (start_line as usize..end_line as usize).rev() {
        if line_offsets[i] == total {
            line_offsets[i] = next_offset;
        } else {
            next_offset = line_offsets[i];
        }
    }

    line_offsets
}

/// Sort one line's segments by generated column when they are out of order.
///
/// Lookups binary-search each line by column. The sort is stable, so segments
/// that share a column keep their input order.
fn sort_line_by_column(line: &mut [Mapping]) {
    if !line.is_sorted_by_key(|m| m.generated_column) {
        line.sort_by_key(|m| m.generated_column);
    }
}

/// Sort each line of decoded mappings with [`sort_line_by_column`].
///
/// Call this after range marking, because `rangeMappings` refers to segments
/// by their input position in a line. Does nothing when `checks` shows that
/// every line is in order.
fn sort_unsorted_lines(mappings: &mut [Mapping], line_offsets: &[u32], checks: &DecodeChecks) {
    if !checks.unsorted {
        return;
    }
    for window in line_offsets.windows(2) {
        let (start, end) = (window[0] as usize, window[1] as usize);
        if let Some(line) = mappings.get_mut(start..end) {
            sort_line_by_column(line);
        }
    }
}

/// Decode all mappings, and check source and name indices against
/// `sources_len` and `names_len` (see [`DecodeChecks`]).
fn decode_mappings(
    input: &str,
    sources_len: usize,
    names_len: usize,
) -> Result<(Vec<Mapping>, Vec<u32>, DecodeChecks), DecodeError> {
    decode_mappings_with(input, DecodeChecks::new(sources_len, names_len))
}

/// Decode all mappings like [`decode_mappings`], with the limits and modes of `checks`.
fn decode_mappings_with(
    input: &str,
    mut checks: DecodeChecks,
) -> Result<(Vec<Mapping>, Vec<u32>, DecodeChecks), DecodeError> {
    if input.is_empty() {
        return Ok((Vec::new(), vec![0], checks));
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    // Pre-count lines and segments for capacity hints. memchr's count is
    // SIMD-accelerated; a scalar byte loop here is not auto-vectorized and
    // measured at ~18% of total decode time on large maps (~7x slower).
    let semicolons = memchr::memchr_iter(b';', bytes).count();
    let commas = memchr::memchr_iter(b',', bytes).count();
    let line_count = semicolons + 1;
    // Each segment holds at least one byte that is not a delimiter.
    let approx_segments = (commas + line_count).min(len - commas - semicolons);

    let mut mappings: Vec<Mapping> = Vec::with_capacity(approx_segments);
    let mut line_offsets: Vec<u32> = Vec::with_capacity(line_count + 1);

    let mut state = VlqState::default();
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

            mappings.push(decode_mapping_segment(
                bytes,
                &mut pos,
                generated_line,
                &mut generated_column,
                &mut state,
                &mut checks,
            )?);
        }

        if !saw_semicolon {
            break;
        }
        generated_line += 1;
    }

    // Sentinel for line range computation
    line_offsets.push(mappings.len() as u32);

    Ok((mappings, line_offsets, checks))
}

fn actual_mapping_line_count(input: &str) -> u32 {
    if input.is_empty() {
        return 0;
    }

    memchr::memchr_iter(b';', input.as_bytes()).count() as u32 + 1
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
    mut checks: DecodeChecks,
) -> Result<(Vec<Mapping>, Vec<u32>, DecodeChecks), DecodeError> {
    // Cap end_line against actual line count to prevent OOM on pathological input.
    let actual_lines = actual_mapping_line_count(input);
    let end_line = end_line.min(actual_lines);

    if input.is_empty() || start_line >= end_line {
        return Ok((Vec::new(), vec![0; end_line as usize + 1], checks));
    }

    let bytes = input.as_bytes();
    let len = bytes.len();

    let mut mappings: Vec<Mapping> = Vec::new();

    let mut state = VlqState::default();
    let mut generated_line: u32 = 0;
    let mut pos: usize = 0;

    let mut line_starts: Vec<(u32, u32)> =
        Vec::with_capacity((end_line - start_line).min(actual_lines) as usize);

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

            // Lines before `start_line` also update `checks`. That can only cause
            // an extra index scan or sort pass, never a missed error.
            let mapping = decode_mapping_segment(
                bytes,
                &mut pos,
                generated_line,
                &mut generated_column,
                &mut state,
                &mut checks,
            )?;
            if in_range {
                mappings.push(mapping);
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

    let total = mappings.len() as u32;
    Ok((mappings, build_range_line_offsets(start_line, end_line, &line_starts, total), checks))
}

/// Build reverse index: mapping indices sorted by (source, original_line, original_column).
fn build_reverse_index(mappings: &[Mapping]) -> Vec<u32> {
    let mut indices: Vec<u32> =
        (0..mappings.len() as u32).filter(|&i| mappings[i as usize].source != NO_SOURCE).collect();

    indices.sort_unstable_by(|&a, &b| {
        let ma = &mappings[a as usize];
        let mb = &mappings[b as usize];
        ma.source
            .cmp(&mb.source)
            .then(ma.original_line.cmp(&mb.original_line))
            .then(ma.original_column.cmp(&mb.original_column))
            .then(ma.generated_line.cmp(&mb.generated_line))
            .then(ma.generated_column.cmp(&mb.generated_column))
    });

    indices
}

// ── Streaming iterator ────────────────────────────────────────────

/// Lazy iterator over VLQ-encoded source map mappings.
///
/// Decodes one mapping at a time without allocating a full `Vec<Mapping>`.
/// Useful for streaming composition pipelines where intermediate allocation
/// is undesirable.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::MappingsIter;
///
/// let vlq = "AAAA;AACA,EAAA;AACA";
/// let mappings: Vec<_> = MappingsIter::new(vlq).collect::<Result<_, _>>().unwrap();
/// assert_eq!(mappings.len(), 4);
/// assert_eq!(mappings[0].generated_line, 0);
/// assert_eq!(mappings[1].generated_line, 1);
/// ```
pub struct MappingsIter<'a> {
    bytes: &'a [u8],
    len: usize,
    pos: usize,
    source_index: i64,
    original_line: i64,
    original_column: i64,
    name_index: i64,
    generated_line: u32,
    generated_column: i64,
    done: bool,
}

impl<'a> MappingsIter<'a> {
    /// Create a new iterator over VLQ-encoded mappings.
    pub fn new(vlq: &'a str) -> Self {
        let bytes = vlq.as_bytes();
        Self {
            bytes,
            len: bytes.len(),
            pos: 0,
            source_index: 0,
            original_line: 0,
            original_column: 0,
            name_index: 0,
            generated_line: 0,
            generated_column: 0,
            done: false,
        }
    }

    fn stop_with_error(&mut self, error: DecodeError) -> Option<Result<Mapping, DecodeError>> {
        self.done = true;
        Some(Err(error))
    }

    fn at_segment_delimiter(&self) -> bool {
        at_segment_delimiter(self.bytes, self.pos)
    }

    fn decode_sourced_mapping(&mut self, generated_column: u32) -> Result<Mapping, DecodeError> {
        let source = add_vlq(&mut self.source_index, self.bytes, &mut self.pos)?;
        if self.at_segment_delimiter() {
            return Err(DecodeError::InvalidSegmentLength { fields: 2, offset: self.pos });
        }

        let original_line = add_vlq(&mut self.original_line, self.bytes, &mut self.pos)?;
        if self.at_segment_delimiter() {
            return Err(DecodeError::InvalidSegmentLength { fields: 3, offset: self.pos });
        }

        let original_column = add_vlq(&mut self.original_column, self.bytes, &mut self.pos)?;
        let name = if self.at_segment_delimiter() {
            NO_NAME
        } else {
            add_vlq(&mut self.name_index, self.bytes, &mut self.pos)?
        };

        if !self.at_segment_delimiter() {
            return Err(DecodeError::InvalidSegmentLength { fields: 6, offset: self.pos });
        }

        Ok(Mapping {
            generated_line: self.generated_line,
            generated_column,
            source,
            original_line,
            original_column,
            name,
            is_range_mapping: false,
        })
    }
}

impl Iterator for MappingsIter<'_> {
    type Item = Result<Mapping, DecodeError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            if self.pos >= self.len {
                self.done = true;
                return None;
            }

            let byte = self.bytes[self.pos];

            if byte == b';' {
                self.pos += 1;
                self.generated_line += 1;
                self.generated_column = 0;
                continue;
            }

            if byte == b',' {
                self.pos += 1;
                continue;
            }

            // Field 1: generated column
            let generated_column =
                match add_vlq(&mut self.generated_column, self.bytes, &mut self.pos) {
                    Ok(column) => column,
                    Err(e) => return self.stop_with_error(e),
                };

            if self.at_segment_delimiter() {
                return Some(Ok(generated_only_mapping(self.generated_line, generated_column)));
            }

            return match self.decode_sourced_mapping(generated_column) {
                Ok(mapping) => Some(Ok(mapping)),
                Err(e) => self.stop_with_error(e),
            };
        }
    }
}

// ── Builder ────────────────────────────────────────────────────────

/// Builder for incrementally constructing a [`SourceMap`] from iterators.
///
/// Avoids the need to pre-collect sources, names, and mappings into `Vec`s.
/// Delegates to [`SourceMap::from_parts`] internally.
#[must_use]
pub struct SourceMapBuilder {
    file: Option<String>,
    source_root: Option<String>,
    sources: Vec<String>,
    sources_content: Vec<Option<String>>,
    names: Vec<String>,
    mappings: Vec<Mapping>,
    ignore_list: Vec<u32>,
    debug_id: Option<String>,
    scopes: Option<ScopeInfo>,
    extensions: HashMap<String, serde_json::Value>,
}

impl SourceMapBuilder {
    /// Create an empty source map builder.
    pub fn new() -> Self {
        Self {
            file: None,
            source_root: None,
            sources: Vec::new(),
            sources_content: Vec::new(),
            names: Vec::new(),
            mappings: Vec::new(),
            ignore_list: Vec::new(),
            debug_id: None,
            scopes: None,
            extensions: HashMap::new(),
        }
    }

    /// Set the generated file name.
    pub fn file(mut self, file: impl Into<String>) -> Self {
        self.file = Some(file.into());
        self
    }

    /// Set the source root prefix.
    pub fn source_root(mut self, root: impl Into<String>) -> Self {
        self.source_root = Some(root.into());
        self
    }

    /// Replace the source list.
    pub fn sources(mut self, sources: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.sources = sources.into_iter().map(Into::into).collect();
        self
    }

    /// Replace the source content list. Entries are parallel to `sources`.
    pub fn sources_content(
        mut self,
        content: impl IntoIterator<Item = Option<impl Into<String>>>,
    ) -> Self {
        self.sources_content = content.into_iter().map(|c| c.map(Into::into)).collect();
        self
    }

    /// Replace the name list.
    pub fn names(mut self, names: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.names = names.into_iter().map(Into::into).collect();
        self
    }

    /// Replace the decoded mapping list.
    ///
    /// Mappings must be sorted by (generated_line, generated_column).
    pub fn mappings(mut self, mappings: impl IntoIterator<Item = Mapping>) -> Self {
        self.mappings = mappings.into_iter().collect();
        self
    }

    /// Replace the ignore-list source indices.
    pub fn ignore_list(mut self, list: impl IntoIterator<Item = u32>) -> Self {
        self.ignore_list = list.into_iter().collect();
        self
    }

    /// Set the source map debug ID.
    pub fn debug_id(mut self, id: impl Into<String>) -> Self {
        self.debug_id = Some(id.into());
        self
    }

    /// Set ECMA-426 scopes data.
    pub fn scopes(mut self, scopes: ScopeInfo) -> Self {
        self.scopes = Some(scopes);
        self
    }

    /// Add one extension field.
    ///
    /// Only `x_` and `x-` extension keys are retained in the built `SourceMap`,
    /// matching JSON parsing behavior.
    pub fn extension(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.extensions.insert(key.into(), value);
        self
    }

    /// Replace extension fields.
    ///
    /// Only `x_` and `x-` extension keys are retained in the built `SourceMap`,
    /// matching JSON parsing behavior.
    pub fn extensions<K, I>(mut self, extensions: I) -> Self
    where
        K: Into<String>,
        I: IntoIterator<Item = (K, serde_json::Value)>,
    {
        self.extensions = extensions.into_iter().map(|(k, v)| (k.into(), v)).collect();
        self
    }

    /// Consume the builder and produce a [`SourceMap`].
    ///
    /// Mappings must be sorted by (generated_line, generated_column).
    pub fn build(self) -> SourceMap {
        SourceMap::from_parts_with_extensions(
            self.file,
            self.source_root,
            self.sources,
            self.sources_content,
            self.names,
            self.mappings,
            self.ignore_list,
            self.debug_id,
            self.scopes,
            self.extensions,
        )
    }
}

impl Default for SourceMapBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The tie-break of `@jridgewell/trace-mapping`: the earliest mapping at
    /// `column` on an exact match, otherwise the last mapping before `column`.
    fn linear_glb_index(line_mappings: &[Mapping], column: u32) -> Option<usize> {
        line_mappings
            .iter()
            .position(|m| m.generated_column == column)
            .or_else(|| line_mappings.iter().rposition(|m| m.generated_column < column))
    }

    #[test]
    fn glb_index_matches_a_linear_search_for_every_line_shape() {
        // Each bit of `shape` says if a column repeats the column before it,
        // so the lines cover every pattern of equal columns up to 9 segments.
        for len in 0..=9u32 {
            for shape in 0..(1u32 << len.saturating_sub(1)) {
                let mut column = 2;
                let line: Vec<Mapping> = (0..len)
                    .map(|i| {
                        if i > 0 && shape & (1 << (i - 1)) == 0 {
                            column += 3;
                        }
                        generated_only_mapping(0, column)
                    })
                    .collect();
                for query in 0..=column + 2 {
                    assert_eq!(
                        original_position_glb_index(&line, query),
                        linear_glb_index(&line, query),
                        "len {len}, shape {shape:b}, column {query}"
                    );
                }
            }
        }
    }

    fn simple_map() -> &'static str {
        r#"{"version":3,"sources":["input.js"],"names":["hello"],"mappings":"AAAA;AACA,EAAA;AACA"}"#
    }

    fn indexed_map() -> &'static str {
        r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}}]}"#
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
                _ => panic!("lookup mismatch at ({}, {})", m.generated_line, m.generated_column),
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
    fn original_position_for_nonexistent_line() {
        let sm = SourceMap::from_json(simple_map()).unwrap();
        assert!(sm.original_position_for(999, 0).is_none());
    }

    #[test]
    fn original_position_for_duplicate_column_prefers_first_segment() {
        // Regression: when two segments share a generated column and the second
        // is a single-value segment (no source), GLB/default lookup must return
        // the first (source-bearing) segment, not the second. `@jridgewell/trace-mapping`
        // specifies GLB = earliest-equal tie-break; Rust's `binary_search_by_key`
        // returns an unspecified index among duplicates and previously picked the
        // NO_SOURCE segment, breaking drop-in parity.
        //
        // VLQ "AASA,A;AAIA,C;AAIA" decodes to:
        //   line 0: [(col=0, src=0, orig_line=9, orig_col=0), (col=0)]   <- duplicate col
        //   line 1: [(col=0, src=0, orig_line=13, orig_col=0), (col=1)]
        //   line 2: [(col=0, src=0, orig_line=17, orig_col=0)]
        let json = r#"{
            "version":3,
            "sources":["src/original.ts"],
            "names":["originalFn","helperFn"],
            "mappings":"AASA,A;AAIA,C;AAIA"
        }"#;
        let sm = SourceMap::from_json(json).unwrap();

        // GLB (default) at (0, 0) must pick the source-bearing segment.
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.source, 0);
        assert_eq!(loc.line, 9);
        assert_eq!(loc.column, 0);

        // LUB at (0, 0) walks forward to the latest duplicate — the NO_SOURCE
        // segment — and returns None, matching `@jridgewell`'s OMapping(null, …).
        assert!(sm.original_position_for_with_bias(0, 0, Bias::LeastUpperBound).is_none());
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

    fn assert_index_error(json: &str, want_kind: IndexKind, want_index: u32, want_len: usize) {
        let err = SourceMap::from_json(json).unwrap_err();
        match err {
            ParseError::IndexOutOfRange { kind, index, len } => {
                assert_eq!((kind, index, len), (want_kind, want_index, want_len), "{json}");
            }
            other => panic!("expected IndexOutOfRange for {json}, got {other:?}"),
        }
    }

    #[test]
    fn rejects_source_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA"}"#,
            IndexKind::Source,
            1,
            1,
        );
    }

    #[test]
    fn rejects_name_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAAA"}"#,
            IndexKind::Name,
            0,
            0,
        );
    }

    #[test]
    fn rejects_ignore_list_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","ignoreList":[5]}"#,
            IndexKind::IgnoreList,
            5,
            1,
        );
    }

    #[test]
    fn rejects_legacy_ignore_list_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_google_ignoreList":[1]}"#,
            IndexKind::IgnoreList,
            1,
            1,
        );
    }

    #[test]
    fn rejects_section_source_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a"],"names":[],"mappings":"ACAA"}}]}"#,
            IndexKind::Source,
            1,
            1,
        );
    }

    #[test]
    fn rejects_section_ignore_list_index_out_of_range() {
        assert_index_error(
            r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a"],"names":[],"mappings":"AAAA","ignoreList":[5]}}]}"#,
            IndexKind::IgnoreList,
            5,
            1,
        );
    }

    #[test]
    fn accepts_indices_at_last_position() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":["x","y"],"mappings":"AAAAA,CCAAC","ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 2);
        let loc = sm.original_position_for(0, 1).unwrap();
        assert_eq!((loc.source, loc.name), (1, Some(1)));
    }

    #[test]
    fn rejects_out_of_range_indices_in_every_entry_point() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA"}"#;
        let is_source_error = |r: Result<SourceMap, ParseError>| {
            matches!(
                r,
                Err(ParseError::IndexOutOfRange { kind: IndexKind::Source, index: 1, len: 1 })
            )
        };
        assert!(is_source_error(SourceMap::from_json_no_content(json)));
        assert!(is_source_error(SourceMap::from_json_lines(json, 0, 1)));
        assert!(is_source_error(SourceMap::from_data_url(&format!(
            "data:application/json,{json}"
        ))));
        assert!(is_source_error(SourceMap::from_vlq(
            "ACAA",
            vec!["a.js".into()],
            vec![],
            None,
            None,
            vec![],
            vec![],
            None,
        )));
        assert!(matches!(
            SourceMap::from_vlq(
                "AAAA",
                vec!["a.js".into()],
                vec![],
                None,
                None,
                vec![],
                vec![3],
                None,
            ),
            Err(ParseError::IndexOutOfRange { kind: IndexKind::IgnoreList, index: 3, len: 1 })
        ));
    }

    #[test]
    fn lazy_into_sourcemap_rejects_out_of_range_indices() {
        let into = |json: &str| LazySourceMap::from_json(json).unwrap().into_sourcemap();
        assert!(matches!(
            into(r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA"}"#),
            Err(ParseError::IndexOutOfRange { kind: IndexKind::Source, index: 1, len: 1 })
        ));
        assert!(matches!(
            into(r#"{"version":3,"sources":["a.js"],"names":["n"],"mappings":"AAAAC"}"#),
            Err(ParseError::IndexOutOfRange { kind: IndexKind::Name, index: 1, len: 1 })
        ));
    }

    fn one_scope_info(range: GeneratedRange) -> ScopeInfo {
        ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![range],
        }
    }

    fn base_range() -> GeneratedRange {
        GeneratedRange {
            start: Position { line: 0, column: 0 },
            end: Position { line: 5, column: 0 },
            is_stack_frame: false,
            is_hidden: false,
            definition: Some(0),
            call_site: None,
            bindings: vec![],
            children: vec![],
        }
    }

    fn scopes_json(info: &ScopeInfo) -> String {
        let scopes_str = srcmap_scopes::encode_scopes(info, &mut vec![]);
        format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        )
    }

    #[test]
    fn accepts_scopes_definition_and_call_site_in_range() {
        let mut inner = base_range();
        inner.call_site = Some(CallSite { source_index: 0, line: 0, column: 0 });
        let mut outer = base_range();
        outer.children = vec![inner];
        let sm = SourceMap::from_json(&scopes_json(&one_scope_info(outer))).unwrap();
        assert_eq!(sm.scopes.unwrap().ranges[0].children.len(), 1);
    }

    fn nested_scopes_json(depth: usize) -> String {
        // One original scope per level: `B` opens a scope at (0, 0), `C` closes it.
        let scopes = [vec!["BAAA"; depth], vec!["CAA"; depth]].concat().join(",");
        format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes}"}}"#
        )
    }

    #[test]
    fn original_scopes_at_the_nesting_limit_parse_and_drop_on_a_small_stack() {
        let json = nested_scopes_json(srcmap_scopes::MAX_NESTING_DEPTH);
        let mapping_count = std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(move || {
                SourceMap::from_json(&json).map(|sm| sm.mappings.len()).map_err(|e| e.to_string())
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(mapping_count, Ok(1));
    }

    #[test]
    fn rejects_original_scopes_above_the_nesting_limit() {
        let json = nested_scopes_json(100_000);
        let err = SourceMap::from_json(&json).unwrap_err();
        assert!(matches!(err, ParseError::Scopes(srcmap_scopes::ScopesError::NestingTooDeep)));
    }

    #[test]
    fn rejects_scopes_definition_out_of_range() {
        let mut inner = base_range();
        inner.definition = Some(1);
        let mut outer = base_range();
        outer.children = vec![inner];
        assert_index_error(&scopes_json(&one_scope_info(outer)), IndexKind::ScopeDefinition, 1, 1);
    }

    #[test]
    fn rejects_scopes_call_site_source_out_of_range() {
        let mut inner = base_range();
        inner.call_site = Some(CallSite { source_index: 1, line: 0, column: 0 });
        let mut outer = base_range();
        outer.children = vec![inner];
        assert_index_error(&scopes_json(&one_scope_info(outer)), IndexKind::CallSiteSource, 1, 1);
    }

    fn indexed_map_with_offset(line: u32, column: u32, mappings: &str) -> String {
        format!(
            r#"{{"version":3,"sections":[{{"offset":{{"line":{line},"column":{column}}},"map":{{"version":3,"sources":["a"],"names":[],"mappings":"{mappings}"}}}}]}}"#
        )
    }

    #[test]
    fn rejects_section_offset_line_above_bound() {
        let json = indexed_map_with_offset(4_000_000_000, 0, "AAAA");
        let err = SourceMap::from_json(&json).unwrap_err();
        assert!(matches!(err, ParseError::SectionOffsetOutOfRange), "{err:?}");
    }

    #[test]
    fn rejects_section_column_offset_overflow() {
        let json = indexed_map_with_offset(0, u32::MAX, "CAAA");
        let err = SourceMap::from_json(&json).unwrap_err();
        assert!(matches!(err, ParseError::SectionOffsetOutOfRange), "{err:?}");
    }

    #[test]
    fn accepts_section_offset_below_bound() {
        let sm = SourceMap::from_json(&indexed_map_with_offset(100_000, 0, "AAAA")).unwrap();
        assert!(sm.original_position_for(100_000, 0).is_some());
    }

    #[test]
    fn accepts_empty_section_with_large_offset() {
        let json = r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a"],"names":[],"mappings":"AAAA"}},{"offset":{"line":4000000000,"column":0},"map":{"version":3,"sources":[],"names":[],"mappings":""}}]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.line_count(), 1);
    }

    /// A running VLQ sum that leaves the `u32` range fails before it can overflow `i64`.
    const fn is_sum_range_error(err: &ParseError) -> bool {
        matches!(
            err,
            ParseError::Vlq(DecodeError::VlqOverflow { .. } | DecodeError::ValueOutOfRange { .. })
        )
    }

    fn sum_overflow_map(segment: &str) -> String {
        let mappings = [segment; 40].join(",");
        format!(r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"{mappings}"}}"#)
    }

    #[test]
    fn rejects_generated_column_sum_overflow() {
        let err = SourceMap::from_json(&sum_overflow_map("gggggggggggQ")).unwrap_err();
        assert!(is_sum_range_error(&err), "{err:?}");
    }

    #[test]
    fn rejects_source_index_sum_overflow() {
        let err = SourceMap::from_json(&sum_overflow_map("AgggggggggggQAA")).unwrap_err();
        assert!(is_sum_range_error(&err), "{err:?}");
    }

    #[test]
    fn mappings_iter_reports_sum_overflow() {
        let mappings = ["gggggggggggQ"; 40].join(",");
        let result = MappingsIter::new(&mappings).collect::<Result<Vec<_>, _>>();
        assert!(
            matches!(
                result,
                Err(DecodeError::VlqOverflow { .. } | DecodeError::ValueOutOfRange { .. })
            ),
            "{result:?}"
        );
    }

    #[test]
    fn ignore_list_merge_is_linear() {
        const COUNT: u32 = 20_000;
        let sources: Vec<String> = (0..COUNT).map(|i| format!("\"s{i}\"")).collect();
        let ignore: Vec<String> = (0..COUNT).map(|i| i.to_string()).collect();
        let json = format!(
            r#"{{"version":3,"sections":[{{"offset":{{"line":0,"column":0}},"map":{{"version":3,"sources":[{}],"names":[],"mappings":"AAAA","ignoreList":[{}]}}}}]}}"#,
            sources.join(","),
            ignore.join(","),
        );
        let sm = SourceMap::from_json(&json).unwrap();
        assert_eq!(sm.ignore_list, (0..COUNT).collect::<Vec<_>>());
    }

    #[test]
    fn range_mapping_index_sum_does_not_wrap() {
        let range_mappings = ["gggggggggggQ"; 40].join(",");
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","rangeMappings":"{range_mappings}"}}"#
        );
        let sm = SourceMap::from_json(&json).unwrap();
        assert!(!sm.has_range_mappings());
    }

    #[test]
    fn range_mapping_lookup_column_overflow_returns_none() {
        // Original column u32::MAX - 1, so any column delta above 1 overflows
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAA8/////H","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.original_position_for(0, 1).unwrap().column, u32::MAX);
        assert!(sm.original_position_for(0, 5).is_none());
    }

    #[test]
    fn range_mapping_fallback_line_overflow_returns_none() {
        // Original line u32::MAX - 1 on generated line 0; generated line 2 overflows it
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AA8/////HA","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.original_position_for(1, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_none());
    }

    fn vlq_error(mappings: &str, names: &str) -> DecodeError {
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[{names}],"mappings":"{mappings}"}}"#
        );
        match SourceMap::from_json(&json).unwrap_err() {
            ParseError::Vlq(err) => err,
            other => panic!("expected a VLQ error for {mappings}, got {other:?}"),
        }
    }

    #[test]
    fn rejects_negative_source_index() {
        assert!(matches!(vlq_error("ADAA", ""), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_negative_generated_column() {
        assert!(matches!(vlq_error("DAAA", ""), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_generated_column_above_u32() {
        assert!(matches!(vlq_error("AAAA,ggggggIAAA", ""), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_original_line_above_u32() {
        assert!(matches!(vlq_error("AAggggggIA", ""), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_original_column_and_name_above_u32() {
        assert!(matches!(vlq_error("AAAggggggI", ""), DecodeError::ValueOutOfRange { .. }));
        assert!(matches!(vlq_error("AAAAggggggI", r#""n""#), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_u32_max_sentinel_value() {
        // `NO_SOURCE` uses u32::MAX as its sentinel
        let u32_max = "+/////H";
        let mappings = format!("A{u32_max}AA");
        assert!(matches!(vlq_error(&mappings, ""), DecodeError::ValueOutOfRange { .. }));
    }

    #[test]
    fn rejects_six_field_segment() {
        assert!(matches!(
            vlq_error("AAAAAA", r#""n""#),
            DecodeError::InvalidSegmentLength { fields: 6, .. }
        ));
    }

    #[test]
    fn accepts_large_valid_values() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,ggggggEAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.all_mappings()[1].generated_column, 2_147_483_648);
    }

    #[test]
    fn accepts_negative_delta_with_non_negative_value() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"UAAA,DACA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.mapping_count(), 2);
    }

    #[test]
    fn mappings_iter_rejects_negative_source() {
        let result: Result<Vec<_>, _> = MappingsIter::new("ADAA").collect();
        assert!(matches!(result, Err(DecodeError::ValueOutOfRange { .. })), "{result:?}");
    }

    #[test]
    fn mappings_iter_rejects_negative_generated_column() {
        let result: Result<Vec<_>, _> = MappingsIter::new("D").collect();
        assert!(matches!(result, Err(DecodeError::ValueOutOfRange { .. })), "{result:?}");
    }

    #[test]
    fn mappings_iter_keeps_name_index_without_names_check() {
        // `remap_streaming` reads names through this iterator and looks each one up.
        let mappings: Vec<_> = MappingsIter::new("AAAAE").collect::<Result<_, _>>().unwrap();
        assert_eq!(mappings[0].name, 2);
    }

    #[test]
    fn mappings_iter_rejects_six_fields() {
        let result: Result<Vec<_>, _> = MappingsIter::new("AAAAAA").collect();
        assert!(
            matches!(result, Err(DecodeError::InvalidSegmentLength { fields: 6, .. })),
            "{result:?}"
        );
    }

    #[test]
    fn lazy_rejects_negative_source() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ADAA"}"#;
        // The lazy constructor walks the VLQ state of every line, so it reports the error
        let err = LazySourceMap::from_json(json).unwrap_err();
        assert!(matches!(err, ParseError::Vlq(DecodeError::ValueOutOfRange { .. })), "{err:?}");
    }

    // The codec decodes any i64 value; the sourcemap crate requires u32 positions.
    #[test]
    fn codec_and_sourcemap_value_range_differ_by_design() {
        let mut encoded = Vec::new();
        srcmap_codec::vlq_encode(&mut encoded, 1_i64 << 60);
        let mappings = format!("A{}AA", String::from_utf8(encoded).unwrap());
        assert!(srcmap_codec::decode(&mappings).is_ok());
        let json =
            format!(r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"{mappings}"}}"#);
        assert!(SourceMap::from_json(&json).is_err());
    }

    const UNSORTED_LINE_JSON: &str =
        r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"UAAA,DACA,DACA,DACA"}"#;
    const UNSORTED_LINE_EXPECTED: [(u32, u32); 4] = [(7, 3), (8, 2), (9, 1), (10, 0)];

    #[test]
    fn unsorted_line_lookups_match_sorted_order() {
        let sm = SourceMap::from_json(UNSORTED_LINE_JSON).unwrap();
        for (column, expected_line) in UNSORTED_LINE_EXPECTED {
            assert_eq!(
                sm.original_position_for(0, column).unwrap().line,
                expected_line,
                "{column}"
            );
        }
    }

    #[test]
    fn unsorted_line_mappings_for_line_is_sorted() {
        let sm = SourceMap::from_json(UNSORTED_LINE_JSON).unwrap();
        let columns: Vec<u32> =
            sm.mappings_for_line(0).iter().map(|m| m.generated_column).collect();
        assert_eq!(columns, [7, 8, 9, 10]);
    }

    #[test]
    fn unsorted_line_from_json_lines() {
        let sm = SourceMap::from_json_lines(UNSORTED_LINE_JSON, 0, 1).unwrap();
        for (column, expected_line) in UNSORTED_LINE_EXPECTED {
            assert_eq!(
                sm.original_position_for(0, column).unwrap().line,
                expected_line,
                "{column}"
            );
        }
    }

    #[test]
    fn unsorted_line_lazy() {
        let sm = LazySourceMap::from_json(UNSORTED_LINE_JSON).unwrap();
        for (column, expected_line) in UNSORTED_LINE_EXPECTED {
            assert_eq!(
                sm.original_position_for(0, column).unwrap().line,
                expected_line,
                "{column}"
            );
        }
        let eager = LazySourceMap::from_json(UNSORTED_LINE_JSON).unwrap().into_sourcemap().unwrap();
        for (column, expected_line) in UNSORTED_LINE_EXPECTED {
            assert_eq!(eager.original_position_for(0, column).unwrap().line, expected_line);
        }
    }

    #[test]
    fn unsorted_line_keeps_range_mapping_on_its_segment() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"KAAA,DACA","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let line = sm.mappings_for_line(0);
        let at = |column: u32| line.iter().find(|m| m.generated_column == column).unwrap();
        assert!(at(5).is_range_mapping);
        assert!(!at(4).is_range_mapping);
    }

    #[test]
    fn unsorted_line_stable_for_equal_columns() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"EAAA,AACA,DACA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let line: Vec<(u32, u32)> =
            sm.mappings_for_line(0).iter().map(|m| (m.generated_column, m.original_line)).collect();
        assert_eq!(line, [(1, 2), (2, 0), (2, 1)]);
    }

    #[test]
    fn unsorted_second_line_is_sorted() {
        // Line 0 is in order. Line 1 starts at column 3 and drops to column 2.
        let json =
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,CACA;GACA,DACA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let columns: Vec<u32> =
            sm.mappings_for_line(1).iter().map(|m| m.generated_column).collect();
        assert_eq!(columns, [2, 3]);
        assert_eq!(sm.original_position_for(1, 2).unwrap().line, 3);
    }

    #[test]
    fn decode_error_after_out_of_range_index_takes_precedence() {
        // Segment 1 has source index 1 of 1 source. Segment 2 has 2 fields.
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA,AA"}"#;
        let err = SourceMap::from_json(json).unwrap_err();
        assert!(
            matches!(err, ParseError::Vlq(DecodeError::InvalidSegmentLength { fields: 2, .. })),
            "{err:?}"
        );
    }

    #[test]
    fn reports_first_out_of_range_index_in_mapping_order() {
        // Name index 1 comes before source index 2 in mapping order.
        let json = r#"{"version":3,"sources":["a.js"],"names":["n"],"mappings":"AAAAC;AEAA"}"#;
        assert_index_error(json, IndexKind::Name, 1, 1);
    }

    #[test]
    fn from_json_lines_ignores_out_of_range_index_before_start_line() {
        // Line 0 has source index 1 of 1 source. Line 1 points back to source 0.
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA;ADAA"}"#;
        let sm = SourceMap::from_json_lines(json, 1, 2).unwrap();
        assert_eq!(sm.original_position_for(1, 0).unwrap().source, 0);
        let err = SourceMap::from_json_lines(json, 0, 2).unwrap_err();
        assert!(matches!(err, ParseError::IndexOutOfRange { kind: IndexKind::Source, .. }));
    }

    fn names_options(mode: OutOfRangeNames) -> ParseOptions {
        ParseOptions { out_of_range_names: mode, ..Default::default() }
    }

    fn lenient_options() -> ParseOptions {
        ParseOptions { lenient_names: true, ..names_options(OutOfRangeNames::Unnamed) }
    }

    #[test]
    fn default_options_reject_null_names_entry() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[null],"mappings":"AAAA"}"#;
        let err = SourceMap::from_json_with_options(json, &ParseOptions::default()).unwrap_err();
        assert!(matches!(err, ParseError::Json(_)), "{err:?}");
    }

    #[test]
    fn lenient_names_reads_falsy_entries_as_empty_names() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[null,false,0,-0.0,"x"],"mappings":"AAAAA,CAAAC,CAAAC,CAAAC,CAAAC"}"#;
        let sm = SourceMap::from_json_with_options(json, &lenient_options()).unwrap();
        assert_eq!(sm.names, ["", "", "", "", "x"]);
        assert_eq!(mapping_names(&sm), [Some(""), Some(""), Some(""), Some(""), Some("x")]);
    }

    #[test]
    fn lenient_names_reads_falsy_or_scalar_names_as_no_names() {
        for names in ["null", "false", "true", "0", "5", r#""""#] {
            let json =
                format!(r#"{{"version":3,"sources":["a.js"],"names":{names},"mappings":"AAAAA"}}"#);
            let sm = SourceMap::from_json_with_options(&json, &lenient_options()).unwrap();
            assert!(sm.names.is_empty(), "{names}");
            assert_eq!(mapping_names(&sm), [None], "{names}");
        }
    }

    #[test]
    fn lenient_names_keeps_other_invalid_names_as_errors() {
        for names in ["[1]", "[true]", "[{}]", "[[]]", r#""ab""#, "{}"] {
            let json =
                format!(r#"{{"version":3,"sources":["a.js"],"names":{names},"mappings":"AAAA"}}"#);
            let err = SourceMap::from_json_with_options(&json, &lenient_options()).unwrap_err();
            assert!(matches!(err, ParseError::Json(_)), "{names}: {err:?}");
        }
    }

    #[test]
    fn lenient_names_keeps_other_json_errors() {
        let err = SourceMap::from_json_with_options("{", &lenient_options()).unwrap_err();
        assert!(matches!(err, ParseError::Json(_)), "{err:?}");
        let json = r#"{"version":3,"sources":[null,1],"names":[null],"mappings":"AAAA"}"#;
        let err = SourceMap::from_json_with_options(json, &lenient_options()).unwrap_err();
        assert!(err.to_string().contains("integer `1`"), "{err}");
    }

    fn mapping_names(sm: &SourceMap) -> Vec<Option<&str>> {
        sm.all_mappings()
            .iter()
            .map(|m| (m.name != NO_NAME).then(|| sm.names[m.name as usize].as_str()))
            .collect()
    }

    #[test]
    fn default_options_reject_name_index_out_of_range() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAAA"}"#;
        let err = SourceMap::from_json_with_options(json, &ParseOptions::default()).unwrap_err();
        assert!(matches!(
            err,
            ParseError::IndexOutOfRange { kind: IndexKind::Name, index: 0, len: 0 }
        ));
    }

    #[test]
    fn unnamed_mode_drops_name_index_out_of_range() {
        // Name indices: 0 (valid), 2 (past the end), 1 (valid), -1 (negative), 0 (valid).
        let json = r#"{"version":3,"sources":["a.js"],"names":["x","y"],"mappings":"AAAAA,CAAAE,CAAAD,CAAAF,CAAAC"}"#;
        let sm = SourceMap::from_json_with_options(json, &names_options(OutOfRangeNames::Unnamed))
            .unwrap();
        assert_eq!(mapping_names(&sm), [Some("x"), None, Some("y"), None, Some("x")]);
        assert_eq!(sm.names, ["x", "y"]);
        assert_eq!(sm.original_position_for(0, 1).unwrap().name, None);
    }

    #[test]
    fn unnamed_mode_accepts_missing_names_field() {
        let json = r#"{"version":3,"sources":["a.js"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json_with_options(json, &names_options(OutOfRangeNames::Unnamed))
            .unwrap();
        assert_eq!(mapping_names(&sm), [None]);
    }

    #[test]
    fn unnamed_mode_keeps_source_index_check() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAAA"}"#;
        let err = SourceMap::from_json_with_options(json, &names_options(OutOfRangeNames::Unnamed))
            .unwrap_err();
        assert!(matches!(err, ParseError::IndexOutOfRange { kind: IndexKind::Source, .. }));
    }

    #[test]
    fn unnamed_mode_applies_to_sections() {
        let json = r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a.js"],"names":["x"],"mappings":"AAAAA,CAAAC"}}]}"#;
        assert!(SourceMap::from_json(json).is_err());
        let sm = SourceMap::from_json_with_options(json, &names_options(OutOfRangeNames::Unnamed))
            .unwrap();
        assert_eq!(mapping_names(&sm), [Some("x"), None]);
    }

    #[test]
    fn placeholder_mode_adds_placeholder_only_when_used() {
        let mode = OutOfRangeNames::Placeholder("?".to_owned());
        let json =
            r#"{"version":3,"sources":["a.js"],"names":["x"],"mappings":"AAAAA,CAAAC,CAAAF"}"#;
        let sm = SourceMap::from_json_with_options(json, &names_options(mode.clone())).unwrap();
        assert_eq!(sm.names, ["x", "?"]);
        assert_eq!(mapping_names(&sm), [Some("x"), Some("?"), Some("?")]);

        let valid = r#"{"version":3,"sources":["a.js"],"names":["x"],"mappings":"AAAAA"}"#;
        let sm = SourceMap::from_json_with_options(valid, &names_options(mode)).unwrap();
        assert_eq!(sm.names, ["x"]);
    }

    #[test]
    fn from_vlq_with_options_matches_json_parse() {
        let mode = OutOfRangeNames::Placeholder("?".to_owned());
        let options = names_options(mode);
        // An out-of-range name, an unsorted line, a sourceless segment and an empty last line.
        let mappings = "EAAAC,AAAAA,C;AACA;";
        let json = format!(
            r#"{{"version":3,"file":"out.js","sources":["a.js",""],"sourcesContent":["x",null],"names":["n"],"mappings":"{mappings}","ignoreList":[1]}}"#
        );
        let expected = SourceMap::from_json_with_options(&json, &options).unwrap();
        let sm = SourceMap::from_vlq_with_options(
            mappings,
            vec!["a.js".to_owned(), String::new()],
            vec!["n".to_owned()],
            Some("out.js".to_owned()),
            vec![Some("x".to_owned()), None],
            vec![1],
            &options,
        )
        .unwrap();
        assert_eq!(sm.to_json(), expected.to_json());
        assert_eq!(sm.names, ["n", "?"]);
        assert_eq!(sm.line_count(), expected.line_count());
    }

    #[test]
    fn from_vlq_with_options_gives_the_json_parse_errors() {
        let options = ParseOptions::default();
        let parse = |mappings: &str, ignore_list: Vec<u32>| {
            SourceMap::from_vlq_with_options(
                mappings,
                vec!["a.js".to_owned()],
                Vec::new(),
                None,
                Vec::new(),
                ignore_list,
                &options,
            )
            .unwrap_err()
            .to_string()
        };
        let json_error = |json: &str| SourceMap::from_json(json).unwrap_err().to_string();
        assert_eq!(
            parse("ACAA", Vec::new()),
            json_error(r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"ACAA"}"#)
        );
        assert_eq!(
            parse("!", Vec::new()),
            json_error(r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"!"}"#)
        );
        assert_eq!(
            parse("AAAA", vec![3]),
            json_error(
                r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","ignoreList":[3]}"#
            )
        );
    }

    #[test]
    fn placeholder_mode_does_not_resolve_scopes_names() {
        // The placeholder goes into `names` after the scopes decode.
        // A scope name with index 0 and no names: the placeholder must not become that name.
        let mut info = one_scope_info(base_range());
        if let Some(Some(scope)) = info.scopes.first_mut() {
            scope.name = Some("f".to_owned());
        }
        let mut scope_names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&info, &mut scope_names);
        assert_eq!(scope_names, ["f"]);
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAAA","scopes":"{scopes_str}"}}"#
        );
        let mode = OutOfRangeNames::Placeholder("?".to_owned());
        let err = SourceMap::from_json_with_options(&json, &names_options(mode)).unwrap_err();
        assert!(matches!(err, ParseError::Scopes(_)), "{err:?}");
        let with_name = json.replace(r#""names":[]"#, r#""names":["f"]"#);
        assert!(SourceMap::from_json(&with_name).is_ok());
    }

    #[test]
    fn accepts_last_valid_source_and_name_index() {
        let json =
            r#"{"version":3,"sources":["a.js","b.js"],"names":["x","y"],"mappings":"ACAAC"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!((loc.source, loc.name), (1, Some(1)));
    }

    #[test]
    fn rejects_column_at_u32_max() {
        // u32::MAX is the first column past the valid range.
        let mut encoded = Vec::new();
        srcmap_codec::vlq_encode(&mut encoded, i64::from(u32::MAX));
        let mappings = String::from_utf8(encoded).unwrap();
        assert!(matches!(vlq_error(&mappings, ""), DecodeError::ValueOutOfRange { offset: 0 }));
        let mut below = Vec::new();
        srcmap_codec::vlq_encode(&mut below, i64::from(u32::MAX) - 1);
        let json = format!(
            r#"{{"version":3,"sources":[],"names":[],"mappings":"{}"}}"#,
            String::from_utf8(below).unwrap()
        );
        assert!(SourceMap::from_json(&json).is_ok());
    }

    type Loc = Option<(u32, u32, u32, Option<u32>)>;

    fn loc_tuple(loc: Option<OriginalLocation>) -> Loc {
        let l = loc?;
        Some((l.source, l.line, l.column, l.name))
    }

    /// Compare every lookup path against `SourceMap::from_json` on a grid of positions.
    fn assert_lookup_parity(json: &str, max_line: u32, max_col: u32) {
        let eager = SourceMap::from_json(json).unwrap();
        let lines = SourceMap::from_json_lines(json, 0, max_line + 1).unwrap();
        let lazy = LazySourceMap::from_json(json).unwrap();
        let lazy_no_content = LazySourceMap::from_json_no_content(json).unwrap();
        let lazy_fast = LazySourceMap::from_json_fast(json).unwrap();
        let roundtrip = LazySourceMap::from_json(json).unwrap().into_sourcemap().unwrap();
        for line in 0..=max_line {
            for col in 0..=max_col {
                let want = loc_tuple(eager.original_position_for(line, col));
                let paths = [
                    ("from_json_lines", loc_tuple(lines.original_position_for(line, col))),
                    ("lazy", loc_tuple(lazy.original_position_for(line, col))),
                    (
                        "lazy_no_content",
                        loc_tuple(lazy_no_content.original_position_for(line, col)),
                    ),
                    ("lazy_fast", loc_tuple(lazy_fast.original_position_for(line, col))),
                    ("into_sourcemap", loc_tuple(roundtrip.original_position_for(line, col))),
                ];
                for (path, got) in paths {
                    assert_eq!(got, want, "{path} {line}:{col}");
                }
            }
        }
    }

    #[test]
    fn lookup_parity_simple() {
        assert_lookup_parity(simple_map(), 3, 6);
    }

    #[test]
    fn lookup_parity_generated() {
        assert_lookup_parity(&generate_test_sourcemap(20, 5, 3), 21, 60);
    }

    #[test]
    fn lookup_parity_duplicate_columns() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["src/original.ts"],"names":["originalFn","helperFn"],"mappings":"AASA,A,A;AAIA,C;AAIA"}"#,
            3,
            3,
        );
    }

    #[test]
    fn lookup_parity_range_mappings_across_lines() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;KACK","rangeMappings":"A"}"#,
            3,
            8,
        );
    }

    #[test]
    fn lookup_parity_range_mappings_same_line() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,KACK","rangeMappings":"A"}"#,
            1,
            8,
        );
    }

    #[test]
    fn lookup_parity_range_mappings_after_empty_lines() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,KACK;;;EAAE","rangeMappings":"C;;;A"}"#,
            5,
            8,
        );
    }

    #[test]
    fn lookup_parity_unsorted_line() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"UAAA,DACA,DACA,DACA"}"#,
            1,
            12,
        );
    }

    #[test]
    fn lookup_parity_unsorted_line_with_range_mapping() {
        assert_lookup_parity(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"KAAA,DACA;","rangeMappings":"A"}"#,
            2,
            8,
        );
    }

    #[test]
    fn lazy_fast_keeps_scopes() {
        let scopes_str = single_scope_str();
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA","scopes":"{scopes_str}"}}"#
        );
        assert!(LazySourceMap::from_json_fast(&json).unwrap().scopes.is_some());
    }

    #[test]
    fn range_mappings_survive_from_json_lines_and_into_sourcemap() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;KACK","rangeMappings":"A"}"#;
        let lines = SourceMap::from_json_lines(json, 0, 2).unwrap();
        assert!(lines.to_json().contains("\"rangeMappings\""));
        let roundtrip = LazySourceMap::from_json(json).unwrap().into_sourcemap().unwrap();
        assert!(roundtrip.to_json().contains("\"rangeMappings\""));
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

    /// Helper: build a source map JSON from absolute mappings data.
    fn build_sourcemap_json(
        sources: &[&str],
        names: &[&str],
        mappings_data: &[Vec<Vec<i64>>],
    ) -> String {
        let converted: Vec<Vec<srcmap_codec::Segment>> = mappings_data
            .iter()
            .map(|line| {
                line.iter().map(|seg| srcmap_codec::Segment::from(seg.as_slice())).collect()
            })
            .collect();
        let encoded = srcmap_codec::encode(&converted);
        format!(
            r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
            sources.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","),
            names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(","),
            encoded,
        )
    }

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
    fn parse_missing_optional_fields() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.file.is_none());
        assert!(sm.source_root.is_none());
        assert_eq!(sm.sources_content, Vec::<Option<String>>::new());
        assert_eq!(sm.ignore_list, Vec::<u32>::new());
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
    fn lookup_exact_match() {
        let mappings_data =
            vec![vec![vec![0_i64, 0, 10, 20], vec![5, 0, 10, 25], vec![15, 0, 11, 0]]];
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
        let mappings_data = vec![vec![vec![0_i64, 0, 1, 0], vec![10, 0, 2, 0], vec![20, 0, 3, 0]]];
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
        let mappings_data = vec![vec![vec![0_i64, 0, 0, 0]], vec![], vec![vec![0_i64, 0, 2, 0]]];
        let json = build_sourcemap_json(&["src.js"], &[], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();

        assert!(sm.original_position_for(1, 0).is_none());
        assert!(sm.original_position_for(1, 10).is_none());
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(2, 0).is_some());
    }

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
        let json = r#"{"version":3,"sources":["a.js"],"names":["n"],"mappings":"A,KAAAA"}"#;
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
            assert_eq!(loc.line, orig_line, "reverse lookup for orig line {orig_line}");
            assert_eq!(loc.column, 0);
        }
    }

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
        assert_eq!(sm.ignore_list, Vec::<u32>::new());
    }

    #[test]
    fn source_index_lookup() {
        let json = r#"{"version":3,"sources":["a.js","b.js","c.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.source_index("a.js"), Some(0));
        assert_eq!(sm.source_index("b.js"), Some(1));
        assert_eq!(sm.source_index("c.js"), Some(2));
        assert_eq!(sm.source_index("d.js"), None);
    }

    const DUPLICATE_SOURCES_JSON: &str =
        r#"{"version":3,"sources":["a.js","a.js"],"names":[],"mappings":"AAAA"}"#;

    #[test]
    fn source_index_duplicate_sources_returns_first() {
        let sm = SourceMap::from_json(DUPLICATE_SOURCES_JSON).unwrap();
        assert_eq!(sm.source_index("a.js"), Some(0));
    }

    #[test]
    fn generated_position_for_duplicate_sources() {
        let sm = SourceMap::from_json(DUPLICATE_SOURCES_JSON).unwrap();
        let loc = sm.generated_position_for("a.js", 0, 0).unwrap();
        assert_eq!((loc.line, loc.column), (0, 0));
    }

    #[test]
    fn all_generated_positions_for_duplicate_sources() {
        let sm = SourceMap::from_json(DUPLICATE_SOURCES_JSON).unwrap();
        let locs = sm.all_generated_positions_for("a.js", 0, 0);
        assert_eq!(locs.len(), 1);
        assert_eq!((locs[0].line, locs[0].column), (0, 0));
    }

    #[test]
    fn lazy_source_index_duplicate_sources_returns_first() {
        let sm = LazySourceMap::from_json(DUPLICATE_SOURCES_JSON).unwrap();
        assert_eq!(sm.source_index("a.js"), Some(0));
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

            let orig = sm.original_position_for(m.generated_line, m.generated_column).unwrap();
            assert_eq!(orig.source, m.source);
            assert_eq!(orig.line, m.original_line);
            assert_eq!(orig.column, m.original_column);

            let gen_loc =
                sm.generated_position_for(source_name, m.original_line, m.original_column).unwrap();
            assert_eq!(gen_loc.line, m.generated_line);
            assert_eq!(gen_loc.column, m.generated_column);
        }
    }

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
    fn source_root_reverse_lookup_uses_prefixed_name() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Must use the prefixed name for reverse lookups
        assert!(sm.generated_position_for("src/a.js", 0, 0).is_some());
        assert!(sm.generated_position_for("a.js", 0, 0).is_none());
    }

    #[test]
    fn source_root_without_trailing_slash() {
        let json =
            r#"{"version":3,"sourceRoot":"src","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // sourceRoot is applied as raw prefix during parsing
        assert_eq!(sm.sources[0], "srca.js");
        // Roundtrip should strip the prefix back correctly
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.sources[0], "srca.js");
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

    #[test]
    fn to_json_produces_valid_json() {
        let json = r#"{"version":3,"file":"out.js","sourceRoot":"src/","sources":["a.ts","b.ts"],"sourcesContent":["const x = 1;\nconst y = \"hello\";",null],"names":["x","y"],"mappings":"AAAAA,KACAC;AACA","ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        // Must be valid JSON that serde can parse
        let _: serde_json::Value = serde_json::from_str(&output).unwrap();
    }

    #[test]
    fn to_json_empty_map() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();
        assert_eq!(sm2.mapping_count(), 0);
        assert_eq!(sm2.sources, Vec::<String>::new());
    }

    #[test]
    fn to_json_roundtrip_with_names() {
        let mappings_data =
            vec![vec![vec![0_i64, 0, 0, 0, 0], vec![10, 0, 0, 10, 1], vec![20, 0, 1, 0, 2]]];
        let json = build_sourcemap_json(&["src.js"], &["foo", "bar", "baz"], &mappings_data);
        let sm = SourceMap::from_json(&json).unwrap();
        let output = sm.to_json();
        let sm2 = SourceMap::from_json(&output).unwrap();

        for m in sm2.all_mappings() {
            if m.source != NO_SOURCE && m.name != NO_NAME {
                let loc = sm2.original_position_for(m.generated_line, m.generated_column).unwrap();
                assert!(loc.name.is_some());
            }
        }
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
    fn parse_debug_id_snake_case() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","debug_id":"85314830-023f-4cf1-a267-535f4e37bb17"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.debug_id.as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
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
        let sources: Vec<String> = (0..num_sources).map(|i| format!("src/file{i}.js")).collect();
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

                let src_delta = i64::from(s % 7 == 0);
                src = (src + src_delta) % num_sources as i64;

                src_line += 1;
                src_col = (s as i64 * 5 + 1) % 30;

                let has_name = s % 4 == 0;
                if has_name {
                    name = (name + 1) % names.len() as i64;
                }

                // Build segment using codec encode
                let segment = if has_name {
                    srcmap_codec::Segment::five(gen_col, src, src_line, src_col, name)
                } else {
                    srcmap_codec::Segment::four(gen_col, src, src_line, src_col)
                };

                line_parts.push(segment);
            }

            mappings_parts.push(line_parts);
        }

        let encoded = srcmap_codec::encode(&mappings_parts);

        format!(
            r#"{{"version":3,"sources":[{}],"names":[{}],"mappings":"{}"}}"#,
            sources.iter().map(|s| format!("\"{s}\"")).collect::<Vec<_>>().join(","),
            names.iter().map(|n| format!("\"{n}\"")).collect::<Vec<_>>().join(","),
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
        let loc = sm.original_position_for_with_bias(0, 5, Bias::GreatestLowerBound).unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_glb_snaps_left() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 7 should snap to the mapping at column 5
        let loc = sm.original_position_for_with_bias(0, 7, Bias::GreatestLowerBound).unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_exact_match() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        let loc = sm.original_position_for_with_bias(0, 5, Bias::LeastUpperBound).unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_snaps_right() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 3 with LUB should snap to the mapping at column 5
        let loc = sm.original_position_for_with_bias(0, 3, Bias::LeastUpperBound).unwrap();
        assert_eq!(loc.column, 5);
    }

    #[test]
    fn original_position_lub_after_last() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // Column 15 with LUB should return None (no mapping at or after 15)
        let loc = sm.original_position_for_with_bias(0, 15, Bias::LeastUpperBound);
        assert!(loc.is_none());
    }

    #[test]
    fn generated_position_lub() {
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // LUB: find first generated position at or after original col 3
        let loc =
            sm.generated_position_for_with_bias("input.js", 0, 3, Bias::LeastUpperBound).unwrap();
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

    #[test]
    fn generated_position_for_default_bias_is_glb() {
        // The default bias must be GreatestLowerBound to match jridgewell's
        // generatedPositionFor semantics.
        let sm = SourceMap::from_json(bias_map()).unwrap();
        // With GLB: looking for original col 7, GLB finds the mapping at col 5
        let glb = sm.generated_position_for("input.js", 0, 7).unwrap();
        let glb_explicit = sm
            .generated_position_for_with_bias("input.js", 0, 7, Bias::GreatestLowerBound)
            .unwrap();
        assert_eq!(glb.line, glb_explicit.line);
        assert_eq!(glb.column, glb_explicit.column);
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

    #[test]
    fn extension_fields_preserved() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","x_facebook_sources":[[{"names":["<global>"]}]],"x_google_linecount":42}"#;
        let sm = SourceMap::from_json(json).unwrap();

        assert!(sm.extensions.contains_key("x_facebook_sources"));
        assert!(sm.extensions.contains_key("x_google_linecount"));
        assert_eq!(sm.extensions.get("x_google_linecount"), Some(&serde_json::json!(42)));

        // Round-trip preserves extension fields
        let output = sm.to_json();
        assert!(output.contains("x_facebook_sources"));
        assert!(output.contains("x_google_linecount"));
    }

    #[test]
    fn ignorelist_takes_precedence_over_x_google() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA","ignoreList":[0],"x_google_ignoreList":[1]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![0]);
    }

    #[test]
    fn source_mapping_url_inline() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let b64 = utils::base64_encode(json.as_bytes());
        let source =
            format!("var a = 1;\n//# sourceMappingURL=data:application/json;base64,{b64}\n");
        match parse_source_mapping_url(&source).unwrap() {
            SourceMappingUrl::Inline(decoded) => {
                assert_eq!(decoded, json);
            }
            SourceMappingUrl::External(_) => panic!("expected inline"),
        }
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
                is_range_mapping: false,
            },
            Mapping {
                generated_line: 1,
                generated_column: 4,
                source: 0,
                original_line: 1,
                original_column: 2,
                name: NO_NAME,
                is_range_mapping: false,
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
        let sm =
            SourceMap::from_parts(None, None, vec![], vec![], vec![], vec![], vec![], None, None);
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
            is_range_mapping: false,
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
    fn from_parts_with_extensions_preserves_source_map_extensions() {
        let mut extensions = HashMap::new();
        extensions.insert("x_google_linecount".to_string(), serde_json::json!(7));
        extensions.insert("x-custom".to_string(), serde_json::json!({ "producer": "oxc" }));
        extensions.insert("vendorField".to_string(), serde_json::json!("ignored"));

        let sm = SourceMap::from_parts_with_extensions(
            Some("out.js".to_string()),
            Some("src/".to_string()),
            vec!["src/input.ts".to_string()],
            vec![Some("export const value = 1;".to_string())],
            vec!["value".to_string()],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 13,
                name: 0,
                is_range_mapping: false,
            }],
            vec![0],
            Some("85314830-023f-4cf1-a267-535f4e37bb17".to_string()),
            None,
            extensions,
        );

        assert_eq!(sm.file.as_deref(), Some("out.js"));
        assert_eq!(sm.source_root.as_deref(), Some("src/"));
        assert_eq!(sm.ignore_list, vec![0]);
        assert_eq!(sm.debug_id.as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
        assert_eq!(sm.extensions.get("x_google_linecount"), Some(&serde_json::json!(7)));
        assert!(sm.extensions.contains_key("x-custom"));
        assert!(!sm.extensions.contains_key("vendorField"));

        let json = sm.to_json();
        let parsed = SourceMap::from_json(&json).unwrap();
        assert_eq!(parsed.mapping_count(), sm.mapping_count());
        assert_eq!(parsed.extensions, sm.extensions);
        assert_eq!(parsed.original_position_for(0, 0).unwrap().column, 13);
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
                is_range_mapping: false,
            },
            Mapping {
                generated_line: 1,
                generated_column: 8,
                source: 0,
                original_line: 20,
                original_column: 0,
                name: NO_NAME,
                is_range_mapping: false,
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
                is_range_mapping: false,
            },
            Mapping {
                generated_line: 5,
                generated_column: 0,
                source: 0,
                original_line: 5,
                original_column: 0,
                name: NO_NAME,
                is_range_mapping: false,
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
            let eager_loc =
                sm_eager.original_position_for(m.generated_line, m.generated_column).unwrap();
            let lazy_loc =
                sm_lazy.original_position_for(m.generated_line, m.generated_column).unwrap();
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
            assert_eq!(eager_mappings.len(), lazy_mappings.len(), "line {line} count mismatch");
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

    #[test]
    fn parse_error_display_scopes() {
        let err = ParseError::Scopes(srcmap_scopes::ScopesError::UnclosedScope);
        assert!(err.to_string().contains("scopes decode error"));
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
    fn from_json_lines_with_source_root() {
        let json = r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA;AACA"}"#;
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

    /// Encoded `scopes` string with one unnamed scope spanning lines 0..5.
    fn single_scope_str() -> String {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![],
        };
        srcmap_scopes::encode_scopes(&info, &mut vec![])
    }

    #[test]
    fn scopes_in_from_json() {
        let scopes_str = single_scope_str();
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );

        let sm = SourceMap::from_json(&json).unwrap();
        assert!(sm.scopes.is_some());
    }

    #[test]
    fn from_json_lines_with_scopes() {
        let scopes_str = single_scope_str();
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
    fn lazy_sourcemap_with_ignore_list_and_extensions() {
        let json = r#"{"version":3,"sources":["v.js"],"names":[],"mappings":"AAAA","ignoreList":[0],"x_custom":"val","not_x":"skip"}"#;
        let sm = LazySourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, vec![0]);
        assert!(sm.extensions.contains_key("x_custom"));
        assert!(!sm.extensions.contains_key("not_x"));
    }

    #[test]
    fn lazy_sourcemap_with_scopes() {
        let scopes_str = single_scope_str();
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","scopes":"{scopes_str}"}}"#
        );
        let sm = LazySourceMap::from_json(&json).unwrap();
        assert!(sm.scopes.is_some());
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
        let encoded = utils::base64_encode(map_json.as_bytes());
        let input = format!("var x;\n//# sourceMappingURL=data:application/json;base64,{encoded}");
        let url = parse_source_mapping_url(&input);
        assert!(matches!(url, Some(SourceMappingUrl::Inline(_))));
    }

    #[test]
    fn from_json_lines_with_names() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA;AACAA"}"#;
        let sm = SourceMap::from_json_lines(json, 0, 2).unwrap();
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.name, Some(0));
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
        // b.js is deduplicated across sections and lands at global index 1
        assert_eq!(sm.ignore_list, vec![1]);
    }

    #[test]
    fn to_json_with_ignore_list_and_extensions() {
        let mut sm = SourceMap::from_parts(
            None,
            None,
            vec!["a.js".to_string(), "b.js".to_string()],
            vec![None, None],
            vec![],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
                is_range_mapping: false,
            }],
            vec![1],
            None,
            None,
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
        )
        .unwrap();
        assert_eq!(sm.source(0), "a.js");
        assert!(sm.original_position_for(0, 0).is_some());
        assert!(sm.original_position_for(1, 0).is_some());
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
            None,
            None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![
                Mapping {
                    generated_line: 1,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: NO_NAME,
                    is_range_mapping: false,
                },
                Mapping {
                    generated_line: 0,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: NO_NAME,
                    is_range_mapping: false,
                },
            ],
            vec![],
            None,
            None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("out of order")));
    }

    #[test]
    fn validate_deep_out_of_bounds_source() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 5,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
                is_range_mapping: false,
            }],
            vec![],
            None,
            None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("source index") && w.contains("out of bounds")));
    }

    #[test]
    fn validate_deep_out_of_bounds_name() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["a.js".to_string()],
            vec![None],
            vec!["foo".to_string()],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: 5,
                is_range_mapping: false,
            }],
            vec![],
            None,
            None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("name index") && w.contains("out of bounds")));
    }

    #[test]
    fn validate_deep_out_of_bounds_ignore_list() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["a.js".to_string()],
            vec![None],
            vec![],
            vec![Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: NO_NAME,
                is_range_mapping: false,
            }],
            vec![10],
            None,
            None,
        );
        let warnings = validate_deep(&sm);
        assert!(warnings.iter().any(|w| w.contains("ignoreList") && w.contains("out of bounds")));
    }

    #[test]
    fn source_mapping_url_charset_variant() {
        let map_json = r#"{"version":3}"#;
        let encoded = utils::base64_encode(map_json.as_bytes());
        let input =
            format!("x\n//# sourceMappingURL=data:application/json;charset=utf-8;base64,{encoded}");
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

    // ── MappingsIter tests ──────────────────────────────────────

    #[test]
    fn mappings_iter_matches_decode() {
        let vlq = "AAAA;AACA,EAAA;AACA";
        let iter_mappings: Vec<Mapping> = MappingsIter::new(vlq).collect::<Result<_, _>>().unwrap();
        let (decoded, _, _) = decode_mappings(vlq, usize::MAX, usize::MAX).unwrap();
        assert_eq!(iter_mappings.len(), decoded.len());
        for (a, b) in iter_mappings.iter().zip(decoded.iter()) {
            assert_eq!(a.generated_line, b.generated_line);
            assert_eq!(a.generated_column, b.generated_column);
            assert_eq!(a.source, b.source);
            assert_eq!(a.original_line, b.original_line);
            assert_eq!(a.original_column, b.original_column);
            assert_eq!(a.name, b.name);
        }
    }

    #[test]
    fn mappings_iter_empty() {
        let mappings: Vec<Mapping> = MappingsIter::new("").collect::<Result<_, _>>().unwrap();
        assert!(mappings.is_empty());
    }

    #[test]
    fn mappings_iter_generated_only() {
        let vlq = "A,AAAA";
        let mappings: Vec<Mapping> = MappingsIter::new(vlq).collect::<Result<_, _>>().unwrap();
        assert_eq!(mappings.len(), 2);
        assert_eq!(mappings[0].source, u32::MAX);
        assert_eq!(mappings[1].source, 0);
    }

    #[test]
    fn mappings_iter_with_names() {
        let vlq = "AAAAA";
        let mappings: Vec<Mapping> = MappingsIter::new(vlq).collect::<Result<_, _>>().unwrap();
        assert_eq!(mappings.len(), 1);
        assert_eq!(mappings[0].name, 0);
    }

    // ── Range mappings tests ──────────────────────────────────────

    #[test]
    fn range_mappings_basic_decode() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,CAAC,GAAG","rangeMappings":"A,C"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.all_mappings()[0].is_range_mapping);
        assert!(!sm.all_mappings()[1].is_range_mapping);
        assert!(sm.all_mappings()[2].is_range_mapping);
    }

    #[test]
    fn range_mapping_lookup_with_delta() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,GAAG","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.original_position_for(0, 0).unwrap().column, 0);
        assert_eq!(sm.original_position_for(0, 1).unwrap().column, 1);
        assert_eq!(sm.original_position_for(0, 2).unwrap().column, 2);
        assert_eq!(sm.original_position_for(0, 3).unwrap().column, 3);
    }

    #[test]
    fn range_mapping_cross_line() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.original_position_for(1, 5).unwrap().line, 1);
        assert_eq!(sm.original_position_for(1, 5).unwrap().column, 0);
        assert_eq!(sm.original_position_for(2, 10).unwrap().line, 2);
    }

    #[test]
    fn range_mapping_encode_roundtrip() {
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,CAAC,GAAG","rangeMappings":"A,C"}"#;
        assert_eq!(SourceMap::from_json(json).unwrap().encode_range_mappings().unwrap(), "A,C");
    }

    #[test]
    fn no_range_mappings_test() {
        let sm = SourceMap::from_json(
            r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        assert!(!sm.has_range_mappings());
        assert!(sm.encode_range_mappings().is_none());
    }

    #[test]
    fn range_mappings_multi_line_test() {
        let sm = SourceMap::from_json(r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,CAAC;AAAA","rangeMappings":"A;A"}"#).unwrap();
        assert!(sm.all_mappings()[0].is_range_mapping);
        assert!(!sm.all_mappings()[1].is_range_mapping);
        assert!(sm.all_mappings()[2].is_range_mapping);
    }

    #[test]
    fn range_mappings_json_roundtrip() {
        let sm = SourceMap::from_json(r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,CAAC,GAAG","rangeMappings":"A,C"}"#).unwrap();
        let output = sm.to_json();
        assert!(output.contains("rangeMappings"));
        assert_eq!(SourceMap::from_json(&output).unwrap().range_mapping_count(), 2);
    }

    #[test]
    fn range_mappings_absent_from_json_test() {
        assert!(
            !SourceMap::from_json(
                r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA"}"#
            )
            .unwrap()
            .to_json()
            .contains("rangeMappings")
        );
    }

    #[test]
    fn range_mapping_fallback_test() {
        let sm = SourceMap::from_json(r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA;KACK","rangeMappings":"A"}"#).unwrap();
        let loc = sm.original_position_for(1, 2).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn range_mapping_from_vlq_test() {
        let sm = SourceMap::from_vlq_with_range_mappings(
            "AAAA,CAAC",
            vec!["input.js".into()],
            vec![],
            None,
            None,
            vec![],
            vec![],
            None,
            Some("A"),
        )
        .unwrap();
        assert!(sm.all_mappings()[0].is_range_mapping);
        assert!(!sm.all_mappings()[1].is_range_mapping);
    }

    #[test]
    fn range_mapping_encode_multi_line_test() {
        let sm = SourceMap::from_json(r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA,CAAC;AAAA,CAAC","rangeMappings":"A;B"}"#).unwrap();
        assert!(sm.all_mappings()[0].is_range_mapping);
        assert!(!sm.all_mappings()[1].is_range_mapping);
        assert!(!sm.all_mappings()[2].is_range_mapping);
        assert!(sm.all_mappings()[3].is_range_mapping);
        assert_eq!(sm.encode_range_mappings().unwrap(), "A;B");
    }

    #[test]
    fn range_mapping_from_parts_test() {
        let sm = SourceMap::from_parts(
            None,
            None,
            vec!["input.js".into()],
            vec![],
            vec![],
            vec![
                Mapping {
                    generated_line: 0,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: NO_NAME,
                    is_range_mapping: true,
                },
                Mapping {
                    generated_line: 0,
                    generated_column: 5,
                    source: 0,
                    original_line: 0,
                    original_column: 5,
                    name: NO_NAME,
                    is_range_mapping: false,
                },
            ],
            vec![],
            None,
            None,
        );
        assert_eq!(sm.original_position_for(0, 2).unwrap().column, 2);
        assert_eq!(sm.original_position_for(0, 6).unwrap().column, 5);
    }

    #[test]
    fn range_mapping_indexed_test() {
        let sm = SourceMap::from_json(r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","rangeMappings":"A"}}]}"#).unwrap();
        assert!(sm.has_range_mappings());
        assert_eq!(sm.original_position_for(1, 3).unwrap().line, 1);
    }

    #[test]
    fn indexed_map_preserves_debug_id_extensions_and_scopes() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 2, column: 0 },
                name: None,
                kind: Some("function".to_string()),
                is_stack_frame: true,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 0, column: 4 },
                is_stack_frame: true,
                is_hidden: false,
                definition: Some(0),
                call_site: Some(CallSite { source_index: 0, line: 7, column: 2 }),
                bindings: vec![Binding::Unavailable],
                children: vec![],
            }],
        };
        let mut names = vec![];
        let scopes_str = srcmap_scopes::encode_scopes(&info, &mut names);
        let names_json = serde_json::to_string(&names).unwrap();
        let json = format!(
            r#"{{"version":3,"debugId":"indexed-debug","x_custom":{{"enabled":true}},"sections":[{{"offset":{{"line":2,"column":3}},"map":{{"version":3,"sources":["a.js"],"names":{names_json},"mappings":"AAAA","scopes":"{scopes_str}"}}}}]}}"#
        );

        let sm = SourceMap::from_json(&json).unwrap();

        assert_eq!(sm.debug_id.as_deref(), Some("indexed-debug"));
        assert_eq!(sm.extensions.get("x_custom"), Some(&serde_json::json!({ "enabled": true })));

        let scopes = sm.scopes.as_ref().unwrap();
        assert_eq!(scopes.scopes.len(), 1);
        assert!(scopes.scopes[0].is_some());
        assert_eq!(scopes.ranges.len(), 1);
        assert_eq!(scopes.ranges[0].start.line, 2);
        assert_eq!(scopes.ranges[0].start.column, 3);
        assert_eq!(scopes.ranges[0].end.line, 2);
        assert_eq!(scopes.ranges[0].end.column, 7);
        assert_eq!(scopes.ranges[0].definition, Some(0));
        assert_eq!(
            scopes.ranges[0].call_site,
            Some(CallSite { source_index: 0, line: 7, column: 2 })
        );
    }

    #[test]
    fn range_mapping_empty_string_test() {
        assert!(!SourceMap::from_json(r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"AAAA","rangeMappings":""}"#).unwrap().has_range_mappings());
    }

    #[test]
    fn range_mapping_lub_no_underflow() {
        // Range mapping at col 5, query col 2 with LUB bias
        // LUB should find the mapping at col 5, but NOT apply range delta
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"KAAK","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();

        let loc = sm.original_position_for_with_bias(0, 2, Bias::LeastUpperBound);
        assert!(loc.is_some());
        let loc = loc.unwrap();
        // Should return the mapping's own position, not apply a delta
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 5);
    }

    // ── Builder tests ──────────────────────────────────────────────

    #[test]
    fn builder_basic() {
        let sm = SourceMap::builder()
            .file("output.js")
            .sources(["input.ts"])
            .sources_content([Some("let x = 1;")])
            .names(["x"])
            .mappings([Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 4,
                name: 0,
                is_range_mapping: false,
            }])
            .build();

        assert_eq!(sm.file.as_deref(), Some("output.js"));
        assert_eq!(sm.sources, vec!["input.ts"]);
        assert_eq!(sm.sources_content, vec![Some("let x = 1;".to_string())]);
        assert_eq!(sm.names, vec!["x"]);
        assert_eq!(sm.mapping_count(), 1);

        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc.source), "input.ts");
        assert_eq!(loc.column, 4);
        assert_eq!(sm.name(loc.name.unwrap()), "x");
    }

    #[test]
    fn builder_empty() {
        let sm = SourceMap::builder().build();
        assert_eq!(sm.mapping_count(), 0);
        assert_eq!(sm.sources.len(), 0);
        assert_eq!(sm.names.len(), 0);
        assert!(sm.file.is_none());
    }

    #[test]
    fn builder_multiple_sources() {
        let sm = SourceMap::builder()
            .sources(["a.ts", "b.ts", "c.ts"])
            .sources_content([Some("// a"), Some("// b"), None])
            .mappings([
                Mapping {
                    generated_line: 0,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: u32::MAX,
                    is_range_mapping: false,
                },
                Mapping {
                    generated_line: 1,
                    generated_column: 0,
                    source: 1,
                    original_line: 0,
                    original_column: 0,
                    name: u32::MAX,
                    is_range_mapping: false,
                },
                Mapping {
                    generated_line: 2,
                    generated_column: 0,
                    source: 2,
                    original_line: 0,
                    original_column: 0,
                    name: u32::MAX,
                    is_range_mapping: false,
                },
            ])
            .build();

        assert_eq!(sm.sources.len(), 3);
        assert_eq!(sm.mapping_count(), 3);
        assert_eq!(sm.line_count(), 3);

        let loc0 = sm.original_position_for(0, 0).unwrap();
        assert_eq!(sm.source(loc0.source), "a.ts");

        let loc1 = sm.original_position_for(1, 0).unwrap();
        assert_eq!(sm.source(loc1.source), "b.ts");

        let loc2 = sm.original_position_for(2, 0).unwrap();
        assert_eq!(sm.source(loc2.source), "c.ts");
    }

    #[test]
    fn builder_with_iterators() {
        let source_names: Vec<String> = (0..5).map(|i| format!("mod_{i}.ts")).collect();
        let mappings = (0..5u32).map(|i| Mapping {
            generated_line: i,
            generated_column: 0,
            source: i,
            original_line: i,
            original_column: 0,
            name: u32::MAX,
            is_range_mapping: false,
        });

        let sm = SourceMap::builder()
            .sources(source_names.iter().map(|s| s.as_str()))
            .mappings(mappings)
            .build();

        assert_eq!(sm.sources.len(), 5);
        assert_eq!(sm.mapping_count(), 5);
        for i in 0..5u32 {
            let loc = sm.original_position_for(i, 0).unwrap();
            assert_eq!(sm.source(loc.source), format!("mod_{i}.ts"));
        }
    }

    #[test]
    fn builder_ignore_list_and_debug_id() {
        let sm = SourceMap::builder()
            .sources(["app.ts", "node_modules/lib.js"])
            .ignore_list([1])
            .debug_id("85314830-023f-4cf1-a267-535f4e37bb17")
            .build();

        assert_eq!(sm.ignore_list, vec![1]);
        assert_eq!(sm.debug_id.as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
    }

    #[test]
    fn builder_extensions_match_json_extension_filtering() {
        let sm = SourceMap::builder()
            .sources(["input.ts"])
            .mappings([Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: u32::MAX,
                is_range_mapping: false,
            }])
            .extension("x_google_ignoreList", serde_json::json!([0]))
            .extension("notExtension", serde_json::json!(true))
            .build();

        assert!(sm.extensions.contains_key("x_google_ignoreList"));
        assert!(!sm.extensions.contains_key("notExtension"));
        assert!(sm.original_position_for(0, 0).is_some());
    }

    #[test]
    fn builder_range_mappings() {
        let sm = SourceMap::builder()
            .sources(["input.ts"])
            .mappings([
                Mapping {
                    generated_line: 0,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: u32::MAX,
                    is_range_mapping: true,
                },
                Mapping {
                    generated_line: 0,
                    generated_column: 10,
                    source: 0,
                    original_line: 5,
                    original_column: 0,
                    name: u32::MAX,
                    is_range_mapping: false,
                },
            ])
            .build();

        assert!(sm.has_range_mappings());
        assert_eq!(sm.mapping_count(), 2);
    }

    #[test]
    fn builder_json_roundtrip() {
        let sm = SourceMap::builder()
            .file("out.js")
            .source_root("/src/")
            .sources(["a.ts", "b.ts"])
            .sources_content([Some("// a"), Some("// b")])
            .names(["foo", "bar"])
            .mappings([
                Mapping {
                    generated_line: 0,
                    generated_column: 0,
                    source: 0,
                    original_line: 0,
                    original_column: 0,
                    name: 0,
                    is_range_mapping: false,
                },
                Mapping {
                    generated_line: 1,
                    generated_column: 5,
                    source: 1,
                    original_line: 3,
                    original_column: 2,
                    name: 1,
                    is_range_mapping: false,
                },
            ])
            .build();

        let json = sm.to_json();
        let sm2 = SourceMap::from_json(&json).unwrap();

        assert_eq!(sm2.file, sm.file);
        // source_root is prepended to sources on parse
        assert_eq!(sm2.sources, vec!["/src/a.ts", "/src/b.ts"]);
        assert_eq!(sm2.names, sm.names);
        assert_eq!(sm2.mapping_count(), sm.mapping_count());

        for m in sm.all_mappings() {
            let a = sm.original_position_for(m.generated_line, m.generated_column);
            let b = sm2.original_position_for(m.generated_line, m.generated_column);
            match (a, b) {
                (Some(a), Some(b)) => {
                    assert_eq!(a.source, b.source);
                    assert_eq!(a.line, b.line);
                    assert_eq!(a.column, b.column);
                    assert_eq!(a.name, b.name);
                }
                (None, None) => {}
                _ => panic!("lookup mismatch"),
            }
        }
    }

    #[test]
    fn range_mapping_fallback_column_underflow() {
        // Range mapping at col 5, query col 2: nothing at or before col 2, so
        // None rather than an underflowing column delta.
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"KAAK","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.original_position_for(0, 2).is_none());
    }

    #[test]
    fn range_mapping_fallback_cross_line_column_zero() {
        // Range mapping on line 0, col 10, orig(0,10). Query line 1, col 0.
        // line_delta = 1, column_delta = 0 (else branch).
        // Result: orig_line = 0 + 1 = 1, orig_column = 10 + 0 = 10.
        let json = r#"{"version":3,"sources":["input.js"],"names":[],"mappings":"UAAU","rangeMappings":"A"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let loc = sm.original_position_for(1, 0).unwrap();
        assert_eq!(loc.line, 1);
        assert_eq!(loc.column, 10);
    }

    #[test]
    fn vlq_overflow_at_shift_60() {
        // Build a VLQ that uses exactly shift=60 (13 continuation chars + 1 terminator)
        // This should be rejected by vlq_fast (shift >= 60)
        // 13 continuation chars: each is base64 with continuation bit set (e.g. 'g' = 0x20 | 0x00)
        // followed by a terminator (e.g. 'A' = 0x00)
        let overflow_vlq = "ggggggggggggggA"; // 14 continuation + terminator
        let json = format!(
            r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"{}"}}"#,
            overflow_vlq
        );
        let result = SourceMap::from_json(&json);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::Vlq(_)));
    }

    #[test]
    fn lazy_sourcemap_rejects_indexed_maps() {
        let json = indexed_map();
        let result = LazySourceMap::from_json_fast(json);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::NestedIndexMap));

        let result = LazySourceMap::from_json_no_content(json);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), ParseError::NestedIndexMap));
    }

    #[test]
    fn source_map_no_content_rejects_indexed_maps() {
        let json = indexed_map();
        let err = SourceMap::from_json_no_content(json)
            .expect_err("indexed maps must not be parsed as empty regular maps");

        assert!(matches!(err, ParseError::NestedIndexMap));
    }

    #[test]
    fn source_map_lines_rejects_indexed_maps() {
        let json = indexed_map();
        let err = SourceMap::from_json_lines(json, 0, 1)
            .expect_err("indexed maps must not be parsed as empty regular maps");

        assert!(matches!(err, ParseError::NestedIndexMap));
    }

    #[test]
    fn lazy_source_map_rejects_indexed_maps() {
        let json = indexed_map();
        let err = LazySourceMap::from_json(json)
            .expect_err("indexed maps must not be parsed as empty regular maps");

        assert!(matches!(err, ParseError::NestedIndexMap));
    }

    #[test]
    fn lazy_sourcemap_get_source_name_bounds() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA"}"#;
        let sm = LazySourceMap::from_json_fast(json).unwrap();
        assert_eq!(sm.get_source(0), Some("a.js"));
        assert_eq!(sm.get_source(1), None);
        assert_eq!(sm.get_source(u32::MAX), None);
        assert_eq!(sm.get_name(0), Some("foo"));
        assert_eq!(sm.get_name(1), None);
        assert_eq!(sm.get_name(u32::MAX), None);
    }

    #[test]
    fn lazy_sourcemap_backward_seek() {
        // Test that backward seek works correctly in fast-scan mode
        let json =
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA;AACA;AACA;AACA"}"#;
        let sm = LazySourceMap::from_json_fast(json).unwrap();

        // Forward: decode lines 0, 1, 2, 3
        let loc3 = sm.original_position_for(3, 0).unwrap();
        assert_eq!(loc3.line, 3);

        // Backward: seek line 1 (below watermark of 4)
        let loc1 = sm.original_position_for(1, 0).unwrap();
        assert_eq!(loc1.line, 1);

        // Forward again: line 4
        let loc4 = sm.original_position_for(4, 0).unwrap();
        assert_eq!(loc4.line, 4);

        // Backward again to line 0
        let loc0 = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc0.line, 0);
    }

    #[test]
    fn lazy_sourcemap_fast_scan_matches_prescan_in_descending_and_randomized_order() {
        let mappings = std::iter::repeat_n("AACA,CAAC", 64).collect::<Vec<_>>().join(";");
        let json =
            format!(r#"{{"version":3,"sources":["a.js"],"names":[],"mappings":"{mappings}"}}"#);
        let fast = LazySourceMap::from_json_fast(&json).unwrap();
        let prescan = LazySourceMap::from_json_no_content(&json).unwrap();
        let position = |map: &LazySourceMap, line, column| {
            map.original_position_for(line, column)
                .map(|loc| (loc.source, loc.line, loc.column, loc.name))
        };

        for line in (0..64).rev() {
            assert_eq!(position(&fast, line, 1), position(&prescan, line, 1));
        }

        for line in [17, 2, 61, 9, 42, 0, 31, 5, 63, 23, 17, 42] {
            assert_eq!(position(&fast, line, 2), position(&prescan, line, 2));
        }
    }

    #[test]
    fn lazy_sourcemap_fast_scan_vs_prescan_consistency() {
        // Verify fast_scan and prescan produce identical lookup results
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":["x","y"],"mappings":"AAAAA,KACAC;ACAAD,KACAC"}"#;
        let fast = LazySourceMap::from_json_fast(json).unwrap();
        let prescan = LazySourceMap::from_json_no_content(json).unwrap();

        for line in 0..2 {
            for col in [0, 5, 10] {
                let a = fast.original_position_for(line, col);
                let b = prescan.original_position_for(line, col);
                match (&a, &b) {
                    (Some(a), Some(b)) => {
                        assert_eq!(a.source, b.source, "line={line}, col={col}");
                        assert_eq!(a.line, b.line, "line={line}, col={col}");
                        assert_eq!(a.column, b.column, "line={line}, col={col}");
                        assert_eq!(a.name, b.name, "line={line}, col={col}");
                    }
                    (None, None) => {}
                    _ => panic!("mismatch at line={line}, col={col}: {a:?} vs {b:?}"),
                }
            }
        }
    }

    #[test]
    fn mappings_iter_rejects_two_field_segment() {
        // "AA" is 2 fields (generated column + source index, missing original line/column)
        let result: Result<Vec<_>, _> = MappingsIter::new("AA").collect();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::InvalidSegmentLength { fields: 2, .. }));
    }

    #[test]
    fn mappings_iter_rejects_three_field_segment() {
        // "AAA" is 3 fields (generated column + source index + original line, missing original column)
        let result: Result<Vec<_>, _> = MappingsIter::new("AAA").collect();
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), DecodeError::InvalidSegmentLength { fields: 3, .. }));
    }

    #[test]
    fn decode_mappings_range_caps_end_line() {
        // Pathological end_line should not OOM — capped against actual line count
        let mappings = "AAAA;AACA";
        let (result, offsets, _) =
            decode_mappings_range(mappings, 0, 1_000_000, DecodeChecks::without_index_limits())
                .unwrap();
        // Should produce mappings for the 2 actual lines, not allocate 1M entries
        assert_eq!(result.len(), 2);
        assert!(offsets.len() <= 3); // 2 lines + sentinel
    }

    #[test]
    fn decode_range_mappings_cross_line_bound_check() {
        // Range mapping index that exceeds the current line's mappings
        // should NOT mark a mapping on the next line
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AAAA","rangeMappings":"E"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        // Line 0 has 1 mapping (idx 0). rangeMappings="E" encodes index 2, which is out of bounds
        // for line 0. Line 1's mapping (idx 1) should NOT be marked as range mapping.
        assert!(!sm.all_mappings()[1].is_range_mapping);
    }

    #[test]
    fn fast_scan_lines_no_semicolons() {
        let result = fast_scan_lines("AAAA,CAAC");
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].byte_offset, 0);
        assert_eq!(result[0].byte_end, 9);
    }

    #[test]
    fn fast_scan_lines_only_semicolons() {
        let result = fast_scan_lines(";;;");
        assert_eq!(result.len(), 4);
        for info in &result {
            assert_eq!(info.byte_offset, info.byte_end); // empty lines
        }
    }

    // ── from_data_url ────────────────────────────────────────────

    #[test]
    fn from_data_url_base64() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let encoded = utils::base64_encode(json.as_bytes());
        let url = format!("data:application/json;base64,{encoded}");
        let sm = SourceMap::from_data_url(&url).unwrap();
        assert_eq!(sm.sources, vec!["a.js"]);
        let loc = sm.original_position_for(0, 0).unwrap();
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn from_data_url_base64_charset_utf8() {
        let json = r#"{"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}"#;
        let encoded = utils::base64_encode(json.as_bytes());
        let url = format!("data:application/json;charset=utf-8;base64,{encoded}");
        let sm = SourceMap::from_data_url(&url).unwrap();
        assert_eq!(sm.sources, vec!["b.js"]);
    }

    #[test]
    fn from_data_url_plain_json() {
        let json = r#"{"version":3,"sources":["c.js"],"names":[],"mappings":"AAAA"}"#;
        let url = format!("data:application/json,{json}");
        let sm = SourceMap::from_data_url(&url).unwrap();
        assert_eq!(sm.sources, vec!["c.js"]);
    }

    #[test]
    fn from_data_url_percent_encoded() {
        let url = "data:application/json,%7B%22version%22%3A3%2C%22sources%22%3A%5B%22d.js%22%5D%2C%22names%22%3A%5B%5D%2C%22mappings%22%3A%22AAAA%22%7D";
        let sm = SourceMap::from_data_url(url).unwrap();
        assert_eq!(sm.sources, vec!["d.js"]);
    }

    #[test]
    fn from_data_url_invalid_prefix() {
        let result = SourceMap::from_data_url("data:text/plain;base64,abc");
        assert!(result.is_err());
    }

    #[test]
    fn from_data_url_not_a_data_url() {
        let result = SourceMap::from_data_url("https://example.com/foo.map");
        assert!(result.is_err());
    }

    #[test]
    fn from_data_url_invalid_base64() {
        let result = SourceMap::from_data_url("data:application/json;base64,!!!invalid!!!");
        assert!(result.is_err());
    }

    // ── to_writer ────────────────────────────────────────────────

    #[test]
    fn to_writer_matches_to_json() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":["foo"],"mappings":"AACAA,GCAA","sourcesContent":["var foo;","var bar;"]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let expected = sm.to_json();
        let mut buf = Vec::new();
        sm.to_writer(&mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert_eq!(output, expected);
    }

    #[test]
    fn to_writer_with_options_excludes_content() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","sourcesContent":["var x;"]}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let mut buf = Vec::new();
        sm.to_writer_with_options(&mut buf, true).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(!output.contains("sourcesContent"));
    }

    // ── Setter tests ─────────────────────────────────────────────

    #[test]
    fn set_file() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.file, None);

        sm.set_file(Some("output.js".to_string()));
        assert_eq!(sm.file, Some("output.js".to_string()));
        assert!(sm.to_json().contains(r#""file":"output.js""#));

        sm.set_file(None);
        assert_eq!(sm.file, None);
        assert!(!sm.to_json().contains("file"));
    }

    #[test]
    fn set_source_root() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.source_root, None);

        sm.set_source_root(Some("src/".to_string()));
        assert_eq!(sm.source_root, Some("src/".to_string()));
        assert!(sm.to_json().contains(r#""sourceRoot":"src/""#));

        sm.set_source_root(None);
        assert_eq!(sm.source_root, None);
    }

    #[test]
    fn set_debug_id() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.debug_id, None);

        sm.set_debug_id(Some("abc-123".to_string()));
        assert_eq!(sm.debug_id, Some("abc-123".to_string()));
        assert!(sm.to_json().contains(r#""debugId":"abc-123""#));

        sm.set_debug_id(None);
        assert_eq!(sm.debug_id, None);
        assert!(!sm.to_json().contains("debugId"));
    }

    #[test]
    fn set_ignore_list() {
        let json = r#"{"version":3,"sources":["a.js","b.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.ignore_list, Vec::<u32>::new());

        sm.set_ignore_list(vec![0, 1]);
        assert_eq!(sm.ignore_list, vec![0, 1]);
        assert!(sm.to_json().contains("\"ignoreList\":[0,1]"));

        sm.set_ignore_list(vec![]);
        assert_eq!(sm.ignore_list, Vec::<u32>::new());
        assert!(!sm.to_json().contains("ignoreList"));
    }

    #[test]
    fn set_sources() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources, vec!["a.js"]);

        sm.set_sources(vec![Some("x.js".to_string()), Some("y.js".to_string())]);
        assert_eq!(sm.sources, vec!["x.js", "y.js"]);
        assert_eq!(sm.source_index("x.js"), Some(0));
        assert_eq!(sm.source_index("y.js"), Some(1));
        assert_eq!(sm.source_index("a.js"), None);
    }

    #[test]
    fn set_sources_with_source_root() {
        let json =
            r#"{"version":3,"sourceRoot":"src/","sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let mut sm = SourceMap::from_json(json).unwrap();
        assert_eq!(sm.sources, vec!["src/a.js"]);

        sm.set_sources(vec![Some("b.js".to_string())]);
        assert_eq!(sm.sources, vec!["src/b.js"]);
    }

    #[test]
    fn to_data_url_roundtrip() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let sm = SourceMap::from_json(json).unwrap();
        let url = sm.to_data_url();
        assert!(url.starts_with("data:application/json;base64,"));
        let sm2 = SourceMap::from_data_url(&url).unwrap();
        assert_eq!(sm.sources, sm2.sources);
        assert_eq!(sm.to_json(), sm2.to_json());
    }
}
