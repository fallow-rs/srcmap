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

    /// Build a source map from pre-parsed components.
    ///
    /// This is the fast path: JS does JSON.parse() (V8-native speed),
    /// then only the VLQ mappings string is sent to WASM for decoding.
    /// Avoids copying large sourcesContent into WASM linear memory.
    #[wasm_bindgen(js_name = "fromVlq")]
    pub fn from_vlq(
        mappings: &str,
        sources: Vec<JsValue>,
        names: Vec<JsValue>,
        file: Option<String>,
        source_root: Option<String>,
        sources_content: Vec<JsValue>,
        ignore_list: Vec<u32>,
        debug_id: Option<String>,
    ) -> Result<SourceMap, JsError> {
        let sources: Vec<String> = sources
            .iter()
            .map(|s| s.as_string().unwrap_or_default())
            .collect();
        let names: Vec<String> = names
            .iter()
            .map(|s| s.as_string().unwrap_or_default())
            .collect();
        let sources_content: Vec<Option<String>> = sources_content
            .iter()
            .map(|s| s.as_string())
            .collect();

        let inner = srcmap_sourcemap::SourceMap::from_vlq(
            mappings,
            sources,
            names,
            file,
            source_root,
            sources_content,
            ignore_list,
            debug_id,
        )
        .map_err(|e| JsError::new(&e.to_string()))?;

        Ok(Self { inner })
    }

    /// Look up the original source position for a generated position.
    /// Both line and column are 0-based.
    /// Returns null if no mapping exists, or an object {source, line, column, name}.
    #[wasm_bindgen(js_name = "originalPositionFor")]
    pub fn original_position_for(&self, line: u32, column: u32) -> JsValue {
        self.original_position_for_with_bias(line, column, 0)
    }

    /// Look up the original source position with a bias.
    /// bias: 0 = GREATEST_LOWER_BOUND (default), -1 = LEAST_UPPER_BOUND
    #[wasm_bindgen(js_name = "originalPositionForWithBias")]
    pub fn original_position_for_with_bias(&self, line: u32, column: u32, bias: i32) -> JsValue {
        let b = if bias == -1 {
            srcmap_sourcemap::Bias::LeastUpperBound
        } else {
            srcmap_sourcemap::Bias::GreatestLowerBound
        };
        match self.inner.original_position_for_with_bias(line, column, b) {
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

    /// Fast single lookup returning flat array [sourceIdx, line, column, nameIdx].
    /// Returns [-1, -1, -1, -1] for unmapped positions.
    /// Use source(idx) and name(idx) to resolve strings.
    #[wasm_bindgen(js_name = "originalPositionFlat")]
    pub fn original_position_flat(&self, line: u32, column: u32) -> Vec<i32> {
        match self.inner.original_position_for(line, column) {
            Some(loc) => vec![
                loc.source as i32,
                loc.line as i32,
                loc.column as i32,
                loc.name.map_or(-1, |n| n as i32),
            ],
            None => vec![-1, -1, -1, -1],
        }
    }

    /// Look up the generated position for an original source position.
    /// Returns null if no mapping exists, or an object {line, column}.
    #[wasm_bindgen(js_name = "generatedPositionFor")]
    pub fn generated_position_for(&self, source: &str, line: u32, column: u32) -> JsValue {
        self.generated_position_for_with_bias(source, line, column, 0)
    }

    /// Look up the generated position with a bias.
    /// bias: 0 = default, -1 = LEAST_UPPER_BOUND, 1 = GREATEST_LOWER_BOUND
    #[wasm_bindgen(js_name = "generatedPositionForWithBias")]
    pub fn generated_position_for_with_bias(
        &self,
        source: &str,
        line: u32,
        column: u32,
        bias: i32,
    ) -> JsValue {
        let b = if bias == 1 {
            srcmap_sourcemap::Bias::GreatestLowerBound
        } else {
            srcmap_sourcemap::Bias::LeastUpperBound
        };
        match self
            .inner
            .generated_position_for_with_bias(source, line, column, b)
        {
            Some(loc) => {
                let obj = js_sys::Object::new();
                js_sys::Reflect::set(&obj, &"line".into(), &loc.line.into()).unwrap_or(false);
                js_sys::Reflect::set(&obj, &"column".into(), &loc.column.into()).unwrap_or(false);
                obj.into()
            }
            None => JsValue::NULL,
        }
    }

    /// Map a generated range to its original range.
    /// Returns null if either endpoint has no mapping or endpoints map to different sources.
    #[wasm_bindgen(js_name = "mapRange")]
    pub fn map_range(
        &self,
        start_line: u32,
        start_column: u32,
        end_line: u32,
        end_column: u32,
    ) -> JsValue {
        match self
            .inner
            .map_range(start_line, start_column, end_line, end_column)
        {
            Some(range) => {
                let obj = js_sys::Object::new();
                let source = self.inner.source(range.source);
                js_sys::Reflect::set(&obj, &"source".into(), &source.into()).unwrap_or(false);
                js_sys::Reflect::set(
                    &obj,
                    &"originalStartLine".into(),
                    &range.original_start_line.into(),
                )
                .unwrap_or(false);
                js_sys::Reflect::set(
                    &obj,
                    &"originalStartColumn".into(),
                    &range.original_start_column.into(),
                )
                .unwrap_or(false);
                js_sys::Reflect::set(
                    &obj,
                    &"originalEndLine".into(),
                    &range.original_end_line.into(),
                )
                .unwrap_or(false);
                js_sys::Reflect::set(
                    &obj,
                    &"originalEndColumn".into(),
                    &range.original_end_column.into(),
                )
                .unwrap_or(false);
                obj.into()
            }
            None => JsValue::NULL,
        }
    }

    /// Find all generated positions for an original source position.
    /// Returns an array of {line, column} objects.
    #[wasm_bindgen(js_name = "allGeneratedPositionsFor")]
    pub fn all_generated_positions_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Vec<JsValue> {
        self.inner
            .all_generated_positions_for(source, line, column)
            .into_iter()
            .map(|loc| {
                let obj = js_sys::Object::new();
                js_sys::Reflect::set(&obj, &"line".into(), &loc.line.into()).unwrap_or(false);
                js_sys::Reflect::set(&obj, &"column".into(), &loc.column.into()).unwrap_or(false);
                obj.into()
            })
            .collect()
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

    /// Get the file property.
    #[wasm_bindgen(getter)]
    pub fn file(&self) -> Option<String> {
        self.inner.file.clone()
    }

    /// Get the sourceRoot property.
    #[wasm_bindgen(getter, js_name = "sourceRoot")]
    pub fn source_root(&self) -> Option<String> {
        self.inner.source_root.clone()
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

    /// Get all sources content (array of string|null).
    #[wasm_bindgen(getter, js_name = "sourcesContent")]
    pub fn sources_content(&self) -> Vec<JsValue> {
        self.inner
            .sources_content
            .iter()
            .map(|c| match c {
                Some(s) => JsValue::from_str(s),
                None => JsValue::NULL,
            })
            .collect()
    }

    /// Get the ignore list (array of source indices).
    #[wasm_bindgen(getter, js_name = "ignoreList")]
    pub fn ignore_list(&self) -> Vec<u32> {
        self.inner.ignore_list.clone()
    }

    /// Get source content for a given source index.
    #[wasm_bindgen(js_name = "sourceContentFor")]
    pub fn source_content_for(&self, index: u32) -> JsValue {
        match self.inner.sources_content.get(index as usize) {
            Some(Some(content)) => JsValue::from_str(content),
            _ => JsValue::NULL,
        }
    }

    /// Check if a source index is in the ignore list.
    #[wasm_bindgen(js_name = "isIgnoredIndex")]
    pub fn is_ignored_index(&self, index: u32) -> bool {
        self.inner.ignore_list.contains(&index)
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

    /// Encode all mappings back to a VLQ mappings string.
    #[wasm_bindgen(js_name = "encodedMappings")]
    pub fn encoded_mappings(&self) -> String {
        self.inner.encode_mappings()
    }

    /// Get all mappings as a flat Int32Array.
    /// Format: [genLine, genCol, source, origLine, origCol, name, ...] per mapping.
    /// source = -1 means unmapped, name = -1 means no name.
    #[wasm_bindgen(js_name = "allMappingsFlat")]
    pub fn all_mappings_flat(&self) -> Vec<i32> {
        let mappings = self.inner.all_mappings();
        let mut out = Vec::with_capacity(mappings.len() * 6);

        for m in mappings {
            out.push(m.generated_line as i32);
            out.push(m.generated_column as i32);
            out.push(if m.source == u32::MAX {
                -1
            } else {
                m.source as i32
            });
            out.push(m.original_line as i32);
            out.push(m.original_column as i32);
            out.push(if m.name == u32::MAX {
                -1
            } else {
                m.name as i32
            });
        }

        out
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
