use napi_derive::napi;

#[napi(object)]
pub struct OriginalPosition {
    pub source: Option<String>,
    pub line: u32,
    pub column: u32,
    pub name: Option<String>,
}

#[napi(object)]
pub struct GeneratedPosition {
    pub line: u32,
    pub column: u32,
}

#[napi(js_name = "SourceMap")]
pub struct JsSourceMap {
    inner: srcmap_sourcemap::SourceMap,
}

#[napi]
impl JsSourceMap {
    #[napi(constructor)]
    pub fn new(json: String) -> napi::Result<Self> {
        let inner = srcmap_sourcemap::SourceMap::from_json(&json)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Look up the original source position for a generated position.
    /// Both line and column are 0-based.
    #[napi]
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalPosition> {
        self.inner
            .original_position_for(line, column)
            .map(|loc| OriginalPosition {
                source: Some(self.inner.source(loc.source).to_string()),
                line: loc.line,
                column: loc.column,
                name: loc.name.map(|i| self.inner.name(i).to_string()),
            })
    }

    /// Look up the original source position with a search bias.
    /// bias: 0 = GREATEST_LOWER_BOUND (default), -1 = LEAST_UPPER_BOUND
    #[napi]
    pub fn original_position_for_with_bias(
        &self,
        line: u32,
        column: u32,
        bias: i32,
    ) -> Option<OriginalPosition> {
        let b = if bias == -1 {
            srcmap_sourcemap::Bias::LeastUpperBound
        } else {
            srcmap_sourcemap::Bias::GreatestLowerBound
        };
        self.inner
            .original_position_for_with_bias(line, column, b)
            .map(|loc| OriginalPosition {
                source: Some(self.inner.source(loc.source).to_string()),
                line: loc.line,
                column: loc.column,
                name: loc.name.map(|i| self.inner.name(i).to_string()),
            })
    }

    /// Look up the generated position for an original source position.
    /// Both line and column are 0-based.
    #[napi]
    pub fn generated_position_for(
        &self,
        source: String,
        line: u32,
        column: u32,
    ) -> Option<GeneratedPosition> {
        self.inner
            .generated_position_for(&source, line, column)
            .map(|loc| GeneratedPosition {
                line: loc.line,
                column: loc.column,
            })
    }

    /// Look up the generated position with a search bias.
    /// bias: 0 = default, -1 = LEAST_UPPER_BOUND, 1 = GREATEST_LOWER_BOUND
    #[napi]
    pub fn generated_position_for_with_bias(
        &self,
        source: String,
        line: u32,
        column: u32,
        bias: i32,
    ) -> Option<GeneratedPosition> {
        let b = if bias == 1 {
            srcmap_sourcemap::Bias::GreatestLowerBound
        } else {
            srcmap_sourcemap::Bias::LeastUpperBound
        };
        self.inner
            .generated_position_for_with_bias(&source, line, column, b)
            .map(|loc| GeneratedPosition {
                line: loc.line,
                column: loc.column,
            })
    }

    #[napi(getter)]
    pub fn sources(&self) -> Vec<String> {
        self.inner.sources.clone()
    }

    #[napi(getter)]
    pub fn names(&self) -> Vec<String> {
        self.inner.names.clone()
    }

    /// Batch lookup: find original positions for multiple generated positions.
    /// Takes a flat array [line0, col0, line1, col1, ...].
    /// Returns a flat array [srcIdx0, line0, col0, nameIdx0, srcIdx1, ...].
    /// -1 means no mapping found / no name.
    #[napi]
    pub fn original_positions_for(&self, positions: Vec<i32>) -> Vec<i32> {
        let count = positions.len() / 2;
        let mut results = Vec::with_capacity(count * 4);

        for i in 0..count {
            let line = positions[i * 2] as u32;
            let column = positions[i * 2 + 1] as u32;

            match self.inner.original_position_for(line, column) {
                Some(loc) => {
                    results.push(loc.source as i32);
                    results.push(loc.line as i32);
                    results.push(loc.column as i32);
                    results.push(loc.name.map_or(-1, |n| n as i32));
                }
                None => {
                    results.push(-1);
                    results.push(-1);
                    results.push(-1);
                    results.push(-1);
                }
            }
        }

        results
    }

    #[napi(getter)]
    pub fn debug_id(&self) -> Option<String> {
        self.inner.debug_id.clone()
    }

    #[napi(getter)]
    pub fn mapping_count(&self) -> u32 {
        self.inner.mapping_count() as u32
    }

    #[napi(getter)]
    pub fn line_count(&self) -> u32 {
        self.inner.line_count() as u32
    }

    #[napi(getter)]
    pub fn has_range_mappings(&self) -> bool {
        self.inner.has_range_mappings()
    }

    #[napi(getter)]
    pub fn range_mapping_count(&self) -> u32 {
        self.inner.range_mapping_count() as u32
    }

    #[napi]
    pub fn encoded_range_mappings(&self) -> Option<String> {
        self.inner.encode_range_mappings()
    }
}
