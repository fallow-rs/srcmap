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
        Self {
            builder: SourceMapGenerator::new(file),
        }
    }

    /// Add a source map to the concatenated output.
    ///
    /// `line_offset` is the number of lines to shift all mappings by
    /// (i.e. the line at which this chunk starts in the output).
    pub fn add_map(&mut self, sm: &SourceMap, line_offset: u32) {
        // Pre-build source index remap table (once per input map, not per mapping)
        let source_indices: Vec<u32> = sm
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let idx = self.builder.add_source(s);
                if let Some(Some(content)) = sm.sources_content.get(i) {
                    self.builder.set_source_content(idx, content.clone());
                }
                idx
            })
            .collect();

        // Pre-build name index remap table (once per input map)
        let name_indices: Vec<u32> = sm.names.iter().map(|n| self.builder.add_name(n)).collect();

        // Copy ignore_list entries
        for &ignored in &sm.ignore_list {
            let global_idx = source_indices[ignored as usize];
            self.builder.add_to_ignore_list(global_idx);
        }

        // Add all mappings with line offset, using pre-built index tables
        for m in sm.all_mappings() {
            let gen_line = m.generated_line + line_offset;

            if m.source == NO_SOURCE {
                self.builder
                    .add_generated_mapping(gen_line, m.generated_column);
            } else {
                let src = source_indices[m.source as usize];
                let has_name = m.name != NO_NAME;
                match (has_name, m.is_range_mapping) {
                    (true, true) => self.builder.add_named_range_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                        name_indices[m.name as usize],
                    ),
                    (true, false) => self.builder.add_named_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                        name_indices[m.name as usize],
                    ),
                    (false, true) => self.builder.add_range_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                    ),
                    (false, false) => self.builder.add_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                    ),
                }
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
}

/// Build lazy index remap tables for an upstream map.
fn build_upstream_cache(upstream_sm: &SourceMap) -> UpstreamCache {
    UpstreamCache {
        source_remap: vec![None; upstream_sm.sources.len()],
        name_remap: vec![None; upstream_sm.names.len()],
    }
}

/// Resolve an upstream source index to a builder source index, lazily
/// registering the source (and its content/ignore status) on first use.
#[inline]
fn resolve_upstream_source(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_src: u32,
    builder: &mut SourceMapGenerator,
    ignored_sources: &mut HashSet<u32>,
) -> u32 {
    let si = upstream_src as usize;
    if let Some(idx) = cache.source_remap[si] {
        return idx;
    }
    let idx = builder.add_source(&upstream_sm.sources[si]);
    if let Some(Some(content)) = upstream_sm.sources_content.get(si) {
        builder.set_source_content(idx, content.clone());
    }
    if upstream_sm.ignore_list.contains(&upstream_src) && ignored_sources.insert(idx) {
        builder.add_to_ignore_list(idx);
    }
    cache.source_remap[si] = Some(idx);
    idx
}

/// Resolve an upstream name index to a builder name index, lazily
/// registering the name on first use.
#[inline]
fn resolve_upstream_name(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_name: u32,
    builder: &mut SourceMapGenerator,
) -> u32 {
    let ni = upstream_name as usize;
    if let Some(idx) = cache.name_remap[ni] {
        return idx;
    }
    let idx = builder.add_name(&upstream_sm.names[ni]);
    cache.name_remap[ni] = Some(idx);
    idx
}

/// Resolve an upstream source index to a streaming builder source index.
#[inline]
fn resolve_upstream_source_streaming(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_src: u32,
    builder: &mut StreamingGenerator,
    ignored_sources: &mut HashSet<u32>,
) -> u32 {
    let si = upstream_src as usize;
    if let Some(idx) = cache.source_remap[si] {
        return idx;
    }
    let idx = builder.add_source(&upstream_sm.sources[si]);
    if let Some(Some(content)) = upstream_sm.sources_content.get(si) {
        builder.set_source_content(idx, content.clone());
    }
    if upstream_sm.ignore_list.contains(&upstream_src) && ignored_sources.insert(idx) {
        builder.add_to_ignore_list(idx);
    }
    cache.source_remap[si] = Some(idx);
    idx
}

/// Resolve an upstream name index to a streaming builder name index.
#[inline]
fn resolve_upstream_name_streaming(
    cache: &mut UpstreamCache,
    upstream_sm: &SourceMap,
    upstream_name: u32,
    builder: &mut StreamingGenerator,
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
#[inline]
fn lookup_upstream(upstream_sm: &SourceMap, line: u32, column: u32) -> Option<UpstreamLookup> {
    let line_mappings = upstream_sm.mappings_for_line(line);
    if line_mappings.is_empty() {
        return fallback_to_full_lookup(upstream_sm, line, column);
    }

    let idx = match line_mappings.binary_search_by_key(&column, |m| m.generated_column) {
        Ok(i) => i,
        Err(0) => return fallback_to_full_lookup(upstream_sm, line, column),
        Err(i) => i - 1,
    };

    let mapping = &line_mappings[idx];
    if mapping.source == NO_SOURCE {
        return None;
    }

    let original_column = if mapping.is_range_mapping && column >= mapping.generated_column {
        mapping.original_column + (column - mapping.generated_column)
    } else {
        mapping.original_column
    };

    Some(UpstreamLookup {
        source: mapping.source,
        original_line: mapping.original_line,
        original_column,
        name: mapping.name,
        is_range_mapping: mapping.is_range_mapping,
    })
}

/// Result of looking up a position in an upstream source map.
/// Carries the resolved source/name indices and original position directly,
/// so callers don't need to re-inspect the mapping.
struct UpstreamLookup {
    source: u32,
    original_line: u32,
    original_column: u32,
    name: u32,
    is_range_mapping: bool,
}

/// Fall back to the full `original_position_for` when the inlined lookup can't
/// resolve (empty line or column before first mapping). This handles range mapping
/// fallback correctly. Only called on the rare path where the line has no direct
/// mappings, so the function call overhead is acceptable.
fn fallback_to_full_lookup(
    upstream_sm: &SourceMap,
    line: u32,
    column: u32,
) -> Option<UpstreamLookup> {
    let loc = upstream_sm.original_position_for(line, column)?;
    Some(UpstreamLookup {
        source: loc.source,
        original_line: loc.line,
        original_column: loc.column,
        name: loc.name.unwrap_or(NO_NAME),
        is_range_mapping: false, // doesn't matter for the builder, position is already resolved
    })
}

/// Emit a mapping to the builder using pre-built index remap tables.
/// Uses indices directly, avoiding per-mapping string hashing.
#[inline]
#[allow(clippy::too_many_arguments)]
fn emit_remapped_mapping(
    builder: &mut SourceMapGenerator,
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

/// Emit a mapping to the streaming builder using pre-built index remap tables.
#[inline]
#[allow(clippy::too_many_arguments)]
fn emit_remapped_mapping_streaming(
    builder: &mut StreamingGenerator,
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

/// Per-source entry: either an upstream map + cache, or a passthrough.
/// Using an enum avoids two separate HashMap lookups per mapping.
enum SourceEntry {
    /// Has an upstream map: trace mappings through it.
    Upstream {
        map: Box<SourceMap>,
        cache: UpstreamCache,
    },
    /// No upstream map: pass through with builder source index.
    Passthrough { builder_src: u32 },
    /// Empty-string source (from JSON `null`): emit as generated-only.
    /// Matches jridgewell's behavior where `!source` triggers a sourceless segment.
    EmptySource,
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
    let mapping_count = outer.mapping_count();
    let source_count = outer.sources.len();
    let mut builder = SourceMapGenerator::with_capacity(outer.file.clone(), mapping_count);
    // Mappings are emitted in the same order as outer (already sorted).
    builder.set_assume_sorted(true);

    // Flat Vec indexed by outer source index — avoids HashMap per mapping.
    let mut source_entries: Vec<SourceEntry> =
        (0..source_count).map(|_| SourceEntry::Unloaded).collect();

    let mut ignored_sources: HashSet<u32> = HashSet::new();

    // Lazy outer name passthrough table (outer name idx → builder name idx)
    let mut outer_name_remap: Vec<Option<u32>> = vec![None; outer.names.len()];

    // Pre-compute outer ignore set for O(1) lookups
    let outer_ignore_set: HashSet<u32> = outer.ignore_list.iter().copied().collect();

    let mut dedup = DedupeState::new();

    for m in outer.all_mappings() {
        if m.source == NO_SOURCE {
            if !dedup.skip_sourceless(m.generated_line) {
                builder.add_generated_mapping(m.generated_line, m.generated_column);
            }
            dedup.record_sourceless(m.generated_line);
            continue;
        }

        let si = m.source as usize;

        // Load upstream map if not yet cached — Vec index, no hash
        if matches!(source_entries[si], SourceEntry::Unloaded) {
            let source_name = outer.source(m.source);
            // Empty-string sources (from JSON null) are treated as generated-only,
            // matching jridgewell's `if (!source)` check in addSegmentInternal.
            if source_name.is_empty() {
                source_entries[si] = SourceEntry::EmptySource;
            } else {
                match loader(source_name) {
                    Some(upstream_sm) => {
                        let cache = build_upstream_cache(&upstream_sm);
                        source_entries[si] = SourceEntry::Upstream {
                            map: Box::new(upstream_sm),
                            cache,
                        };
                    }
                    None => {
                        let idx = builder.add_source(source_name);
                        if let Some(Some(content)) = outer.sources_content.get(si) {
                            builder.set_source_content(idx, content.clone());
                        }
                        if outer_ignore_set.contains(&m.source) && ignored_sources.insert(idx) {
                            builder.add_to_ignore_list(idx);
                        }
                        source_entries[si] = SourceEntry::Passthrough { builder_src: idx };
                    }
                }
            }
        }

        match &mut source_entries[si] {
            SourceEntry::Upstream { map, cache } => {
                match lookup_upstream(map, m.original_line, m.original_column) {
                    Some(upstream_m) => {
                        let builder_src = resolve_upstream_source(
                            cache,
                            map,
                            upstream_m.source,
                            &mut builder,
                            &mut ignored_sources,
                        );

                        // Resolve name: prefer upstream name, fall back to outer name
                        let builder_name = if upstream_m.name != NO_NAME {
                            Some(resolve_upstream_name(
                                cache,
                                map,
                                upstream_m.name,
                                &mut builder,
                            ))
                        } else if m.name != NO_NAME {
                            let name_idx = *outer_name_remap[m.name as usize]
                                .get_or_insert_with(|| builder.add_name(outer.name(m.name)));
                            Some(name_idx)
                        } else {
                            None
                        };

                        if !dedup.skip_source(
                            m.generated_line,
                            builder_src,
                            upstream_m.original_line,
                            upstream_m.original_column,
                            builder_name,
                        ) {
                            emit_remapped_mapping(
                                &mut builder,
                                m.generated_line,
                                m.generated_column,
                                builder_src,
                                upstream_m.original_line,
                                upstream_m.original_column,
                                builder_name,
                                m.is_range_mapping,
                            );
                        }
                        dedup.record_source(
                            m.generated_line,
                            builder_src,
                            upstream_m.original_line,
                            upstream_m.original_column,
                            builder_name,
                        );
                    }
                    None => {
                        // No mapping in upstream — drop the segment entirely.
                        // jridgewell's traceMappings does `if (traced == null) continue;`
                    }
                }
            }
            SourceEntry::Passthrough { builder_src } => {
                let builder_src = *builder_src;

                let builder_name = if m.name != NO_NAME {
                    Some(
                        *outer_name_remap[m.name as usize]
                            .get_or_insert_with(|| builder.add_name(outer.name(m.name))),
                    )
                } else {
                    None
                };

                if !dedup.skip_source(
                    m.generated_line,
                    builder_src,
                    m.original_line,
                    m.original_column,
                    builder_name,
                ) {
                    emit_remapped_mapping(
                        &mut builder,
                        m.generated_line,
                        m.generated_column,
                        builder_src,
                        m.original_line,
                        m.original_column,
                        builder_name,
                        m.is_range_mapping,
                    );
                }
                dedup.record_source(
                    m.generated_line,
                    builder_src,
                    m.original_line,
                    m.original_column,
                    builder_name,
                );
            }
            SourceEntry::EmptySource => {
                // Empty-string source → emit as generated-only (no source info)
                if !dedup.skip_sourceless(m.generated_line) {
                    builder.add_generated_mapping(m.generated_line, m.generated_column);
                }
                dedup.record_sourceless(m.generated_line);
            }
            SourceEntry::Unloaded => unreachable!(),
        }
    }

    builder.to_decoded_map()
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

    // Compose from the end: start with the second-to-last as outer,
    // last as inner, then work backwards.
    // maps[0] is outermost, maps[len-1] is innermost.
    // We compose pairwise: result = remap(maps[0], maps[1]), then
    // result = remap(result, maps[2]), etc. But actually the chain is:
    // maps[0] (outermost) sources reference maps[1], which sources reference maps[2], etc.
    // So we compose maps[0] with maps[1], then the result with maps[2], etc.
    // But remap expects a loader that returns maps for each source.
    // For a simple chain, each map has sources that map to the next map in the chain.

    // Start with the last two and work forward
    let mut current = compose_pair(maps[maps.len() - 2], maps[maps.len() - 1]);

    // Compose with remaining maps from right to left
    for i in (0..maps.len() - 2).rev() {
        current = compose_pair(maps[i], &current);
    }

    Some(current)
}

/// Compose two source maps: outer maps generated → intermediate, inner maps intermediate → original.
/// All sources in outer are resolved through inner.
fn compose_pair(outer: &SourceMap, inner: &SourceMap) -> SourceMap {
    remap(outer, |_source| Some(inner.clone()))
}

/// Per-source entry for streaming variant.
enum StreamingSourceEntry {
    /// Has an upstream map: trace mappings through it.
    Upstream {
        map: Box<SourceMap>,
        cache: UpstreamCache,
    },
    /// No upstream map: pass through with builder source index.
    Passthrough { builder_src: u32 },
    /// Empty-string source (from JSON `null`): emit as generated-only.
    EmptySource,
    /// Not yet loaded.
    Unloaded,
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
    let mut source_entries: Vec<StreamingSourceEntry> = (0..sources.len())
        .map(|_| StreamingSourceEntry::Unloaded)
        .collect();

    let mut ignored_sources: HashSet<u32> = HashSet::new();

    // Lazy outer name remap table
    let mut outer_name_remap: Vec<Option<u32>> = vec![None; names.len()];

    // Pre-compute outer ignore set for O(1) lookups
    let outer_ignore_set: HashSet<u32> = ignore_list.iter().copied().collect();

    let mut dedup = DedupeState::new();

    for item in mappings_iter {
        let m = match item {
            Ok(m) => m,
            Err(_) => continue,
        };

        if m.source == NO_SOURCE {
            if !dedup.skip_sourceless(m.generated_line) {
                builder.add_generated_mapping(m.generated_line, m.generated_column);
            }
            dedup.record_sourceless(m.generated_line);
            continue;
        }

        let si = m.source as usize;
        if si >= sources.len() {
            continue;
        }

        // Load upstream map if not yet cached
        if matches!(source_entries[si], StreamingSourceEntry::Unloaded) {
            let source_name = &sources[si];
            if source_name.is_empty() {
                source_entries[si] = StreamingSourceEntry::EmptySource;
            } else {
                match loader(source_name) {
                    Some(upstream_sm) => {
                        let cache = build_upstream_cache(&upstream_sm);
                        source_entries[si] = StreamingSourceEntry::Upstream {
                            map: Box::new(upstream_sm),
                            cache,
                        };
                    }
                    None => {
                        let idx = builder.add_source(source_name);
                        if let Some(Some(content)) = sources_content.get(si) {
                            builder.set_source_content(idx, content.clone());
                        }
                        if outer_ignore_set.contains(&m.source) && ignored_sources.insert(idx) {
                            builder.add_to_ignore_list(idx);
                        }
                        source_entries[si] =
                            StreamingSourceEntry::Passthrough { builder_src: idx };
                    }
                }
            }
        }

        match &mut source_entries[si] {
            StreamingSourceEntry::Upstream { map, cache } => {
                match lookup_upstream(map, m.original_line, m.original_column) {
                    Some(upstream_m) => {
                        let builder_src = resolve_upstream_source_streaming(
                            cache,
                            map,
                            upstream_m.source,
                            &mut builder,
                            &mut ignored_sources,
                        );

                        let builder_name = if upstream_m.name != NO_NAME {
                            Some(resolve_upstream_name_streaming(
                                cache,
                                map,
                                upstream_m.name,
                                &mut builder,
                            ))
                        } else {
                            resolve_outer_name(&mut outer_name_remap, m.name, names, &mut builder)
                        };

                        if !dedup.skip_source(
                            m.generated_line,
                            builder_src,
                            upstream_m.original_line,
                            upstream_m.original_column,
                            builder_name,
                        ) {
                            emit_remapped_mapping_streaming(
                                &mut builder,
                                m.generated_line,
                                m.generated_column,
                                builder_src,
                                upstream_m.original_line,
                                upstream_m.original_column,
                                builder_name,
                                m.is_range_mapping,
                            );
                        }
                        dedup.record_source(
                            m.generated_line,
                            builder_src,
                            upstream_m.original_line,
                            upstream_m.original_column,
                            builder_name,
                        );
                    }
                    None => {
                        // No mapping in upstream — drop the segment entirely.
                        // jridgewell's traceMappings does `if (traced == null) continue;`
                    }
                }
            }
            StreamingSourceEntry::Passthrough { builder_src } => {
                let builder_src = *builder_src;

                let builder_name =
                    resolve_outer_name(&mut outer_name_remap, m.name, names, &mut builder);

                if !dedup.skip_source(
                    m.generated_line,
                    builder_src,
                    m.original_line,
                    m.original_column,
                    builder_name,
                ) {
                    emit_remapped_mapping_streaming(
                        &mut builder,
                        m.generated_line,
                        m.generated_column,
                        builder_src,
                        m.original_line,
                        m.original_column,
                        builder_name,
                        m.is_range_mapping,
                    );
                }
                dedup.record_source(
                    m.generated_line,
                    builder_src,
                    m.original_line,
                    m.original_column,
                    builder_name,
                );
            }
            StreamingSourceEntry::EmptySource => {
                if !dedup.skip_sourceless(m.generated_line) {
                    builder.add_generated_mapping(m.generated_line, m.generated_column);
                }
                dedup.record_sourceless(m.generated_line);
            }
            StreamingSourceEntry::Unloaded => unreachable!(),
        }
    }

    builder
        .to_decoded_map()
        .expect("streaming VLQ should be valid")
}

/// Resolve an outer name index to a builder name index, caching the result.
#[inline]
fn resolve_outer_name(
    outer_name_remap: &mut [Option<u32>],
    name_idx: u32,
    names: &[String],
    builder: &mut StreamingGenerator,
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

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

        let result = remap(&outer, |source| {
            if source == "intermediate.js" {
                Some(inner.clone())
            } else {
                None
            }
        });

        assert!(result.sources.contains(&"original.js".to_string()));
        // other.js passes through since loader returns None
        assert!(result.sources.contains(&"other.js".to_string()));

        // Line 0 col 0 in outer → line 0 col 0 in intermediate → line 1 col 0 in original
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "original.js");
        assert_eq!(loc.line, 1);
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

        let result = remap(&outer, |source| {
            if source == "compiled.js" {
                Some(inner.clone())
            } else {
                None
            }
        });

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

        assert_eq!(
            result.sources_content,
            vec![Some("const x = 1;".to_string())]
        );
    }

    // ── Clone needed for SourceMap in tests ──────────────────────

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

        let result = remap(&outer, |source| {
            if source == "a.js" {
                Some(inner.clone())
            } else {
                None
            }
        });

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
    fn remap_no_upstream_mapping_no_name() {
        // Outer has a mapping with NO name pointing to compiled.js
        // AAAA = gen(0,0) → compiled.js(0,0), no name (4-field segment)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        // Inner map only has mappings at line 5, not at line 0
        // So original_position_for(0, 0) returns None → takes the None branch
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":";;;;AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        // jridgewell drops the segment when upstream trace returns null
        let loc = result.original_position_for(0, 0);
        assert!(loc.is_none());
    }

    #[test]
    fn remap_upstream_found_no_name() {
        // Outer has a named mapping, but upstream has NO name
        // The upstream mapping is found but has no name_index
        // Since upstream has no name, the name resolution falls to the outer name
        // This is already covered by remap_preserves_names
        //
        // What we need instead: outer has NO name AND upstream has NO name
        // → name_idx is None → hits the add_mapping branch (line 246-252)
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
        assert!(result.names.is_empty());
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
            if source == "intermediate.js" {
                Some(inner.clone())
            } else {
                None
            }
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

        assert_eq!(
            result.sources_content,
            vec![Some("const x = 1;".to_string())]
        );
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

        let result = streaming_from_sm(&outer, |source| {
            if source == "a.js" {
                Some(inner.clone())
            } else {
                None
            }
        });

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
            if source == "intermediate.js" {
                Some(inner.clone())
            } else {
                None
            }
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
                assert_eq!(
                    result_normal.source(ln.source),
                    result_stream.source(ls.source)
                );
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

    #[test]
    fn streaming_no_upstream_mapping_no_name() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":["compiled.js"],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

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

    // ── Empty-string source filtering ────────────────────────────

    #[test]
    fn remap_empty_string_source_filtered() {
        // Outer map has an empty-string source (from JSON null)
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":[""],"names":[],"mappings":"AAAA"}"#,
        )
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

    #[test]
    fn remap_null_source_filtered() {
        // JSON null in sources array becomes "" after resolve_sources
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":[null],"names":[],"mappings":"AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| None);

        assert!(
            !result.sources.iter().any(|s| s.is_empty()),
            "null sources should be filtered out"
        );
    }

    #[test]
    fn streaming_empty_string_source_filtered() {
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":[""],"names":[],"mappings":"AAAA"}"#,
        )
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
        let outer = SourceMap::from_json(
            r#"{"version":3,"sources":[],"names":[],"mappings":"A"}"#,
        )
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
}
