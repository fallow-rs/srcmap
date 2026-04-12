use napi_derive::napi;

// ── ConcatBuilder ────────────────────────────────────────────────

#[napi(js_name = "ConcatBuilder")]
pub struct JsConcatBuilder {
    inner: srcmap_remapping::ConcatBuilder,
}

#[napi]
impl JsConcatBuilder {
    /// Create a new concatenation builder.
    #[napi(constructor)]
    pub fn new(file: Option<String>) -> Self {
        Self { inner: srcmap_remapping::ConcatBuilder::new(file) }
    }

    /// Add a source map (as JSON string) at the given line offset.
    #[napi]
    pub fn add_map(&mut self, json: String, line_offset: u32) -> napi::Result<()> {
        let sm = srcmap_sourcemap::SourceMap::from_json(&json)
            .map_err(|e| napi::Error::from_reason(e.to_string()))?;
        self.inner.add_map(&sm, line_offset);
        Ok(())
    }

    /// Finish and return the concatenated source map as a JSON string.
    #[napi]
    pub fn to_json(&self) -> String {
        self.inner.to_json()
    }
}

// ── Remap ────────────────────────────────────────────────────────

/// Compose/remap source maps through a transform chain.
///
/// `outerJson` is the final-stage source map as a JSON string.
/// `loaderMap` is a plain JS object mapping source filenames to their
/// upstream source map JSON strings. Sources not in the object are kept as-is.
///
/// Returns the remapped source map as a JSON string.
#[napi]
pub fn remap(outer_json: String, loader_map: napi::JsObject) -> napi::Result<String> {
    let outer = srcmap_sourcemap::SourceMap::from_json(&outer_json)
        .map_err(|e| napi::Error::from_reason(e.to_string()))?;

    // Pre-load all upstream source maps from the JS object
    let mut upstream_maps: std::collections::HashMap<String, Option<srcmap_sourcemap::SourceMap>> =
        std::collections::HashMap::new();

    for source in &outer.sources {
        if !upstream_maps.contains_key(source) {
            let val: Option<napi::JsUnknown> = loader_map.get_named_property(source).ok();
            let upstream = val.and_then(|v| {
                let val_type = v.get_type().ok()?;
                if val_type == napi::ValueType::Null || val_type == napi::ValueType::Undefined {
                    return None;
                }
                let json_str: String =
                    v.coerce_to_string().ok()?.into_utf8().ok()?.into_owned().ok()?;
                srcmap_sourcemap::SourceMap::from_json(&json_str).ok()
            });
            upstream_maps.insert(source.clone(), upstream);
        }
    }

    let result =
        srcmap_remapping::remap(&outer, |source| upstream_maps.get(source).and_then(|v| v.clone()));

    Ok(result.to_json())
}
