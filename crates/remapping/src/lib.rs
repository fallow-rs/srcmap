//! Source map concatenation and composition/remapping.
//!
//! **Concatenation** merges source maps from multiple bundled files into one,
//! adjusting line/column offsets. Used by bundlers (esbuild, Rollup, Webpack).
//!
//! **Composition/remapping** chains source maps through multiple transforms
//! (e.g. TS → JS → minified) into a single map pointing to original sources.
//! Equivalent to `@ampproject/remapping` in the JS ecosystem.
//!
//! # Examples
//!
//! ## Concatenation
//!
//! ```
//! use srcmap_remapping::ConcatBuilder;
//! use srcmap_sourcemap::SourceMap;
//!
//! fn main() {
//!     let map_a = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
//!     let map_b = r#"{"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}"#;
//!
//!     let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
//!     builder.add_map(&SourceMap::from_json(map_a).unwrap(), 0);
//!     builder.add_map(&SourceMap::from_json(map_b).unwrap(), 1);
//!
//!     let result = builder.build();
//!     assert_eq!(result.mapping_count(), 2);
//!     assert_eq!(result.line_count(), 2);
//! }
//! ```
//!
//! ## Composition / Remapping
//!
//! ```
//! use srcmap_remapping::remap;
//! use srcmap_sourcemap::SourceMap;
//!
//! fn main() {
//!     // Transform: original.js → intermediate.js → output.js
//!     let outer = r#"{"version":3,"sources":["intermediate.js"],"names":[],"mappings":"AAAA;AACA"}"#;
//!     let inner = r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AACA;AACA"}"#;
//!
//!     let result = remap(
//!         &SourceMap::from_json(outer).unwrap(),
//!         |source| {
//!             if source == "intermediate.js" {
//!                 Some(SourceMap::from_json(inner).unwrap())
//!             } else {
//!                 None
//!             }
//!         },
//!     );
//!
//!     // Result maps output.js directly to original.js
//!     assert_eq!(result.sources, vec!["original.js"]);
//! }
//! ```

use srcmap_generator::{SourceMapGenerator, StreamingGenerator};
use srcmap_sourcemap::SourceMap;
use std::collections::HashSet;

const NO_SOURCE: u32 = u32::MAX;
const NO_NAME: u32 = u32::MAX;

// ── Concatenation ─────────────────────────────────────────────────

/// Builder for concatenating multiple source maps into one.
///
/// Each added source map is offset by a line delta, producing a single
/// combined map. Sources and names are deduplicated across inputs.
pub struct ConcatBuilder {
    builder: SourceMapGenerator,
}

impl ConcatBuilder {
    /// Create a new concatenation builder.
    pub fn new(file: Option<String>) -> Self {
        Self { builder: SourceMapGenerator::new(file) }
    }

    /// Add a source map to the concatenated output.
    ///
    /// `line_offset` is the number of lines to shift all mappings by
    /// (i.e. the line at which this chunk starts in the output).
    pub fn add_map(&mut self, sm: &SourceMap, line_offset: u32) {
        let source_indices = add_concat_sources(&mut self.builder, sm);
        // Pre-build name index remap table (once per input map)
        let name_indices: Vec<u32> = sm.names.iter().map(|n| self.builder.add_name(n)).collect();

        // Copy ignore_list entries
        for &ignored in &sm.ignore_list {
            if let Some(&global_idx) = source_indices.get(ignored as usize) {
                self.builder.add_to_ignore_list(global_idx);
            }
        }

        // Add all mappings with line offset, using pre-built index tables.
        // Out-of-range source and name indices are dropped, not followed.
        for m in sm.all_mappings() {
            let gen_line = m.generated_line + line_offset;

            if let Some(&src) = source_indices.get(m.source as usize) {
                let name = name_indices.get(m.name as usize).copied();
                match (name, m.is_range_mapping) {
                    (Some(name), true) => self.builder.add_named_range_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                        name,
                    ),
                    (Some(name), false) => self.builder.add_named_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                        name,
                    ),
                    (None, true) => self.builder.add_range_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                    ),
                    (None, false) => self.builder.add_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                    ),
                }
            } else {
                self.builder.add_generated_mapping(gen_line, m.generated_column);
            }
        }
    }

    /// Serialize the current state as a JSON string.
    pub fn to_json(&self) -> String {
        self.builder.to_json()
    }

    /// Serialize the current state as a decoded `SourceMap`.
    pub fn build(&self) -> SourceMap {
        self.builder.to_decoded_map()
    }
}

fn add_concat_sources(builder: &mut SourceMapGenerator, sm: &SourceMap) -> Vec<u32> {
    sm.sources
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let idx = builder.add_source(s);
            if let Some(Some(content)) = sm.sources_content.get(i) {
                builder.set_source_content(idx, content.clone());
            }
            idx
        })
        .collect()
}

// ── Composition / Remapping ───────────────────────────────────────

/// Cached per-upstream-map data: lazy index remap tables.
/// Sources and names are only registered in the builder when a mapping
/// actually references them, matching jridgewell's behavior of not
/// including unreferenced sources/names in the output.
struct UpstreamCache {
    /// upstream source idx → builder source idx (lazily populated)
    source_remap: Vec<Option<u32>>,
    /// upstream name idx → builder name idx (lazily populated)
    name_remap: Vec<Option<u32>>,
    /// Where the last lookup in this upstream map ended.
    cursor: LookupCursor,
}

/// The line and the index in that line of the last lookup hit.
///
/// The segments of an outer map mostly trace to rising positions in the
/// upstream map. The next lookup starts at the last hit and looks at a few
/// segments after it before it does a binary search.
#[derive(Clone, Copy, Default)]
struct LookupCursor {
    line: u32,
    index: usize,
}

/// How many segments after the last hit a lookup reads before a binary search.
const CURSOR_SCAN_LIMIT: usize = 8;

/// The source content that each builder source got last.
///
/// jridgewell sets the content of a source on each traced segment, so when two
/// sources share a name, the content of the last traced segment wins. The
/// tracker gives the same result and calls `set_source_content` only when the
/// content comes from another place than the last time.
#[derive(Default)]
struct ContentTracker {
    last: Vec<*const String>,
}

impl ContentTracker {
    #[inline]
    fn apply<B: RemapBuilder>(&mut self, builder: &mut B, idx: u32, content: Option<&String>) {
        let Some(content) = content else { return };
        let i = idx as usize;
        if i >= self.last.len() {
            self.last.resize(i + 1, std::ptr::null());
        }
        if !std::ptr::eq(self.last[i], content) {
            builder.set_source_content(idx, content.clone());
            self.last[i] = content;
        }
    }
}

/// Build lazy index remap tables for an upstream map.
fn build_upstream_cache(upstream_sm: &SourceMap) -> UpstreamCache {
    UpstreamCache {
        source_remap: vec![None; upstream_sm.sources.len()],
        name_remap: vec![None; upstream_sm.names.len()],
        cursor: LookupCursor::default(),
    }
}

/// Builder operations needed by the remapping hot path.
trait RemapBuilder {
    fn add_source(&mut self, source: &str) -> u32;
    fn set_source_content(&mut self, idx: u32, content: String);
    fn add_name(&mut self, name: &str) -> u32;
    fn add_to_ignore_list(&mut self, idx: u32);
    fn add_generated_mapping(&mut self, gen_line: u32, gen_col: u32);
    fn add_mapping(&mut self, gen_line: u32, gen_col: u32, src: u32, orig_line: u32, orig_col: u32);
    fn add_named_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    );
    fn add_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
    );
    fn add_named_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    );
}

impl RemapBuilder for SourceMapGenerator {
    fn add_source(&mut self, source: &str) -> u32 {
        SourceMapGenerator::add_source(self, source)
    }

    fn set_source_content(&mut self, idx: u32, content: String) {
        SourceMapGenerator::set_source_content(self, idx, content)
    }

    fn add_name(&mut self, name: &str) -> u32 {
        SourceMapGenerator::add_name(self, name)
    }

    fn add_to_ignore_list(&mut self, idx: u32) {
        SourceMapGenerator::add_to_ignore_list(self, idx)
    }

    fn add_generated_mapping(&mut self, gen_line: u32, gen_col: u32) {
        SourceMapGenerator::add_generated_mapping(self, gen_line, gen_col)
    }

    fn add_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
    ) {
        SourceMapGenerator::add_mapping(self, gen_line, gen_col, src, orig_line, orig_col)
    }

    fn add_named_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    ) {
        SourceMapGenerator::add_named_mapping(
            self, gen_line, gen_col, src, orig_line, orig_col, name,
        )
    }

    fn add_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
    ) {
        SourceMapGenerator::add_range_mapping(self, gen_line, gen_col, src, orig_line, orig_col)
    }

    fn add_named_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    ) {
        SourceMapGenerator::add_named_range_mapping(
            self, gen_line, gen_col, src, orig_line, orig_col, name,
        )
    }
}

impl RemapBuilder for StreamingGenerator {
    fn add_source(&mut self, source: &str) -> u32 {
        StreamingGenerator::add_source(self, source)
    }

    fn set_source_content(&mut self, idx: u32, content: String) {
        StreamingGenerator::set_source_content(self, idx, content)
    }

    fn add_name(&mut self, name: &str) -> u32 {
        StreamingGenerator::add_name(self, name)
    }

    fn add_to_ignore_list(&mut self, idx: u32) {
        StreamingGenerator::add_to_ignore_list(self, idx)
    }

    fn add_generated_mapping(&mut self, gen_line: u32, gen_col: u32) {
        StreamingGenerator::add_generated_mapping(self, gen_line, gen_col)
    }

    fn add_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
    ) {
        StreamingGenerator::add_mapping(self, gen_line, gen_col, src, orig_line, orig_col)
    }

    fn add_named_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    ) {
        StreamingGenerator::add_named_mapping(
            self, gen_line, gen_col, src, orig_line, orig_col, name,
        )
    }

    fn add_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
    ) {
        StreamingGenerator::add_range_mapping(self, gen_line, gen_col, src, orig_line, orig_col)
    }

    fn add_named_range_mapping(
        &mut self,
        gen_line: u32,
        gen_col: u32,
        src: u32,
        orig_line: u32,
        orig_col: u32,
        name: u32,
    ) {
        StreamingGenerator::add_named_range_mapping(
            self, gen_line, gen_col, src, orig_line, orig_col, name,
        )
    }
}

/// Resolve an upstream source index to a builder source index, lazily
/// registering the source (and its ignore status) on first use.
#[inline]
fn resolve_upstream_source<B: RemapBuilder>(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_src: u32,
    builder: &mut B,
    ignored_sources: &mut HashSet<u32>,
) -> u32 {
    let si = upstream_src as usize;
    if let Some(idx) = cache.source_remap[si] {
        return idx;
    }
    let idx = builder.add_source(&upstream_sm.sources[si]);
    if upstream_sm.ignore_list.contains(&upstream_src) && ignored_sources.insert(idx) {
        builder.add_to_ignore_list(idx);
    }
    cache.source_remap[si] = Some(idx);
    idx
}

/// Resolve an upstream name index to a builder name index, lazily
/// registering the name on first use.
#[inline]
fn resolve_upstream_name<B: RemapBuilder>(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_name: u32,
    builder: &mut B,
) -> u32 {
    let ni = upstream_name as usize;
    if let Some(idx) = cache.name_remap[ni] {
        return idx;
    }
    let idx = builder.add_name(&upstream_sm.names[ni]);
    cache.name_remap[ni] = Some(idx);
    idx
}

/// Look up the original position using the upstream map's line_offsets for O(1)
/// line access, then binary search within the line slice.
/// This is semantically equivalent to `upstream_sm.original_position_for()` with
/// `GreatestLowerBound` bias, but inlined to avoid function call overhead and
/// to return the raw `Mapping` reference for index-based remapping.
///
/// Falls back to range mapping search when the queried line has no mappings or the
/// column is before the first mapping on the line — matching `original_position_for`.
///
/// A position that lands on a generated-only segment gives `UpstreamTrace::Sourceless`.
/// The caller then emits a sourceless segment, as jridgewell does: that segment ends
/// the range of the previous mapping on the line.
#[inline]
fn lookup_upstream(
    upstream_sm: &SourceMap,
    cursor: &mut LookupCursor,
    line: u32,
    column: u32,
) -> UpstreamTrace {
    let line_mappings = upstream_sm.mappings_for_line(line);
    if line_mappings.is_empty() {
        return fallback_to_full_lookup(upstream_sm, line, column);
    }

    let Some(last_at_or_before) = last_at_or_before(line_mappings, cursor, line, column) else {
        return fallback_to_full_lookup(upstream_sm, line, column);
    };
    *cursor = LookupCursor { line, index: last_at_or_before };

    // On an exact column match, take the first segment at that column, as the
    // `lowerBound` step of jridgewell's `traceSegment` does. A line can hold
    // several segments at one column.
    let mut idx = last_at_or_before;
    if line_mappings[idx].generated_column == column {
        while idx > 0 && line_mappings[idx - 1].generated_column == column {
            idx -= 1;
        }
    }

    let mapping = &line_mappings[idx];
    if mapping.source == NO_SOURCE {
        return UpstreamTrace::Sourceless;
    }

    let original_column = if mapping.is_range_mapping && column >= mapping.generated_column {
        mapping.original_column + (column - mapping.generated_column)
    } else {
        mapping.original_column
    };

    UpstreamTrace::Found(UpstreamLookup {
        source: mapping.source,
        original_line: mapping.original_line,
        original_column,
        name: mapping.name,
    })
}

/// The index of the last segment in `line_mappings` with a column at or before
/// `column`, or `None` when the first segment is after `column`.
///
/// When the cursor is on this line at or before `column`, the search starts
/// there. Otherwise it is a binary search of the full line.
#[inline]
fn last_at_or_before(
    line_mappings: &[srcmap_sourcemap::Mapping],
    cursor: &LookupCursor,
    line: u32,
    column: u32,
) -> Option<usize> {
    let on_cursor = cursor.line == line
        && line_mappings.get(cursor.index).is_some_and(|m| m.generated_column <= column);
    let start = if on_cursor {
        let last = line_mappings.len() - 1;
        let scan_end = (cursor.index + CURSOR_SCAN_LIMIT).min(last);
        let mut index = cursor.index;
        while index < scan_end && line_mappings[index + 1].generated_column <= column {
            index += 1;
        }
        if index < scan_end || index == last {
            return Some(index);
        }
        // Every segment in the scan is at or before `column`. Search the rest.
        index
    } else {
        0
    };
    let after = start + line_mappings[start..].partition_point(|m| m.generated_column <= column);
    after.checked_sub(1)
}

/// Result of tracing one position through an upstream source map.
enum UpstreamTrace {
    /// The position maps to an original location.
    Found(UpstreamLookup),
    /// The position lands on a generated-only segment.
    Sourceless,
    /// No segment covers the position. The caller drops the outer segment.
    Missing,
}

/// Result of looking up a position in an upstream source map.
/// Carries the resolved source/name indices and original position directly,
/// so callers don't need to re-inspect the mapping.
struct UpstreamLookup {
    source: u32,
    original_line: u32,
    original_column: u32,
    name: u32,
}

/// Fall back to the full `original_position_for` when the inlined lookup can't
/// resolve (empty line or column before first mapping). This handles range mapping
/// fallback correctly. Only called on the rare path where the line has no direct
/// mappings, so the function call overhead is acceptable.
fn fallback_to_full_lookup(upstream_sm: &SourceMap, line: u32, column: u32) -> UpstreamTrace {
    let Some(loc) = upstream_sm.original_position_for(line, column) else {
        return UpstreamTrace::Missing;
    };
    UpstreamTrace::Found(UpstreamLookup {
        source: loc.source,
        original_line: loc.line,
        original_column: loc.column,
        name: loc.name.unwrap_or(NO_NAME),
    })
}

/// Resolve an outer name index to a builder name index, caching the result.
#[inline]
fn resolve_outer_name_cached<B: RemapBuilder>(
    outer_name_remap: &mut [Option<u32>],
    name_idx: u32,
    names: &[String],
    builder: &mut B,
) -> Option<u32> {
    if name_idx == NO_NAME {
        return None;
    }
    let slot = outer_name_remap.get_mut(name_idx as usize)?;
    if let Some(idx) = *slot {
        return Some(idx);
    }
    let outer_name = names.get(name_idx as usize)?;
    let idx = builder.add_name(outer_name);
    *slot = Some(idx);
    Some(idx)
}

/// Emit a mapping to the builder using pre-built index remap tables.
/// Uses indices directly, avoiding per-mapping string hashing.
#[inline]
#[allow(
    clippy::too_many_arguments,
    reason = "passing remapped indices avoids per-mapping hashing in the hot path"
)]
fn emit_remapped_mapping<B: RemapBuilder>(
    builder: &mut B,
    gen_line: u32,
    gen_col: u32,
    builder_src: u32,
    orig_line: u32,
    orig_col: u32,
    builder_name: Option<u32>,
    is_range: bool,
) {
    match (builder_name, is_range) {
        (Some(n), true) => {
            builder.add_named_range_mapping(gen_line, gen_col, builder_src, orig_line, orig_col, n);
        }
        (Some(n), false) => {
            builder.add_named_mapping(gen_line, gen_col, builder_src, orig_line, orig_col, n);
        }
        (None, true) => {
            builder.add_range_mapping(gen_line, gen_col, builder_src, orig_line, orig_col);
        }
        (None, false) => {
            builder.add_mapping(gen_line, gen_col, builder_src, orig_line, orig_col);
        }
    }
}

/// An upstream map that a loader gave: its own copy, or a reference to a map
/// of the caller.
enum UpstreamMap<'a> {
    Owned(Box<SourceMap>),
    Borrowed(&'a SourceMap),
}

impl UpstreamMap<'_> {
    #[inline]
    fn get(&self) -> &SourceMap {
        match self {
            Self::Owned(map) => map,
            Self::Borrowed(map) => map,
        }
    }
}

/// A loader for the trace loop. A trait object keeps one copy of that loop in
/// the binary for all loader types. The loop calls it one time per source.
type UpstreamLoader<'l, 'a> = &'l dyn Fn(&str) -> Option<UpstreamMap<'a>>;

/// Per-source entry: either an upstream map + cache, or a passthrough.
/// Using an enum avoids two separate HashMap lookups per mapping.
enum SourceEntry<'a> {
    /// Has an upstream map: trace mappings through it.
    Upstream { map: UpstreamMap<'a>, cache: UpstreamCache },
    /// No upstream map: pass through with builder source index.
    Passthrough { builder_src: u32 },
    /// Empty-string source (from JSON `null`): emit as generated-only.
    /// Matches jridgewell's behavior where `!source` triggers a sourceless segment.
    /// `ignored` is true when the outer `ignoreList` holds the source.
    EmptySource { ignored: bool },
    /// Not yet loaded.
    Unloaded,
}

/// State for tracking the last emitted segment per generated line.
/// Used to implement jridgewell's `skipSourceless` and `skipSource` deduplication.
struct DedupeState {
    /// Generated line of the last emitted segment.
    last_gen_line: u32,
    /// Index of the last emitted segment on the current generated line (0-based).
    line_index: u32,
    /// Whether the last emitted segment was sourceless.
    last_was_sourceless: bool,
    /// (source, orig_line, orig_col, name) of the last emitted sourced segment on this line.
    last_source: Option<(u32, u32, u32, Option<u32>)>,
}

impl DedupeState {
    fn new() -> Self {
        Self {
            last_gen_line: u32::MAX,
            line_index: 0,
            last_was_sourceless: false,
            last_source: None,
        }
    }

    /// Check if a sourceless segment should be skipped (jridgewell's `skipSourceless`).
    /// Skip if: (1) first segment on the line, or (2) previous segment was also sourceless.
    fn skip_sourceless(&self, gen_line: u32) -> bool {
        if gen_line != self.last_gen_line {
            // First segment on a new line → skip
            return true;
        }
        // Consecutive sourceless → skip
        self.last_was_sourceless
    }

    /// Check if a sourced segment should be skipped (jridgewell's `skipSource`).
    /// Skip if previous segment on the same line has identical (source, line, col, name).
    fn skip_source(
        &self,
        gen_line: u32,
        source: u32,
        orig_line: u32,
        orig_col: u32,
        name: Option<u32>,
    ) -> bool {
        if gen_line != self.last_gen_line {
            // First segment on a new line → never skip
            return false;
        }
        if self.last_was_sourceless {
            // Previous was sourceless → never skip (transition to sourced)
            return false;
        }
        // Skip if identical to the previous sourced segment
        self.last_source == Some((source, orig_line, orig_col, name))
    }

    /// Record that a sourceless segment was emitted.
    fn record_sourceless(&mut self, gen_line: u32) {
        if gen_line != self.last_gen_line {
            self.last_gen_line = gen_line;
            self.line_index = 0;
            self.last_source = None;
        }
        self.line_index += 1;
        self.last_was_sourceless = true;
    }

    /// Record that a sourced segment was emitted.
    fn record_source(
        &mut self,
        gen_line: u32,
        source: u32,
        orig_line: u32,
        orig_col: u32,
        name: Option<u32>,
    ) {
        if gen_line != self.last_gen_line {
            self.last_gen_line = gen_line;
            self.line_index = 0;
        }
        self.line_index += 1;
        self.last_was_sourceless = false;
        self.last_source = Some((source, orig_line, orig_col, name));
    }
}

/// Remap a source map by resolving each source through upstream source maps.
///
/// For each source in the `outer` map, the `loader` function is called to
/// retrieve the upstream source map. If a source map is returned, mappings
/// are traced through it to the original source. If `None` is returned,
/// the source is kept as-is.
///
/// Range mappings (`is_range_mapping`) are preserved through composition.
/// The `ignore_list` from both upstream and outer maps is propagated.
///
/// Redundant mappings are skipped to match `@jridgewell/remapping` output:
/// - Sourceless segments at position 0 on a line are dropped.
/// - Consecutive sourceless segments on the same line are dropped.
/// - Sourced segments identical to the previous segment on the same line are dropped.
///
/// This is equivalent to `@ampproject/remapping` in the JS ecosystem.
pub fn remap<F>(outer: &SourceMap, loader: F) -> SourceMap
where
    F: Fn(&str) -> Option<SourceMap>,
{
    remap_with(outer, &|source| loader(source).map(|map| UpstreamMap::Owned(Box::new(map))))
}

/// Like [`remap`], for upstream maps that the caller already has in memory.
///
/// The loader returns a reference, so the function does not copy an upstream
/// map. Use it when the upstream maps live longer than the call.
///
/// # Examples
///
/// ```
/// use srcmap_remapping::remap_ref;
/// use srcmap_sourcemap::SourceMap;
///
/// let outer = SourceMap::from_json(
///     r#"{"version":3,"sources":["inter.js"],"names":[],"mappings":"AAAA;AACA"}"#,
/// )
/// .unwrap();
/// let inner = SourceMap::from_json(
///     r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AAAA;AACA"}"#,
/// )
/// .unwrap();
///
/// let composed = remap_ref(&outer, |_source| Some(&inner));
/// assert_eq!(composed.sources, vec!["original.js"]);
/// ```
pub fn remap_ref<'a, F>(outer: &SourceMap, loader: F) -> SourceMap
where
    F: Fn(&str) -> Option<&'a SourceMap>,
{
    remap_with(outer, &|source| loader(source).map(UpstreamMap::Borrowed))
}

fn remap_with(outer: &SourceMap, loader: UpstreamLoader<'_, '_>) -> SourceMap {
    let mapping_count = outer.mapping_count();
    let source_count = outer.sources.len();
    let mut builder = SourceMapGenerator::with_capacity(outer.file.clone(), mapping_count);
    // Mappings are emitted in the same order as outer (already sorted).
    builder.set_assume_sorted(true);

    // Flat Vec indexed by outer source index — avoids HashMap per mapping.
    let mut source_entries: Vec<SourceEntry<'_>> =
        std::iter::repeat_with(|| SourceEntry::Unloaded).take(source_count).collect();

    let mut ignored_sources: HashSet<u32> = HashSet::new();

    // Lazy outer name passthrough table (outer name idx → builder name idx)
    let mut outer_name_remap: Vec<Option<u32>> = vec![None; outer.names.len()];

    // Pre-compute outer ignore set for O(1) lookups
    let outer_ignore_set: HashSet<u32> = outer.ignore_list.iter().copied().collect();

    let mut dedup = DedupeState::new();
    let mut contents = ContentTracker::default();

    for m in outer.all_mappings() {
        if m.source == NO_SOURCE {
            trace_and_emit_sourceless(
                &mut builder,
                &mut dedup,
                m.generated_line,
                m.generated_column,
            );
            continue;
        }

        let si = m.source as usize;
        if si >= outer.sources.len() {
            continue;
        }

        load_source_entry(
            &mut source_entries,
            si,
            &outer.sources,
            SourceEntryLoadContext {
                builder: &mut builder,
                outer_ignore_set: &outer_ignore_set,
                ignored_sources: &mut ignored_sources,
            },
            loader,
        );

        trace_source_entry(
            &mut source_entries[si],
            m,
            TraceContext {
                builder: &mut builder,
                dedup: &mut dedup,
                contents: &mut contents,
                outer_name_remap: &mut outer_name_remap,
                names: &outer.names,
                sources_content: &outer.sources_content,
                ignored_sources: &mut ignored_sources,
            },
        );
    }

    builder.to_decoded_map()
}

struct TraceContext<'a, B> {
    builder: &'a mut B,
    dedup: &'a mut DedupeState,
    contents: &'a mut ContentTracker,
    outer_name_remap: &'a mut [Option<u32>],
    names: &'a [String],
    sources_content: &'a [Option<String>],
    ignored_sources: &'a mut HashSet<u32>,
}

fn trace_source_entry<B: RemapBuilder>(
    entry: &mut SourceEntry<'_>,
    m: &srcmap_sourcemap::Mapping,
    ctx: TraceContext<'_, B>,
) {
    match entry {
        SourceEntry::Upstream { map, cache } => {
            let map = map.get();
            match lookup_upstream(map, &mut cache.cursor, m.original_line, m.original_column) {
                UpstreamTrace::Found(upstream_m) => trace_and_emit_upstream(
                    ctx.builder,
                    ctx.dedup,
                    UpstreamEmitContext {
                        contents: ctx.contents,
                        gen_line: m.generated_line,
                        gen_col: m.generated_column,
                        upstream_m,
                        cache,
                        upstream_map: map,
                        outer_name_remap: ctx.outer_name_remap,
                        outer_name_idx: m.name,
                        names: ctx.names,
                        ignored_sources: ctx.ignored_sources,
                        is_range: m.is_range_mapping,
                    },
                ),
                UpstreamTrace::Sourceless => trace_and_emit_sourceless(
                    ctx.builder,
                    ctx.dedup,
                    m.generated_line,
                    m.generated_column,
                ),
                UpstreamTrace::Missing => {}
            }
        }
        SourceEntry::Passthrough { builder_src } => {
            trace_and_emit_passthrough(
                ctx.builder,
                ctx.dedup,
                PassthroughEmitContext {
                    contents: ctx.contents,
                    content: ctx.sources_content.get(m.source as usize).and_then(Option::as_ref),
                    gen_line: m.generated_line,
                    gen_col: m.generated_column,
                    orig_line: m.original_line,
                    orig_col: m.original_column,
                    builder_src: *builder_src,
                    outer_name_remap: ctx.outer_name_remap,
                    outer_name_idx: m.name,
                    names: ctx.names,
                    is_range: m.is_range_mapping,
                },
            );
        }
        SourceEntry::EmptySource { ignored } => {
            if *ignored {
                ignore_empty_source(ctx.builder, ctx.ignored_sources);
            }
            trace_and_emit_sourceless(ctx.builder, ctx.dedup, m.generated_line, m.generated_column);
        }
        SourceEntry::Unloaded => unreachable!(),
    }
}

struct SourceEntryLoadContext<'a, B> {
    builder: &'a mut B,
    outer_ignore_set: &'a HashSet<u32>,
    ignored_sources: &'a mut HashSet<u32>,
}

/// Resolve outer source `si` on first use: load its upstream map through
/// `loader`, or register it in the builder as a passthrough.
fn load_source_entry<'a, B: RemapBuilder>(
    source_entries: &mut [SourceEntry<'a>],
    si: usize,
    sources: &[String],
    ctx: SourceEntryLoadContext<'_, B>,
    loader: UpstreamLoader<'_, 'a>,
) {
    if !matches!(source_entries[si], SourceEntry::Unloaded) {
        return;
    }

    let source_name = &sources[si];

    // jridgewell also calls the loader for an empty source (from `""` or JSON
    // null). Only when the loader gives no map is the source generated-only,
    // as in the `if (!source)` check of its `addSegmentInternal`.
    match loader(source_name) {
        Some(upstream_sm) => {
            let cache = build_upstream_cache(upstream_sm.get());
            source_entries[si] = SourceEntry::Upstream { map: upstream_sm, cache };
        }
        None if source_name.is_empty() => {
            let ignored = ctx.outer_ignore_set.contains(&(si as u32));
            source_entries[si] = SourceEntry::EmptySource { ignored };
        }
        None => {
            let idx = ctx.builder.add_source(source_name);
            if ctx.outer_ignore_set.contains(&(si as u32)) && ctx.ignored_sources.insert(idx) {
                ctx.builder.add_to_ignore_list(idx);
            }
            source_entries[si] = SourceEntry::Passthrough { builder_src: idx };
        }
    }
}

/// Compose a chain of pre-parsed source maps into a single source map.
///
/// Takes a slice of source maps in chain order: the first map is the outermost
/// (final transform), and the last is the innermost (closest to original sources).
/// Each consecutive pair is composed, threading mappings from generated → original.
///
/// This is more ergonomic than [`remap`] for cases where all maps are already
/// parsed (e.g. Rolldown), since no loader closure is needed.
///
/// Returns the composed source map, or `None` if the slice is empty.
///
/// # Examples
///
/// ```
/// use srcmap_remapping::remap_chain;
/// use srcmap_sourcemap::SourceMap;
///
/// let step1 = r#"{"version":3,"file":"inter.js","sources":["original.js"],"names":[],"mappings":"AAAA;AACA"}"#;
/// let step2 = r#"{"version":3,"file":"output.js","sources":["inter.js"],"names":[],"mappings":"AAAA;AACA"}"#;
///
/// let maps: Vec<SourceMap> = vec![
///     SourceMap::from_json(step2).unwrap(),
///     SourceMap::from_json(step1).unwrap(),
/// ];
/// let refs: Vec<&SourceMap> = maps.iter().collect();
/// let result = remap_chain(&refs);
/// assert!(result.is_some());
/// let result = result.unwrap();
/// assert_eq!(result.sources, vec!["original.js"]);
/// ```
pub fn remap_chain(maps: &[&SourceMap]) -> Option<SourceMap> {
    if maps.is_empty() {
        return None;
    }
    if maps.len() == 1 {
        return Some(maps[0].clone());
    }

    // Fold from the innermost pair outwards so each step composes one outer
    // map with the already-collapsed remainder of the chain.
    let mut current = compose_pair(maps[maps.len() - 2], maps[maps.len() - 1]);

    for i in (0..maps.len() - 2).rev() {
        current = compose_pair(maps[i], &current);
    }

    Some(current)
}

/// Compose two source maps: outer maps generated → intermediate, inner maps intermediate → original.
/// Only the outer source matching `inner.file` is resolved through inner; other outer sources
/// pass through unchanged. When `inner.file` is unset and outer has a single non-empty source,
/// that source is treated as the match target.
fn compose_pair(outer: &SourceMap, inner: &SourceMap) -> SourceMap {
    let fallback_source = if inner.file.is_none() {
        let mut sources = outer.sources.iter().filter(|source| !source.is_empty());
        match (sources.next(), sources.next()) {
            (Some(source), None) => Some(source.clone()),
            _ => None,
        }
    } else {
        None
    };

    remap_ref(outer, |source| {
        if inner.file.as_deref() == Some(source) || fallback_source.as_deref() == Some(source) {
            Some(inner)
        } else {
            None
        }
    })
}

/// Streaming variant of [`remap`] that avoids materializing the outer map.
///
/// Accepts pre-parsed metadata and a [`MappingsIter`](srcmap_sourcemap::MappingsIter)
/// over the outer map's VLQ-encoded mappings. Uses [`StreamingGenerator`] to
/// encode the result on-the-fly without collecting all mappings first.
///
/// Because `MappingsIter` yields mappings in sorted order, the streaming
/// generator can encode VLQ incrementally, avoiding the sort + re-encode
/// pass that [`remap`] requires.
///
/// The `ignore_list` from both upstream and outer maps is propagated.
/// Invalid segments from the iterator are silently skipped.
pub fn remap_streaming<'a, F>(
    mappings_iter: srcmap_sourcemap::MappingsIter<'a>,
    sources: &[String],
    names: &[String],
    sources_content: &[Option<String>],
    ignore_list: &[u32],
    file: Option<String>,
    loader: F,
) -> SourceMap
where
    F: Fn(&str) -> Option<SourceMap>,
{
    let mut builder = StreamingGenerator::with_capacity(file, 4096);

    // Flat Vec indexed by outer source index — avoids HashMap per mapping
    let loader: UpstreamLoader<'_, '_> =
        &|source| loader(source).map(|map| UpstreamMap::Owned(Box::new(map)));
    let mut source_entries: Vec<SourceEntry<'_>> =
        std::iter::repeat_with(|| SourceEntry::Unloaded).take(sources.len()).collect();

    let mut ignored_sources: HashSet<u32> = HashSet::new();

    // Lazy outer name remap table
    let mut outer_name_remap: Vec<Option<u32>> = vec![None; names.len()];

    // Pre-compute outer ignore set for O(1) lookups
    let outer_ignore_set: HashSet<u32> = ignore_list.iter().copied().collect();

    let mut dedup = DedupeState::new();
    let mut contents = ContentTracker::default();

    for item in mappings_iter {
        let m = match item {
            Ok(m) => m,
            Err(_) => continue,
        };

        if m.source == NO_SOURCE {
            trace_and_emit_sourceless(
                &mut builder,
                &mut dedup,
                m.generated_line,
                m.generated_column,
            );
            continue;
        }

        let si = m.source as usize;
        if si >= sources.len() {
            continue;
        }

        load_source_entry(
            &mut source_entries,
            si,
            sources,
            SourceEntryLoadContext {
                builder: &mut builder,
                outer_ignore_set: &outer_ignore_set,
                ignored_sources: &mut ignored_sources,
            },
            loader,
        );

        trace_source_entry(
            &mut source_entries[si],
            &m,
            TraceContext {
                builder: &mut builder,
                dedup: &mut dedup,
                contents: &mut contents,
                outer_name_remap: &mut outer_name_remap,
                names,
                sources_content,
                ignored_sources: &mut ignored_sources,
            },
        );
    }

    builder.to_decoded_map().expect("streaming VLQ should be valid")
}

struct UpstreamEmitContext<'a> {
    contents: &'a mut ContentTracker,
    gen_line: u32,
    gen_col: u32,
    upstream_m: UpstreamLookup,
    cache: &'a mut UpstreamCache,
    upstream_map: &'a SourceMap,
    outer_name_remap: &'a mut [Option<u32>],
    outer_name_idx: u32,
    names: &'a [String],
    ignored_sources: &'a mut HashSet<u32>,
    is_range: bool,
}

#[inline]
fn trace_and_emit_upstream<B: RemapBuilder>(
    builder: &mut B,
    dedup: &mut DedupeState,
    ctx: UpstreamEmitContext<'_>,
) {
    let UpstreamEmitContext {
        contents,
        gen_line,
        gen_col,
        upstream_m,
        cache,
        upstream_map,
        outer_name_remap,
        outer_name_idx,
        names,
        ignored_sources,
        is_range,
    } = ctx;
    let si = upstream_m.source as usize;
    if si >= upstream_map.sources.len() {
        return;
    }
    // An empty upstream source (from `""` or JSON null) gives a sourceless segment,
    // as the `if (!source)` check of jridgewell's `addSegmentInternal` does.
    if upstream_map.sources[si].is_empty() {
        if upstream_map.ignore_list.contains(&upstream_m.source) {
            ignore_empty_source(builder, ignored_sources);
        }
        trace_and_emit_sourceless(builder, dedup, gen_line, gen_col);
        return;
    }
    let builder_src =
        resolve_upstream_source(cache, upstream_map, upstream_m.source, builder, ignored_sources);
    contents.apply(
        builder,
        builder_src,
        upstream_map.sources_content.get(si).and_then(Option::as_ref),
    );

    let builder_name = if (upstream_m.name as usize) < upstream_map.names.len() {
        Some(resolve_upstream_name(cache, upstream_map, upstream_m.name, builder))
    } else {
        resolve_outer_name_cached(outer_name_remap, outer_name_idx, names, builder)
    };

    if !dedup.skip_source(
        gen_line,
        builder_src,
        upstream_m.original_line,
        upstream_m.original_column,
        builder_name,
    ) {
        emit_remapped_mapping(
            builder,
            gen_line,
            gen_col,
            builder_src,
            upstream_m.original_line,
            upstream_m.original_column,
            builder_name,
            is_range,
        );
    }
    dedup.record_source(
        gen_line,
        builder_src,
        upstream_m.original_line,
        upstream_m.original_column,
        builder_name,
    );
}

struct PassthroughEmitContext<'a> {
    contents: &'a mut ContentTracker,
    content: Option<&'a String>,
    gen_line: u32,
    gen_col: u32,
    orig_line: u32,
    orig_col: u32,
    builder_src: u32,
    outer_name_remap: &'a mut [Option<u32>],
    outer_name_idx: u32,
    names: &'a [String],
    is_range: bool,
}

#[inline]
fn trace_and_emit_passthrough<B: RemapBuilder>(
    builder: &mut B,
    dedup: &mut DedupeState,
    ctx: PassthroughEmitContext<'_>,
) {
    let PassthroughEmitContext {
        contents,
        content,
        gen_line,
        gen_col,
        orig_line,
        orig_col,
        builder_src,
        outer_name_remap,
        outer_name_idx,
        names,
        is_range,
    } = ctx;
    contents.apply(builder, builder_src, content);
    let builder_name = resolve_outer_name_cached(outer_name_remap, outer_name_idx, names, builder);

    if !dedup.skip_source(gen_line, builder_src, orig_line, orig_col, builder_name) {
        emit_remapped_mapping(
            builder,
            gen_line,
            gen_col,
            builder_src,
            orig_line,
            orig_col,
            builder_name,
            is_range,
        );
    }
    dedup.record_source(gen_line, builder_src, orig_line, orig_col, builder_name);
}

/// jridgewell calls `setIgnore` for an ignored source on each traced segment, also
/// for an empty source. That adds `""` to `sources` and `ignoreList`, although no
/// segment uses it. This function gives the same result.
fn ignore_empty_source<B: RemapBuilder>(builder: &mut B, ignored_sources: &mut HashSet<u32>) {
    let idx = builder.add_source("");
    if ignored_sources.insert(idx) {
        builder.add_to_ignore_list(idx);
    }
}

#[inline]
fn trace_and_emit_sourceless<B: RemapBuilder>(
    builder: &mut B,
    dedup: &mut DedupeState,
    gen_line: u32,
    gen_col: u32,
) {
    if !dedup.skip_sourceless(gen_line) {
        builder.add_generated_mapping(gen_line, gen_col);
    }
    dedup.record_sourceless(gen_line);
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Upstream lookup tests ────────────────────────────────────

    fn segment_at(column: u32) -> srcmap_sourcemap::Mapping {
        srcmap_sourcemap::Mapping {
            generated_line: 0,
            generated_column: column,
            source: 0,
            original_line: 0,
            original_column: 0,
            name: NO_NAME,
            is_range_mapping: false,
        }
    }

    #[test]
    fn cursor_search_matches_a_linear_search_from_every_cursor() {
        // 30 segments, more than the scan limit, with repeated columns.
        let mut sorted: Vec<u32> = (0..30).map(|i| 4 + (i / 2) * 5 + (i % 3)).collect();
        sorted.sort_unstable();
        let line: Vec<_> = sorted.iter().map(|&c| segment_at(c)).collect();
        let last_column = *sorted.last().unwrap();

        for cursor_line in [0, 1] {
            for cursor_index in 0..=line.len() + 1 {
                let cursor = LookupCursor { line: cursor_line, index: cursor_index };
                for column in 0..=last_column + 3 {
                    let expected = line.iter().rposition(|m| m.generated_column <= column);
                    assert_eq!(
                        last_at_or_before(&line, &cursor, 0, column),
                        expected,
                        "cursor {cursor_line}:{cursor_index}, column {column}"
                    );
                }
            }
        }
    }

    #[test]
    fn remap_gives_the_same_map_for_any_order_of_outer_segments() {
        // The outer segments trace to falling, rising and repeated upstream
        // columns, so the lookup cursor is before, at and after each target.
        let inner = SourceMap::from_json(
            r#"{"version":3,"file":"inter.js","sources":["a.js"],"names":[],"mappings":"AAAA,CAAC,EAAE,GAAG,IAAI,KAAK,MAAM,OAAO,QAAQ,SAAS;AACV,CAAC,EAAE"}"#,
        )
        .unwrap();
        let mut builder = SourceMapGenerator::new(Some("out.js".to_string()));
        let source = builder.add_source("inter.js");
        let targets = [(0, 40), (0, 3), (0, 3), (0, 0), (1, 2), (0, 44), (0, 12), (1, 0), (0, 13)];
        for (i, (line, column)) in targets.iter().enumerate() {
            builder.add_mapping(0, i as u32 * 2, source, *line, *column);
        }
        let outer = SourceMap::from_json(&builder.to_json()).unwrap();

        let composed = remap_ref(&outer, |_| Some(&inner));
        for (i, (line, column)) in targets.iter().enumerate() {
            let expected = inner.original_position_for(*line, *column).unwrap();
            let actual = composed.original_position_for(0, i as u32 * 2).unwrap();
            assert_eq!(
                (actual.line, actual.column),
                (expected.line, expected.column),
                "outer segment {i} to {line}:{column}"
            );
        }
    }

    #[test]
    fn remap_ref_gives_the_same_map_as_remap() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"file":"out.js","sources":["inter.js","other.js"],"sourcesContent":["a","b"],"names":["x"],"mappings":"AAAAA,EAAE;ACAA,GDCA"}"#,
        )
        .unwrap();
        let inner = SourceMap::from_json(
            r#"{"version":3,"file":"inter.js","sources":["a.ts"],"sourcesContent":["source"],"names":["y"],"mappings":"AAAAA,CAAC;AACA"}"#,
        )
        .unwrap();
        let owned = remap(&outer, |source| (source == "inter.js").then(|| inner.clone()));
        let borrowed = remap_ref(&outer, |source| (source == "inter.js").then_some(&inner));
        assert_eq!(borrowed.to_json(), owned.to_json());
        assert_eq!(borrowed.sources, vec!["a.ts", "other.js"]);
    }

    // ── Concatenation tests ──────────────────────────────────────

    #[test]
    fn concat_two_simple_maps() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        let b = SourceMap::from_json(
            r#"{"version":3,"sources":["b.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(Some("bundle.js".to_string()));
        builder.add_map(&a, 0);
        builder.add_map(&b, 1);

        let result = builder.build();
        assert_eq!(result.sources, vec!["a.js", "b.js"]);
        assert_eq!(result.mapping_count(), 2);

        let loc0 = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc0.source), "a.js");

        let loc1 = result.original_position_for(1, 0).unwrap();
        assert_eq!(result.source(loc1.source), "b.js");
    }

    #[test]
    fn concat_deduplicates_sources() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["shared.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        let b = SourceMap::from_json(
            r#"{"version":3,"sources":["shared.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);
        builder.add_map(&b, 10);

        let result = builder.build();
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources[0], "shared.js");
    }

    #[test]
    fn concat_with_names() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AAAAA"}"#,
        )
        .unwrap();
        let b = SourceMap::from_json(
            r#"{"version":3,"sources":["b.js"],"names":["bar"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);
        builder.add_map(&b, 1);

        let result = builder.build();
        assert_eq!(result.names.len(), 2);

        let loc0 = result.original_position_for(0, 0).unwrap();
        assert_eq!(loc0.name, Some(0));
        assert_eq!(result.name(0), "foo");

        let loc1 = result.original_position_for(1, 0).unwrap();
        assert_eq!(loc1.name, Some(1));
        assert_eq!(result.name(1), "bar");
    }

    #[test]
    fn concat_preserves_multi_line_maps() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA;AACA;AACA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 5); // offset by 5 lines

        let result = builder.build();
        assert!(result.original_position_for(5, 0).is_some());
        assert!(result.original_position_for(6, 0).is_some());
        assert!(result.original_position_for(7, 0).is_some());
        assert!(result.original_position_for(4, 0).is_none());
    }

    #[test]
    fn concat_with_sources_content() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);

        let result = builder.build();
        assert_eq!(result.sources_content, vec![Some("var a;".to_string())]);
    }

    #[test]
    fn concat_empty_builder() {
        let builder = ConcatBuilder::new(Some("empty.js".to_string()));
        let result = builder.build();
        assert_eq!(result.mapping_count(), 0);
        assert_eq!(result.sources.len(), 0);
    }

    // ── Remapping tests ──────────────────────────────────────────

    #[test]
    fn remap_single_level() {
        // outer: output.js → intermediate.js + other.js (second source has no upstream)
        // AAAA maps gen(0,0) → intermediate.js(0,0)
        // KCAA maps gen(0,5) → other.js(0,0) (source delta +1)
        // ;ADCA maps gen(1,0) → intermediate.js(1,0) (source delta -1, line delta +1)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["intermediate.js","other.js"],"names":[],"mappings":"AAAA,KCAA;ADCA"}"#,
        )
        .unwrap();

        // inner: intermediate.js → original.js
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AACA;AACA"}"#,
        )
        .unwrap();

        let result =
            remap(
                &outer,
                |source| {
                    if source == "intermediate.js" { Some(inner.clone()) } else { None }
                },
            );

        assert!(result.sources.contains(&"original.js".to_string()));
        // other.js passes through since loader returns None
        assert!(result.sources.contains(&"other.js".to_string()));

        // Line 0 col 0 in outer → line 0 col 0 in intermediate → line 1 col 0 in original
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.js");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn remap_accepts_source_map_built_from_parts() {
        let outer = SourceMap::from_parts(
            Some("output.js".to_string()),
            None,
            vec!["intermediate.js".to_string()],
            vec![],
            vec![],
            vec![srcmap_sourcemap::Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 0,
                original_column: 0,
                name: u32::MAX,
                is_range_mapping: false,
            }],
            vec![],
            None,
            None,
        );
        let inner = SourceMap::builder()
            .file("intermediate.js")
            .sources(["original.ts"])
            .sources_content([Some("export const value = 1;")])
            .mappings([srcmap_sourcemap::Mapping {
                generated_line: 0,
                generated_column: 0,
                source: 0,
                original_line: 3,
                original_column: 12,
                name: u32::MAX,
                is_range_mapping: false,
            }])
            .build();

        let result =
            remap(
                &outer,
                |source| {
                    if source == "intermediate.js" { Some(inner.clone()) } else { None }
                },
            );

        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.ts");
        assert_eq!(loc.line, 3);
        assert_eq!(loc.column, 12);
        assert_eq!(result.sources_content, vec![Some("export const value = 1;".to_string())]);
    }

    #[test]
    fn remap_no_upstream_passthrough() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["already-original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        // No upstream maps — everything passes through
        let result = remap(&outer, |_| None);

        assert_eq!(result.sources, vec!["already-original.js"]);
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "already-original.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn remap_partial_sources() {
        // outer has two sources: one with upstream, one without
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js","passthrough.js"],"names":[],"mappings":"AAAA,KCCA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result =
            remap(
                &outer,
                |source| {
                    if source == "compiled.js" { Some(inner.clone()) } else { None }
                },
            );

        // Should have both the remapped source and the passthrough source
        assert!(result.sources.contains(&"original.ts".to_string()));
        assert!(result.sources.contains(&"passthrough.js".to_string()));
    }

    #[test]
    fn remap_preserves_names() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["myFunc"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        // upstream has no names — outer name should be preserved
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "myFunc");
    }

    #[test]
    fn remap_upstream_name_wins() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["outerName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        // upstream has its own name — should take precedence
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":["innerName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "innerName");
    }

    #[test]
    fn remap_sources_content_from_upstream() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"sourcesContent":["const x = 1;"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        assert_eq!(result.sources_content, vec![Some("const x = 1;".to_string())]);
    }

    #[test]
    fn concat_updates_source_content_on_duplicate() {
        // First map has no sourcesContent, second has it for same source
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["shared.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        let b = SourceMap::from_json(
            r#"{"version":3,"sources":["shared.js"],"sourcesContent":["var x = 1;"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);
        builder.add_map(&b, 1);

        let result = builder.build();
        assert_eq!(result.sources.len(), 1);
        assert_eq!(result.sources_content, vec![Some("var x = 1;".to_string())]);
    }

    #[test]
    fn concat_deduplicates_names() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":["sharedName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();
        let b = SourceMap::from_json(
            r#"{"version":3,"sources":["b.js"],"names":["sharedName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);
        builder.add_map(&b, 1);

        let result = builder.build();
        // Names should be deduplicated
        assert_eq!(result.names.len(), 1);
        assert_eq!(result.names[0], "sharedName");
    }

    #[test]
    fn concat_with_ignore_list() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["vendor.js"],"names":[],"mappings":"AAAA","ignoreList":[0]}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);

        let result = builder.build();
        assert_eq!(result.ignore_list, vec![0]);
    }

    fn raw_mapping(column: u32, source: u32, name: u32) -> srcmap_sourcemap::Mapping {
        srcmap_sourcemap::Mapping {
            generated_line: 0,
            generated_column: column,
            source,
            original_line: 0,
            original_column: column,
            name,
            is_range_mapping: false,
        }
    }

    fn map_from_parts(
        sources: &[&str],
        names: &[&str],
        mappings: Vec<srcmap_sourcemap::Mapping>,
        ignore_list: Vec<u32>,
    ) -> SourceMap {
        SourceMap::from_parts(
            None,
            None,
            sources.iter().map(|s| (*s).to_string()).collect(),
            vec![],
            names.iter().map(|s| (*s).to_string()).collect(),
            mappings,
            ignore_list,
            None,
            None,
        )
    }

    #[test]
    fn concat_skips_out_of_range_indices() {
        let sm = map_from_parts(
            &["a.js"],
            &[],
            vec![raw_mapping(0, 5, NO_NAME), raw_mapping(4, 0, 5)],
            vec![7],
        );

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&sm, 0);
        let result = builder.build();

        assert_eq!(result.ignore_list, Vec::<u32>::new());
        assert_eq!(result.mapping_count(), 2);
        assert!(result.original_position_for(0, 0).is_none());
        let loc = result.original_position_for(0, 4).unwrap();
        assert_eq!((result.source(loc.source), loc.name), ("a.js", None));
    }

    #[test]
    fn remap_skips_out_of_range_outer_source() {
        let outer = map_from_parts(
            &["a.js"],
            &[],
            vec![raw_mapping(0, 5, NO_NAME), raw_mapping(4, 0, NO_NAME)],
            vec![],
        );

        let result = remap(&outer, |_| None);

        assert!(result.original_position_for(0, 0).is_none());
        let loc = result.original_position_for(0, 4).unwrap();
        assert_eq!(result.source(loc.source), "a.js");
    }

    #[test]
    fn remap_skips_out_of_range_upstream_indices() {
        let outer = map_from_parts(
            &["mid.js"],
            &[],
            vec![raw_mapping(0, 0, NO_NAME), raw_mapping(4, 0, NO_NAME)],
            vec![],
        );
        let upstream = map_from_parts(
            &["orig.ts"],
            &[],
            vec![raw_mapping(0, 5, NO_NAME), raw_mapping(4, 0, 5)],
            vec![],
        );

        let result = remap(&outer, |_| Some(upstream.clone()));

        assert!(result.original_position_for(0, 0).is_none());
        let loc = result.original_position_for(0, 4).unwrap();
        assert_eq!((result.source(loc.source), loc.name), ("orig.ts", None));
    }

    #[test]
    fn concat_with_generated_only_mappings() {
        // Map with a generated-only segment (1-field segment, no source info)
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"A,AAAA"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);

        let result = builder.build();
        // Should have both mappings, including the generated-only one
        assert!(result.mapping_count() >= 1);
    }

    #[test]
    fn remap_generated_only_passthrough() {
        // Outer map with a generated-only segment and two sources (second has no upstream)
        // A = generated-only segment at col 0
        // ,AAAA = gen(0,4)→a.js(0,0)
        // ,KCAA = gen(0,9)→other.js(0,0) (source delta +1)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js","other.js"],"names":[],"mappings":"A,AAAA,KCAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result =
            remap(&outer, |source| if source == "a.js" { Some(inner.clone()) } else { None });

        // Result should have mappings for the generated-only, remapped, and passthrough
        assert!(result.mapping_count() >= 2);
        assert!(result.sources.contains(&"original.js".to_string()));
        assert!(result.sources.contains(&"other.js".to_string()));
    }

    #[test]
    fn remap_no_upstream_mapping_with_name() {
        // Outer has named mapping but upstream lookup finds no match at that position
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["myFunc"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        // Inner map maps different position (line 5, not line 0)
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":";;;;AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        // jridgewell drops the segment when upstream trace returns null:
        // `if (traced == null) continue;`
        // So there should be no mapping at (0,0) in the result.
        let loc = result.original_position_for(0, 0);
        assert!(loc.is_none());
    }

    #[test]
    fn remap_no_upstream_with_sources_content_and_name() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":["fn1"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        // No upstream — everything passes through
        let result = remap(&outer, |_| None);

        assert_eq!(result.sources, vec!["a.js"]);
        assert_eq!(result.sources_content, vec![Some("var a;".to_string())]);
        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "fn1");
    }

    #[test]
    fn remap_no_upstream_no_name() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| None);
        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_none());
    }

    #[test]
    fn remap_upstream_found_no_name() {
        // Neither outer nor upstream carries a name, so the result has none.
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["intermediate.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        // Inner maps intermediate.js(0,0) → original.js(0,0) with NO name
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        assert_eq!(result.sources, vec!["original.js"]);
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
        // Neither outer nor upstream has a name, so result has no name
        assert!(loc.name.is_none());
        assert_eq!(result.names, [] as [std::string::String; 0]);
    }

    // ── Range mapping preservation tests ────────────────────────

    #[test]
    fn concat_preserves_range_mappings() {
        let a = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,CAAC","rangeMappings":"A"}"#,
        )
        .unwrap();

        let mut builder = ConcatBuilder::new(None);
        builder.add_map(&a, 0);

        let result = builder.build();
        assert!(result.has_range_mappings());
        let mappings = result.all_mappings();
        assert!(mappings[0].is_range_mapping);
        assert!(!mappings[1].is_range_mapping);
    }

    #[test]
    fn remap_preserves_range_mappings_passthrough() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA","rangeMappings":"A"}"#,
        )
        .unwrap();

        // No upstream — range mapping passes through
        let result = remap(&outer, |_| None);
        assert!(result.has_range_mappings());
        let mappings = result.all_mappings();
        assert!(mappings[0].is_range_mapping);
    }

    #[test]
    fn remap_preserves_range_through_upstream() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["intermediate.js"],"names":[],"mappings":"AAAA","rangeMappings":"A"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AACA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));
        assert!(result.has_range_mappings());
    }

    #[test]
    fn remap_non_range_stays_non_range() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| None);
        assert!(!result.has_range_mappings());
    }

    // ── Streaming remapping tests ────────────────────────────────

    /// Helper: run `remap_streaming` from a parsed SourceMap, re-encoding
    /// the VLQ string from its decoded mappings.
    fn streaming_from_sm<F>(sm: &SourceMap, loader: F) -> SourceMap
    where
        F: Fn(&str) -> Option<SourceMap>,
    {
        let vlq = sm.encode_mappings();
        let iter = srcmap_sourcemap::MappingsIter::new(&vlq);
        remap_streaming(
            iter,
            &sm.sources,
            &sm.names,
            &sm.sources_content,
            &sm.ignore_list,
            sm.file.clone(),
            loader,
        )
    }

    #[test]
    fn streaming_single_level() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["intermediate.js","other.js"],"names":[],"mappings":"AAAA,KCAA;ADCA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AACA;AACA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |source| {
            if source == "intermediate.js" { Some(inner.clone()) } else { None }
        });

        assert!(result.sources.contains(&"original.js".to_string()));
        assert!(result.sources.contains(&"other.js".to_string()));

        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.js");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn streaming_no_upstream_passthrough() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["already-original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| None);

        assert_eq!(result.sources, vec!["already-original.js"]);
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "already-original.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn streaming_preserves_names() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["myFunc"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| Some(inner.clone()));

        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "myFunc");
    }

    #[test]
    fn streaming_upstream_name_wins() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["outerName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":["innerName"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| Some(inner.clone()));

        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "innerName");
    }

    #[test]
    fn streaming_sources_content_from_upstream() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"sourcesContent":["const x = 1;"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| Some(inner.clone()));

        assert_eq!(result.sources_content, vec![Some("const x = 1;".to_string())]);
    }

    #[test]
    fn streaming_no_upstream_with_sources_content() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"sourcesContent":["var a;"],"names":["fn1"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| None);

        assert_eq!(result.sources, vec!["a.js"]);
        assert_eq!(result.sources_content, vec![Some("var a;".to_string())]);
        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "fn1");
    }

    #[test]
    fn streaming_generated_only_passthrough() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js","other.js"],"names":[],"mappings":"A,AAAA,KCAA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result =
            streaming_from_sm(
                &outer,
                |source| {
                    if source == "a.js" { Some(inner.clone()) } else { None }
                },
            );

        assert!(result.mapping_count() >= 2);
        assert!(result.sources.contains(&"original.js".to_string()));
        assert!(result.sources.contains(&"other.js".to_string()));
    }

    #[test]
    fn streaming_matches_remap() {
        // Verify streaming produces identical results to non-streaming
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["intermediate.js","other.js"],"names":["foo"],"mappings":"AAAAA,KCAA;ADCA"}"#,
        )
        .unwrap();

        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.js"],"sourcesContent":["// src"],"names":["bar"],"mappings":"AAAAA;AACA"}"#,
        )
        .unwrap();

        let loader = |source: &str| -> Option<SourceMap> {
            if source == "intermediate.js" { Some(inner.clone()) } else { None }
        };

        let result_normal = remap(&outer, loader);
        let result_stream = streaming_from_sm(&outer, loader);

        assert_eq!(result_normal.sources, result_stream.sources);
        assert_eq!(result_normal.names, result_stream.names);
        assert_eq!(result_normal.sources_content, result_stream.sources_content);
        assert_eq!(result_normal.mapping_count(), result_stream.mapping_count());

        // Verify all lookups match
        for m in result_normal.all_mappings() {
            let loc_n = result_normal.original_position_for(m.generated_line, m.generated_column);
            let loc_s = result_stream.original_position_for(m.generated_line, m.generated_column);
            assert_eq!(loc_n.is_some(), loc_s.is_some());
            if let (Some(ln), Some(ls)) = (loc_n, loc_s) {
                assert_eq!(result_normal.source(ln.source), result_stream.source(ls.source));
                assert_eq!(ln.line, ls.line);
                assert_eq!(ln.column, ls.column);
            }
        }
    }

    #[test]
    fn streaming_no_upstream_mapping_fallback() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":["myFunc"],"mappings":"AAAAA"}"#,
        )
        .unwrap();

        // Inner map maps different position (line 5, not line 0)
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":";;;;AAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| Some(inner.clone()));

        // jridgewell drops the segment when upstream trace returns null
        let loc = result.original_position_for(0, 0);
        assert!(loc.is_none());
    }

    // ── remap_chain tests ────────────────────────────────────────

    #[test]
    fn remap_chain_empty() {
        assert!(remap_chain(&[]).is_none());
    }

    #[test]
    fn remap_chain_single() {
        let sm = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        let result = remap_chain(&[&sm]).unwrap();
        assert_eq!(result.sources, vec!["a.js"]);
        assert_eq!(result.mapping_count(), 1);
    }

    #[test]
    fn remap_chain_two_maps() {
        // step1: original.js → intermediate.js
        let step1 = SourceMap::from_json(
            r#"{"version":3,"file":"intermediate.js","sources":["original.js"],"names":[],"mappings":"AACA;AACA"}"#,
        )
        .unwrap();
        // step2: intermediate.js → output.js
        let step2 = SourceMap::from_json(
            r#"{"version":3,"file":"output.js","sources":["intermediate.js"],"names":[],"mappings":"AAAA;AACA"}"#,
        )
        .unwrap();

        // Chain: outer (step2) → inner (step1)
        let result = remap_chain(&[&step2, &step1]).unwrap();
        assert_eq!(result.sources, vec!["original.js"]);

        // output line 0 → intermediate line 0 → original line 1
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.js");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn remap_chain_three_maps() {
        // a.js → b.js: line 0 → line 1
        let a_to_b = SourceMap::from_json(
            r#"{"version":3,"file":"b.js","sources":["a.js"],"names":[],"mappings":"AACA"}"#,
        )
        .unwrap();
        // b.js → c.js: line 0 → line 0
        let b_to_c = SourceMap::from_json(
            r#"{"version":3,"file":"c.js","sources":["b.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        // c.js → d.js: line 0 → line 0
        let c_to_d = SourceMap::from_json(
            r#"{"version":3,"file":"d.js","sources":["c.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        // Chain: d.js → c.js → b.js → a.js
        let result = remap_chain(&[&c_to_d, &b_to_c, &a_to_b]).unwrap();
        assert_eq!(result.sources, vec!["a.js"]);

        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "a.js");
        assert_eq!(loc.line, 1);
    }

    #[test]
    fn remap_chain_only_composes_matching_inner_file() {
        let inner = SourceMap::from_json(
            r#"{"version":3,"file":"intermediate.js","sources":["original.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();
        let outer = SourceMap::from_json(
            r#"{"version":3,"file":"output.js","sources":["intermediate.js","passthrough.js"],"names":[],"mappings":"AAAA,KCAA"}"#,
        )
        .unwrap();

        let result = remap_chain(&[&outer, &inner]).unwrap();

        assert!(result.sources.contains(&"original.js".to_string()));
        assert!(result.sources.contains(&"passthrough.js".to_string()));

        let remapped = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(remapped.source), "original.js");

        let passthrough = result.original_position_for(0, 5).unwrap();
        assert_eq!(result.source(passthrough.source), "passthrough.js");
    }

    // ── Empty-string source filtering ────────────────────────────

    #[test]
    fn remap_empty_string_source_filtered() {
        // Outer map has an empty-string source (from JSON null)
        let outer =
            SourceMap::from_json(r#"{"version":3,"sources":[""],"names":[],"mappings":"AAAA"}"#)
                .unwrap();

        let result = remap(&outer, |_| None);

        // Empty-string sources should not appear in output sources
        assert!(
            !result.sources.iter().any(|s| s.is_empty()),
            "empty-string sources should be filtered out"
        );
        // The segment should be dropped (no source info)
        let loc = result.original_position_for(0, 0);
        assert!(loc.is_none());
    }

    // jridgewell calls the loader for an empty source too. A map from the loader
    // replaces the source; without one, the source stays generated-only.
    #[test]
    fn remap_loads_upstream_for_empty_source() {
        let outer =
            SourceMap::from_json(r#"{"version":3,"sources":[null],"names":[],"mappings":"AAAA"}"#)
                .unwrap();
        let loader = |source: &str| {
            source.is_empty().then(|| {
                SourceMap::from_json(
                    r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#,
                )
                .unwrap()
            })
        };

        for result in [remap(&outer, loader), streaming_from_sm(&outer, loader)] {
            assert_eq!(result.sources, vec!["a.js".to_string()]);
            assert_eq!(result.encode_mappings(), "AAAA");
        }
    }

    // A segment that traces to an empty upstream source is sourceless, as in jridgewell.
    #[test]
    fn remap_makes_segment_sourceless_for_empty_upstream_source() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAE"}"#,
        )
        .unwrap();
        let loader = |_: &str| {
            Some(
                SourceMap::from_json(
                    r#"{"version":3,"sources":["b.js",null],"names":[],"mappings":"AAAA,ECAE"}"#,
                )
                .unwrap(),
            )
        };

        for result in [remap(&outer, loader), streaming_from_sm(&outer, loader)] {
            assert_eq!(result.sources, vec!["b.js".to_string()]);
            assert_eq!(result.encode_mappings(), "AAAA,E");
        }
    }

    // Two sources with one name: the content of the last traced segment wins.
    #[test]
    fn remap_keeps_content_of_last_traced_segment() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js","a.js"],"sourcesContent":["first","second"],"names":[],"mappings":"AAAA,ECAA;ADAA"}"#,
        )
        .unwrap();

        for result in [remap(&outer, |_| None), streaming_from_sm(&outer, |_| None)] {
            assert_eq!(result.sources_content, vec![Some("first".to_string())]);
        }
    }

    #[test]
    fn streaming_empty_string_source_filtered() {
        let outer =
            SourceMap::from_json(r#"{"version":3,"sources":[""],"names":[],"mappings":"AAAA"}"#)
                .unwrap();

        let result = streaming_from_sm(&outer, |_| None);

        assert!(
            !result.sources.iter().any(|s| s.is_empty()),
            "streaming: empty-string sources should be filtered out"
        );
    }

    // ── Mapping deduplication ────────────────────────────────────

    #[test]
    fn remap_skips_redundant_sourced_segments() {
        // Outer has three segments on the same line all mapping to the same
        // original position. jridgewell deduplicates the second and third.
        // AAAA,EAAA,EAAA = gen(0,0)→src(0,0), gen(0,2)→src(0,0), gen(0,4)→src(0,0)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAA,EAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| None);

        // Should deduplicate: only 1 segment (the first) should remain
        assert_eq!(result.mapping_count(), 1);
    }

    #[test]
    fn remap_keeps_different_sourced_segments() {
        // Two segments on the same line mapping to different original columns
        // AAAA,EAAC = gen(0,0)→src(0,0), gen(0,2)→src(0,1)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAC"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| None);

        // Both should be kept (different original positions)
        assert_eq!(result.mapping_count(), 2);
    }

    #[test]
    fn remap_skips_sourceless_at_line_start() {
        // Outer has a sourceless segment at position 0 on a line
        // A = gen(0,0) with no source
        let outer = SourceMap::from_json(r#"{"version":3,"sources":[],"names":[],"mappings":"A"}"#)
            .unwrap();

        let result = remap(&outer, |_| None);

        // Sourceless at line start should be dropped
        assert_eq!(result.mapping_count(), 0);
    }

    #[test]
    fn streaming_skips_redundant_sourced_segments() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,EAAA,EAAA"}"#,
        )
        .unwrap();

        let result = streaming_from_sm(&outer, |_| None);

        assert_eq!(result.mapping_count(), 1);
    }

    fn inner_map_loader(json: &'static str) -> impl Fn(&str) -> Option<SourceMap> {
        move |source| (source == "mid.js").then(|| SourceMap::from_json(json).unwrap())
    }

    // An outer segment that lands on a generated-only inner segment must stay as a
    // sourceless segment. Without it, columns 8 and up map to the inner segment at 4.
    #[test]
    fn remap_keeps_sourceless_segment_from_upstream() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["mid.js"],"names":[],"mappings":"AAAA,IAAI,IAAI"}"#,
        )
        .unwrap();
        let loader = inner_map_loader(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA,IAAI,I"}"#,
        );

        assert_eq!(remap(&outer, &loader).encode_mappings(), "AAAA,IAAI,I");
        assert_eq!(streaming_from_sm(&outer, &loader).encode_mappings(), "AAAA,IAAI,I");
    }

    // With several inner segments at one column, an exact match takes the first one.
    #[test]
    fn remap_takes_first_upstream_segment_at_equal_column() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["mid.js"],"names":[],"mappings":"CAAC,MAAM"}"#,
        )
        .unwrap();
        let loader = inner_map_loader(
            r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"CAAG,AAAC,OAAE"}"#,
        );

        assert_eq!(remap(&outer, &loader).encode_mappings(), "CAAG,MAAC");
        assert_eq!(streaming_from_sm(&outer, &loader).encode_mappings(), "CAAG,MAAC");
    }
}
