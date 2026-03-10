use napi_derive::napi;

#[napi(js_name = "SourceMapGenerator")]
pub struct JsSourceMapGenerator {
    inner: srcmap_generator::SourceMapGenerator,
}

#[napi]
impl JsSourceMapGenerator {
    /// Create a new source map generator.
    #[napi(constructor)]
    pub fn new(file: Option<String>) -> Self {
        Self {
            inner: srcmap_generator::SourceMapGenerator::new(file),
        }
    }

    /// Register a source file and return its index.
    #[napi]
    pub fn add_source(&mut self, source: String) -> u32 {
        self.inner.add_source(&source)
    }

    /// Register a name and return its index.
    #[napi]
    pub fn add_name(&mut self, name: String) -> u32 {
        self.inner.add_name(&name)
    }

    /// Set the source root prefix.
    #[napi]
    pub fn set_source_root(&mut self, root: String) {
        self.inner.set_source_root(root);
    }

    /// Set the debug ID (UUID) for this source map (ECMA-426).
    #[napi]
    pub fn set_debug_id(&mut self, id: String) {
        self.inner.set_debug_id(id);
    }

    /// Set the content for a source file by index.
    #[napi]
    pub fn set_source_content(&mut self, source_idx: u32, content: String) {
        self.inner.set_source_content(source_idx, content);
    }

    /// Add a source index to the ignore list.
    #[napi]
    pub fn add_to_ignore_list(&mut self, source_idx: u32) {
        self.inner.add_to_ignore_list(source_idx);
    }

    /// Add a mapping with no source information (generated-only).
    #[napi]
    pub fn add_generated_mapping(&mut self, generated_line: u32, generated_column: u32) {
        self.inner
            .add_generated_mapping(generated_line, generated_column);
    }

    /// Add a mapping from generated position to original position.
    #[napi]
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
    #[napi]
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
    #[napi]
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
    #[napi]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }

    /// Get the number of mappings.
    #[napi(getter)]
    pub fn mapping_count(&self) -> u32 {
        self.inner.mapping_count() as u32
    }
}
