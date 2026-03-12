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
use std::collections::HashMap;

// ── Concatenation ─────────────────────────────────────────────────

/// Builder for concatenating multiple source maps into one.
///
/// Each added source map is offset by a line delta, producing a single
/// combined map. Sources and names are deduplicated across inputs.
pub struct ConcatBuilder {
    builder: SourceMapGenerator,
    source_remap: HashMap<String, u32>,
    name_remap: HashMap<String, u32>,
}

impl ConcatBuilder {
    /// Create a new concatenation builder.
    pub fn new(file: Option<String>) -> Self {
        Self {
            builder: SourceMapGenerator::new(file),
            source_remap: HashMap::new(),
            name_remap: HashMap::new(),
        }
    }

    /// Add a source map to the concatenated output.
    ///
    /// `line_offset` is the number of lines to shift all mappings by
    /// (i.e. the line at which this chunk starts in the output).
    pub fn add_map(&mut self, sm: &SourceMap, line_offset: u32) {
        // Remap sources
        let source_indices: Vec<u32> = sm
            .sources
            .iter()
            .enumerate()
            .map(|(i, s)| {
                if let Some(&idx) = self.source_remap.get(s) {
                    // If this source has content and we don't yet, update it
                    if let Some(Some(content)) = sm.sources_content.get(i) {
                        self.builder.set_source_content(idx, content.clone());
                    }
                    idx
                } else {
                    let idx = self.builder.add_source(s);
                    if let Some(Some(content)) = sm.sources_content.get(i) {
                        self.builder.set_source_content(idx, content.clone());
                    }
                    self.source_remap.insert(s.clone(), idx);
                    idx
                }
            })
            .collect();

        // Remap names
        let name_indices: Vec<u32> = sm
            .names
            .iter()
            .map(|n| {
                if let Some(&idx) = self.name_remap.get(n) {
                    idx
                } else {
                    let idx = self.builder.add_name(n);
                    self.name_remap.insert(n.clone(), idx);
                    idx
                }
            })
            .collect();

        // Copy ignore_list entries
        for &ignored in &sm.ignore_list {
            let global_idx = source_indices[ignored as usize];
            self.builder.add_to_ignore_list(global_idx);
        }

        // Add all mappings with line offset
        for m in sm.all_mappings() {
            let gen_line = m.generated_line + line_offset;

            if m.source == u32::MAX {
                self.builder
                    .add_generated_mapping(gen_line, m.generated_column);
            } else {
                let src = source_indices[m.source as usize];
                if m.is_range_mapping {
                    if m.name != u32::MAX {
                        let name = name_indices[m.name as usize];
                        self.builder.add_named_range_mapping(
                            gen_line,
                            m.generated_column,
                            src,
                            m.original_line,
                            m.original_column,
                            name,
                        );
                    } else {
                        self.builder.add_range_mapping(
                            gen_line,
                            m.generated_column,
                            src,
                            m.original_line,
                            m.original_column,
                        );
                    }
                } else if m.name != u32::MAX {
                    let name = name_indices[m.name as usize];
                    self.builder.add_named_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                        name,
                    );
                } else {
                    self.builder.add_mapping(
                        gen_line,
                        m.generated_column,
                        src,
                        m.original_line,
                        m.original_column,
                    );
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

/// Resolved original-source parameters for a single mapping.
struct MappingParams<'a> {
    source: Option<&'a str>,
    source_content: Option<&'a str>,
    original_line: u32,
    original_column: u32,
    name: Option<&'a str>,
}

/// Add a mapping to the generator, dispatching to range/non-range variants.
fn add_mapping_to_builder(
    builder: &mut SourceMapGenerator,
    gen_line: u32,
    gen_col: u32,
    params: &MappingParams<'_>,
    is_range: bool,
) {
    let source = params.source.expect("source required for source mapping");
    let src_idx = builder.add_source(source);

    if let Some(content) = params.source_content {
        builder.set_source_content(src_idx, content.to_string());
    }

    let name_idx = params.name.map(|n| builder.add_name(n));

    match (name_idx, is_range) {
        (Some(n), true) => builder.add_named_range_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
            n,
        ),
        (Some(n), false) => builder.add_named_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
            n,
        ),
        (None, true) => builder.add_range_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
        ),
        (None, false) => builder.add_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
        ),
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
/// This is equivalent to `@ampproject/remapping` in the JS ecosystem.
pub fn remap<F>(outer: &SourceMap, loader: F) -> SourceMap
where
    F: Fn(&str) -> Option<SourceMap>,
{
    let mut builder = SourceMapGenerator::new(outer.file.clone());

    // Cache: source name → loaded upstream map (or None)
    let mut upstream_maps: HashMap<u32, Option<SourceMap>> = HashMap::new();
    // Track which builder source indices have already been marked as ignored
    let mut ignored_sources: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for m in outer.all_mappings() {
        if m.source == u32::MAX {
            builder.add_generated_mapping(m.generated_line, m.generated_column);
            continue;
        }

        let source_name = outer.source(m.source);

        // Load upstream map if we haven't already
        let upstream = upstream_maps
            .entry(m.source)
            .or_insert_with(|| loader(source_name));

        match upstream {
            Some(upstream_sm) => {
                // Trace through the upstream map
                match upstream_sm.original_position_for(m.original_line, m.original_column) {
                    Some(loc) => {
                        let orig_source = upstream_sm.source(loc.source);
                        let source_content = upstream_sm
                            .sources_content
                            .get(loc.source as usize)
                            .and_then(|c| c.as_deref());

                        // Resolve name: prefer upstream name if available, else outer name
                        let name = loc.name.map(|n| upstream_sm.name(n)).or_else(|| {
                            if m.name != u32::MAX {
                                Some(outer.name(m.name))
                            } else {
                                None
                            }
                        });

                        let params = MappingParams {
                            source: Some(orig_source),
                            source_content,
                            original_line: loc.line,
                            original_column: loc.column,
                            name,
                        };

                        add_mapping_to_builder(
                            &mut builder,
                            m.generated_line,
                            m.generated_column,
                            &params,
                            m.is_range_mapping,
                        );

                        // Propagate ignore_list from upstream map
                        if upstream_sm.ignore_list.contains(&loc.source) {
                            let src_idx = builder.add_source(orig_source);
                            if ignored_sources.insert(src_idx) {
                                builder.add_to_ignore_list(src_idx);
                            }
                        }
                    }
                    None => {
                        // No mapping in upstream — keep original reference
                        let name = if m.name != u32::MAX {
                            Some(outer.name(m.name))
                        } else {
                            None
                        };

                        let params = MappingParams {
                            source: Some(source_name),
                            source_content: None,
                            original_line: m.original_line,
                            original_column: m.original_column,
                            name,
                        };

                        add_mapping_to_builder(
                            &mut builder,
                            m.generated_line,
                            m.generated_column,
                            &params,
                            m.is_range_mapping,
                        );
                    }
                }
            }
            None => {
                // No upstream map — pass through as-is
                let source_content = outer
                    .sources_content
                    .get(m.source as usize)
                    .and_then(|c| c.as_deref());

                let name = if m.name != u32::MAX {
                    Some(outer.name(m.name))
                } else {
                    None
                };

                let params = MappingParams {
                    source: Some(source_name),
                    source_content,
                    original_line: m.original_line,
                    original_column: m.original_column,
                    name,
                };

                add_mapping_to_builder(
                    &mut builder,
                    m.generated_line,
                    m.generated_column,
                    &params,
                    m.is_range_mapping,
                );

                // Propagate ignore_list from outer map
                if outer.ignore_list.contains(&m.source) {
                    let src_idx = builder.add_source(source_name);
                    if ignored_sources.insert(src_idx) {
                        builder.add_to_ignore_list(src_idx);
                    }
                }
            }
        }
    }

    builder.to_decoded_map()
}

/// Add a mapping to a streaming generator, dispatching to range/non-range variants.
fn add_mapping_to_streaming(
    builder: &mut StreamingGenerator,
    gen_line: u32,
    gen_col: u32,
    params: &MappingParams<'_>,
    is_range: bool,
) {
    let source = params.source.expect("source required for source mapping");
    let src_idx = builder.add_source(source);

    if let Some(content) = params.source_content {
        builder.set_source_content(src_idx, content.to_string());
    }

    let name_idx = params.name.map(|n| builder.add_name(n));

    match (name_idx, is_range) {
        (Some(n), true) => builder.add_named_range_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
            n,
        ),
        (Some(n), false) => builder.add_named_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
            n,
        ),
        (None, true) => builder.add_range_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
        ),
        (None, false) => builder.add_mapping(
            gen_line,
            gen_col,
            src_idx,
            params.original_line,
            params.original_column,
        ),
    }
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
    let mut builder = StreamingGenerator::new(file);

    // Cache: source index → loaded upstream map (or None)
    let mut upstream_maps: HashMap<u32, Option<SourceMap>> = HashMap::new();
    // Track which builder source indices have already been marked as ignored
    let mut ignored_sources: std::collections::HashSet<u32> = std::collections::HashSet::new();

    for item in mappings_iter {
        let m = match item {
            Ok(m) => m,
            Err(_) => continue, // skip invalid segments
        };

        if m.source == u32::MAX {
            builder.add_generated_mapping(m.generated_line, m.generated_column);
            continue;
        }

        let Some(source_name) = sources.get(m.source as usize) else {
            continue;
        };

        // Load upstream map if we haven't already
        let upstream = upstream_maps
            .entry(m.source)
            .or_insert_with(|| loader(source_name));

        match upstream {
            Some(upstream_sm) => {
                // Trace through the upstream map
                match upstream_sm.original_position_for(m.original_line, m.original_column) {
                    Some(loc) => {
                        let orig_source = upstream_sm.source(loc.source);
                        let source_content = upstream_sm
                            .sources_content
                            .get(loc.source as usize)
                            .and_then(|c| c.as_deref());

                        // Resolve name: prefer upstream name if available, else outer name
                        let name = loc.name.map(|n| upstream_sm.name(n)).or_else(|| {
                            if m.name != u32::MAX {
                                names.get(m.name as usize).map(|s| s.as_str())
                            } else {
                                None
                            }
                        });

                        let params = MappingParams {
                            source: Some(orig_source),
                            source_content,
                            original_line: loc.line,
                            original_column: loc.column,
                            name,
                        };

                        add_mapping_to_streaming(
                            &mut builder,
                            m.generated_line,
                            m.generated_column,
                            &params,
                            m.is_range_mapping,
                        );

                        // Propagate ignore_list from upstream map
                        if upstream_sm.ignore_list.contains(&loc.source) {
                            let src_idx = builder.add_source(orig_source);
                            if ignored_sources.insert(src_idx) {
                                builder.add_to_ignore_list(src_idx);
                            }
                        }
                    }
                    None => {
                        // No mapping in upstream — keep original reference
                        let name = if m.name != u32::MAX {
                            names.get(m.name as usize).map(|s| s.as_str())
                        } else {
                            None
                        };

                        let params = MappingParams {
                            source: Some(source_name),
                            source_content: None,
                            original_line: m.original_line,
                            original_column: m.original_column,
                            name,
                        };

                        add_mapping_to_streaming(
                            &mut builder,
                            m.generated_line,
                            m.generated_column,
                            &params,
                            m.is_range_mapping,
                        );
                    }
                }
            }
            None => {
                // No upstream map — pass through as-is
                let source_content = sources_content
                    .get(m.source as usize)
                    .and_then(|c| c.as_deref());

                let name = if m.name != u32::MAX {
                    names.get(m.name as usize).map(|s| s.as_str())
                } else {
                    None
                };

                let params = MappingParams {
                    source: Some(source_name),
                    source_content,
                    original_line: m.original_line,
                    original_column: m.original_column,
                    name,
                };

                add_mapping_to_streaming(
                    &mut builder,
                    m.generated_line,
                    m.generated_column,
                    &params,
                    m.is_range_mapping,
                );

                // Propagate ignore_list from outer
                if ignore_list.contains(&m.source) {
                    let src_idx = builder.add_source(source_name);
                    if ignored_sources.insert(src_idx) {
                        builder.add_to_ignore_list(src_idx);
                    }
                }
            }
        }
    }

    builder.to_decoded_map()
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

        // The outer mapping at (0,0) maps to (0,0) in compiled.js
        // Inner doesn't have a mapping at (0,0), so it falls through
        // The name from outer should be preserved
        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "myFunc");
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
        // Since the outer mapping has no name, this hits the else at lines 268-272
        let inner = SourceMap::from_json(
            r#"{"version":3,"sources":["original.ts"],"names":[],"mappings":";;;;AAAA"}"#,
        )
        .unwrap();

        let result = remap(&outer, |_| Some(inner.clone()));

        // Falls through to the None branch (no upstream match at position)
        // Since outer has no name, the mapping is added without a name
        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "compiled.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
        assert!(loc.name.is_none());
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

        let loc = result.original_position_for(0, 0).unwrap();
        assert!(loc.name.is_some());
        assert_eq!(result.name(loc.name.unwrap()), "myFunc");
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

        let loc = result.original_position_for(0, 0).unwrap();
        assert_eq!(result.source(loc.source), "compiled.js");
        assert!(loc.name.is_none());
    }
}
