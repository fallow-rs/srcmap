use std::cell::RefCell;

use wasm_bindgen::prelude::*;

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
        let sm = srcmap_sourcemap::SourceMap::from_json(json)
            .map_err(|e| JsError::new(&e.to_string()))?;
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
#[wasm_bindgen]
pub fn remap(outer_json: &str, loader: &js_sys::Function) -> Result<String, JsValue> {
    let outer = srcmap_sourcemap::SourceMap::from_json(outer_json)
        .map_err(|e| JsError::new(&e.to_string()))?;

    let failure: RefCell<Option<JsValue>> = RefCell::new(None);
    let result = srcmap_remapping::remap(&outer, |source| {
        if failure.borrow().is_some() {
            return None;
        }
        match load_upstream(loader, source) {
            Ok(upstream) => upstream,
            Err(error) => {
                *failure.borrow_mut() = Some(error);
                None
            }
        }
    });

    match failure.into_inner() {
        Some(error) => Err(error),
        None => Ok(result.to_json()),
    }
}

fn load_upstream(
    loader: &js_sys::Function,
    source: &str,
) -> Result<Option<srcmap_sourcemap::SourceMap>, JsValue> {
    let ret = loader.call1(&JsValue::NULL, &JsValue::from_str(source))?;
    if ret.is_null() || ret.is_undefined() {
        return Ok(None);
    }
    let json = ret.as_string().ok_or_else(|| {
        JsError::new(&format!("loader result for `{source}` is not a JSON string"))
    })?;
    srcmap_sourcemap::SourceMap::from_json(&json)
        .map(Some)
        .map_err(|e| JsError::new(&e.to_string()).into())
}
