use serde::Deserialize;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(typescript_custom_section)]
const TYPES: &str = r#"
/** A 0-based position. */
export interface Position {
  line: number;
  column: number;
}

/** An original scope of a source. */
export interface OriginalScope {
  start: Position;
  end: Position;
  name: string | null;
  kind: string | null;
  isStackFrame: boolean;
  /** The empty string is a variable with no known name. */
  variables: string[];
  children: OriginalScope[];
}

/** A reference from a generated range to an original scope. */
export interface ScopeDefinition {
  /** Index of the source in `sources`. */
  sourceIndex: number;
  /** Index of the scope in the pre-order list of the scopes of that source. */
  scopeIndex: number;
}

export interface CallSite {
  sourceIndex: number;
  line: number;
  column: number;
}

export interface SubRangeBinding {
  /** `null` is a variable that is not available. */
  expression: string | null;
  from: Position;
}

export type Binding =
  | { type: "expression"; expression: string }
  | { type: "unavailable" }
  | { type: "subRanges"; subRanges: SubRangeBinding[] };

export interface GeneratedRange {
  start: Position;
  end: Position;
  isStackFrame: boolean;
  isHidden: boolean;
  definition: ScopeDefinition | null;
  callSite: CallSite | null;
  bindings: Binding[];
  children: GeneratedRange[];
}

export interface ScopeInfo {
  /** The root scopes of each source. An empty list is a source without scopes. */
  scopes: OriginalScope[][];
  ranges: GeneratedRange[];
}

/** The `scopes` and `ranges` fields of a source map, and the `names` array. */
export interface EncodedScopes {
  /** One entry for each source. `null` is a source without scopes. */
  scopes: (string | null)[];
  ranges: string[];
  names: string[];
}
"#;

fn js_strings(values: &[JsValue]) -> Vec<String> {
    values.iter().map(|v| v.as_string().unwrap_or_default()).collect()
}

fn scope_info_to_js(info: &srcmap_scopes::ScopeInfo) -> JsValue {
    let result = js_sys::Object::new();

    let scopes_arr = js_sys::Array::new();
    for roots in &info.scopes {
        let roots_arr = js_sys::Array::new();
        for scope in roots {
            roots_arr.push(&encode_original_scope(scope));
        }
        scopes_arr.push(&roots_arr);
    }
    js_sys::Reflect::set(&result, &"scopes".into(), &scopes_arr).unwrap_or(false);

    let ranges_arr = js_sys::Array::new();
    for range in &info.ranges {
        ranges_arr.push(&encode_generated_range(range));
    }
    js_sys::Reflect::set(&result, &"ranges".into(), &ranges_arr).unwrap_or(false);

    result.into()
}

/// Decode the `scopes` and `ranges` fields of a source map into structured
/// scope information. The fields have the layout of the ECMA-426 draft
/// specification.
///
/// `scopes` is the `scopes` array of the source map: one string for each
/// source, or `null` for a source without scopes.
/// `ranges` is the `ranges` array of the source map.
/// `names` is the `names` array of the source map.
/// `num_sources` is the number of source files in the source map.
///
/// Returns a JS object with `scopes` and `ranges` arrays.
#[wasm_bindgen(js_name = "decodeScopes", unchecked_return_type = "ScopeInfo")]
pub fn decode_scopes(
    #[wasm_bindgen(unchecked_param_type = "(string | null)[]")] scopes: Vec<JsValue>,
    #[wasm_bindgen(unchecked_param_type = "string[]")] ranges: Vec<JsValue>,
    #[wasm_bindgen(unchecked_param_type = "string[]")] names: Vec<JsValue>,
    num_sources: u32,
) -> Result<JsValue, JsError> {
    let scopes: Vec<Option<String>> = scopes.iter().map(JsValue::as_string).collect();

    let info = srcmap_scopes::decode_scopes(
        &scopes,
        &js_strings(&ranges),
        &js_strings(&names),
        num_sources as usize,
    )
    .map_err(|e| JsError::new(&e.to_string()))?;

    Ok(scope_info_to_js(&info))
}

/// Decode a `scopes` string of the older layout of the scopes proposal, which
/// has one string for the full map.
///
/// Returns the same JS object as `decodeScopes`.
#[wasm_bindgen(js_name = "decodeLegacyScopes", unchecked_return_type = "ScopeInfo")]
pub fn decode_legacy_scopes(
    scopes_str: &str,
    #[wasm_bindgen(unchecked_param_type = "string[]")] names: Vec<JsValue>,
    num_sources: u32,
) -> Result<JsValue, JsError> {
    let info =
        srcmap_scopes::decode_legacy_scopes(scopes_str, &js_strings(&names), num_sources as usize)
            .map_err(|e| JsError::new(&e.to_string()))?;

    Ok(scope_info_to_js(&info))
}

/// Encode structured scope information into the `scopes` and `ranges` fields
/// of a source map, in the layout of the ECMA-426 draft specification.
///
/// `scopes_json` is a JSON string of a `ScopeInfo` object.
/// `names` is the current names array (may be extended with new names).
///
/// Returns a JS object with `scopes` (one string or `null` for each source),
/// `ranges` (an array of strings) and `names` (the updated names array).
#[wasm_bindgen(js_name = "encodeScopes", unchecked_return_type = "EncodedScopes")]
pub fn encode_scopes(
    scopes_json: &str,
    #[wasm_bindgen(unchecked_param_type = "string[]")] names: Vec<JsValue>,
) -> Result<JsValue, JsError> {
    let info: ScopeInfoJson =
        serde_json::from_str(scopes_json).map_err(|e| JsError::new(&e.to_string()))?;

    let scope_info = info.into_scope_info();

    let mut names_vec = js_strings(&names);

    let encoded = srcmap_scopes::encode_scopes(&scope_info, &mut names_vec);

    let result = js_sys::Object::new();

    let scopes_arr = js_sys::Array::new();
    for entry in &encoded.scopes {
        scopes_arr.push(&entry.as_deref().map_or(JsValue::NULL, JsValue::from_str));
    }
    js_sys::Reflect::set(&result, &"scopes".into(), &scopes_arr).unwrap_or(false);

    let ranges_arr = js_sys::Array::new();
    for entry in &encoded.ranges {
        ranges_arr.push(&JsValue::from_str(entry));
    }
    js_sys::Reflect::set(&result, &"ranges".into(), &ranges_arr).unwrap_or(false);

    let names_arr = js_sys::Array::new();
    for name in &names_vec {
        names_arr.push(&JsValue::from_str(name));
    }
    js_sys::Reflect::set(&result, &"names".into(), &names_arr).unwrap_or(false);

    Ok(result.into())
}

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
        Some(d) => {
            let def_obj = js_sys::Object::new();
            js_sys::Reflect::set(&def_obj, &"sourceIndex".into(), &d.source_index.into())
                .unwrap_or(false);
            js_sys::Reflect::set(&def_obj, &"scopeIndex".into(), &d.scope_index.into())
                .unwrap_or(false);
            def_obj.into()
        }
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

#[derive(Deserialize)]
struct ScopeInfoJson {
    #[serde(default)]
    scopes: Vec<Option<Vec<OriginalScopeJson>>>,
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
    definition: Option<DefinitionJson>,
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
struct DefinitionJson {
    #[serde(rename = "sourceIndex")]
    source_index: u32,
    #[serde(rename = "scopeIndex")]
    scope_index: u32,
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

impl From<PositionJson> for srcmap_scopes::Position {
    fn from(p: PositionJson) -> Self {
        Self { line: p.line, column: p.column }
    }
}

impl ScopeInfoJson {
    fn into_scope_info(self) -> srcmap_scopes::ScopeInfo {
        srcmap_scopes::ScopeInfo {
            scopes: self
                .scopes
                .into_iter()
                // `null` is a source without scopes, as an empty list is.
                .map(|roots| roots.unwrap_or_default().into_iter().map(Into::into).collect())
                .collect(),
            ranges: self.ranges.into_iter().map(|r| r.into()).collect(),
        }
    }
}

impl From<OriginalScopeJson> for srcmap_scopes::OriginalScope {
    fn from(s: OriginalScopeJson) -> Self {
        Self {
            start: s.start.into(),
            end: s.end.into(),
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
            start: r.start.into(),
            end: r.end.into(),
            is_stack_frame: r.is_stack_frame,
            is_hidden: r.is_hidden,
            definition: r.definition.map(|d| srcmap_scopes::ScopeDefinition {
                source_index: d.source_index,
                scope_index: d.scope_index,
            }),
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
        Self { expression: s.expression, from: s.from.into() }
    }
}
