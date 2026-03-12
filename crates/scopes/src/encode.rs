//! Encoder for the ECMA-426 scopes proposal.
//!
//! Encodes structured `ScopeInfo` into a VLQ-encoded `scopes` string.

use std::collections::HashMap;

use srcmap_codec::{vlq_encode, vlq_encode_unsigned};

use crate::{
    Binding, GeneratedRange, OriginalScope, ScopeInfo, TAG_GENERATED_RANGE_BINDINGS,
    TAG_GENERATED_RANGE_CALL_SITE, TAG_GENERATED_RANGE_END, TAG_GENERATED_RANGE_START,
    TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS, TAG_ORIGINAL_SCOPE_END, TAG_ORIGINAL_SCOPE_START,
    TAG_ORIGINAL_SCOPE_VARIABLES, resolve_or_add_name,
};

// ── Encoder ──────────────────────────────────────────────────────

struct ScopesEncoder<'a> {
    output: Vec<u8>,
    names: &'a mut Vec<String>,
    name_map: HashMap<String, u32>,
    first_item: bool,

    // Original scope relative state
    os_line: u32,
    os_col: u32,
    os_name: i64,
    os_kind: i64,
    os_var: i64,

    // Generated range relative state
    gr_line: u32,
    gr_col: u32,
    gr_def: i64,
}

impl<'a> ScopesEncoder<'a> {
    fn new(names: &'a mut Vec<String>) -> Self {
        let name_map: HashMap<String, u32> = names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.clone(), i as u32))
            .collect();

        Self {
            output: Vec::with_capacity(256),
            names,
            name_map,
            first_item: true,
            os_line: 0,
            os_col: 0,
            os_name: 0,
            os_kind: 0,
            os_var: 0,
            gr_line: 0,
            gr_col: 0,
            gr_def: 0,
        }
    }

    fn emit_comma(&mut self) {
        if !self.first_item {
            self.output.push(b',');
        }
        self.first_item = false;
    }

    fn emit_tag(&mut self, tag: u64) {
        vlq_encode_unsigned(&mut self.output, tag);
    }

    fn emit_unsigned(&mut self, value: u64) {
        vlq_encode_unsigned(&mut self.output, value);
    }

    fn emit_signed(&mut self, value: i64) {
        vlq_encode(&mut self.output, value);
    }

    fn name_idx(&mut self, name: &str) -> u32 {
        resolve_or_add_name(name, self.names, &mut self.name_map)
    }

    fn encode(mut self, info: &ScopeInfo) -> String {
        // Phase 1: Encode original scope trees
        for scope in &info.scopes {
            match scope {
                Some(s) => {
                    // Reset position state for new top-level tree
                    self.os_line = 0;
                    self.os_col = 0;
                    self.encode_original_scope(s);
                }
                None => {
                    // Empty item: just emit a comma separator
                    // The comma between items handles this
                    if !self.first_item {
                        self.output.push(b',');
                    }
                    self.first_item = false;
                    // We need to mark that this was an empty item
                    // The next item's comma will create the empty slot
                }
            }
        }

        // Phase 2: Encode generated ranges
        for range in &info.ranges {
            self.encode_generated_range(range);
        }

        // SAFETY: VLQ output is always valid ASCII/UTF-8
        unsafe { String::from_utf8_unchecked(self.output) }
    }

    fn encode_original_scope(&mut self, scope: &OriginalScope) {
        // B item: scope start
        self.emit_comma();
        self.emit_tag(TAG_ORIGINAL_SCOPE_START);

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

        // Line (relative)
        let line_delta = scope.start.line - self.os_line;
        self.emit_unsigned(line_delta as u64);
        self.os_line = scope.start.line;

        // Column (absolute if line changed, relative if same line)
        let col = if line_delta != 0 {
            scope.start.column
        } else {
            scope.start.column - self.os_col
        };
        self.emit_unsigned(col as u64);
        self.os_col = scope.start.column;

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
            self.emit_comma();
            self.emit_tag(TAG_ORIGINAL_SCOPE_VARIABLES);
            for var in &scope.variables {
                let idx = self.name_idx(var) as i64;
                self.emit_signed(idx - self.os_var);
                self.os_var = idx;
            }
        }

        // Recursively encode children
        for child in &scope.children {
            self.encode_original_scope(child);
        }

        // C item: scope end
        self.emit_comma();
        self.emit_tag(TAG_ORIGINAL_SCOPE_END);

        let line_delta = scope.end.line - self.os_line;
        self.emit_unsigned(line_delta as u64);
        self.os_line = scope.end.line;

        let col = if line_delta != 0 {
            scope.end.column
        } else {
            scope.end.column - self.os_col
        };
        self.emit_unsigned(col as u64);
        self.os_col = scope.end.column;
    }

    fn encode_generated_range(&mut self, range: &GeneratedRange) {
        // E item: range start
        self.emit_comma();
        self.emit_tag(TAG_GENERATED_RANGE_START);

        let line_delta = range.start.line - self.gr_line;

        let mut flags: u64 = 0;
        if line_delta != 0 {
            flags |= crate::GR_FLAG_HAS_LINE;
        }
        if range.definition.is_some() {
            flags |= crate::GR_FLAG_HAS_DEFINITION;
        }
        if range.is_stack_frame {
            flags |= crate::GR_FLAG_IS_STACK_FRAME;
        }
        if range.is_hidden {
            flags |= crate::GR_FLAG_IS_HIDDEN;
        }
        self.emit_unsigned(flags);

        if line_delta != 0 {
            self.emit_unsigned(line_delta as u64);
        }
        self.gr_line = range.start.line;

        let col = if line_delta != 0 {
            range.start.column
        } else {
            range.start.column - self.gr_col
        };
        self.emit_unsigned(col as u64);
        self.gr_col = range.start.column;

        if let Some(def) = range.definition {
            let def_i64 = def as i64;
            self.emit_signed(def_i64 - self.gr_def);
            self.gr_def = def_i64;
        }

        // G item: bindings
        if !range.bindings.is_empty() {
            self.emit_comma();
            self.emit_tag(TAG_GENERATED_RANGE_BINDINGS);
            for binding in &range.bindings {
                match binding {
                    Binding::Expression(expr) => {
                        let idx = self.name_idx(expr);
                        self.emit_unsigned(idx as u64 + 1); // 1-based
                    }
                    Binding::Unavailable => {
                        self.emit_unsigned(0);
                    }
                    Binding::SubRanges(subs) => {
                        // G gets the first sub-range's binding
                        if let Some(first) = subs.first() {
                            match &first.expression {
                                Some(expr) => {
                                    let idx = self.name_idx(expr);
                                    self.emit_unsigned(idx as u64 + 1);
                                }
                                None => {
                                    self.emit_unsigned(0);
                                }
                            }
                        } else {
                            self.emit_unsigned(0);
                        }
                    }
                }
            }
        }

        // H items: sub-range bindings
        let mut h_var_idx = 0u64;
        let mut h_first = true;
        for (i, binding) in range.bindings.iter().enumerate() {
            if let Binding::SubRanges(subs) = binding
                && subs.len() > 1
            {
                self.emit_comma();
                self.emit_tag(TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS);

                // Variable index (relative)
                let var_delta = i as u64 - if h_first { 0 } else { h_var_idx };
                self.emit_unsigned(var_delta);
                h_var_idx = i as u64;
                h_first = false;

                // Sub-range line/col state (relative to range start)
                let mut h_line = range.start.line;
                let mut h_col = range.start.column;

                // Skip first sub-range (that's in G), encode the rest
                for sub in &subs[1..] {
                    // Binding (1-based absolute)
                    match &sub.expression {
                        Some(expr) => {
                            let idx = self.name_idx(expr);
                            self.emit_unsigned(idx as u64 + 1);
                        }
                        None => {
                            self.emit_unsigned(0);
                        }
                    }

                    let sub_line_delta = sub.from.line - h_line;
                    self.emit_unsigned(sub_line_delta as u64);
                    h_line = sub.from.line;

                    let sub_col = if sub_line_delta != 0 {
                        sub.from.column
                    } else {
                        sub.from.column - h_col
                    };
                    self.emit_unsigned(sub_col as u64);
                    h_col = sub.from.column;
                }
            }
        }

        // I item: call site
        if let Some(ref cs) = range.call_site {
            self.emit_comma();
            self.emit_tag(TAG_GENERATED_RANGE_CALL_SITE);
            self.emit_unsigned(cs.source_index as u64);
            self.emit_unsigned(cs.line as u64);
            self.emit_unsigned(cs.column as u64);
        }

        // Recursively encode children
        for child in &range.children {
            self.encode_generated_range(child);
        }

        // F item: range end
        self.emit_comma();
        self.emit_tag(TAG_GENERATED_RANGE_END);

        let line_delta = range.end.line - self.gr_line;
        if line_delta != 0 {
            self.emit_unsigned(line_delta as u64);
        }
        self.gr_line = range.end.line;

        let col = if line_delta != 0 {
            range.end.column
        } else {
            range.end.column - self.gr_col
        };
        self.emit_unsigned(col as u64);
        self.gr_col = range.end.column;
    }
}

/// Encode scope information into a VLQ-encoded `scopes` string.
///
/// New names may be added to the `names` array during encoding.
pub fn encode_scopes(info: &ScopeInfo, names: &mut Vec<String>) -> String {
    let encoder = ScopesEncoder::new(names);
    encoder.encode(info)
}
