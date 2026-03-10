use wasm_bindgen::prelude::*;

#[wasm_bindgen]
pub struct SourceMapGenerator {
    inner: srcmap_generator::SourceMapGenerator,
}

#[wasm_bindgen]
impl SourceMapGenerator {
    /// Create a new source map generator.
    #[wasm_bindgen(constructor)]
    pub fn new(file: Option<String>) -> Self {
        Self {
            inner: srcmap_generator::SourceMapGenerator::new(file),
        }
    }

    /// Register a source file and return its index.
    #[wasm_bindgen(js_name = "addSource")]
    pub fn add_source(&mut self, source: &str) -> u32 {
        self.inner.add_source(source)
    }

    /// Register a name and return its index.
    #[wasm_bindgen(js_name = "addName")]
    pub fn add_name(&mut self, name: &str) -> u32 {
        self.inner.add_name(name)
    }

    /// Set the source root prefix.
    #[wasm_bindgen(js_name = "setSourceRoot")]
    pub fn set_source_root(&mut self, root: &str) {
        self.inner.set_source_root(root.to_string());
    }

    /// Set the debug ID (UUID) for this source map (ECMA-426).
    #[wasm_bindgen(js_name = "setDebugId")]
    pub fn set_debug_id(&mut self, id: &str) {
        self.inner.set_debug_id(id.to_string());
    }

    /// Set the content for a source file by index.
    #[wasm_bindgen(js_name = "setSourceContent")]
    pub fn set_source_content(&mut self, source_idx: u32, content: &str) {
        self.inner
            .set_source_content(source_idx, content.to_string());
    }

    /// Add a source index to the ignore list.
    #[wasm_bindgen(js_name = "addToIgnoreList")]
    pub fn add_to_ignore_list(&mut self, source_idx: u32) {
        self.inner.add_to_ignore_list(source_idx);
    }

    /// Add a mapping with no source information (generated-only).
    #[wasm_bindgen(js_name = "addGeneratedMapping")]
    pub fn add_generated_mapping(&mut self, generated_line: u32, generated_column: u32) {
        self.inner
            .add_generated_mapping(generated_line, generated_column);
    }

    /// Add a mapping from generated position to original position.
    #[wasm_bindgen(js_name = "addMapping")]
    pub fn add_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
    ) {
        self.inner.add_mapping(
            generated_line,
            generated_column,
            source,
            original_line,
            original_column,
        );
    }

    /// Add a mapping with a name.
    #[wasm_bindgen(js_name = "addNamedMapping")]
    pub fn add_named_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
        name: u32,
    ) {
        self.inner.add_named_mapping(
            generated_line,
            generated_column,
            source,
            original_line,
            original_column,
            name,
        );
    }

    /// Add a mapping only if it differs from the previous mapping on the same line.
    /// Returns true if the mapping was added, false if skipped.
    #[wasm_bindgen(js_name = "maybeAddMapping")]
    pub fn maybe_add_mapping(
        &mut self,
        generated_line: u32,
        generated_column: u32,
        source: u32,
        original_line: u32,
        original_column: u32,
    ) -> bool {
        self.inner.maybe_add_mapping(
            generated_line,
            generated_column,
            source,
            original_line,
            original_column,
        )
    }

    /// Generate the source map as a JSON string.
    #[wasm_bindgen(js_name = "toJSON")]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Get the number of mappings.
    #[wasm_bindgen(getter, js_name = "mappingCount")]
    pub fn mapping_count(&self) -> u32 {
        self.inner.mapping_count() as u32
    }
}
