//! Scopes and variables decoder/encoder for source maps (ECMA-426).
//!
//! Implements the "Scopes" proposal for source maps, enabling debuggers to
//! reconstruct original scope trees, variable bindings, and inlined function
//! call sites from generated code.
//!
//! # Examples
//!
//! ```
//! use srcmap_scopes::{
//!     decode_scopes, encode_scopes, Binding, CallSite, GeneratedRange,
//!     OriginalScope, Position, ScopeInfo,
//! };
//!
//! // Build scope info
//! let info = ScopeInfo {
//!     scopes: vec![Some(OriginalScope {
//!         start: Position { line: 0, column: 0 },
//!         end: Position { line: 5, column: 0 },
//!         name: None,
//!         kind: Some("global".to_string()),
//!         is_stack_frame: false,
//!         variables: vec!["x".to_string()],
//!         children: vec![],
//!     })],
//!     ranges: vec![GeneratedRange {
//!         start: Position { line: 0, column: 0 },
//!         end: Position { line: 5, column: 0 },
//!         is_stack_frame: false,
//!         is_hidden: false,
//!         definition: Some(0),
//!         call_site: None,
//!         bindings: vec![Binding::Expression("_x".to_string())],
//!         children: vec![],
//!     }],
//! };
//!
//! // Encode
//! let mut names = vec![];
//! let encoded = encode_scopes(&info, &mut names);
//! assert!(!encoded.is_empty());
//!
//! // Decode
//! let decoded = decode_scopes(&encoded, &names, 1).unwrap();
//! assert_eq!(decoded.scopes.len(), 1);
//! assert!(decoded.scopes[0].is_some());
//! ```

mod decode;
mod encode;

use std::fmt;

pub use decode::decode_scopes;
pub use encode::encode_scopes;

use srcmap_codec::DecodeError;

// ── Tag constants ────────────────────────────────────────────────

const TAG_ORIGINAL_SCOPE_START: u64 = 0x1;
const TAG_ORIGINAL_SCOPE_END: u64 = 0x2;
const TAG_ORIGINAL_SCOPE_VARIABLES: u64 = 0x3;
const TAG_GENERATED_RANGE_START: u64 = 0x4;
const TAG_GENERATED_RANGE_END: u64 = 0x5;
const TAG_GENERATED_RANGE_BINDINGS: u64 = 0x6;
const TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS: u64 = 0x7;
const TAG_GENERATED_RANGE_CALL_SITE: u64 = 0x8;

// ── Flag constants ───────────────────────────────────────────────

/// Flags for original scope start (B tag).
const OS_FLAG_HAS_NAME: u64 = 0x1;
const OS_FLAG_HAS_KIND: u64 = 0x2;
const OS_FLAG_IS_STACK_FRAME: u64 = 0x4;

/// Flags for generated range start (E tag).
const GR_FLAG_HAS_LINE: u64 = 0x1;
const GR_FLAG_HAS_DEFINITION: u64 = 0x2;
const GR_FLAG_IS_STACK_FRAME: u64 = 0x4;
const GR_FLAG_IS_HIDDEN: u64 = 0x8;

// ── Public types ─────────────────────────────────────────────────

/// A 0-based position in source code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// An original scope from authored source code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalScope {
    pub start: Position,
    pub end: Position,
    /// Scope name (e.g., function name). Stored in the `names` array.
    pub name: Option<String>,
    /// Scope kind (e.g., "global", "function", "block"). Stored in `names`.
    pub kind: Option<String>,
    /// Whether this scope is a stack frame (function boundary).
    pub is_stack_frame: bool,
    /// Variables declared in this scope.
    ///
    /// An empty string is a variable with no known name. The decoder gives it
    /// for an index past the end of `names`, as ECMA-426 specifies, so that
    /// each binding of a generated range keeps its variable.
    pub variables: Vec<String>,
    /// Child scopes nested within this one.
    pub children: Vec<OriginalScope>,
}

/// A binding expression for a variable in a generated range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Binding {
    /// Variable is available via this JS expression.
    Expression(String),
    /// Variable is not available in this range.
    Unavailable,
    /// Variable has different bindings in different sub-ranges.
    SubRanges(Vec<SubRangeBinding>),
}

/// A sub-range binding within a generated range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRangeBinding {
    /// The JS expression evaluating to the variable's value. `None` = unavailable.
    pub expression: Option<String>,
    /// Start position of this sub-range within the generated range.
    pub from: Position,
}

/// A call site in original source code (for inlined functions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CallSite {
    pub source_index: u32,
    pub line: u32,
    pub column: u32,
}

/// A generated range in the output code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedRange {
    pub start: Position,
    pub end: Position,
    /// Whether this range is a stack frame (function boundary).
    pub is_stack_frame: bool,
    /// Whether this stack frame should be hidden from traces.
    pub is_hidden: bool,
    /// Index into the pre-order list of all original scope starts.
    pub definition: Option<u32>,
    /// Call site if this range represents an inlined function body.
    pub call_site: Option<CallSite>,
    /// Variable bindings (one per variable in the referenced original scope).
    pub bindings: Vec<Binding>,
    /// Child ranges nested within this one.
    pub children: Vec<GeneratedRange>,
}

/// Decoded scope information from a source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScopeInfo {
    /// Original scope trees, one per source file (aligned with `sources`).
    /// `None` means no scope info for that source file.
    pub scopes: Vec<Option<OriginalScope>>,
    /// Top-level generated ranges for the output code.
    pub ranges: Vec<GeneratedRange>,
}

impl ScopeInfo {
    /// Get the original scope referenced by a generated range's `definition` index.
    ///
    /// The definition index references scopes in pre-order traversal order
    /// across all source files.
    pub fn original_scope_for_definition(&self, definition: u32) -> Option<&OriginalScope> {
        let mut count = 0u32;
        for scope in self.scopes.iter().flatten() {
            if let Some(result) = find_nth_scope(scope, definition, &mut count) {
                return Some(result);
            }
        }
        None
    }
}

fn find_nth_scope<'a>(
    scope: &'a OriginalScope,
    target: u32,
    count: &mut u32,
) -> Option<&'a OriginalScope> {
    // Iterative pre-order traversal to avoid stack overflow on deeply nested scopes
    let mut stack: Vec<&'a OriginalScope> = vec![scope];

    while let Some(node) = stack.pop() {
        if *count == target {
            return Some(node);
        }
        *count += 1;
        // Push children in reverse order so leftmost is processed first
        for child in node.children.iter().rev() {
            stack.push(child);
        }
    }
    None
}

// ── Errors ───────────────────────────────────────────────────────

/// The maximum depth of nested original scopes, and of nested generated
/// ranges, that [`decode_scopes`] accepts.
///
/// The scope types drop, clone, compare, encode and print with recursion, one
/// call per level. At this limit, each of these steps fits in half of the
/// 2 MiB default stack of a spawned thread, also in a debug build.
pub const MAX_NESTING_DEPTH: usize = 512;

/// Errors during scopes decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopesError {
    /// VLQ decoding failed.
    Vlq(DecodeError),
    /// Scope end without matching scope start.
    UnmatchedScopeEnd,
    /// Scope was opened but never closed.
    UnclosedScope,
    /// Range end without matching range start.
    UnmatchedRangeEnd,
    /// Range was opened but never closed.
    UnclosedRange,
    /// Name index out of bounds.
    InvalidNameIndex(i64),
    /// A line, column or index does not fit in a `u32`, or a delta makes it
    /// overflow.
    ValueOverflow,
    /// Scopes or ranges nest deeper than [`MAX_NESTING_DEPTH`].
    NestingTooDeep,
}

impl fmt::Display for ScopesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Vlq(e) => write!(f, "VLQ decode error: {e}"),
            Self::UnmatchedScopeEnd => write!(f, "scope end without matching start"),
            Self::UnclosedScope => write!(f, "scope opened but never closed"),
            Self::UnmatchedRangeEnd => write!(f, "range end without matching start"),
            Self::UnclosedRange => write!(f, "range opened but never closed"),
            Self::InvalidNameIndex(idx) => write!(f, "invalid name index: {idx}"),
            Self::ValueOverflow => write!(f, "scopes position or index is out of range"),
            Self::NestingTooDeep => {
                write!(f, "scopes nest deeper than {MAX_NESTING_DEPTH} levels")
            }
        }
    }
}

impl std::error::Error for ScopesError {}

impl From<DecodeError> for ScopesError {
    fn from(e: DecodeError) -> Self {
        Self::Vlq(e)
    }
}

// ── Internal helpers ─────────────────────────────────────────────

/// Resolve a name from the names array by absolute index.
fn resolve_name(names: &[String], index: i64) -> Result<String, ScopesError> {
    usize::try_from(index)
        .ok()
        .and_then(|i| names.get(i))
        .cloned()
        .ok_or(ScopesError::InvalidNameIndex(index))
}

/// Resolve a variable name from the names array by absolute index.
///
/// ECMA-426 (`OriginalScopeVariables`) keeps an index past the end of `names`
/// as the empty string. A variable that the decoder drops moves each later
/// binding to the wrong variable.
fn resolve_variable(names: &[String], index: i64) -> Result<String, ScopesError> {
    let index = usize::try_from(index).map_err(|_| ScopesError::InvalidNameIndex(index))?;
    Ok(names.get(index).cloned().unwrap_or_default())
}

/// Resolve a 1-based binding name (0 = unavailable).
fn resolve_binding(names: &[String], index: u64) -> Result<Option<String>, ScopesError> {
    if index == 0 {
        return Ok(None);
    }
    usize::try_from(index - 1)
        .ok()
        .and_then(|i| names.get(i))
        .map(|name| Some(name.clone()))
        .ok_or_else(|| ScopesError::InvalidNameIndex(i64::try_from(index).unwrap_or(i64::MAX)))
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scopes() {
        let info = decode_scopes("", &[], 0).unwrap();
        assert_eq!(info.scopes, Vec::<Option<OriginalScope>>::new());
        assert_eq!(info.ranges, Vec::<GeneratedRange>::new());
    }

    #[test]
    fn empty_scopes_with_sources() {
        // Two empty items (commas) for two source files with no scopes
        let info = decode_scopes(",", &[], 2).unwrap();
        assert_eq!(info.scopes.len(), 2);
        assert!(info.scopes[0].is_none());
        assert!(info.scopes[1].is_none());
    }

    #[test]
    fn single_global_scope_roundtrip() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["x".to_string(), "y".to_string()],
                children: vec![],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.scopes.len(), 1);
        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.start, Position { line: 0, column: 0 });
        assert_eq!(scope.end, Position { line: 10, column: 0 });
        assert_eq!(scope.kind.as_deref(), Some("global"));
        assert_eq!(scope.name, None);
        assert!(!scope.is_stack_frame);
        assert_eq!(scope.variables, vec!["x", "y"]);
    }

    #[test]
    fn nested_scopes_roundtrip() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 1 },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["z".to_string()],
                children: vec![OriginalScope {
                    start: Position { line: 1, column: 0 },
                    end: Position { line: 5, column: 1 },
                    name: Some("hello".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec!["msg".to_string(), "result".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.children.len(), 1);
        let child = &scope.children[0];
        assert_eq!(child.start, Position { line: 1, column: 0 });
        assert_eq!(child.end, Position { line: 5, column: 1 });
        assert_eq!(child.name.as_deref(), Some("hello"));
        assert_eq!(child.kind.as_deref(), Some("function"));
        assert!(child.is_stack_frame);
        assert_eq!(child.variables, vec!["msg", "result"]);
    }

    #[test]
    fn multiple_sources_with_gaps() {
        let info = ScopeInfo {
            scopes: vec![
                Some(OriginalScope {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 5, column: 0 },
                    name: None,
                    kind: None,
                    is_stack_frame: false,
                    variables: vec![],
                    children: vec![],
                }),
                None, // second source has no scopes
                Some(OriginalScope {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 3, column: 0 },
                    name: None,
                    kind: None,
                    is_stack_frame: false,
                    variables: vec![],
                    children: vec![],
                }),
            ],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 3).unwrap();

        assert_eq!(decoded.scopes.len(), 3);
        assert!(decoded.scopes[0].is_some());
        assert!(decoded.scopes[1].is_none());
        assert!(decoded.scopes[2].is_some());
    }

    #[test]
    fn generated_ranges_roundtrip() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("_x".to_string())],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.ranges.len(), 1);
        let range = &decoded.ranges[0];
        assert_eq!(range.start, Position { line: 0, column: 0 });
        assert_eq!(range.end, Position { line: 10, column: 0 });
        assert_eq!(range.definition, Some(0));
        assert_eq!(range.bindings, vec![Binding::Expression("_x".to_string())]);
    }

    #[test]
    fn definition_resolution() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec![],
                children: vec![
                    OriginalScope {
                        start: Position { line: 1, column: 0 },
                        end: Position { line: 4, column: 1 },
                        name: Some("foo".to_string()),
                        kind: Some("function".to_string()),
                        is_stack_frame: true,
                        variables: vec![],
                        children: vec![],
                    },
                    OriginalScope {
                        start: Position { line: 5, column: 0 },
                        end: Position { line: 9, column: 1 },
                        name: Some("bar".to_string()),
                        kind: Some("function".to_string()),
                        is_stack_frame: true,
                        variables: vec![],
                        children: vec![],
                    },
                ],
            })],
            ranges: vec![],
        };

        // Definition 0 = global scope
        let scope0 = info.original_scope_for_definition(0).unwrap();
        assert_eq!(scope0.kind.as_deref(), Some("global"));

        // Definition 1 = foo
        let scope1 = info.original_scope_for_definition(1).unwrap();
        assert_eq!(scope1.name.as_deref(), Some("foo"));

        // Definition 2 = bar
        let scope2 = info.original_scope_for_definition(2).unwrap();
        assert_eq!(scope2.name.as_deref(), Some("bar"));

        // Definition 3 = out of bounds
        assert!(info.original_scope_for_definition(3).is_none());
    }

    #[test]
    fn range_no_definition() {
        let info = ScopeInfo {
            scopes: vec![],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: None,
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 0).unwrap();

        assert_eq!(decoded.ranges.len(), 1);
        assert_eq!(decoded.ranges[0].definition, None);
    }

    #[test]
    fn invalid_name_index_error() {
        // Encode scopes that reference name index 0, but pass empty names array to decode
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: Some("test".to_string()),
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        // Now decode with empty names - should fail with InvalidNameIndex
        let err = decode_scopes(&encoded, &[], 1).unwrap_err();
        assert!(matches!(err, ScopesError::InvalidNameIndex(_)));
    }

    /// One scope with the variables `a`, `b`, `c` and a range that binds them.
    fn three_variable_info() -> ScopeInfo {
        ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["a".to_string(), "b".to_string(), "c".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![
                    Binding::Expression("_a".to_string()),
                    Binding::Expression("_b".to_string()),
                    Binding::Expression("_c".to_string()),
                ],
                children: vec![],
            }],
        }
    }

    #[test]
    fn variable_index_out_of_bounds_decodes_as_empty_name() {
        let info = three_variable_info();
        // Put the binding expressions first, so that only the variable `c`
        // points past the end of the shortened names array.
        let mut names: Vec<String> =
            ["_a", "_b", "_c", "a", "b"].iter().map(ToString::to_string).collect();
        let encoded = encode_scopes(&info, &mut names);
        assert_eq!(names.len(), 6, "the encoder adds only `c`");

        let decoded = decode_scopes(&encoded, &names[..5], 1).unwrap();
        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.variables, vec!["a", "b", ""]);
        // Each binding stays with its variable.
        assert_eq!(decoded.ranges[0].bindings, info.ranges[0].bindings);
    }

    #[test]
    fn empty_variable_name_roundtrip() {
        let mut info = three_variable_info();
        info.scopes[0].as_mut().unwrap().variables[1] = String::new();

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();
        assert_eq!(decoded, info);
    }

    #[test]
    fn negative_variable_index_error() {
        // B flags=0 line=0 column=0, D variable=-1, C line=0 column=0
        let err = decode_scopes("BAAA,DD,CAA", &["a".to_string()], 1).unwrap_err();
        assert!(matches!(err, ScopesError::InvalidNameIndex(-1)));
    }

    #[test]
    fn invalid_binding_index_error() {
        // Create a range with a binding expression that requires name index 0
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("_x".to_string())],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        // Decode with truncated names (remove the binding expression name)
        let short_names: Vec<String> = names.iter().take(1).cloned().collect();
        let err = decode_scopes(&encoded, &short_names, 1).unwrap_err();
        assert!(matches!(err, ScopesError::InvalidNameIndex(_)));
    }

    #[test]
    fn sub_range_same_line_bindings() {
        // Sub-ranges where positions are on the same line (column-only delta)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::SubRanges(vec![
                    SubRangeBinding {
                        expression: Some("a".to_string()),
                        from: Position { line: 0, column: 0 },
                    },
                    SubRangeBinding {
                        expression: Some("b".to_string()),
                        from: Position { line: 0, column: 15 },
                    },
                ])],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        match &decoded.ranges[0].bindings[0] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 2);
                assert_eq!(subs[0].from, Position { line: 0, column: 0 });
                assert_eq!(subs[1].from, Position { line: 0, column: 15 });
            }
            other => panic!("expected SubRanges, got {other:?}"),
        }
    }

    #[test]
    fn sub_range_multiple_variables() {
        // Two H items in one range: the second variable index is relative to the first.
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 30, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string(), "y".to_string(), "z".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 30, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![
                    // Variable 0 (x): sub-ranges
                    Binding::SubRanges(vec![
                        SubRangeBinding {
                            expression: Some("_x1".to_string()),
                            from: Position { line: 0, column: 0 },
                        },
                        SubRangeBinding {
                            expression: Some("_x2".to_string()),
                            from: Position { line: 10, column: 0 },
                        },
                    ]),
                    // Variable 1 (y): simple binding
                    Binding::Expression("_y".to_string()),
                    // Variable 2 (z): sub-ranges (second H item)
                    Binding::SubRanges(vec![
                        SubRangeBinding {
                            expression: Some("_z1".to_string()),
                            from: Position { line: 0, column: 0 },
                        },
                        SubRangeBinding {
                            expression: None,
                            from: Position { line: 15, column: 5 },
                        },
                        SubRangeBinding {
                            expression: Some("_z3".to_string()),
                            from: Position { line: 20, column: 0 },
                        },
                    ]),
                ],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let bindings = &decoded.ranges[0].bindings;
        assert_eq!(bindings.len(), 3);

        // Variable 0: sub-ranges
        match &bindings[0] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 2);
                assert_eq!(subs[0].expression.as_deref(), Some("_x1"));
                assert_eq!(subs[1].expression.as_deref(), Some("_x2"));
                assert_eq!(subs[1].from, Position { line: 10, column: 0 });
            }
            other => panic!("expected SubRanges for x, got {other:?}"),
        }

        // Variable 1: simple expression
        assert_eq!(bindings[1], Binding::Expression("_y".to_string()));

        // Variable 2: sub-ranges (second H item)
        match &bindings[2] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 3);
                assert_eq!(subs[0].expression.as_deref(), Some("_z1"));
                assert_eq!(subs[1].expression, None);
                assert_eq!(subs[1].from, Position { line: 15, column: 5 });
                assert_eq!(subs[2].expression.as_deref(), Some("_z3"));
            }
            other => panic!("expected SubRanges for z, got {other:?}"),
        }
    }

    #[test]
    fn unknown_tag_skipped() {
        // Unknown tags must be skipped so newer spec revisions still decode.
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);

        // Build unknown tag item: tag 0x12 (= 18, unused) followed by two
        // unsigned VLQs, then comma, then the valid encoded data.
        let mut crafted = Vec::new();
        srcmap_codec::vlq_encode_unsigned(&mut crafted, 18); // unknown tag
        srcmap_codec::vlq_encode_unsigned(&mut crafted, 42); // dummy VLQ 1
        srcmap_codec::vlq_encode_unsigned(&mut crafted, 7); // dummy VLQ 2
        crafted.push(b',');
        crafted.extend_from_slice(encoded.as_bytes());

        let crafted_str = std::str::from_utf8(&crafted).unwrap();
        let decoded = decode_scopes(crafted_str, &names, 1).unwrap();

        assert_eq!(decoded.scopes.len(), 1);
        assert!(decoded.scopes[0].is_some());
        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.start, Position { line: 0, column: 0 });
        assert_eq!(scope.end, Position { line: 5, column: 0 });
    }

    #[test]
    fn leading_sources_without_scopes() {
        let info = ScopeInfo {
            scopes: vec![
                None,
                None,
                Some(OriginalScope {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 5, column: 0 },
                    name: None,
                    kind: None,
                    is_stack_frame: false,
                    variables: vec![],
                    children: vec![],
                }),
            ],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 3).unwrap();

        assert_eq!(decoded.scopes.len(), 3);
        assert!(decoded.scopes[0].is_none());
        assert!(decoded.scopes[1].is_none());
        assert!(decoded.scopes[2].is_some());
    }

    #[test]
    fn scope_end_same_line_as_child_end() {
        // Parent scope ends on the same line where the child scope ended (column-relative end).
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 20 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![OriginalScope {
                    start: Position { line: 5, column: 0 },
                    end: Position { line: 10, column: 10 },
                    name: None,
                    kind: None,
                    is_stack_frame: false,
                    variables: vec![],
                    children: vec![],
                }],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.end, Position { line: 10, column: 20 });
        assert_eq!(scope.children[0].end, Position { line: 10, column: 10 });
    }

    #[test]
    fn generated_range_has_line_flag() {
        // Sibling ranges: the second starts on a later line (GR_FLAG_HAS_LINE).
        let info = ScopeInfo {
            scopes: vec![],
            ranges: vec![
                GeneratedRange {
                    start: Position { line: 0, column: 5 },
                    end: Position { line: 0, column: 50 },
                    is_stack_frame: false,
                    is_hidden: false,
                    definition: None,
                    call_site: None,
                    bindings: vec![],
                    children: vec![],
                },
                GeneratedRange {
                    start: Position { line: 3, column: 10 },
                    end: Position { line: 8, column: 20 },
                    is_stack_frame: false,
                    is_hidden: false,
                    definition: None,
                    call_site: None,
                    bindings: vec![],
                    children: vec![],
                },
            ],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 0).unwrap();

        assert_eq!(decoded.ranges.len(), 2);
        assert_eq!(decoded.ranges[0].start, Position { line: 0, column: 5 });
        assert_eq!(decoded.ranges[0].end, Position { line: 0, column: 50 });
        assert_eq!(decoded.ranges[1].start, Position { line: 3, column: 10 });
        assert_eq!(decoded.ranges[1].end, Position { line: 8, column: 20 });
    }

    #[test]
    fn sub_range_first_expression_none() {
        // First sub-range unavailable: the G item carries 0 for that variable.
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 20, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["v".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 20, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::SubRanges(vec![
                    SubRangeBinding { expression: None, from: Position { line: 0, column: 0 } },
                    SubRangeBinding {
                        expression: Some("_v".to_string()),
                        from: Position { line: 8, column: 0 },
                    },
                ])],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        match &decoded.ranges[0].bindings[0] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 2);
                assert_eq!(subs[0].expression, None);
                assert_eq!(subs[0].from, Position { line: 0, column: 0 });
                assert_eq!(subs[1].expression.as_deref(), Some("_v"));
                assert_eq!(subs[1].from, Position { line: 8, column: 0 });
            }
            other => panic!("expected SubRanges, got {other:?}"),
        }
    }

    #[test]
    fn comprehensive_roundtrip() {
        // Full roundtrip combining scopes with name+kind, nested ranges with
        // call sites, sub-range bindings, unavailable bindings, and hidden ranges.
        let info = ScopeInfo {
            scopes: vec![
                Some(OriginalScope {
                    start: Position { line: 0, column: 0 },
                    end: Position { line: 50, column: 0 },
                    name: None,
                    kind: Some("module".to_string()),
                    is_stack_frame: false,
                    variables: vec!["exports".to_string()],
                    children: vec![
                        OriginalScope {
                            start: Position { line: 2, column: 0 },
                            end: Position { line: 20, column: 1 },
                            name: Some("add".to_string()),
                            kind: Some("function".to_string()),
                            is_stack_frame: true,
                            variables: vec!["a".to_string(), "b".to_string()],
                            children: vec![],
                        },
                        OriginalScope {
                            start: Position { line: 22, column: 0 },
                            end: Position { line: 40, column: 1 },
                            name: Some("multiply".to_string()),
                            kind: Some("function".to_string()),
                            is_stack_frame: true,
                            variables: vec!["x".to_string(), "y".to_string()],
                            children: vec![],
                        },
                    ],
                }),
                None, // second source: no scopes
            ],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 25, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("module.exports".to_string())],
                children: vec![
                    GeneratedRange {
                        start: Position { line: 1, column: 0 },
                        end: Position { line: 10, column: 0 },
                        is_stack_frame: true,
                        is_hidden: false,
                        definition: Some(1),
                        call_site: Some(CallSite { source_index: 0, line: 45, column: 2 }),
                        bindings: vec![Binding::Expression("_a".to_string()), Binding::Unavailable],
                        children: vec![],
                    },
                    GeneratedRange {
                        start: Position { line: 12, column: 0 },
                        end: Position { line: 20, column: 0 },
                        is_stack_frame: true,
                        is_hidden: true,
                        definition: Some(2),
                        call_site: Some(CallSite { source_index: 0, line: 46, column: 0 }),
                        bindings: vec![
                            Binding::SubRanges(vec![
                                SubRangeBinding {
                                    expression: Some("p1".to_string()),
                                    from: Position { line: 12, column: 0 },
                                },
                                SubRangeBinding {
                                    expression: Some("p2".to_string()),
                                    from: Position { line: 16, column: 0 },
                                },
                            ]),
                            Binding::Expression("_y".to_string()),
                        ],
                        children: vec![],
                    },
                ],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 2).unwrap();

        // Verify scopes
        assert_eq!(decoded.scopes.len(), 2);
        assert!(decoded.scopes[1].is_none());
        let root = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(root.kind.as_deref(), Some("module"));
        assert_eq!(root.children.len(), 2);
        assert_eq!(root.children[0].name.as_deref(), Some("add"));
        assert_eq!(root.children[1].name.as_deref(), Some("multiply"));

        // Verify ranges
        assert_eq!(decoded.ranges.len(), 1);
        let outer = &decoded.ranges[0];
        assert_eq!(outer.children.len(), 2);

        // First child: inlined add
        let add_range = &outer.children[0];
        assert!(add_range.is_stack_frame);
        assert!(!add_range.is_hidden);
        assert_eq!(add_range.definition, Some(1));
        assert_eq!(add_range.call_site, Some(CallSite { source_index: 0, line: 45, column: 2 }));
        assert_eq!(add_range.bindings[0], Binding::Expression("_a".to_string()));
        assert_eq!(add_range.bindings[1], Binding::Unavailable);

        // Second child: inlined multiply (hidden)
        let mul_range = &outer.children[1];
        assert!(mul_range.is_stack_frame);
        assert!(mul_range.is_hidden);
        assert_eq!(mul_range.definition, Some(2));
        match &mul_range.bindings[0] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 2);
                assert_eq!(subs[0].expression.as_deref(), Some("p1"));
                assert_eq!(subs[1].expression.as_deref(), Some("p2"));
            }
            other => panic!("expected SubRanges, got {other:?}"),
        }
        assert_eq!(mul_range.bindings[1], Binding::Expression("_y".to_string()));
    }

    #[test]
    fn range_end_column_only_1vlq() {
        // Range where end is on the same line as a previous position, resulting
        // in a 1-VLQ range end (column only). The parent range ending on line 5
        // after the child also ended on line 5.
        let info = ScopeInfo {
            scopes: vec![],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 50 },
                is_stack_frame: false,
                is_hidden: false,
                definition: None,
                call_site: None,
                bindings: vec![],
                children: vec![GeneratedRange {
                    start: Position { line: 2, column: 0 },
                    end: Position { line: 5, column: 30 },
                    is_stack_frame: false,
                    is_hidden: false,
                    definition: None,
                    call_site: None,
                    bindings: vec![],
                    children: vec![],
                }],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 0).unwrap();

        let outer = &decoded.ranges[0];
        assert_eq!(outer.end, Position { line: 5, column: 50 });
        let inner = &outer.children[0];
        assert_eq!(inner.end, Position { line: 5, column: 30 });
    }

    #[test]
    fn all_sources_none_with_ranges() {
        // All sources have None scopes, but ranges exist.
        // Exercises the transition to in_generated_ranges when scopes are all None.
        let info = ScopeInfo {
            scopes: vec![None, None, None],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: None,
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 3).unwrap();

        assert_eq!(decoded.scopes.len(), 3);
        assert!(decoded.scopes.iter().all(|s| s.is_none()));
        assert_eq!(decoded.ranges.len(), 1);
    }

    #[test]
    fn scope_name_only_no_kind() {
        // Scope with name but no kind (exercises OS_FLAG_HAS_NAME without OS_FLAG_HAS_KIND)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: Some("myVar".to_string()),
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.name.as_deref(), Some("myVar"));
        assert_eq!(scope.kind, None);
    }

    #[test]
    fn decode_unmatched_scope_end() {
        // Craft a raw string with TAG_ORIGINAL_SCOPE_END (0x2 = 'C') without preceding start
        // TAG=2 (C), line_delta=0 (A), col=0 (A)
        let raw = "CAA";
        let names: Vec<String> = vec![];
        let err = decode_scopes(raw, &names, 1).unwrap_err();
        assert!(matches!(err, ScopesError::UnmatchedScopeEnd));
    }

    #[test]
    fn decode_unclosed_scope() {
        // TAG_ORIGINAL_SCOPE_START (0x1 = 'B'), flags=0 (A), line=0 (A), col=0 (A)
        // No matching end tag
        let raw = "BAAA";
        let names: Vec<String> = vec![];
        let err = decode_scopes(raw, &names, 1).unwrap_err();
        assert!(matches!(err, ScopesError::UnclosedScope));
    }

    #[test]
    fn decode_unmatched_range_end() {
        // First fill 1 source scope (empty) with comma separator
        // Then TAG_GENERATED_RANGE_END (0x5 = 'F'), col=0 (A)
        // Empty scope for source 0, then comma, then range end tag
        let raw = ",FA";
        let names: Vec<String> = vec![];
        let err = decode_scopes(raw, &names, 1).unwrap_err();
        assert!(matches!(err, ScopesError::UnmatchedRangeEnd));
    }

    #[test]
    fn decode_unclosed_range() {
        // Fill 1 source (empty), then start a range without closing it
        // TAG_GENERATED_RANGE_START (0x4 = 'E'), flags=0 (A), col=0 (A)
        let raw = ",EAAA";
        let names: Vec<String> = vec![];
        let err = decode_scopes(raw, &names, 1).unwrap_err();
        assert!(matches!(err, ScopesError::UnclosedRange));
    }

    #[test]
    fn scopes_error_display() {
        assert_eq!(ScopesError::UnmatchedScopeEnd.to_string(), "scope end without matching start");
        assert_eq!(ScopesError::UnclosedScope.to_string(), "scope opened but never closed");
        assert_eq!(ScopesError::UnmatchedRangeEnd.to_string(), "range end without matching start");
        assert_eq!(ScopesError::UnclosedRange.to_string(), "range opened but never closed");
        assert_eq!(ScopesError::InvalidNameIndex(42).to_string(), "invalid name index: 42");

        let err: ScopesError = srcmap_codec::DecodeError::UnexpectedEof { offset: 5 }.into();
        assert!(matches!(err, ScopesError::Vlq(_)));
        assert!(err.to_string().contains("VLQ decode error"));
    }

    fn vlq_unsigned(value: u64) -> String {
        let mut out = Vec::new();
        srcmap_codec::vlq_encode_unsigned(&mut out, value);
        String::from_utf8(out).unwrap()
    }

    fn vlq_signed(value: i64) -> String {
        let mut out = Vec::new();
        srcmap_codec::vlq_encode(&mut out, value);
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn decode_rejects_original_column_overflow() {
        // Outer scope at column u32::MAX, nested scope one column later.
        let max = vlq_unsigned(u64::from(u32::MAX));
        let raw = format!("BAA{max},BAAC,CAA,CAA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_original_line_overflow() {
        let max = vlq_unsigned(u64::from(u32::MAX));
        let raw = format!("BA{max}A,BACA,CAA,CAA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_position_value_above_u32() {
        let big = vlq_unsigned(1 << 32);
        let raw = format!("BAA{big},CAA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_generated_column_overflow() {
        let max = vlq_unsigned(u64::from(u32::MAX));
        let raw = format!(",EA{max},EAC,FA,FA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_definition_index_overflow() {
        // The second range start adds i64::MAX to definition index 1.
        let max = vlq_signed(i64::MAX);
        let raw = format!(",ECAC,FA,ECA{max},FA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_name_index_overflow() {
        // The first scope moves the name index to 1, the second adds i64::MAX.
        let names = vec!["a".to_string(), "b".to_string()];
        let max = vlq_signed(i64::MAX);
        let raw = format!("BBAAC,CAA,BBAA{max},CAA");
        assert_eq!(decode_scopes(&raw, &names, 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn new_error_variants_display() {
        assert_eq!(
            ScopesError::ValueOverflow.to_string(),
            "scopes position or index is out of range"
        );
        assert_eq!(
            ScopesError::NestingTooDeep.to_string(),
            format!("scopes nest deeper than {MAX_NESTING_DEPTH} levels")
        );
    }

    #[test]
    fn decode_rejects_definition_index_above_u32() {
        let delta = vlq_signed(1 << 32);
        let raw = format!(",ECA{delta},FA");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_rejects_sub_range_variable_index_overflow() {
        // Each H item adds a large delta to the variable index of one range.
        let delta = vlq_unsigned(1 << 58);
        let item = format!("H{delta}");
        let raw = format!(",EAA,{},FA", vec![item.as_str(); 80].join(","));
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::ValueOverflow);
    }

    #[test]
    fn decode_accepts_original_scopes_at_the_nesting_limit() {
        let raw =
            [vec!["BAAA"; MAX_NESTING_DEPTH], vec!["CAA"; MAX_NESTING_DEPTH]].concat().join(",");
        let info = decode_scopes(&raw, &[], 1).unwrap();
        assert!(info.scopes[0].is_some());
    }

    #[test]
    fn decode_rejects_original_scopes_above_the_nesting_limit() {
        let depth = MAX_NESTING_DEPTH + 1;
        let raw = [vec!["BAAA"; depth], vec!["CAA"; depth]].concat().join(",");
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::NestingTooDeep);
    }

    #[test]
    fn decode_accepts_generated_ranges_at_the_nesting_limit() {
        let raw = format!(
            ",{}",
            [vec!["EAA"; MAX_NESTING_DEPTH], vec!["FA"; MAX_NESTING_DEPTH]].concat().join(",")
        );
        let info = decode_scopes(&raw, &[], 1).unwrap();
        assert_eq!(info.ranges.len(), 1);
    }

    #[test]
    fn decode_rejects_generated_ranges_above_the_nesting_limit() {
        let depth = MAX_NESTING_DEPTH + 1;
        let raw = format!(",{}", [vec!["EAA"; depth], vec!["FA"; depth]].concat().join(","));
        assert_eq!(decode_scopes(&raw, &[], 1).unwrap_err(), ScopesError::NestingTooDeep);
    }

    #[test]
    fn scope_trees_at_the_nesting_limit_fit_on_a_default_thread_stack() {
        const DEFAULT_THREAD_STACK: usize = 2 * 1024 * 1024;
        let raw = format!(
            "{},{}",
            [vec!["BAAA"; MAX_NESTING_DEPTH], vec!["CAA"; MAX_NESTING_DEPTH]].concat().join(","),
            [vec!["EAA"; MAX_NESTING_DEPTH], vec!["FA"; MAX_NESTING_DEPTH]].concat().join(",")
        );
        std::thread::Builder::new()
            .stack_size(DEFAULT_THREAD_STACK)
            .spawn(move || {
                let info = decode_scopes(&raw, &[], 1).unwrap();
                let mut names = Vec::new();
                let encoded = encode_scopes(&info, &mut names);
                let copy = info.clone();
                assert!(format!("{copy:?}").starts_with("ScopeInfo"));
                assert_eq!(decode_scopes(&encoded, &names, 1).unwrap(), copy);
            })
            .unwrap()
            .join()
            .unwrap();
    }
}
