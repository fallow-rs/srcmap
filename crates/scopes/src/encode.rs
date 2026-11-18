//! Encoder for the ECMA-426 scopes proposal.
//!
//! The encoder writes the layout of the draft specification: one `scopes`
//! string for each source, and a list of `ranges` strings.

use std::collections::HashMap;

use srcmap_codec::{vlq_encode, vlq_encode_unsigned};

use crate::{
    Binding, EncodedScopes, GeneratedRange, OriginalScope, Position, ScopeInfo,
    TAG_GENERATED_RANGE_BINDINGS, TAG_GENERATED_RANGE_CALL_SITE, TAG_GENERATED_RANGE_END,
    TAG_GENERATED_RANGE_START, TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS, TAG_ORIGINAL_SCOPE_END,
    TAG_ORIGINAL_SCOPE_START, TAG_ORIGINAL_SCOPE_VARIABLES,
};

// ── Encoder ──────────────────────────────────────────────────────

struct ScopesEncoder<'a> {
    output: Vec<u8>,
    names: &'a mut Vec<String>,
    name_map: HashMap<String, u32>,

    // Original scope relative state
    os_pos: Position,
    os_name: i64,
    os_kind: i64,
    os_var: i64,

    // Generated range relative state
    gr_pos: Position,
    gr_def_source: i64,
    gr_def_scope: i64,
}

impl<'a> ScopesEncoder<'a> {
    fn new(names: &'a mut Vec<String>) -> Self {
        let name_map: HashMap<String, u32> =
            names.iter().enumerate().map(|(i, n)| (n.clone(), i as u32)).collect();

        Self {
            output: Vec::with_capacity(256),
            names,
            name_map,
            os_pos: Position { line: 0, column: 0 },
            os_name: 0,
            os_kind: 0,
            os_var: 0,
            gr_pos: Position { line: 0, column: 0 },
            gr_def_source: 0,
            gr_def_scope: 0,
        }
    }

    /// Start an item: write the separator and the tag.
    #[inline]
    fn start_item(&mut self, tag: u64) {
        if !self.output.is_empty() {
            self.output.push(b',');
        }
        self.emit_unsigned(tag);
    }

    #[inline]
    fn emit_unsigned(&mut self, value: u64) {
        vlq_encode_unsigned(&mut self.output, value);
    }

    #[inline]
    fn emit_signed(&mut self, value: i64) {
        vlq_encode(&mut self.output, value);
    }

    /// Write a relative line and its column, and return the line increment.
    ///
    /// The column is relative when the line stays the same, and absolute when
    /// the line changes. `optional_line` leaves out a line increment of zero.
    fn emit_position(&mut self, state: Position, pos: Position, optional_line: bool) {
        // The wire format has no negative increment. A position before `state`
        // gets the increment zero.
        let line_delta = pos.line.saturating_sub(state.line);
        if line_delta != 0 || !optional_line {
            self.emit_unsigned(u64::from(line_delta));
        }
        let column =
            if line_delta != 0 { pos.column } else { pos.column.saturating_sub(state.column) };
        self.emit_unsigned(u64::from(column));
    }

    /// Look up or append a name, returning its 0-based index.
    fn name_idx(&mut self, name: &str) -> u32 {
        if let Some(&idx) = self.name_map.get(name) {
            return idx;
        }
        let idx = self.names.len() as u32;
        self.names.push(name.to_string());
        self.name_map.insert(name.to_string(), idx);
        idx
    }

    /// Write a binding expression: 0 for a variable that is not available, or
    /// the index of the expression in `names` plus 1.
    fn emit_binding(&mut self, expression: Option<&str>) {
        let value = expression.map_or(0, |expr| u64::from(self.name_idx(expr)) + 1);
        self.emit_unsigned(value);
    }

    /// Take the string that the encoder wrote since the last call.
    fn take_output(&mut self) -> String {
        let output = std::mem::take(&mut self.output);
        debug_assert!(output.is_ascii());
        // SAFETY: vlq_encode and vlq_encode_unsigned push only bytes from the
        // base64 alphabet, and `start_item` pushes only b','. All are ASCII.
        unsafe { String::from_utf8_unchecked(output) }
    }

    fn encode(mut self, info: &ScopeInfo) -> EncodedScopes {
        let scopes = info
            .scopes
            .iter()
            .map(|roots| {
                if roots.is_empty() {
                    return None;
                }
                // Each source starts with new relative state.
                self.os_pos = Position { line: 0, column: 0 };
                self.os_name = 0;
                self.os_kind = 0;
                self.os_var = 0;
                for root in roots {
                    self.encode_original_scope(root);
                }
                Some(self.take_output())
            })
            .collect();

        let mut ranges = Vec::new();
        for range in &info.ranges {
            // A string cannot go back to an earlier position. A range that
            // starts before the end of the range before it starts a new string.
            if range.start < self.gr_pos {
                ranges.push(self.take_output());
                self.gr_pos = Position { line: 0, column: 0 };
                self.gr_def_source = 0;
                self.gr_def_scope = 0;
            }
            self.encode_generated_range(range);
        }
        if !info.ranges.is_empty() {
            ranges.push(self.take_output());
        }

        EncodedScopes { scopes, ranges }
    }

    fn encode_original_scope(&mut self, scope: &OriginalScope) {
        // B item: scope start
        self.start_item(TAG_ORIGINAL_SCOPE_START);

        let mut flags: u64 = 0;
        if scope.name.is_some() {
            flags |= crate::OS_FLAG_HAS_NAME;
        }
        if scope.kind.is_some() {
            flags |= crate::OS_FLAG_HAS_KIND;
        }
        if scope.is_stack_frame {
            flags |= crate::OS_FLAG_IS_STACK_FRAME;
        }
        self.emit_unsigned(flags);

        self.emit_position(self.os_pos, scope.start, false);
        self.os_pos = scope.start;

        // Name (signed relative)
        if let Some(ref name) = scope.name {
            let idx = self.name_idx(name) as i64;
            self.emit_signed(idx - self.os_name);
            self.os_name = idx;
        }

        // Kind (signed relative)
        if let Some(ref kind) = scope.kind {
            let idx = self.name_idx(kind) as i64;
            self.emit_signed(idx - self.os_kind);
            self.os_kind = idx;
        }

        // D item: variables
        if !scope.variables.is_empty() {
            self.start_item(TAG_ORIGINAL_SCOPE_VARIABLES);
            for var in &scope.variables {
                let idx = self.name_idx(var) as i64;
                self.emit_signed(idx - self.os_var);
                self.os_var = idx;
            }
        }

        for child in &scope.children {
            self.encode_original_scope(child);
        }

        // C item: scope end
        self.start_item(TAG_ORIGINAL_SCOPE_END);
        self.emit_position(self.os_pos, scope.end, false);
        self.os_pos = scope.end;
    }

    /// Write a generated range. The order of the items is the order of the
    /// grammar: start, bindings, call site, sub-range bindings, children, end.
    fn encode_generated_range(&mut self, range: &GeneratedRange) {
        self.encode_generated_range_start(range);
        self.encode_generated_range_bindings(range);

        // I item: call site. The encoder writes the absolute form. The J item
        // (a call site relative to a parent range) is smaller, but not each
        // decoder reads it.
        if let Some(ref cs) = range.call_site {
            self.start_item(TAG_GENERATED_RANGE_CALL_SITE);
            self.emit_unsigned(u64::from(cs.source_index));
            self.emit_unsigned(u64::from(cs.line));
            self.emit_unsigned(u64::from(cs.column));
        }

        self.encode_generated_range_sub_range_bindings(range);

        for child in &range.children {
            self.encode_generated_range(child);
        }

        // F item: range end
        self.start_item(TAG_GENERATED_RANGE_END);
        self.emit_position(self.gr_pos, range.end, true);
        self.gr_pos = range.end;
    }

    fn encode_generated_range_start(&mut self, range: &GeneratedRange) {
        self.start_item(TAG_GENERATED_RANGE_START);

        let mut flags: u64 = 0;
        if range.start.line != self.gr_pos.line {
            flags |= crate::GR_FLAG_HAS_LINE;
        }
        if range.definition.is_some() {
            flags |= crate::GR_FLAG_HAS_DEFINITION;
        }
        if range.is_stack_frame {
            flags |= crate::GR_FLAG_IS_STACK_FRAME;
            // The hidden flag is valid only on a range that is a stack frame.
            if range.is_hidden {
                flags |= crate::GR_FLAG_IS_HIDDEN;
            }
        }
        self.emit_unsigned(flags);

        self.emit_position(self.gr_pos, range.start, true);
        self.gr_pos = range.start;

        // The scope index is relative when the source stays the same, and
        // absolute when the source changes.
        if let Some(definition) = range.definition {
            let source = i64::from(definition.source_index);
            let scope = i64::from(definition.scope_index);
            let source_delta = source - self.gr_def_source;
            self.emit_signed(source_delta);
            if source_delta == 0 {
                self.emit_signed(scope - self.gr_def_scope);
            } else {
                self.emit_unsigned(u64::from(definition.scope_index));
            }
            self.gr_def_source = source;
            self.gr_def_scope = scope;
        }
    }

    fn encode_generated_range_bindings(&mut self, range: &GeneratedRange) {
        if range.bindings.is_empty() {
            return;
        }

        self.start_item(TAG_GENERATED_RANGE_BINDINGS);
        for binding in &range.bindings {
            let expression = match binding {
                Binding::Expression(expr) => Some(expr.as_str()),
                Binding::Unavailable => None,
                // The G item gets the binding of the first sub-range.
                Binding::SubRanges(subs) => subs.first().and_then(|sub| sub.expression.as_deref()),
            };
            self.emit_binding(expression);
        }
    }

    fn encode_generated_range_sub_range_bindings(&mut self, range: &GeneratedRange) {
        for (i, binding) in range.bindings.iter().enumerate() {
            let Binding::SubRanges(subs) = binding else {
                continue;
            };
            if subs.len() <= 1 {
                continue;
            }

            // H item: the index of the variable, then one (line, column,
            // expression) group for each sub-range after the first one.
            // The positions are relative to the start of the range.
            let mut from = range.start;
            for (group, sub) in subs[1..].iter().enumerate() {
                // An item cannot go back to an earlier position. A sub-range
                // before the one before it starts a new item for the variable.
                if group == 0 || sub.from < from {
                    self.start_item(TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS);
                    self.emit_unsigned(i as u64);
                    from = range.start;
                }
                self.emit_position(from, sub.from, false);
                from = sub.from;
                self.emit_binding(sub.expression.as_deref());
            }
        }
    }
}

/// Encode scope information into the `scopes` and `ranges` fields of a source
/// map, in the layout of the ECMA-426 draft specification.
///
/// The function adds each name that is not in `names` to the end of `names`.
///
/// A source without scopes gets `None`, which is `null` in the JSON. All
/// generated ranges go into one `ranges` string when they are in the order of
/// the generated code. A top-level range that starts before the end of the
/// range before it starts a new string.
pub fn encode_scopes(info: &ScopeInfo, names: &mut Vec<String>) -> EncodedScopes {
    ScopesEncoder::new(names).encode(info)
}

impl EncodedScopes {
    /// Append the `scopes` and `ranges` fields to a JSON object, each with a
    /// leading comma: `,"scopes":[...],"ranges":[...]`.
    ///
    /// The function writes each string as it is. The strings of
    /// [`encode_scopes`] have only base64 characters and commas, which need no
    /// JSON escape.
    pub fn write_json_fields(&self, out: &mut String) {
        out.push_str(r#","scopes":["#);
        for (i, entry) in self.scopes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            match entry {
                Some(encoded) => push_json_string(out, encoded),
                None => out.push_str("null"),
            }
        }
        out.push_str(r#"],"ranges":["#);
        for (i, encoded) in self.ranges.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            push_json_string(out, encoded);
        }
        out.push(']');
    }
}

fn push_json_string(out: &mut String, value: &str) {
    debug_assert!(value.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"+/,".contains(&byte)));
    out.push('"');
    out.push_str(value);
    out.push('"');
}
