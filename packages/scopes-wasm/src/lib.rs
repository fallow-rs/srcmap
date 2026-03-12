use wasm_bindgen::prelude::*;

/// Decode a scopes string into structured scope information.
///
/// `scopes_str` is the VLQ-encoded scopes string from a source map.
/// `names` is the names array from the source map (as a JS string array).
/// `num_sources` is the number of source files in the source map.
///
/// Returns a JS object with `scopes` and `ranges` arrays.
#[wasm_bindgen(js_name = "decodeScopes")]
pub fn decode_scopes(
    scopes_str: &str,
    names: Vec<JsValue>,
    num_sources: u32,
) -> Result<JsValue, JsError> {
    let names_vec: Vec<String> = names
        .iter()
        .map(|v| v.as_string().unwrap_or_default())
        .collect();

    let info = srcmap_scopes::decode_scopes(scopes_str, &names_vec, num_sources as usize)
        .map_err(|e| JsError::new(&e.to_string()))?;

    let result = js_sys::Object::new();

    // Encode scopes array
    let scopes_arr = js_sys::Array::new();
    for scope in &info.scopes {
        match scope {
            Some(s) => {
                scopes_arr.push(&encode_original_scope(s));
            }
            None => {
                scopes_arr.push(&JsValue::NULL);
            }
        }
    }
    js_sys::Reflect::set(&result, &"scopes".into(), &scopes_arr).unwrap_or(false);

    // Encode ranges array
    let ranges_arr = js_sys::Array::new();
    for range in &info.ranges {
        ranges_arr.push(&encode_generated_range(range));
    }
    js_sys::Reflect::set(&result, &"ranges".into(), &ranges_arr).unwrap_or(false);

    Ok(result.into())
}

/// Encode structured scope information into a VLQ-encoded scopes string.
///
/// `scopes_json` is a JSON string representing the scope info structure.
/// `names` is the current names array (may be extended with new names).
///
/// Returns a JS object with `encoded` (the VLQ string) and `names` (updated names array).
#[wasm_bindgen(js_name = "encodeScopes")]
pub fn encode_scopes(scopes_json: &str, names: Vec<JsValue>) -> Result<JsValue, JsError> {
    let info: ScopeInfoJson =
        serde_json::from_str(scopes_json).map_err(|e| JsError::new(&e.to_string()))?;

    let scope_info = info.into_scope_info();

    let mut names_vec: Vec<String> = names
        .iter()
        .map(|v| v.as_string().unwrap_or_default())
        .collect();

    let encoded = srcmap_scopes::encode_scopes(&scope_info, &mut names_vec);

    let result = js_sys::Object::new();
    js_sys::Reflect::set(&result, &"encoded".into(), &encoded.into()).unwrap_or(false);

    let names_arr = js_sys::Array::new();
    for name in &names_vec {
        names_arr.push(&JsValue::from_str(name));
    }
    js_sys::Reflect::set(&result, &"names".into(), &names_arr).unwrap_or(false);

    Ok(result.into())
}

// ── Helper: encode scope types to JS objects ──────────────────────

fn encode_original_scope(scope: &srcmap_scopes::OriginalScope) -> JsValue {
    let obj = js_sys::Object::new();

    let start = js_sys::Object::new();
    js_sys::Reflect::set(&start, &"line".into(), &scope.start.line.into()).unwrap_or(false);
    js_sys::Reflect::set(&start, &"column".into(), &scope.start.column.into()).unwrap_or(false);
    js_sys::Reflect::set(&obj, &"start".into(), &start).unwrap_or(false);

    let end = js_sys::Object::new();
    js_sys::Reflect::set(&end, &"line".into(), &scope.end.line.into()).unwrap_or(false);
    js_sys::Reflect::set(&end, &"column".into(), &scope.end.column.into()).unwrap_or(false);
    js_sys::Reflect::set(&obj, &"end".into(), &end).unwrap_or(false);

    let name_val: JsValue = match &scope.name {
        Some(n) => n.as_str().into(),
        None => JsValue::NULL,
    };
    js_sys::Reflect::set(&obj, &"name".into(), &name_val).unwrap_or(false);

    let kind_val: JsValue = match &scope.kind {
        Some(k) => k.as_str().into(),
        None => JsValue::NULL,
    };
    js_sys::Reflect::set(&obj, &"kind".into(), &kind_val).unwrap_or(false);

    js_sys::Reflect::set(&obj, &"isStackFrame".into(), &scope.is_stack_frame.into())
        .unwrap_or(false);

    let vars = js_sys::Array::new();
    for v in &scope.variables {
        vars.push(&JsValue::from_str(v));
    }
    js_sys::Reflect::set(&obj, &"variables".into(), &vars).unwrap_or(false);

    let children = js_sys::Array::new();
    for child in &scope.children {
        children.push(&encode_original_scope(child));
    }
    js_sys::Reflect::set(&obj, &"children".into(), &children).unwrap_or(false);

    obj.into()
}

fn encode_generated_range(range: &srcmap_scopes::GeneratedRange) -> JsValue {
    let obj = js_sys::Object::new();

    let start = js_sys::Object::new();
    js_sys::Reflect::set(&start, &"line".into(), &range.start.line.into()).unwrap_or(false);
    js_sys::Reflect::set(&start, &"column".into(), &range.start.column.into()).unwrap_or(false);
    js_sys::Reflect::set(&obj, &"start".into(), &start).unwrap_or(false);

    let end = js_sys::Object::new();
    js_sys::Reflect::set(&end, &"line".into(), &range.end.line.into()).unwrap_or(false);
    js_sys::Reflect::set(&end, &"column".into(), &range.end.column.into()).unwrap_or(false);
    js_sys::Reflect::set(&obj, &"end".into(), &end).unwrap_or(false);

    js_sys::Reflect::set(&obj, &"isStackFrame".into(), &range.is_stack_frame.into())
        .unwrap_or(false);
    js_sys::Reflect::set(&obj, &"isHidden".into(), &range.is_hidden.into()).unwrap_or(false);

    let def_val: JsValue = match range.definition {
        Some(d) => d.into(),
        None => JsValue::NULL,
    };
    js_sys::Reflect::set(&obj, &"definition".into(), &def_val).unwrap_or(false);

    let cs_val: JsValue = match &range.call_site {
        Some(cs) => {
            let cs_obj = js_sys::Object::new();
            js_sys::Reflect::set(&cs_obj, &"sourceIndex".into(), &cs.source_index.into())
                .unwrap_or(false);
            js_sys::Reflect::set(&cs_obj, &"line".into(), &cs.line.into()).unwrap_or(false);
            js_sys::Reflect::set(&cs_obj, &"column".into(), &cs.column.into()).unwrap_or(false);
            cs_obj.into()
        }
        None => JsValue::NULL,
    };
    js_sys::Reflect::set(&obj, &"callSite".into(), &cs_val).unwrap_or(false);

    let bindings = js_sys::Array::new();
    for binding in &range.bindings {
        bindings.push(&encode_binding(binding));
    }
    js_sys::Reflect::set(&obj, &"bindings".into(), &bindings).unwrap_or(false);

    let children = js_sys::Array::new();
    for child in &range.children {
        children.push(&encode_generated_range(child));
    }
    js_sys::Reflect::set(&obj, &"children".into(), &children).unwrap_or(false);

    obj.into()
}

fn encode_binding(binding: &srcmap_scopes::Binding) -> JsValue {
    let obj = js_sys::Object::new();
    match binding {
        srcmap_scopes::Binding::Expression(expr) => {
            js_sys::Reflect::set(&obj, &"type".into(), &"expression".into()).unwrap_or(false);
            js_sys::Reflect::set(&obj, &"expression".into(), &expr.as_str().into())
                .unwrap_or(false);
        }
        srcmap_scopes::Binding::Unavailable => {
            js_sys::Reflect::set(&obj, &"type".into(), &"unavailable".into()).unwrap_or(false);
        }
        srcmap_scopes::Binding::SubRanges(subs) => {
            js_sys::Reflect::set(&obj, &"type".into(), &"subRanges".into()).unwrap_or(false);
            let arr = js_sys::Array::new();
            for sub in subs {
                let sub_obj = js_sys::Object::new();
                let expr_val: JsValue = match &sub.expression {
                    Some(e) => e.as_str().into(),
                    None => JsValue::NULL,
                };
                js_sys::Reflect::set(&sub_obj, &"expression".into(), &expr_val).unwrap_or(false);
                let from = js_sys::Object::new();
                js_sys::Reflect::set(&from, &"line".into(), &sub.from.line.into()).unwrap_or(false);
                js_sys::Reflect::set(&from, &"column".into(), &sub.from.column.into())
                    .unwrap_or(false);
                js_sys::Reflect::set(&sub_obj, &"from".into(), &from).unwrap_or(false);
                arr.push(&sub_obj);
            }
            js_sys::Reflect::set(&obj, &"subRanges".into(), &arr).unwrap_or(false);
        }
    }
    obj.into()
}

// ── JSON deserialization for encode input ──────────────────────────

use serde::Deserialize;

#[derive(Deserialize)]
struct ScopeInfoJson {
    #[serde(default)]
    scopes: Vec<Option<OriginalScopeJson>>,
    #[serde(default)]
    ranges: Vec<GeneratedRangeJson>,
}

#[derive(Deserialize)]
struct OriginalScopeJson {
    start: PositionJson,
    end: PositionJson,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, rename = "isStackFrame")]
    is_stack_frame: bool,
    #[serde(default)]
    variables: Vec<String>,
    #[serde(default)]
    children: Vec<OriginalScopeJson>,
}

#[derive(Deserialize)]
struct GeneratedRangeJson {
    start: PositionJson,
    end: PositionJson,
    #[serde(default, rename = "isStackFrame")]
    is_stack_frame: bool,
    #[serde(default, rename = "isHidden")]
    is_hidden: bool,
    #[serde(default)]
    definition: Option<u32>,
    #[serde(default, rename = "callSite")]
    call_site: Option<CallSiteJson>,
    #[serde(default)]
    bindings: Vec<BindingJson>,
    #[serde(default)]
    children: Vec<GeneratedRangeJson>,
}

#[derive(Deserialize)]
struct PositionJson {
    line: u32,
    column: u32,
}

#[derive(Deserialize)]
struct CallSiteJson {
    #[serde(rename = "sourceIndex")]
    source_index: u32,
    line: u32,
    column: u32,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum BindingJson {
    #[serde(rename = "expression")]
    Expression { expression: String },
    #[serde(rename = "unavailable")]
    Unavailable,
    #[serde(rename = "subRanges")]
    SubRanges {
        #[serde(rename = "subRanges")]
        sub_ranges: Vec<SubRangeJson>,
    },
}

#[derive(Deserialize)]
struct SubRangeJson {
    expression: Option<String>,
    from: PositionJson,
}

impl ScopeInfoJson {
    fn into_scope_info(self) -> srcmap_scopes::ScopeInfo {
        srcmap_scopes::ScopeInfo {
            scopes: self
                .scopes
                .into_iter()
                .map(|s| s.map(|s| s.into()))
                .collect(),
            ranges: self.ranges.into_iter().map(|r| r.into()).collect(),
        }
    }
}

impl From<OriginalScopeJson> for srcmap_scopes::OriginalScope {
    fn from(s: OriginalScopeJson) -> Self {
        Self {
            start: srcmap_scopes::Position {
                line: s.start.line,
                column: s.start.column,
            },
            end: srcmap_scopes::Position {
                line: s.end.line,
                column: s.end.column,
            },
            name: s.name,
            kind: s.kind,
            is_stack_frame: s.is_stack_frame,
            variables: s.variables,
            children: s.children.into_iter().map(|c| c.into()).collect(),
        }
    }
}

impl From<GeneratedRangeJson> for srcmap_scopes::GeneratedRange {
    fn from(r: GeneratedRangeJson) -> Self {
        Self {
            start: srcmap_scopes::Position {
                line: r.start.line,
                column: r.start.column,
            },
            end: srcmap_scopes::Position {
                line: r.end.line,
                column: r.end.column,
            },
            is_stack_frame: r.is_stack_frame,
            is_hidden: r.is_hidden,
            definition: r.definition,
            call_site: r.call_site.map(|cs| srcmap_scopes::CallSite {
                source_index: cs.source_index,
                line: cs.line,
                column: cs.column,
            }),
            bindings: r.bindings.into_iter().map(|b| b.into()).collect(),
            children: r.children.into_iter().map(|c| c.into()).collect(),
        }
    }
}

impl From<BindingJson> for srcmap_scopes::Binding {
    fn from(b: BindingJson) -> Self {
        match b {
            BindingJson::Expression { expression } => {
                srcmap_scopes::Binding::Expression(expression)
            }
            BindingJson::Unavailable => srcmap_scopes::Binding::Unavailable,
            BindingJson::SubRanges { sub_ranges } => srcmap_scopes::Binding::SubRanges(
                sub_ranges.into_iter().map(|s| s.into()).collect(),
            ),
        }
    }
}

impl From<SubRangeJson> for srcmap_scopes::SubRangeBinding {
    fn from(s: SubRangeJson) -> Self {
        Self {
            expression: s.expression,
            from: srcmap_scopes::Position {
                line: s.from.line,
                column: s.from.column,
            },
        }
    }
}
