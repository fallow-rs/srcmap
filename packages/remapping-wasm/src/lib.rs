use std::cell::{OnceCell, RefCell};
use std::collections::HashMap;

use srcmap_sourcemap::{Mapping, OutOfRangeNames, ParseOptions, SourceMap};
use wasm_bindgen::prelude::*;

/// The `source` and `name` value of a mapping without a source or a name.
const NONE_INDEX: u32 = u32::MAX;

#[wasm_bindgen]
pub struct ConcatBuilder {
    inner: srcmap_remapping::ConcatBuilder,
}

#[wasm_bindgen]
impl ConcatBuilder {
    /// Create a new concatenation builder.
    #[wasm_bindgen(constructor)]
    pub fn new(file: Option<String>) -> Self {
        Self { inner: srcmap_remapping::ConcatBuilder::new(file) }
    }

    /// Add a source map (as JSON string) at the given line offset.
    #[wasm_bindgen(js_name = "addMap")]
    pub fn add_map(&mut self, json: &str, line_offset: u32) -> Result<(), JsError> {
        let sm = SourceMap::from_json(json).map_err(|e| JsError::new(&e.to_string()))?;
        self.inner.add_map(&sm, line_offset);
        Ok(())
    }

    /// Finish and return the concatenated source map as a JSON string.
    #[wasm_bindgen(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }
}

/// Compose/remap source maps through a transform chain.
///
/// `outer_json` is the final-stage source map as a JSON string.
/// `loader` is a function that receives a source filename and should return
/// the upstream source map JSON string, or null/undefined if none.
///
/// Returns the remapped source map as a JSON string. An error that the loader
/// throws, a loader result that is not a string, and an upstream map that does
/// not parse all stop the remap and go back to the caller.
///
/// A name index outside `names` and an empty name give a segment without a
/// name, as in `@jridgewell/remapping`, which adds a name only when
/// `names[index]` is truthy. A `names` entry or value that is falsy also gives
/// no name (see [`ParseOptions::lenient_names`]).
#[wasm_bindgen]
pub fn remap(outer_json: &str, loader: &js_sys::Function) -> Result<String, JsValue> {
    let placeholder = unnamed_placeholder();
    let options = ParseOptions {
        out_of_range_names: OutOfRangeNames::Placeholder(placeholder.clone()),
        lenient_names: true,
    };
    let outer = SourceMap::from_json_with_options(outer_json, &options)
        .map_err(|e| JsError::new(&e.to_string()))?;

    let failure: RefCell<Option<JsValue>> = RefCell::new(None);
    let result = srcmap_remapping::remap(&outer, |source| {
        if failure.borrow().is_some() {
            return None;
        }
        match load_upstream(loader, source, &options) {
            Ok(upstream) => upstream,
            Err(error) => {
                *failure.borrow_mut() = Some(error);
                None
            }
        }
    });

    match failure.into_inner() {
        Some(error) => Err(error),
        None => Ok(remove_unnamed(result, &placeholder).to_json()),
    }
}

/// The number of `i32` values per segment in the input of [`RemapTree::add_decoded`].
const DECODED_STRIDE: usize = 6;

/// The `sourcesContent` value of a source without content, in the content tokens.
const NO_CONTENT: i32 = -1;

/// The mappings of a [`RemapTree`] map before the parse.
enum TreeMappings {
    Encoded(String),
    Decoded(Box<[i32]>),
}

/// The fields of a [`RemapTree`] map before the parse.
struct TreeParts {
    file: Option<String>,
    sources: Vec<String>,
    contents: Box<[i32]>,
    names: Vec<String>,
    ignore_list: Vec<u32>,
    mappings: TreeMappings,
}

/// A map of a [`RemapTree`]. The parse runs when the remap first needs the map,
/// as [`remap`] parses an upstream map only when a segment needs it.
struct TreeNode {
    parts: RefCell<Option<TreeParts>>,
    parsed: OnceCell<SourceMap>,
}

impl TreeNode {
    fn new(parts: TreeParts) -> Self {
        Self { parts: RefCell::new(Some(parts)), parsed: OnceCell::new() }
    }

    fn get(&self, options: &ParseOptions) -> Result<&SourceMap, JsValue> {
        if let Some(map) = self.parsed.get() {
            return Ok(map);
        }
        // A failed parse stops the remap, so the parts are only taken one time.
        let parts = self.parts.borrow_mut().take().ok_or_else(|| JsError::new("map is gone"))?;
        let map = parse_parts(parts, options).map_err(|e| JsError::new(&e.to_string()))?;
        Ok(self.parsed.get_or_init(|| map))
    }
}

/// The maps of one remap, given as parts instead of JSON.
///
/// `@srcmap/remapping` uses it for maps that it has as objects. Mappings come
/// as a VLQ string or as decoded segments, so that no side encodes or decodes
/// them only to cross the boundary. `sourcesContent` stays on the JS side: each
/// map gives a token per source (an index into a JS array, or -1 for no
/// content), and the result gives the tokens back.
///
/// The remap gives the same result as [`remap`] with the same maps as JSON:
/// the root map is the outer map, and each upstream map has a key that is a
/// source name in another map. [`RemapTree::remap`] runs one remap pass per
/// level, as a caller of [`remap`] does with one call per level.
#[wasm_bindgen]
pub struct RemapTree {
    root: Option<TreeParts>,
    nodes: Vec<TreeNode>,
    keys: HashMap<String, usize>,
    result: Option<SourceMap>,
}

impl Default for RemapTree {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl RemapTree {
    /// Create an empty tree.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Self {
        Self { root: None, nodes: Vec::new(), keys: HashMap::new(), result: None }
    }

    /// Add a map with VLQ `mappings` as UTF-8 bytes. Without `key`, the map is the root.
    ///
    /// `contents` holds a content token per source (see [`RemapTree`]). The
    /// mappings come as bytes, because the JS side can copy bytes into WASM
    /// faster than a string.
    #[wasm_bindgen(js_name = "addEncoded")]
    #[allow(clippy::too_many_arguments, reason = "each argument is one source map field")]
    pub fn add_encoded(
        &mut self,
        key: Option<String>,
        file: Option<String>,
        sources: Vec<String>,
        contents: Box<[i32]>,
        names: Vec<String>,
        ignore_list: Vec<u32>,
        mappings: Vec<u8>,
    ) -> Result<(), JsError> {
        let mappings = String::from_utf8(mappings)
            .map_err(|_| JsError::new("the mappings are not valid UTF-8"))?;
        let parts = TreeParts {
            file,
            sources,
            contents,
            names,
            ignore_list,
            mappings: TreeMappings::Encoded(mappings),
        };
        self.add(key, parts);
        Ok(())
    }

    /// Add a map with decoded mappings. Without `key`, the map is the root.
    ///
    /// `segments` holds 6 values per segment: generated line, generated column,
    /// source index, original line, original column and name index. A segment
    /// without a source has -1 as source index, and a segment without a name has
    /// -1 as name index. The segments come in line order. The caller makes sure
    /// that each value is not negative (except the -1 markers) and that each
    /// source index is inside `sources`. A name index outside `names` gives the
    /// segment no name, as in [`remap`].
    #[wasm_bindgen(js_name = "addDecoded")]
    #[allow(clippy::too_many_arguments, reason = "each argument is one source map field")]
    pub fn add_decoded(
        &mut self,
        key: Option<String>,
        file: Option<String>,
        sources: Vec<String>,
        contents: Box<[i32]>,
        names: Vec<String>,
        ignore_list: Vec<u32>,
        segments: Box<[i32]>,
    ) {
        let parts = TreeParts {
            file,
            sources,
            contents,
            names,
            ignore_list,
            mappings: TreeMappings::Decoded(segments),
        };
        self.add(key, parts);
    }

    /// Remap the root map through the upstream maps, with `passes` remap passes
    /// (at least one). Read the result with the `result*` methods.
    ///
    /// An error in the root map, or in an upstream map that a pass needs, stops
    /// the remap, as in [`remap`].
    pub fn remap(&mut self, passes: u32) -> Result<(), JsValue> {
        let placeholder = unnamed_placeholder();
        let options = ParseOptions {
            out_of_range_names: OutOfRangeNames::Placeholder(placeholder.clone()),
            lenient_names: true,
        };
        let root = self.root.take().ok_or_else(|| JsError::new("the tree has no root map"))?;
        let mut current = parse_parts(root, &options).map_err(|e| JsError::new(&e.to_string()))?;

        for _ in 0..passes.max(1) {
            let failure: RefCell<Option<JsValue>> = RefCell::new(None);
            let result = srcmap_remapping::remap_ref(&current, |source| {
                if failure.borrow().is_some() {
                    return None;
                }
                let node = &self.nodes[*self.keys.get(source)?];
                match node.get(&options) {
                    Ok(map) => Some(map),
                    Err(error) => {
                        *failure.borrow_mut() = Some(error);
                        None
                    }
                }
            });
            if let Some(error) = failure.into_inner() {
                return Err(error);
            }
            current = remove_unnamed(result, &placeholder);
        }
        self.result = Some(current);
        Ok(())
    }

    /// The `file` of the result.
    #[wasm_bindgen(js_name = "resultFile")]
    pub fn result_file(&self) -> Option<String> {
        self.result.as_ref().and_then(|map| map.file.clone())
    }

    /// The VLQ `mappings` of the result.
    #[wasm_bindgen(js_name = "resultMappings")]
    pub fn result_mappings(&self) -> String {
        self.result.as_ref().map(SourceMap::encode_mappings).unwrap_or_default()
    }

    /// The `sources` of the result.
    #[wasm_bindgen(js_name = "resultSources")]
    pub fn result_sources(&self) -> Vec<String> {
        self.result.as_ref().map(|map| map.sources.clone()).unwrap_or_default()
    }

    /// The `names` of the result.
    #[wasm_bindgen(js_name = "resultNames")]
    pub fn result_names(&self) -> Vec<String> {
        self.result.as_ref().map(|map| map.names.clone()).unwrap_or_default()
    }

    /// The content tokens of the result, one per `sourcesContent` entry.
    #[wasm_bindgen(js_name = "resultContents")]
    pub fn result_contents(&self) -> Vec<i32> {
        let Some(map) = &self.result else { return Vec::new() };
        map.sources_content
            .iter()
            .map(|content| content.as_deref().and_then(|t| t.parse().ok()).unwrap_or(NO_CONTENT))
            .collect()
    }

    /// The `ignoreList` of the result.
    #[wasm_bindgen(js_name = "resultIgnoreList")]
    pub fn result_ignore_list(&self) -> Vec<u32> {
        self.result.as_ref().map(|map| map.ignore_list.clone()).unwrap_or_default()
    }
}

impl RemapTree {
    fn add(&mut self, key: Option<String>, parts: TreeParts) {
        match key {
            Some(key) => {
                self.keys.insert(key, self.nodes.len());
                self.nodes.push(TreeNode::new(parts));
            }
            None => self.root = Some(parts),
        }
    }
}

/// Build the map of `parts`, with the checks that [`SourceMap::from_json_with_options`] does.
fn parse_parts(
    parts: TreeParts,
    options: &ParseOptions,
) -> Result<SourceMap, srcmap_sourcemap::ParseError> {
    let TreeParts { file, sources, contents, mut names, ignore_list, mappings } = parts;
    let sources_content =
        contents.iter().map(|&token| (token != NO_CONTENT).then(|| token.to_string())).collect();
    match mappings {
        TreeMappings::Encoded(vlq) => SourceMap::from_vlq_with_options(
            &vlq,
            sources,
            names,
            file,
            sources_content,
            ignore_list,
            options,
        ),
        TreeMappings::Decoded(segments) => {
            let mappings = decoded_mappings(&segments, &mut names, &options.out_of_range_names);
            Ok(SourceMap::from_parts(
                file,
                None,
                sources,
                sources_content,
                names,
                mappings,
                ignore_list,
                None,
                None,
            ))
        }
    }
}

/// Convert the flat segments of [`RemapTree::add_decoded`] to mappings.
///
/// As the VLQ decode does, a name index outside `names` follows `names_mode`,
/// and the segments of a line get a stable sort by generated column.
fn decoded_mappings(
    segments: &[i32],
    names: &mut Vec<String>,
    names_mode: &OutOfRangeNames,
) -> Vec<Mapping> {
    let names_len = names.len();
    let out_of_range_name = u32::try_from(names_len).unwrap_or(NONE_INDEX);
    let index = |value: i32| u32::try_from(value).unwrap_or(NONE_INDEX);
    let mut used_placeholder = false;
    let mut unsorted = false;
    let mut mappings: Vec<Mapping> = Vec::with_capacity(segments.len() / DECODED_STRIDE);
    for segment in segments.as_chunks::<DECODED_STRIDE>().0 {
        let mut name = index(segment[5]);
        if name != NONE_INDEX && name as usize >= names_len {
            name = match names_mode {
                OutOfRangeNames::Placeholder(_) => {
                    used_placeholder = true;
                    out_of_range_name
                }
                OutOfRangeNames::Unnamed | OutOfRangeNames::Reject => NONE_INDEX,
            };
        }
        let mapping = Mapping {
            generated_line: index(segment[0]),
            generated_column: index(segment[1]),
            source: index(segment[2]),
            original_line: index(segment[3]),
            original_column: index(segment[4]),
            name,
            is_range_mapping: false,
        };
        unsorted |= mappings.last().is_some_and(|previous| {
            previous.generated_line == mapping.generated_line
                && previous.generated_column > mapping.generated_column
        });
        mappings.push(mapping);
    }
    if used_placeholder && let OutOfRangeNames::Placeholder(placeholder) = names_mode {
        names.push(placeholder.clone());
    }
    if unsorted {
        // A stable sort, so that segments with one column keep their order.
        for line in mappings.chunk_by_mut(|a, b| a.generated_line == b.generated_line) {
            line.sort_by_key(|m| m.generated_column);
        }
    }
    mappings
}

fn load_upstream(
    loader: &js_sys::Function,
    source: &str,
    options: &ParseOptions,
) -> Result<Option<SourceMap>, JsValue> {
    let ret = loader.call1(&JsValue::NULL, &JsValue::from_str(source))?;
    if ret.is_null() || ret.is_undefined() {
        return Ok(None);
    }
    let json = ret.as_string().ok_or_else(|| {
        JsError::new(&format!("loader result for `{source}` is not a JSON string"))
    })?;
    SourceMap::from_json_with_options(&json, options)
        .map(Some)
        .map_err(|e| JsError::new(&e.to_string()).into())
}

/// A name that no real map holds, for the name indices outside `names`.
///
/// A segment of an upstream map that has a name index outside `names` must not
/// take the name of the outer segment. With this name the remap keeps the
/// segment named, and [`remove_unnamed`] removes the name after the remap.
fn unnamed_placeholder() -> String {
    format!("\u{0}srcmap-unnamed-{}", js_sys::Math::random())
}

/// Remove the empty names and `placeholder` from `map`, as jridgewell does not
/// add a name that is not truthy.
///
/// A segment that then equals the segment before it on the same line is removed,
/// as the `skipSource` check of jridgewell does.
fn remove_unnamed(map: SourceMap, placeholder: &str) -> SourceMap {
    if !map.names.iter().any(|name| name.is_empty() || name == placeholder) {
        return map;
    }

    let mut names = Vec::with_capacity(map.names.len());
    let name_remap: Vec<u32> = map
        .names
        .iter()
        .map(|name| {
            if name.is_empty() || name == placeholder {
                return NONE_INDEX;
            }
            names.push(name.clone());
            u32::try_from(names.len() - 1).unwrap_or(NONE_INDEX)
        })
        .collect();

    let mut mappings: Vec<Mapping> = Vec::with_capacity(map.mapping_count());
    for mapping in map.all_mappings() {
        let mut mapping = *mapping;
        if mapping.name != NONE_INDEX {
            mapping.name = name_remap.get(mapping.name as usize).copied().unwrap_or(NONE_INDEX);
        }
        if !repeats_previous(mappings.last(), &mapping) {
            mappings.push(mapping);
        }
    }

    SourceMap::from_parts_with_extensions(
        map.file,
        map.source_root,
        map.sources,
        map.sources_content,
        names,
        mappings,
        map.ignore_list,
        map.debug_id,
        map.scopes,
        map.extensions,
    )
}

/// True when `mapping` and `previous` are sourced segments on one generated line
/// with the same source, original position and name.
fn repeats_previous(previous: Option<&Mapping>, mapping: &Mapping) -> bool {
    previous.is_some_and(|previous| {
        previous.generated_line == mapping.generated_line
            && previous.source != NONE_INDEX
            && mapping.source != NONE_INDEX
            && previous.source == mapping.source
            && previous.original_line == mapping.original_line
            && previous.original_column == mapping.original_column
            && previous.name == mapping.name
    })
}
