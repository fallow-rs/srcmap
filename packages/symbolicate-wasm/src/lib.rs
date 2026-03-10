use wasm_bindgen::prelude::*;

/// Parse a stack trace string into an array of frame objects.
/// Each frame has: {functionName, file, line, column}
#[wasm_bindgen(js_name = "parseStackTrace")]
pub fn parse_stack_trace(input: &str) -> Vec<JsValue> {
    srcmap_symbolicate::parse_stack_trace(input)
        .into_iter()
        .map(|frame| {
            let obj = js_sys::Object::new();
            let name_val: JsValue = match &frame.function_name {
                Some(n) => n.as_str().into(),
                None => JsValue::NULL,
            };
            js_sys::Reflect::set(&obj, &"functionName".into(), &name_val).unwrap_or(false);
            js_sys::Reflect::set(&obj, &"file".into(), &frame.file.as_str().into())
                .unwrap_or(false);
            js_sys::Reflect::set(&obj, &"line".into(), &frame.line.into()).unwrap_or(false);
            js_sys::Reflect::set(&obj, &"column".into(), &frame.column.into()).unwrap_or(false);
            obj.into()
        })
        .collect()
}

/// Symbolicate a stack trace using a JavaScript loader function.
///
/// The loader is called with each unique source file and should return
/// a source map JSON string, or null/undefined if not available.
///
/// Returns a JSON string with the symbolicated stack.
#[wasm_bindgen]
pub fn symbolicate(input: &str, loader: &js_sys::Function) -> String {
    let result = srcmap_symbolicate::symbolicate(input, |file| {
        let source_val = JsValue::from_str(file);
        let result = loader.call1(&JsValue::NULL, &source_val).ok()?;
        if result.is_null() || result.is_undefined() {
            return None;
        }
        let json_str = result.as_string()?;
        srcmap_sourcemap::SourceMap::from_json(&json_str).ok()
    });

    srcmap_symbolicate::to_json(&result)
}
