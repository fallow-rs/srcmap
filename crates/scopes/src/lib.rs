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
//! let mut names = vec!["global".to_string(), "x".to_string(), "_x".to_string()];
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

use std::collections::HashMap;
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
    pub definition: Option<usize>,
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
    pub fn original_scope_for_definition(&self, definition: usize) -> Option<&OriginalScope> {
        let mut count = 0;
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
    target: usize,
    count: &mut usize,
) -> Option<&'a OriginalScope> {
    if *count == target {
        return Some(scope);
    }
    *count += 1;
    for child in &scope.children {
        if let Some(result) = find_nth_scope(child, target, count) {
            return Some(result);
        }
    }
    None
}

// ── Errors ───────────────────────────────────────────────────────

/// Errors during scopes decoding.
#[derive(Debug)]
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
    if index < 0 || index as usize >= names.len() {
        return Err(ScopesError::InvalidNameIndex(index));
    }
    Ok(names[index as usize].clone())
}

/// Resolve a 1-based binding name (0 = unavailable).
fn resolve_binding(names: &[String], index: u64) -> Result<Option<String>, ScopesError> {
    if index == 0 {
        return Ok(None);
    }
    let actual = (index - 1) as usize;
    if actual >= names.len() {
        return Err(ScopesError::InvalidNameIndex(index as i64));
    }
    Ok(Some(names[actual].clone()))
}

/// Look up or insert a name, returning its 0-based index.
fn resolve_or_add_name(
    name: &str,
    names: &mut Vec<String>,
    name_map: &mut HashMap<String, u32>,
) -> u32 {
    if let Some(&idx) = name_map.get(name) {
        return idx;
    }
    let idx = names.len() as u32;
    names.push(name.to_string());
    name_map.insert(name.to_string(), idx);
    idx
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_scopes() {
        let info = decode_scopes("", &[], 0).unwrap();
        assert!(info.scopes.is_empty());
        assert!(info.ranges.is_empty());
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
                end: Position {
                    line: 10,
                    column: 0,
                },
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
        assert_eq!(
            scope.end,
            Position {
                line: 10,
                column: 0
            }
        );
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
                end: Position {
                    line: 10,
                    column: 1,
                },
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
                end: Position {
                    line: 10,
                    column: 0,
                },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
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
        assert_eq!(
            range.end,
            Position {
                line: 10,
                column: 0
            }
        );
        assert_eq!(range.definition, Some(0));
        assert_eq!(range.bindings, vec![Binding::Expression("_x".to_string())]);
    }

    #[test]
    fn nested_ranges_with_inlining() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![OriginalScope {
                    start: Position { line: 1, column: 0 },
                    end: Position { line: 5, column: 1 },
                    name: Some("fn1".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec!["a".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("_x".to_string())],
                children: vec![GeneratedRange {
                    start: Position { line: 6, column: 0 },
                    end: Position {
                        line: 8,
                        column: 20,
                    },
                    is_stack_frame: true,
                    is_hidden: false,
                    definition: Some(1),
                    call_site: Some(CallSite {
                        source_index: 0,
                        line: 7,
                        column: 0,
                    }),
                    bindings: vec![Binding::Expression("\"hello\"".to_string())],
                    children: vec![],
                }],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.ranges.len(), 1);
        let outer = &decoded.ranges[0];
        assert_eq!(outer.children.len(), 1);
        let inner = &outer.children[0];
        assert!(inner.is_stack_frame);
        assert_eq!(inner.definition, Some(1));
        assert_eq!(
            inner.call_site,
            Some(CallSite {
                source_index: 0,
                line: 7,
                column: 0,
            })
        );
        assert_eq!(
            inner.bindings,
            vec![Binding::Expression("\"hello\"".to_string())]
        );
    }

    #[test]
    fn unavailable_bindings() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["a".to_string(), "b".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("_a".to_string()), Binding::Unavailable],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(
            decoded.ranges[0].bindings,
            vec![Binding::Expression("_a".to_string()), Binding::Unavailable,]
        );
    }

    #[test]
    fn sub_range_bindings_roundtrip() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string(), "y".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![
                    Binding::SubRanges(vec![
                        SubRangeBinding {
                            expression: Some("a".to_string()),
                            from: Position { line: 0, column: 0 },
                        },
                        SubRangeBinding {
                            expression: Some("b".to_string()),
                            from: Position { line: 5, column: 0 },
                        },
                        SubRangeBinding {
                            expression: None,
                            from: Position {
                                line: 10,
                                column: 0,
                            },
                        },
                    ]),
                    Binding::Expression("_y".to_string()),
                ],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let bindings = &decoded.ranges[0].bindings;
        assert_eq!(bindings.len(), 2);

        match &bindings[0] {
            Binding::SubRanges(subs) => {
                assert_eq!(subs.len(), 3);
                assert_eq!(subs[0].expression.as_deref(), Some("a"));
                assert_eq!(subs[0].from, Position { line: 0, column: 0 });
                assert_eq!(subs[1].expression.as_deref(), Some("b"));
                assert_eq!(subs[1].from, Position { line: 5, column: 0 });
                assert_eq!(subs[2].expression, None);
                assert_eq!(
                    subs[2].from,
                    Position {
                        line: 10,
                        column: 0,
                    }
                );
            }
            other => panic!("expected SubRanges, got {other:?}"),
        }
        assert_eq!(bindings[1], Binding::Expression("_y".to_string()));
    }

    #[test]
    fn hidden_range() {
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
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 5, column: 0 },
                is_stack_frame: true,
                is_hidden: true,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert!(decoded.ranges[0].is_stack_frame);
        assert!(decoded.ranges[0].is_hidden);
    }

    #[test]
    fn definition_resolution() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
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
    fn scopes_only_no_ranges() {
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
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.scopes.len(), 1);
        assert!(decoded.scopes[0].is_some());
        assert!(decoded.ranges.is_empty());
    }

    #[test]
    fn ranges_only_no_scopes() {
        let info = ScopeInfo {
            scopes: vec![None],
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
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.scopes.len(), 1);
        assert!(decoded.scopes[0].is_none());
        assert_eq!(decoded.ranges.len(), 1);
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
    fn scopes_error_display() {
        let err = ScopesError::UnmatchedScopeEnd;
        assert_eq!(err.to_string(), "scope end without matching start");

        let err = ScopesError::UnclosedScope;
        assert_eq!(err.to_string(), "scope opened but never closed");

        let err = ScopesError::UnmatchedRangeEnd;
        assert_eq!(err.to_string(), "range end without matching start");

        let err = ScopesError::UnclosedRange;
        assert_eq!(err.to_string(), "range opened but never closed");

        let err = ScopesError::InvalidNameIndex(42);
        assert_eq!(err.to_string(), "invalid name index: 42");

        let vlq_err = srcmap_codec::DecodeError::UnexpectedEof { offset: 5 };
        let err = ScopesError::Vlq(vlq_err);
        assert!(err.to_string().contains("VLQ decode error"));
    }

    #[test]
    fn scopes_error_from_decode_error() {
        let vlq_err = srcmap_codec::DecodeError::UnexpectedEof { offset: 0 };
        let err: ScopesError = vlq_err.into();
        assert!(matches!(err, ScopesError::Vlq(_)));
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
    fn scope_same_line_end() {
        // Scope that starts and ends on the same line (column relative)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 5, column: 10 },
                end: Position { line: 5, column: 30 },
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
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.start, Position { line: 5, column: 10 });
        assert_eq!(scope.end, Position { line: 5, column: 30 });
    }

    #[test]
    fn range_same_line() {
        // Range that starts and ends on the same line
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 10, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 3, column: 5 },
                end: Position { line: 3, column: 25 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let range = &decoded.ranges[0];
        assert_eq!(range.start, Position { line: 3, column: 5 });
        assert_eq!(range.end, Position { line: 3, column: 25 });
    }

    #[test]
    fn scopes_first_empty_second_populated() {
        let info = ScopeInfo {
            scopes: vec![
                None, // First source has no scopes
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
        let decoded = decode_scopes(&encoded, &names, 2).unwrap();

        assert!(decoded.scopes[0].is_none());
        assert!(decoded.scopes[1].is_some());
    }

    #[test]
    fn ranges_only_no_scopes_multi_source() {
        let info = ScopeInfo {
            scopes: vec![None, None],
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
        let decoded = decode_scopes(&encoded, &names, 2).unwrap();

        assert_eq!(decoded.scopes.len(), 2);
        assert!(decoded.scopes[0].is_none());
        assert!(decoded.scopes[1].is_none());
        assert_eq!(decoded.ranges.len(), 1);
    }

    #[test]
    fn range_no_definition_explicit() {
        let info = ScopeInfo {
            scopes: vec![None],
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
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.ranges[0].definition, None);
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
    fn call_site_with_nonzero_values() {
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position { line: 20, column: 0 },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position { line: 20, column: 0 },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: Some(CallSite {
                    source_index: 2,
                    line: 15,
                    column: 8,
                }),
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let cs = decoded.ranges[0].call_site.as_ref().unwrap();
        assert_eq!(cs.source_index, 2);
        assert_eq!(cs.line, 15);
        assert_eq!(cs.column, 8);
    }

    // ── Additional coverage tests ────────────────────────────────

    #[test]
    fn scope_with_name_and_kind_roundtrip() {
        // Exercises OS_FLAG_HAS_NAME + OS_FLAG_HAS_KIND together
        // Covers decode.rs lines 139, 141, 143, 151, 159, 161
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 2, column: 4 },
                end: Position {
                    line: 15,
                    column: 1,
                },
                name: Some("myFunc".to_string()),
                kind: Some("function".to_string()),
                is_stack_frame: true,
                variables: vec!["arg1".to_string(), "arg2".to_string()],
                children: vec![OriginalScope {
                    start: Position { line: 3, column: 8 },
                    end: Position {
                        line: 14,
                        column: 5,
                    },
                    name: Some("innerBlock".to_string()),
                    kind: Some("block".to_string()),
                    is_stack_frame: false,
                    variables: vec!["tmp".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.name.as_deref(), Some("myFunc"));
        assert_eq!(scope.kind.as_deref(), Some("function"));
        assert!(scope.is_stack_frame);
        assert_eq!(scope.variables, vec!["arg1", "arg2"]);

        let child = &scope.children[0];
        assert_eq!(child.name.as_deref(), Some("innerBlock"));
        assert_eq!(child.kind.as_deref(), Some("block"));
        assert!(!child.is_stack_frame);
        assert_eq!(child.variables, vec!["tmp"]);
    }

    #[test]
    fn range_end_multiline_2vlq() {
        // Range where end is on a different line than start, producing a
        // 2-VLQ range end (line_delta + column).
        // Covers decode.rs lines 282, 286, 288 (TAG_GENERATED_RANGE_END with 2 VLQs)
        let info = ScopeInfo {
            scopes: vec![],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 7,
                    column: 15,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: None,
                call_site: None,
                bindings: vec![],
                children: vec![GeneratedRange {
                    start: Position { line: 1, column: 5 },
                    end: Position {
                        line: 4,
                        column: 10,
                    },
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
        assert_eq!(
            outer.end,
            Position {
                line: 7,
                column: 15
            }
        );
        let inner = &outer.children[0];
        assert_eq!(inner.start, Position { line: 1, column: 5 });
        assert_eq!(
            inner.end,
            Position {
                line: 4,
                column: 10
            }
        );
    }

    #[test]
    fn binding_unavailable_roundtrip() {
        // Exercises Binding::Unavailable (binding idx = 0) path
        // Covers decode.rs lines 333, 340 (TAG_GENERATED_RANGE_BINDINGS with idx=0)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![
                    "a".to_string(),
                    "b".to_string(),
                    "c".to_string(),
                ],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![
                    Binding::Unavailable,
                    Binding::Expression("_b".to_string()),
                    Binding::Unavailable,
                ],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        assert_eq!(decoded.ranges[0].bindings.len(), 3);
        assert_eq!(decoded.ranges[0].bindings[0], Binding::Unavailable);
        assert_eq!(
            decoded.ranges[0].bindings[1],
            Binding::Expression("_b".to_string())
        );
        assert_eq!(decoded.ranges[0].bindings[2], Binding::Unavailable);
    }

    #[test]
    fn sub_range_with_none_expression() {
        // Sub-range bindings where a sub-range has expression = None (Unavailable)
        // Covers encode.rs lines 267, 271 (None expression → emit 0)
        // and decode.rs lines 353, 354, 357, 364 (sub-range reading)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
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
                        expression: None,
                        from: Position { line: 5, column: 0 },
                    },
                    SubRangeBinding {
                        expression: Some("c".to_string()),
                        from: Position {
                            line: 10,
                            column: 0,
                        },
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
                assert_eq!(subs.len(), 3);
                assert_eq!(subs[0].expression.as_deref(), Some("a"));
                assert_eq!(subs[1].expression, None);
                assert_eq!(subs[2].expression.as_deref(), Some("c"));
            }
            other => panic!("expected SubRanges, got {other:?}"),
        }
    }

    #[test]
    fn sub_range_multiple_variables() {
        // Multiple variables each with sub-ranges, exercises the H tag
        // encode/decode with h_first handling (encode.rs line 290)
        // and decode.rs lines 353-375 (TAG_GENERATED_RANGE_SUB_RANGE_BINDINGS)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 30,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["x".to_string(), "y".to_string(), "z".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 30,
                    column: 0,
                },
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
                            from: Position {
                                line: 10,
                                column: 0,
                            },
                        },
                    ]),
                    // Variable 1 (y): simple binding
                    Binding::Expression("_y".to_string()),
                    // Variable 2 (z): sub-ranges (second H item, exercises h_first=false)
                    Binding::SubRanges(vec![
                        SubRangeBinding {
                            expression: Some("_z1".to_string()),
                            from: Position { line: 0, column: 0 },
                        },
                        SubRangeBinding {
                            expression: None,
                            from: Position {
                                line: 15,
                                column: 5,
                            },
                        },
                        SubRangeBinding {
                            expression: Some("_z3".to_string()),
                            from: Position {
                                line: 20,
                                column: 0,
                            },
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
                assert_eq!(
                    subs[1].from,
                    Position {
                        line: 10,
                        column: 0
                    }
                );
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
                assert_eq!(
                    subs[1].from,
                    Position {
                        line: 15,
                        column: 5
                    }
                );
                assert_eq!(subs[2].expression.as_deref(), Some("_z3"));
            }
            other => panic!("expected SubRanges for z, got {other:?}"),
        }
    }

    #[test]
    fn call_site_on_standalone_range() {
        // Range with call_site but also a definition (typical inlining pattern).
        // Exercises decode.rs lines 380-388 (TAG_GENERATED_RANGE_CALL_SITE)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 30,
                    column: 0,
                },
                name: None,
                kind: Some("global".to_string()),
                is_stack_frame: false,
                variables: vec![],
                children: vec![OriginalScope {
                    start: Position { line: 5, column: 0 },
                    end: Position {
                        line: 10,
                        column: 1,
                    },
                    name: Some("inlined".to_string()),
                    kind: Some("function".to_string()),
                    is_stack_frame: true,
                    variables: vec!["p".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 30,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![GeneratedRange {
                    start: Position {
                        line: 12,
                        column: 0,
                    },
                    end: Position {
                        line: 18,
                        column: 0,
                    },
                    is_stack_frame: true,
                    is_hidden: false,
                    definition: Some(1),
                    call_site: Some(CallSite {
                        source_index: 0,
                        line: 20,
                        column: 4,
                    }),
                    bindings: vec![Binding::Expression("arg0".to_string())],
                    children: vec![],
                }],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let inner = &decoded.ranges[0].children[0];
        assert!(inner.is_stack_frame);
        assert_eq!(inner.definition, Some(1));
        let cs = inner.call_site.as_ref().unwrap();
        assert_eq!(cs.source_index, 0);
        assert_eq!(cs.line, 20);
        assert_eq!(cs.column, 4);
        assert_eq!(
            inner.bindings,
            vec![Binding::Expression("arg0".to_string())]
        );
    }

    #[test]
    fn unknown_tag_skipped() {
        // Manually craft a string with an unknown tag (e.g., tag 0x9) followed
        // by some VLQs, then a valid scope.
        // This exercises decode.rs lines 393, 394 (unknown tag skip).
        //
        // First encode a simple scope, then prepend an unknown tag item.
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
    fn first_source_none_exercises_empty_path() {
        // First source has no scopes (None), second has scopes.
        // This exercises encode.rs line 89 (first_item && scope.is_none())
        // and decode.rs lines 124, 129 (empty item + skip_comma)
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
        // Parent scope ends on the same line where child scope ended.
        // This exercises the column-relative path in TAG_ORIGINAL_SCOPE_END
        // (decode.rs lines 183, 186, 188 where line_delta = 0)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 10,
                    column: 20,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![OriginalScope {
                    start: Position { line: 5, column: 0 },
                    end: Position {
                        line: 10,
                        column: 10,
                    },
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
        assert_eq!(
            scope.end,
            Position {
                line: 10,
                column: 20
            }
        );
        assert_eq!(
            scope.children[0].end,
            Position {
                line: 10,
                column: 10
            }
        );
    }

    #[test]
    fn generated_range_has_line_flag() {
        // Range starting on a non-zero line (exercises GR_FLAG_HAS_LINE)
        // Covers decode.rs lines 232, 233 (has_line flag, read line delta)
        // and encode.rs line_delta != 0 path
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
                    start: Position {
                        line: 3,
                        column: 10,
                    },
                    end: Position {
                        line: 8,
                        column: 20,
                    },
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
        assert_eq!(
            decoded.ranges[0].start,
            Position { line: 0, column: 5 }
        );
        assert_eq!(
            decoded.ranges[0].end,
            Position {
                line: 0,
                column: 50
            }
        );
        assert_eq!(
            decoded.ranges[1].start,
            Position {
                line: 3,
                column: 10
            }
        );
        assert_eq!(
            decoded.ranges[1].end,
            Position {
                line: 8,
                column: 20
            }
        );
    }

    #[test]
    fn scope_variables_decode_path() {
        // Scope with variables on a child to exercise TAG_ORIGINAL_SCOPE_VARIABLES
        // decode path (decode.rs lines 221, 223, 225)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![
                    "alpha".to_string(),
                    "beta".to_string(),
                    "gamma".to_string(),
                ],
                children: vec![OriginalScope {
                    start: Position { line: 2, column: 0 },
                    end: Position {
                        line: 18,
                        column: 0,
                    },
                    name: None,
                    kind: None,
                    is_stack_frame: false,
                    variables: vec!["delta".to_string(), "epsilon".to_string()],
                    children: vec![],
                }],
            })],
            ranges: vec![],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let scope = decoded.scopes[0].as_ref().unwrap();
        assert_eq!(scope.variables, vec!["alpha", "beta", "gamma"]);
        assert_eq!(scope.children[0].variables, vec!["delta", "epsilon"]);
    }

    #[test]
    fn sub_range_first_expression_none() {
        // Sub-ranges where the first expression is None (Unavailable at range start).
        // This exercises encode.rs line 267 (first sub expression is None → emit 0 in G)
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec!["v".to_string()],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 20,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::SubRanges(vec![
                    SubRangeBinding {
                        expression: None,
                        from: Position { line: 0, column: 0 },
                    },
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
                    end: Position {
                        line: 50,
                        column: 0,
                    },
                    name: None,
                    kind: Some("module".to_string()),
                    is_stack_frame: false,
                    variables: vec!["exports".to_string()],
                    children: vec![
                        OriginalScope {
                            start: Position { line: 2, column: 0 },
                            end: Position {
                                line: 20,
                                column: 1,
                            },
                            name: Some("add".to_string()),
                            kind: Some("function".to_string()),
                            is_stack_frame: true,
                            variables: vec!["a".to_string(), "b".to_string()],
                            children: vec![],
                        },
                        OriginalScope {
                            start: Position {
                                line: 22,
                                column: 0,
                            },
                            end: Position {
                                line: 40,
                                column: 1,
                            },
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
                end: Position {
                    line: 25,
                    column: 0,
                },
                is_stack_frame: false,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![Binding::Expression("module.exports".to_string())],
                children: vec![
                    GeneratedRange {
                        start: Position { line: 1, column: 0 },
                        end: Position {
                            line: 10,
                            column: 0,
                        },
                        is_stack_frame: true,
                        is_hidden: false,
                        definition: Some(1),
                        call_site: Some(CallSite {
                            source_index: 0,
                            line: 45,
                            column: 2,
                        }),
                        bindings: vec![
                            Binding::Expression("_a".to_string()),
                            Binding::Unavailable,
                        ],
                        children: vec![],
                    },
                    GeneratedRange {
                        start: Position {
                            line: 12,
                            column: 0,
                        },
                        end: Position {
                            line: 20,
                            column: 0,
                        },
                        is_stack_frame: true,
                        is_hidden: true,
                        definition: Some(2),
                        call_site: Some(CallSite {
                            source_index: 0,
                            line: 46,
                            column: 0,
                        }),
                        bindings: vec![
                            Binding::SubRanges(vec![
                                SubRangeBinding {
                                    expression: Some("p1".to_string()),
                                    from: Position {
                                        line: 12,
                                        column: 0,
                                    },
                                },
                                SubRangeBinding {
                                    expression: Some("p2".to_string()),
                                    from: Position {
                                        line: 16,
                                        column: 0,
                                    },
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
        assert_eq!(
            add_range.call_site,
            Some(CallSite {
                source_index: 0,
                line: 45,
                column: 2
            })
        );
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
        assert_eq!(
            mul_range.bindings[1],
            Binding::Expression("_y".to_string())
        );
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
                end: Position {
                    line: 10,
                    column: 0,
                },
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
    fn generated_range_with_definition_on_nonzero_line() {
        // Exercises GR_FLAG_HAS_LINE | GR_FLAG_HAS_DEFINITION together
        // Covers decode.rs lines 238, 241, 247, 255
        let info = ScopeInfo {
            scopes: vec![Some(OriginalScope {
                start: Position { line: 0, column: 0 },
                end: Position {
                    line: 50,
                    column: 0,
                },
                name: None,
                kind: None,
                is_stack_frame: false,
                variables: vec![],
                children: vec![],
            })],
            ranges: vec![GeneratedRange {
                start: Position {
                    line: 10,
                    column: 5,
                },
                end: Position {
                    line: 40,
                    column: 0,
                },
                is_stack_frame: true,
                is_hidden: false,
                definition: Some(0),
                call_site: None,
                bindings: vec![],
                children: vec![],
            }],
        };

        let mut names = vec![];
        let encoded = encode_scopes(&info, &mut names);
        let decoded = decode_scopes(&encoded, &names, 1).unwrap();

        let range = &decoded.ranges[0];
        assert_eq!(
            range.start,
            Position {
                line: 10,
                column: 5
            }
        );
        assert_eq!(
            range.end,
            Position {
                line: 40,
                column: 0
            }
        );
        assert!(range.is_stack_frame);
        assert_eq!(range.definition, Some(0));
    }

    // ── Error path tests ───────────────────────────────────────────

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
}
