use std::cell::RefCell;

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
/// `names[index]` is truthy.
#[wasm_bindgen]
pub fn remap(outer_json: &str, loader: &js_sys::Function) -> Result<String, JsValue> {
    let placeholder = unnamed_placeholder();
    let options =
        ParseOptions { out_of_range_names: OutOfRangeNames::Placeholder(placeholder.clone()) };
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
