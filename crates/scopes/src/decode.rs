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
    if input.is_empty() {
        let scopes = vec![None; num_sources];
        return Ok(ScopeInfo {
            scopes,
            ranges: vec![],
        });
    }

    let mut tok = Tokenizer::new(input.as_bytes());

    // Original scope state
    let mut scopes: Vec<Option<OriginalScope>> = Vec::new();
    let mut source_idx = 0usize;
    let mut scope_stack: Vec<BuildingScope> = Vec::new();
    let mut os_line = 0u32;
    let mut os_col = 0u32;
    let mut os_name = 0i64;
    let mut os_kind = 0i64;
    let mut os_var = 0i64;

    // Generated range state
    let mut ranges: Vec<GeneratedRange> = Vec::new();
    let mut range_stack: Vec<BuildingRange> = Vec::new();
    let mut gr_line = 0u32;
    let mut gr_col = 0u32;
    let mut gr_def = 0i64;
    let mut h_var_acc: u64 = 0;
    let mut in_generated_ranges = false;

    while tok.has_next() {
        // Empty item: no scope info for this source file
        if tok.at_item_end() {
            if !in_generated_ranges && source_idx < num_sources && scope_stack.is_empty() {
                scopes.push(None);
                source_idx += 1;
            }
            tok.skip_comma();
            continue;
        }

        let tag = tok.read_unsigned()?;

        match tag {
            TAG_ORIGINAL_SCOPE_START => {
                // Reset position state at start of new top-level tree
                if scope_stack.is_empty() {
                    os_line = 0;
                    os_col = 0;
                }

                let flags = tok.read_unsigned()?;

                let line_delta = tok.read_unsigned()? as u32;
                os_line += line_delta;
                let col_raw = tok.read_unsigned()? as u32;
                os_col = if line_delta != 0 {
                    col_raw
                } else {
                    os_col + col_raw
                };

                let name = if flags & crate::OS_FLAG_HAS_NAME != 0 {
                    let d = tok.read_signed()?;
                    os_name += d;
                    Some(resolve_name(names, os_name)?)
                } else {
                    None
                };

                let kind = if flags & crate::OS_FLAG_HAS_KIND != 0 {
                    let d = tok.read_signed()?;
                    os_kind += d;
                    Some(resolve_name(names, os_kind)?)
                } else {
                    None
                };

                let is_stack_frame = flags & crate::OS_FLAG_IS_STACK_FRAME != 0;

                scope_stack.push(BuildingScope {
                    start: Position {
                        line: os_line,
                        column: os_col,
                    },
                    name,
                    kind,
                    is_stack_frame,
                    variables: Vec::new(),
                    children: Vec::new(),
                });
            }

            TAG_ORIGINAL_SCOPE_END => {
                if scope_stack.is_empty() {
                    return Err(ScopesError::UnmatchedScopeEnd);
                }

                let line_delta = tok.read_unsigned()? as u32;
                os_line += line_delta;
                let col_raw = tok.read_unsigned()? as u32;
                os_col = if line_delta != 0 {
                    col_raw
                } else {
                    os_col + col_raw
                };

                // Safe: scope_stack.is_empty() checked above
                let building = scope_stack.pop().expect("non-empty: checked above");
                let finished = OriginalScope {
                    start: building.start,
                    end: Position {
                        line: os_line,
                        column: os_col,
                    },
                    name: building.name,
                    kind: building.kind,
                    is_stack_frame: building.is_stack_frame,
                    variables: building.variables,
                    children: building.children,
                };

                if scope_stack.is_empty() {
                    scopes.push(Some(finished));
                    source_idx += 1;
                } else {
                    // Safe: just checked !is_empty()
                    scope_stack.last_mut().expect("non-empty: checked above").children.push(finished);
                }
            }

            TAG_ORIGINAL_SCOPE_VARIABLES => {
                if let Some(current) = scope_stack.last_mut() {
                    while !tok.at_item_end() {
                        let d = tok.read_signed()?;
                        os_var += d;
                        current.variables.push(resolve_name(names, os_var)?);
                    }
                } else {
                    while !tok.at_item_end() {
                        let _ = tok.read_signed()?;
                    }
                }
            }

            TAG_GENERATED_RANGE_START => {
                if !in_generated_ranges {
                    in_generated_ranges = true;
                    // Fill remaining source slots
                    while scopes.len() < num_sources {
                        scopes.push(None);
                    }
                    source_idx = num_sources;
                }

                let flags = tok.read_unsigned()?;

                let line_delta = if flags & crate::GR_FLAG_HAS_LINE != 0 {
                    tok.read_unsigned()? as u32
                } else {
                    0
                };
                gr_line += line_delta;

                let col_raw = tok.read_unsigned()? as u32;
                gr_col = if line_delta != 0 {
                    col_raw
                } else {
                    gr_col + col_raw
                };

                let definition = if flags & crate::GR_FLAG_HAS_DEFINITION != 0 {
                    let d = tok.read_signed()?;
                    gr_def += d;
                    Some(gr_def as u32)
                } else {
                    None
                };

                let is_stack_frame = flags & crate::GR_FLAG_IS_STACK_FRAME != 0;
                let is_hidden = flags & crate::GR_FLAG_IS_HIDDEN != 0;

                // Reset H-tag variable index accumulator for each new range
                h_var_acc = 0;

                range_stack.push(BuildingRange {
                    start: Position {
                        line: gr_line,
                        column: gr_col,
                    },
                    is_stack_frame,
                    is_hidden,
                    definition,
                    call_site: None,
                    bindings: Vec::new(),
                    sub_range_bindings: Vec::new(),
                    children: Vec::new(),
                });
            }

            TAG_GENERATED_RANGE_END => {
                if range_stack.is_empty() {
                    return Err(ScopesError::UnmatchedRangeEnd);
                }

                // F tag: 1 VLQ = column only, 2 VLQs = line + column
                let first = tok.read_unsigned()? as u32;
                let (line_delta, col_raw) = if !tok.at_item_end() {
                    let second = tok.read_unsigned()? as u32;
                    (first, second)
                } else {
                    (0, first)
                };
                gr_line += line_delta;
                gr_col = if line_delta != 0 {
                    col_raw
                } else {
                    gr_col + col_raw
                };

                // Safe: range_stack.is_empty() checked above
                let building = range_stack.pop().expect("non-empty: checked above");

                // Merge sub-range bindings into final bindings
                let final_bindings = merge_bindings(
                    building.bindings,
                    &building.sub_range_bindings,
                    building.start,
                );

                let finished = GeneratedRange {
                    start: building.start,
                    end: Position {
                        line: gr_line,
                        column: gr_col,
                    },
                    is_stack_frame: building.is_stack_frame,
                    is_hidden: building.is_hidden,
                    definition: building.definition,
                    call_site: building.call_site,
                    bindings: final_bindings,
                    children: building.children,
                };

                if range_stack.is_empty() {
                    ranges.push(finished);
                } else {
                    // Safe: just checked !is_empty()
                    range_stack.last_mut().expect("non-empty: checked above").children.push(finished);
                }
            }

            TAG_GENERATED_RANGE_BINDINGS => {
                if let Some(current) = range_stack.last_mut() {
                    while !tok.at_item_end() {
                        let idx = tok.read_unsigned()?;
                        let binding = match resolve_binding(names, idx)? {
                            Some(expr) => Binding::Expression(expr),
                            None => Binding::Unavailable,
                        };
                        current.bindings.push(binding);
                    }
                } else {
                    while !tok.at_item_end() {
                        let _ = tok.read_unsigned()?;
                    }
                }
            }

            TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS => {
                if let Some(current) = range_stack.last_mut() {
                    let var_delta = tok.read_unsigned()?;
                    h_var_acc += var_delta;
                    let var_idx = h_var_acc as usize;

                    let mut sub_ranges: Vec<SubRangeBinding> = Vec::new();
                    // Line/column state relative to range start
                    let mut h_line = current.start.line;
                    let mut h_col = current.start.column;

                    while !tok.at_item_end() {
                        let binding_raw = tok.read_unsigned()?;
                        let line_delta = tok.read_unsigned()? as u32;
                        h_line += line_delta;

                        let col_raw = tok.read_unsigned()? as u32;
                        h_col = if line_delta != 0 {
                            col_raw
                        } else {
                            h_col + col_raw
                        };

                        let expression = resolve_binding(names, binding_raw)?;
                        sub_ranges.push(SubRangeBinding {
                            expression,
                            from: Position {
                                line: h_line,
                                column: h_col,
                            },
                        });
                    }

                    current.sub_range_bindings.push((var_idx, sub_ranges));
                } else {
                    while !tok.at_item_end() {
                        let _ = tok.read_unsigned()?;
                    }
                }
            }

            TAG_GENERATED_RANGE_CALL_SITE => {
                if let Some(current) = range_stack.last_mut() {
                    let source_index = tok.read_unsigned()? as u32;
                    let line = tok.read_unsigned()? as u32;
                    let column = tok.read_unsigned()? as u32;
                    current.call_site = Some(CallSite {
                        source_index,
                        line,
                        column,
                    });
                } else {
                    while !tok.at_item_end() {
                        let _ = tok.read_unsigned()?;
                    }
                }
            }

            _ => {
                // Unknown tag: skip remaining VLQs in this item
                while !tok.at_item_end() {
                    let _ = tok.read_unsigned()?;
                }
            }
        }

        tok.skip_comma();
    }

    if !scope_stack.is_empty() {
        return Err(ScopesError::UnclosedScope);
    }
    if !range_stack.is_empty() {
        return Err(ScopesError::UnclosedRange);
    }

    // Fill remaining source slots
    while scopes.len() < num_sources {
        scopes.push(None);
    }

    Ok(ScopeInfo { scopes, ranges })
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
                Binding::Unavailable => None,
                Binding::SubRanges(_) => None, // shouldn't happen
            };

            let mut all_subs = vec![SubRangeBinding {
                expression: initial_expr,
                from: range_start,
            }];
            all_subs.extend(sub_ranges.iter().cloned());

            result[*var_idx] = Binding::SubRanges(all_subs);
        }
    }

    result
}
