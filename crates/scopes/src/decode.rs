//! Decoder for the ECMA-426 scopes proposal.
//!
//! Parses a VLQ-encoded `scopes` string into structured `ScopeInfo`.

use srcmap_codec::{vlq_decode, vlq_decode_unsigned};

use crate::{
    Binding, CallSite, GeneratedRange, OriginalScope, Position, ScopeInfo, ScopesError,
    SubRangeBinding, TAG_GENERATED_RANGE_BINDINGS, TAG_GENERATED_RANGE_CALL_SITE,
    TAG_GENERATED_RANGE_END, TAG_GENERATED_RANGE_START, TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS,
    TAG_ORIGINAL_SCOPE_END, TAG_ORIGINAL_SCOPE_START, TAG_ORIGINAL_SCOPE_VARIABLES,
    resolve_binding, resolve_name,
};

// ── Tokenizer ────────────────────────────────────────────────────

struct Tokenizer<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a [u8]) -> Self {
        Self { input, pos: 0 }
    }

    #[inline]
    fn has_next(&self) -> bool {
        self.pos < self.input.len()
    }

    /// Check if we're at the end of the current item (comma or end of input).
    #[inline]
    fn at_item_end(&self) -> bool {
        self.pos >= self.input.len() || self.input[self.pos] == b','
    }

    /// Skip a comma separator (if present).
    #[inline]
    fn skip_comma(&mut self) {
        if self.pos < self.input.len() && self.input[self.pos] == b',' {
            self.pos += 1;
        }
    }

    #[inline]
    fn read_unsigned(&mut self) -> Result<u64, ScopesError> {
        let (val, consumed) = vlq_decode_unsigned(self.input, self.pos)?;
        self.pos += consumed;
        Ok(val)
    }

    #[inline]
    fn read_signed(&mut self) -> Result<i64, ScopesError> {
        let (val, consumed) = vlq_decode(self.input, self.pos)?;
        self.pos += consumed;
        Ok(val)
    }
}

// ── Building types ───────────────────────────────────────────────

struct BuildingScope {
    start: Position,
    name: Option<String>,
    kind: Option<String>,
    is_stack_frame: bool,
    variables: Vec<String>,
    children: Vec<OriginalScope>,
}

struct BuildingRange {
    start: Position,
    is_stack_frame: bool,
    is_hidden: bool,
    definition: Option<u32>,
    call_site: Option<CallSite>,
    bindings: Vec<Binding>,
    sub_range_bindings: Vec<(usize, Vec<SubRangeBinding>)>,
    children: Vec<GeneratedRange>,
}

struct DecodeState {
    scopes: Vec<Option<OriginalScope>>,
    source_idx: usize,
    scope_stack: Vec<BuildingScope>,
    os_line: u32,
    os_col: u32,
    os_name: i64,
    os_kind: i64,
    os_var: i64,
    ranges: Vec<GeneratedRange>,
    range_stack: Vec<BuildingRange>,
    gr_line: u32,
    gr_col: u32,
    gr_def: i64,
    h_var_acc: u64,
    in_generated_ranges: bool,
    num_sources: usize,
}

impl DecodeState {
    fn new(num_sources: usize) -> Self {
        Self {
            scopes: Vec::new(),
            source_idx: 0,
            scope_stack: Vec::new(),
            os_line: 0,
            os_col: 0,
            os_name: 0,
            os_kind: 0,
            os_var: 0,
            ranges: Vec::new(),
            range_stack: Vec::new(),
            gr_line: 0,
            gr_col: 0,
            gr_def: 0,
            h_var_acc: 0,
            in_generated_ranges: false,
            num_sources,
        }
    }

    fn finish(mut self) -> Result<ScopeInfo, ScopesError> {
        if !self.scope_stack.is_empty() {
            return Err(ScopesError::UnclosedScope);
        }
        if !self.range_stack.is_empty() {
            return Err(ScopesError::UnclosedRange);
        }

        // Fill remaining source slots.
        while self.scopes.len() < self.num_sources {
            self.scopes.push(None);
        }

        Ok(ScopeInfo { scopes: self.scopes, ranges: self.ranges })
    }

    fn handle_empty_item(&mut self) {
        if !self.in_generated_ranges
            && self.source_idx < self.num_sources
            && self.scope_stack.is_empty()
        {
            self.scopes.push(None);
            self.source_idx += 1;
        }
    }

    fn start_generated_ranges_if_needed(&mut self) {
        if self.in_generated_ranges {
            return;
        }

        self.in_generated_ranges = true;
        // Fill remaining source slots.
        while self.scopes.len() < self.num_sources {
            self.scopes.push(None);
        }
        self.source_idx = self.num_sources;
    }

    fn handle_original_scope_start(
        &mut self,
        tok: &mut Tokenizer<'_>,
        names: &[String],
    ) -> Result<(), ScopesError> {
        if self.scope_stack.is_empty() {
            self.os_line = 0;
            self.os_col = 0;
        }

        let flags = tok.read_unsigned()?;

        let line_delta = tok.read_unsigned()? as u32;
        Self::advance_position(
            line_delta,
            tok.read_unsigned()? as u32,
            &mut self.os_line,
            &mut self.os_col,
        );

        let name = if flags & crate::OS_FLAG_HAS_NAME != 0 {
            let d = tok.read_signed()?;
            self.os_name += d;
            Some(resolve_name(names, self.os_name)?)
        } else {
            None
        };

        let kind = if flags & crate::OS_FLAG_HAS_KIND != 0 {
            let d = tok.read_signed()?;
            self.os_kind += d;
            Some(resolve_name(names, self.os_kind)?)
        } else {
            None
        };

        let is_stack_frame = flags & crate::OS_FLAG_IS_STACK_FRAME != 0;

        self.scope_stack.push(BuildingScope {
            start: Position { line: self.os_line, column: self.os_col },
            name,
            kind,
            is_stack_frame,
            variables: Vec::new(),
            children: Vec::new(),
        });

        Ok(())
    }

    fn handle_original_scope_end(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        if self.scope_stack.is_empty() {
            return Err(ScopesError::UnmatchedScopeEnd);
        }

        let line_delta = tok.read_unsigned()? as u32;
        Self::advance_position(
            line_delta,
            tok.read_unsigned()? as u32,
            &mut self.os_line,
            &mut self.os_col,
        );

        // Safe: scope_stack.is_empty() checked above.
        let building = self.scope_stack.pop().expect("non-empty: checked above");
        let finished = OriginalScope {
            start: building.start,
            end: Position { line: self.os_line, column: self.os_col },
            name: building.name,
            kind: building.kind,
            is_stack_frame: building.is_stack_frame,
            variables: building.variables,
            children: building.children,
        };

        if self.scope_stack.is_empty() {
            self.scopes.push(Some(finished));
            self.source_idx += 1;
        } else {
            self.scope_stack.last_mut().expect("non-empty: checked above").children.push(finished);
        }

        Ok(())
    }

    fn handle_original_scope_variables(
        &mut self,
        tok: &mut Tokenizer<'_>,
        names: &[String],
    ) -> Result<(), ScopesError> {
        if let Some(current) = self.scope_stack.last_mut() {
            while !tok.at_item_end() {
                let d = tok.read_signed()?;
                self.os_var += d;
                current.variables.push(resolve_name(names, self.os_var)?);
            }
        } else {
            while !tok.at_item_end() {
                let _ = tok.read_signed()?;
            }
        }

        Ok(())
    }

    fn handle_generated_range_start(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        self.start_generated_ranges_if_needed();

        let flags = tok.read_unsigned()?;

        let line_delta =
            if flags & crate::GR_FLAG_HAS_LINE != 0 { tok.read_unsigned()? as u32 } else { 0 };
        Self::advance_position(
            line_delta,
            tok.read_unsigned()? as u32,
            &mut self.gr_line,
            &mut self.gr_col,
        );

        let definition = if flags & crate::GR_FLAG_HAS_DEFINITION != 0 {
            let d = tok.read_signed()?;
            self.gr_def += d;
            Some(self.gr_def as u32)
        } else {
            None
        };

        let is_stack_frame = flags & crate::GR_FLAG_IS_STACK_FRAME != 0;
        let is_hidden = flags & crate::GR_FLAG_IS_HIDDEN != 0;

        // Reset H-tag variable index accumulator for each new range.
        self.h_var_acc = 0;

        self.range_stack.push(BuildingRange {
            start: Position { line: self.gr_line, column: self.gr_col },
            is_stack_frame,
            is_hidden,
            definition,
            call_site: None,
            bindings: Vec::new(),
            sub_range_bindings: Vec::new(),
            children: Vec::new(),
        });

        Ok(())
    }

    fn handle_generated_range_end(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        if self.range_stack.is_empty() {
            return Err(ScopesError::UnmatchedRangeEnd);
        }

        // F tag: 1 VLQ = column only, 2 VLQs = line + column.
        let first = tok.read_unsigned()? as u32;
        let (line_delta, col_raw) = if !tok.at_item_end() {
            let second = tok.read_unsigned()? as u32;
            (first, second)
        } else {
            (0, first)
        };
        Self::advance_position(line_delta, col_raw, &mut self.gr_line, &mut self.gr_col);

        // Safe: range_stack.is_empty() checked above.
        let building = self.range_stack.pop().expect("non-empty: checked above");

        // Merge sub-range bindings into final bindings.
        let final_bindings =
            merge_bindings(building.bindings, &building.sub_range_bindings, building.start);

        let finished = GeneratedRange {
            start: building.start,
            end: Position { line: self.gr_line, column: self.gr_col },
            is_stack_frame: building.is_stack_frame,
            is_hidden: building.is_hidden,
            definition: building.definition,
            call_site: building.call_site,
            bindings: final_bindings,
            children: building.children,
        };

        if self.range_stack.is_empty() {
            self.ranges.push(finished);
        } else {
            self.range_stack.last_mut().expect("non-empty: checked above").children.push(finished);
        }

        Ok(())
    }

    fn handle_generated_range_bindings(
        &mut self,
        tok: &mut Tokenizer<'_>,
        names: &[String],
    ) -> Result<(), ScopesError> {
        if let Some(current) = self.range_stack.last_mut() {
            while !tok.at_item_end() {
                let idx = tok.read_unsigned()?;
                let binding = match resolve_binding(names, idx)? {
                    Some(expr) => Binding::Expression(expr),
                    None => Binding::Unavailable,
                };
                current.bindings.push(binding);
            }
        } else {
            Self::skip_unsigned_item(tok)?;
        }

        Ok(())
    }

    fn handle_generated_range_sub_range_bindings(
        &mut self,
        tok: &mut Tokenizer<'_>,
        names: &[String],
    ) -> Result<(), ScopesError> {
        if let Some(current) = self.range_stack.last_mut() {
            let var_delta = tok.read_unsigned()?;
            self.h_var_acc += var_delta;
            let var_idx = self.h_var_acc as usize;

            let mut sub_ranges: Vec<SubRangeBinding> = Vec::new();
            // Line/column state relative to range start.
            let mut h_line = current.start.line;
            let mut h_col = current.start.column;

            while !tok.at_item_end() {
                let binding_raw = tok.read_unsigned()?;
                let line_delta = tok.read_unsigned()? as u32;
                Self::advance_position(
                    line_delta,
                    tok.read_unsigned()? as u32,
                    &mut h_line,
                    &mut h_col,
                );

                let expression = resolve_binding(names, binding_raw)?;
                sub_ranges.push(SubRangeBinding {
                    expression,
                    from: Position { line: h_line, column: h_col },
                });
            }

            current.sub_range_bindings.push((var_idx, sub_ranges));
        } else {
            Self::skip_unsigned_item(tok)?;
        }

        Ok(())
    }

    fn handle_generated_range_call_site(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        if let Some(current) = self.range_stack.last_mut() {
            let source_index = tok.read_unsigned()? as u32;
            let line = tok.read_unsigned()? as u32;
            let column = tok.read_unsigned()? as u32;
            current.call_site = Some(CallSite { source_index, line, column });
        } else {
            Self::skip_unsigned_item(tok)?;
        }

        Ok(())
    }

    fn advance_position(line_delta: u32, col_raw: u32, line: &mut u32, column: &mut u32) {
        *line += line_delta;
        *column = if line_delta != 0 { col_raw } else { *column + col_raw };
    }

    fn skip_unsigned_item(tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        while !tok.at_item_end() {
            let _ = tok.read_unsigned()?;
        }
        Ok(())
    }
}

// ── Decode ───────────────────────────────────────────────────────

/// Decode a `scopes` string into structured scope information.
///
/// - `input`: the VLQ-encoded scopes string from the source map
/// - `names`: the `names` array from the source map (for resolving indices).
///   Must contain all names referenced by the encoded string, or
///   `ScopesError::InvalidNameIndex` will be returned.
/// - `num_sources`: number of source files (length of `sources` array)
pub fn decode_scopes(
    input: &str,
    names: &[String],
    num_sources: usize,
) -> Result<ScopeInfo, ScopesError> {
    let mut tok = Tokenizer::new(input.as_bytes());
    let mut state = DecodeState::new(num_sources);

    while tok.has_next() {
        if tok.at_item_end() {
            state.handle_empty_item();
            tok.skip_comma();
            continue;
        }

        let tag = tok.read_unsigned()?;

        match tag {
            TAG_ORIGINAL_SCOPE_START => state.handle_original_scope_start(&mut tok, names)?,
            TAG_ORIGINAL_SCOPE_END => state.handle_original_scope_end(&mut tok)?,
            TAG_ORIGINAL_SCOPE_VARIABLES => {
                state.handle_original_scope_variables(&mut tok, names)?
            }
            TAG_GENERATED_RANGE_START => state.handle_generated_range_start(&mut tok)?,
            TAG_GENERATED_RANGE_END => state.handle_generated_range_end(&mut tok)?,
            TAG_GENERATED_RANGE_BINDINGS => {
                state.handle_generated_range_bindings(&mut tok, names)?
            }
            TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS => {
                state.handle_generated_range_sub_range_bindings(&mut tok, names)?
            }
            TAG_GENERATED_RANGE_CALL_SITE => state.handle_generated_range_call_site(&mut tok)?,
            _ => DecodeState::skip_unsigned_item(&mut tok)?,
        }

        tok.skip_comma();
    }

    state.finish()
}

/// Merge initial bindings from G items with sub-range overrides from H items.
fn merge_bindings(
    initial: Vec<Binding>,
    sub_range_map: &[(usize, Vec<SubRangeBinding>)],
    range_start: Position,
) -> Vec<Binding> {
    if sub_range_map.is_empty() {
        return initial;
    }

    let mut result = initial;

    for (var_idx, sub_ranges) in sub_range_map {
        if *var_idx < result.len() {
            // Get the initial binding expression to use as first sub-range
            let initial_expr = match &result[*var_idx] {
                Binding::Expression(e) => Some(e.clone()),
                Binding::Unavailable | Binding::SubRanges(_) => None, // shouldn't happen
            };

            let mut all_subs =
                vec![SubRangeBinding { expression: initial_expr, from: range_start }];
            all_subs.extend(sub_ranges.iter().cloned());

            result[*var_idx] = Binding::SubRanges(all_subs);
        }
    }

    result
}
