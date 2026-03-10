use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SourceMap {
    inner: srcmap_sourcemap::SourceMap,
}

#[wasm_bindgen]
impl SourceMap {
    /// Parse a source map from a JSON string.
    #[wasm_bindgen(constructor)]
    pub fn new(json: &str) -> Result<SourceMap, JsError> {
        let inner = srcmap_sourcemap::SourceMap::from_json(json)
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(Self { inner })
    }

    /// Look up the original source position for a generated position.
    /// Both line and column are 0-based.
    /// Returns null if no mapping exists, or an object {source, line, column, name}.
    #[wasm_bindgen(js_name = "originalPositionFor")]
    pub fn original_position_for(&self, line: u32, column: u32) -> JsValue {
        match self.inner.original_position_for(line, column) {
            Some(loc) => {
                let obj = js_sys::Object::new();
                let source = self.inner.source(loc.source);
                js_sys::Reflect::set(&obj, &"source".into(), &source.into()).unwrap_or(false);
                js_sys::Reflect::set(&obj, &"line".into(), &loc.line.into()).unwrap_or(false);
                js_sys::Reflect::set(&obj, &"column".into(), &loc.column.into()).unwrap_or(false);
                let name_val: JsValue = match loc.name {
                    Some(i) => self.inner.name(i).into(),
                    None => JsValue::NULL,
                };
                js_sys::Reflect::set(&obj, &"name".into(), &name_val).unwrap_or(false);
                obj.into()
            }
            None => JsValue::NULL,
        }
    }

    /// Look up the generated position for an original source position.
    /// Returns null if no mapping exists, or an object {line, column}.
    #[wasm_bindgen(js_name = "generatedPositionFor")]
    pub fn generated_position_for(&self, source: &str, line: u32, column: u32) -> JsValue {
        match self.inner.generated_position_for(source, line, column) {
            Some(loc) => {
                let obj = js_sys::Object::new();
                js_sys::Reflect::set(&obj, &"line".into(), &loc.line.into()).unwrap_or(false);
                js_sys::Reflect::set(&obj, &"column".into(), &loc.column.into()).unwrap_or(false);
                obj.into()
            }
            None => JsValue::NULL,
        }
    }

    /// Resolve a source index to its filename.
    #[wasm_bindgen]
    pub fn source(&self, index: u32) -> String {
        self.inner.source(index).to_string()
    }

    /// Resolve a name index to its string.
    #[wasm_bindgen]
    pub fn name(&self, index: u32) -> String {
        self.inner.name(index).to_string()
    }

    /// Get all source filenames.
    #[wasm_bindgen(getter)]
    pub fn sources(&self) -> Vec<JsValue> {
        self.inner
            .sources
            .iter()
            .map(|s| JsValue::from_str(s))
            .collect()
    }

    /// Get all names.
    #[wasm_bindgen(getter)]
    pub fn names(&self) -> Vec<JsValue> {
        self.inner
            .names
            .iter()
            .map(|s| JsValue::from_str(s))
            .collect()
    }

    /// Get the debug ID (UUID) if present.
    #[wasm_bindgen(getter, js_name = "debugId")]
    pub fn debug_id(&self) -> Option<String> {
        self.inner.debug_id.clone()
    }

    /// Total number of decoded mappings.
    #[wasm_bindgen(getter, js_name = "mappingCount")]
    pub fn mapping_count(&self) -> u32 {
        self.inner.mapping_count() as u32
    }

    /// Number of generated lines.
    #[wasm_bindgen(getter, js_name = "lineCount")]
    pub fn line_count(&self) -> u32 {
        self.inner.line_count() as u32
    }

    /// Batch lookup: find original positions for multiple generated positions.
    /// Takes a flat array [line0, col0, line1, col1, ...].
    /// Returns a flat array [srcIdx0, line0, col0, nameIdx0, srcIdx1, ...].
    /// -1 means no mapping found / no name.
    #[wasm_bindgen(js_name = "originalPositionsFor")]
    pub fn original_positions_for(&self, positions: &[i32]) -> Vec<i32> {
        let count = positions.len() / 2;
        let mut out = Vec::with_capacity(count * 4);

        for i in 0..count {
            let line = positions[i * 2] as u32;
            let column = positions[i * 2 + 1] as u32;

            match self.inner.original_position_for(line, column) {
                Some(loc) => {
                    out.push(loc.source as i32);
                    out.push(loc.line as i32);
                    out.push(loc.column as i32);
                    out.push(loc.name.map_or(-1, |n| n as i32));
                }
                None => {
                    out.push(-1);
                    out.push(-1);
                    out.push(-1);
                    out.push(-1);
                }
            }
        }

        out
    }
}
