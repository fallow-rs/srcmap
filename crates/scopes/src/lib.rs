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
}
