//! Decoder for the ECMA-426 scopes proposal.
//!
//! The decoder reads the layout of the draft specification: one `scopes`
//! string for each source, and a list of `ranges` strings. It also reads the
//! older layout, which has one `scopes` string for the full map.

use srcmap_codec::{vlq_decode, vlq_decode_unsigned};

use crate::{
    Binding, CallSite, GeneratedRange, MAX_NESTING_DEPTH, OriginalScope, Position, ScopeDefinition,
    ScopeInfo, ScopesError, SubRangeBinding, TAG_GENERATED_RANGE_BINDINGS,
    TAG_GENERATED_RANGE_CALL_SITE, TAG_GENERATED_RANGE_END, TAG_GENERATED_RANGE_RELATIVE_CALL_SITE,
    TAG_GENERATED_RANGE_START, TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS, TAG_ORIGINAL_SCOPE_END,
    TAG_ORIGINAL_SCOPE_START, TAG_ORIGINAL_SCOPE_VARIABLES,
};

/// The largest number of values in a relative call site item.
const MAX_RELATIVE_CALL_SITE_VALUES: usize = 3;

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

    /// Read an unsigned value that must fit in a `u32`.
    #[inline]
    fn read_u32(&mut self) -> Result<u32, ScopesError> {
        u32::try_from(self.read_unsigned()?).map_err(|_| ScopesError::ValueOverflow)
    }

    #[inline]
    fn read_signed(&mut self) -> Result<i64, ScopesError> {
        let (val, consumed) = vlq_decode(self.input, self.pos)?;
        self.pos += consumed;
        Ok(val)
    }

    /// Skip each value that is left in the current item.
    ///
    /// The grammar lets a later version of the specification add values to
    /// the end of an item, and it lets a tool add an item with a new tag.
    fn skip_item(&mut self) -> Result<(), ScopesError> {
        while !self.at_item_end() {
            let _ = self.read_unsigned()?;
        }
        Ok(())
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
    definition: Option<ScopeDefinition>,
    call_site: Option<CallSite>,
    bindings: Vec<Binding>,
    sub_range_bindings: Vec<(usize, Vec<SubRangeBinding>)>,
    children: Vec<GeneratedRange>,
}

/// The wire layout that the decoder reads.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Layout {
    /// The layout of the draft specification.
    Draft,
    /// The older layout, with one string for the full map.
    Legacy,
}

/// The items that one encoded string can contain.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// An entry of `scopes` in the draft layout.
    Scopes,
    /// An entry of `ranges` in the draft layout.
    Ranges,
    /// The one `scopes` string of the older layout.
    Legacy,
}

impl Section {
    const fn has_scopes(self) -> bool {
        !matches!(self, Self::Ranges)
    }

    const fn has_ranges(self) -> bool {
        !matches!(self, Self::Scopes)
    }
}

struct DecodeState<'n> {
    names: &'n [String],
    layout: Layout,
    num_sources: usize,
    /// The root scopes of each source that the decoder completed.
    scopes: Vec<Vec<OriginalScope>>,
    /// The root scopes of the source that the decoder reads now (draft layout).
    current_roots: Vec<OriginalScope>,
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
    gr_def_source: i64,
    gr_def_scope: i64,
    /// Variable index of the last sub-range item (older layout only).
    h_var_acc: u64,
    in_generated_ranges: bool,
    /// The start of each scope of a source, in pre-order. The decoder fills an
    /// entry when a relative call site first needs it.
    scope_starts: Vec<Option<Vec<Position>>>,
}

impl<'n> DecodeState<'n> {
    fn new(names: &'n [String], layout: Layout, num_sources: usize) -> Self {
        Self {
            names,
            layout,
            num_sources,
            scopes: Vec::new(),
            current_roots: Vec::new(),
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
            gr_def_source: 0,
            gr_def_scope: 0,
            h_var_acc: 0,
            in_generated_ranges: false,
            scope_starts: Vec::new(),
        }
    }

    /// Read each item of one encoded string.
    fn decode_items(&mut self, input: &str, section: Section) -> Result<(), ScopesError> {
        let mut tok = Tokenizer::new(input.as_bytes());

        while tok.has_next() {
            if section == Section::Legacy && tok.at_item_end() {
                self.handle_empty_item();
                tok.skip_comma();
                continue;
            }

            let tag = tok.read_unsigned()?;
            let known = match tag {
                TAG_ORIGINAL_SCOPE_START..=TAG_ORIGINAL_SCOPE_VARIABLES => section.has_scopes(),
                TAG_GENERATED_RANGE_START..=TAG_GENERATED_RANGE_CALL_SITE => section.has_ranges(),
                TAG_GENERATED_RANGE_RELATIVE_CALL_SITE => section == Section::Ranges,
                _ => false,
            };
            // An item with another tag is a vendor extension item or an item of
            // a later version of the specification. The decoder ignores it.
            if known {
                self.handle_item(tag, &mut tok)?;
            }

            tok.skip_item()?;
            tok.skip_comma();
        }

        if !self.scope_stack.is_empty() {
            return Err(ScopesError::UnclosedScope);
        }
        if !self.range_stack.is_empty() {
            return Err(ScopesError::UnclosedRange);
        }
        Ok(())
    }

    fn handle_item(&mut self, tag: u64, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        match tag {
            TAG_ORIGINAL_SCOPE_START => self.handle_original_scope_start(tok),
            TAG_ORIGINAL_SCOPE_END => self.handle_original_scope_end(tok),
            TAG_ORIGINAL_SCOPE_VARIABLES => self.handle_original_scope_variables(tok),
            TAG_GENERATED_RANGE_START => self.handle_generated_range_start(tok),
            TAG_GENERATED_RANGE_END => self.handle_generated_range_end(tok),
            TAG_GENERATED_RANGE_BINDINGS => self.handle_generated_range_bindings(tok),
            TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS => {
                self.handle_generated_range_sub_range_bindings(tok)
            }
            TAG_GENERATED_RANGE_CALL_SITE => self.handle_generated_range_call_site(tok),
            _ => self.handle_generated_range_relative_call_site(tok),
        }
    }

    /// Start the decode of the scopes of the next source (draft layout).
    fn start_source(&mut self) {
        self.os_line = 0;
        self.os_col = 0;
        self.os_name = 0;
        self.os_kind = 0;
        self.os_var = 0;
    }

    /// Start the decode of the next `ranges` string (draft layout).
    fn start_ranges_segment(&mut self) {
        self.gr_line = 0;
        self.gr_col = 0;
        self.gr_def_source = 0;
        self.gr_def_scope = 0;
    }

    fn finish(mut self) -> ScopeInfo {
        if self.scopes.len() < self.num_sources {
            self.scopes.resize_with(self.num_sources, Vec::new);
        }
        if self.layout == Layout::Legacy {
            let bases = legacy_definition_bases(&self.scopes);
            for range in &mut self.ranges {
                convert_legacy_definitions(range, &bases);
            }
        }
        ScopeInfo { scopes: self.scopes, ranges: self.ranges }
    }

    /// An empty item of the older layout is a source without scopes.
    fn handle_empty_item(&mut self) {
        if !self.in_generated_ranges
            && self.scopes.len() < self.num_sources
            && self.scope_stack.is_empty()
        {
            self.scopes.push(Vec::new());
        }
    }

    fn handle_original_scope_start(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        // The older layout restarts the position for each source. The draft
        // layout has one string for each source, and `start_source` does it.
        if self.layout == Layout::Legacy && self.scope_stack.is_empty() {
            self.os_line = 0;
            self.os_col = 0;
        }

        if self.scope_stack.len() >= MAX_NESTING_DEPTH {
            return Err(ScopesError::NestingTooDeep);
        }

        let flags = tok.read_unsigned()?;

        let line_delta = tok.read_u32()?;
        advance_position(line_delta, tok.read_u32()?, &mut self.os_line, &mut self.os_col)?;

        let name = if flags & crate::OS_FLAG_HAS_NAME != 0 {
            relative_name(self.names, &mut self.os_name, tok.read_signed()?)?
        } else {
            None
        };

        let kind = if flags & crate::OS_FLAG_HAS_KIND != 0 {
            relative_name(self.names, &mut self.os_kind, tok.read_signed()?)?
        } else {
            None
        };

        self.scope_stack.push(BuildingScope {
            start: Position { line: self.os_line, column: self.os_col },
            name,
            kind,
            is_stack_frame: flags & crate::OS_FLAG_IS_STACK_FRAME != 0,
            variables: Vec::new(),
            children: Vec::new(),
        });

        Ok(())
    }

    fn handle_original_scope_end(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        let Some(building) = self.scope_stack.pop() else {
            return Err(ScopesError::UnmatchedScopeEnd);
        };

        let line_delta = tok.read_u32()?;
        advance_position(line_delta, tok.read_u32()?, &mut self.os_line, &mut self.os_col)?;

        let finished = OriginalScope {
            start: building.start,
            end: Position { line: self.os_line, column: self.os_col },
            name: building.name,
            kind: building.kind,
            is_stack_frame: building.is_stack_frame,
            variables: building.variables,
            children: building.children,
        };

        if let Some(parent) = self.scope_stack.last_mut() {
            parent.children.push(finished);
        } else if self.layout == Layout::Legacy {
            // Each root scope of the older layout is the scope of the next source.
            self.scopes.push(vec![finished]);
        } else {
            self.current_roots.push(finished);
        }

        Ok(())
    }

    fn handle_original_scope_variables(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        let Some(current) = self.scope_stack.last_mut() else {
            return Ok(());
        };
        while !tok.at_item_end() {
            // A variable with an index past the end of `names` keeps its place
            // as the empty string. Each binding then stays with its variable.
            let name = relative_name(self.names, &mut self.os_var, tok.read_signed()?)?;
            current.variables.push(name.unwrap_or_default());
        }
        Ok(())
    }

    fn handle_generated_range_start(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        if self.layout == Layout::Legacy && !self.in_generated_ranges {
            self.in_generated_ranges = true;
            if self.scopes.len() < self.num_sources {
                self.scopes.resize_with(self.num_sources, Vec::new);
            }
        }
        if self.range_stack.len() >= MAX_NESTING_DEPTH {
            return Err(ScopesError::NestingTooDeep);
        }

        let flags = tok.read_unsigned()?;

        let line_delta = if flags & crate::GR_FLAG_HAS_LINE != 0 { tok.read_u32()? } else { 0 };
        advance_position(line_delta, tok.read_u32()?, &mut self.gr_line, &mut self.gr_col)?;

        let definition = if flags & crate::GR_FLAG_HAS_DEFINITION != 0 {
            Some(self.read_definition(tok)?)
        } else {
            None
        };

        let is_stack_frame = flags & crate::GR_FLAG_IS_STACK_FRAME != 0;
        // The hidden flag is valid only on a range that is a stack frame.
        let is_hidden = is_stack_frame && flags & crate::GR_FLAG_IS_HIDDEN != 0;

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

    /// Read the definition of a generated range.
    ///
    /// The draft layout has a source index and a scope index. The scope index
    /// is relative when the source stays the same, and absolute when the source
    /// changes. The older layout has one index into the scopes of all sources.
    /// `finish` converts that index.
    fn read_definition(&mut self, tok: &mut Tokenizer<'_>) -> Result<ScopeDefinition, ScopesError> {
        if self.layout == Layout::Draft {
            let source_delta = tok.read_signed()?;
            accumulate_index(&mut self.gr_def_source, source_delta)?;
            let scope_delta = if source_delta == 0 {
                tok.read_signed()?
            } else {
                self.gr_def_scope = 0;
                i64::try_from(tok.read_unsigned()?).map_err(|_| ScopesError::ValueOverflow)?
            };
            accumulate_index(&mut self.gr_def_scope, scope_delta)?;
        } else {
            accumulate_index(&mut self.gr_def_scope, tok.read_signed()?)?;
        }

        Ok(ScopeDefinition {
            source_index: index_to_u32(self.gr_def_source)?,
            scope_index: index_to_u32(self.gr_def_scope)?,
        })
    }

    fn handle_generated_range_end(&mut self, tok: &mut Tokenizer<'_>) -> Result<(), ScopesError> {
        let Some(building) = self.range_stack.pop() else {
            return Err(ScopesError::UnmatchedRangeEnd);
        };

        // F tag: 1 VLQ = column only, 2 VLQs = line + column.
        let first = tok.read_u32()?;
        let (line_delta, col_raw) =
            if tok.at_item_end() { (0, first) } else { (first, tok.read_u32()?) };
        advance_position(line_delta, col_raw, &mut self.gr_line, &mut self.gr_col)?;

        let finished = GeneratedRange {
            start: building.start,
            end: Position { line: self.gr_line, column: self.gr_col },
            is_stack_frame: building.is_stack_frame,
            is_hidden: building.is_hidden,
            definition: building.definition,
            call_site: building.call_site,
            bindings: merge_bindings(
                building.bindings,
                building.sub_range_bindings,
                building.start,
            ),
            children: building.children,
        };

        if let Some(parent) = self.range_stack.last_mut() {
            parent.children.push(finished);
        } else {
            self.ranges.push(finished);
        }

        Ok(())
    }

    fn handle_generated_range_bindings(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        let Some(current) = self.range_stack.last_mut() else {
            return Ok(());
        };
        while !tok.at_item_end() {
            let binding = match binding_expression(self.names, tok.read_unsigned()?) {
                Some(expr) => Binding::Expression(expr),
                None => Binding::Unavailable,
            };
            current.bindings.push(binding);
        }
        Ok(())
    }

    fn handle_generated_range_sub_range_bindings(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        let Some(current) = self.range_stack.last_mut() else {
            return Ok(());
        };

        // The draft layout has the index of the variable. The older layout has
        // the distance from the variable of the last sub-range item.
        let raw_index = tok.read_unsigned()?;
        let index = if self.layout == Layout::Draft {
            raw_index
        } else {
            self.h_var_acc =
                self.h_var_acc.checked_add(raw_index).ok_or(ScopesError::ValueOverflow)?;
            self.h_var_acc
        };
        // An index above usize::MAX is out of range, and the merge ignores it.
        let var_idx = usize::try_from(index).unwrap_or(usize::MAX);

        let mut sub_ranges: Vec<SubRangeBinding> = Vec::new();
        // The positions are relative to the start of the range.
        let mut line = current.start.line;
        let mut column = current.start.column;

        while !tok.at_item_end() {
            // The draft layout has line, column, expression. The older layout
            // has expression, line, column.
            let binding_raw = if self.layout == Layout::Draft {
                let line_delta = tok.read_u32()?;
                advance_position(line_delta, tok.read_u32()?, &mut line, &mut column)?;
                tok.read_unsigned()?
            } else {
                let binding_raw = tok.read_unsigned()?;
                let line_delta = tok.read_u32()?;
                advance_position(line_delta, tok.read_u32()?, &mut line, &mut column)?;
                binding_raw
            };

            sub_ranges.push(SubRangeBinding {
                expression: binding_expression(self.names, binding_raw),
                from: Position { line, column },
            });
        }

        current.sub_range_bindings.push((var_idx, sub_ranges));
        Ok(())
    }

    fn handle_generated_range_call_site(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        let Some(current) = self.range_stack.last_mut() else {
            return Ok(());
        };
        let source_index = tok.read_u32()?;
        let line = tok.read_u32()?;
        let column = tok.read_u32()?;
        current.call_site = Some(CallSite { source_index, line, column });
        Ok(())
    }

    /// Read a call site that is relative to the definition of a parent range.
    ///
    /// The item has one of three forms:
    /// - parent index, line offset, column
    /// - line offset, column (the nearest parent)
    /// - column offset (the nearest parent, on the start line of its scope)
    fn handle_generated_range_relative_call_site(
        &mut self,
        tok: &mut Tokenizer<'_>,
    ) -> Result<(), ScopesError> {
        if self.range_stack.is_empty() {
            return Ok(());
        }

        let mut values = [0u32; MAX_RELATIVE_CALL_SITE_VALUES];
        let mut count = 0;
        while count < MAX_RELATIVE_CALL_SITE_VALUES && (count == 0 || !tok.at_item_end()) {
            values[count] = tok.read_u32()?;
            count += 1;
        }

        let parent_index = if count == MAX_RELATIVE_CALL_SITE_VALUES { values[0] } else { 0 };
        let definition = self.parent_definition(parent_index)?;
        let start = self.scope_start(definition).ok_or(ScopesError::UnresolvedCallSite)?;

        let (line, column) = if count == 1 {
            (start.line, checked_add_u32(start.column, values[0])?)
        } else {
            (checked_add_u32(start.line, values[count - 2])?, values[count - 1])
        };

        if let Some(current) = self.range_stack.last_mut() {
            current.call_site =
                Some(CallSite { source_index: definition.source_index, line, column });
        }
        Ok(())
    }

    /// Find the definition of a parent of the current range.
    ///
    /// Index 0 is the nearest parent that has a definition. The search skips a
    /// parent without a definition, and it does not look at the current range.
    fn parent_definition(&self, parent_index: u32) -> Result<ScopeDefinition, ScopesError> {
        let parents = &self.range_stack[..self.range_stack.len().saturating_sub(1)];
        parents
            .iter()
            .rev()
            .filter_map(|range| range.definition)
            .nth(parent_index as usize)
            .ok_or(ScopesError::UnresolvedCallSite)
    }

    /// Find the start of the original scope that `definition` refers to.
    fn scope_start(&mut self, definition: ScopeDefinition) -> Option<Position> {
        let source_index = definition.source_index as usize;
        let roots = self.scopes.get(source_index)?;
        if self.scope_starts.len() <= source_index {
            self.scope_starts.resize(source_index + 1, None);
        }
        let starts = self.scope_starts[source_index].get_or_insert_with(|| {
            let mut starts = Vec::new();
            let mut stack: Vec<&OriginalScope> = roots.iter().rev().collect();
            while let Some(scope) = stack.pop() {
                starts.push(scope.start);
                stack.extend(scope.children.iter().rev());
            }
            starts
        });
        starts.get(definition.scope_index as usize).copied()
    }
}

/// Add a line and a column increment to a position.
///
/// The column increment is relative when the line stays the same, and it is
/// the new column when the line changes.
fn advance_position(
    line_delta: u32,
    col_raw: u32,
    line: &mut u32,
    column: &mut u32,
) -> Result<(), ScopesError> {
    *line = checked_add_u32(*line, line_delta)?;
    *column = if line_delta != 0 { col_raw } else { checked_add_u32(*column, col_raw)? };
    Ok(())
}

fn checked_add_u32(value: u32, delta: u32) -> Result<u32, ScopesError> {
    value.checked_add(delta).ok_or(ScopesError::ValueOverflow)
}

/// Add `delta` to a relative index. An index below zero becomes zero, as the
/// `AccumulateIndex` operation of ECMA-426 specifies.
fn accumulate_index(index: &mut i64, delta: i64) -> Result<(), ScopesError> {
    *index = index.checked_add(delta).ok_or(ScopesError::ValueOverflow)?.max(0);
    Ok(())
}

fn index_to_u32(index: i64) -> Result<u32, ScopesError> {
    u32::try_from(index).map_err(|_| ScopesError::ValueOverflow)
}

/// Resolve a relative index into `names`. An index past the end of `names`
/// gives `None`, as the `RelativeName` operation of ECMA-426 specifies.
fn relative_name(
    names: &[String],
    index: &mut i64,
    delta: i64,
) -> Result<Option<String>, ScopesError> {
    accumulate_index(index, delta)?;
    Ok(usize::try_from(*index).ok().and_then(|i| names.get(i)).cloned())
}

/// Resolve a binding expression. The value 0 is a variable that is not
/// available, and each other value is an index into `names` plus 1. An index
/// past the end of `names` also gives `None`.
fn binding_expression(names: &[String], value: u64) -> Option<String> {
    let index = usize::try_from(value.checked_sub(1)?).ok()?;
    names.get(index).cloned()
}

/// Add the bindings of the sub-range items to the bindings of a range.
fn merge_bindings(
    mut bindings: Vec<Binding>,
    sub_range_bindings: Vec<(usize, Vec<SubRangeBinding>)>,
    range_start: Position,
) -> Vec<Binding> {
    for (var_idx, sub_ranges) in sub_range_bindings {
        // The decoder ignores a sub-range item without a sub-range, and a
        // sub-range item for a variable without a binding.
        if sub_ranges.is_empty() {
            continue;
        }
        let Some(binding) = bindings.get_mut(var_idx) else {
            continue;
        };
        let initial = match std::mem::replace(binding, Binding::Unavailable) {
            Binding::Expression(expr) => {
                vec![SubRangeBinding { expression: Some(expr), from: range_start }]
            }
            Binding::Unavailable => vec![SubRangeBinding { expression: None, from: range_start }],
            // A second sub-range item for the same variable adds to the list.
            Binding::SubRanges(existing) => existing,
        };
        *binding = Binding::SubRanges(initial.into_iter().chain(sub_ranges).collect());
    }
    bindings
}

// ── Older layout: definitions ────────────────────────────────────

/// Count `scope` and each scope nested in it, without recursion.
fn count_scope_tree(scope: &OriginalScope) -> u64 {
    let mut count = 0;
    let mut stack = vec![scope];
    while let Some(node) = stack.pop() {
        count += 1;
        stack.extend(&node.children);
    }
    count
}

/// For each source, the number of scopes in the sources before it.
fn legacy_definition_bases(scopes: &[Vec<OriginalScope>]) -> Vec<u64> {
    let mut next = 0;
    scopes
        .iter()
        .map(|roots| {
            let base = next;
            next += roots.iter().map(count_scope_tree).sum::<u64>();
            base
        })
        .collect()
}

/// Convert each definition of the older layout to a source index and a scope
/// index. The older layout counts the scopes of all sources in one sequence.
///
/// An index past the last scope stays in the last source, where it is out of
/// range too.
fn convert_legacy_definitions(range: &mut GeneratedRange, bases: &[u64]) {
    if let Some(definition) = &mut range.definition {
        let global = u64::from(definition.scope_index);
        let source_index = bases.partition_point(|&base| base <= global).saturating_sub(1);
        let base = bases.get(source_index).copied().unwrap_or(0);
        *definition = ScopeDefinition {
            source_index: u32::try_from(source_index).unwrap_or(u32::MAX),
            scope_index: u32::try_from(global - base).unwrap_or(u32::MAX),
        };
    }
    for child in &mut range.children {
        convert_legacy_definitions(child, bases);
    }
}

// ── Decode ───────────────────────────────────────────────────────

/// Decode the `scopes` and `ranges` fields of a source map into structured
/// scope information.
///
/// This function reads the layout of the ECMA-426 draft specification.
///
/// - `scopes`: the `scopes` array of the source map. Each entry is the encoded
///   scopes of the source at the same index, or `None` for a source without
///   scopes. The function ignores an entry past `num_sources`.
/// - `ranges`: the `ranges` array of the source map. Each entry has the
///   generated ranges of one region of the generated code.
/// - `names`: the `names` array of the source map. An index past the end of
///   `names` is not an error. A scope then has no name or no kind, a variable
///   gets the empty name, and a binding is not available.
/// - `num_sources`: the length of the `sources` array. The result has this
///   number of entries in [`ScopeInfo::scopes`].
///
/// The decoder ignores a vendor extension item (tag `V`) and an item with a
/// tag that it does not know.
pub fn decode_scopes<S: AsRef<str>, R: AsRef<str>>(
    scopes: &[Option<S>],
    ranges: &[R],
    names: &[String],
    num_sources: usize,
) -> Result<ScopeInfo, ScopesError> {
    let mut state = DecodeState::new(names, Layout::Draft, num_sources);

    for encoded in scopes.iter().take(num_sources) {
        state.start_source();
        if let Some(encoded) = encoded {
            state.decode_items(encoded.as_ref(), Section::Scopes)?;
        }
        let roots = std::mem::take(&mut state.current_roots);
        state.scopes.push(roots);
    }
    if state.scopes.len() < num_sources {
        state.scopes.resize_with(num_sources, Vec::new);
    }

    for encoded in ranges {
        state.start_ranges_segment();
        state.decode_items(encoded.as_ref(), Section::Ranges)?;
    }

    Ok(state.finish())
}

/// Decode a `scopes` string of the older layout of the scopes proposal.
///
/// The older layout has one `scopes` string for the full map. A map has this
/// layout when its `scopes` field is a JSON string. srcmap reads this layout,
/// and it does not write it.
///
/// - `input`: the `scopes` string of the source map
/// - `names`: the `names` array of the source map
/// - `num_sources`: the length of the `sources` array
pub fn decode_legacy_scopes(
    input: &str,
    names: &[String],
    num_sources: usize,
) -> Result<ScopeInfo, ScopesError> {
    let mut state = DecodeState::new(names, Layout::Legacy, num_sources);
    state.decode_items(input, Section::Legacy)?;
    Ok(state.finish())
}
