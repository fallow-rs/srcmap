//! Scopes and variables decoder/encoder for source maps (ECMA-426).
//!
//! Implements the "Scopes" proposal for source maps, enabling debuggers to
//! reconstruct original scope trees, variable bindings, and inlined function
//! call sites from generated code.
//!
//! The crate reads and writes the wire layout of the
//! [draft specification](https://tc39.es/ecma426/branch/proposal-scopes/):
//! the `scopes` field is an array with one string for each source, and the
//! `ranges` field is an array of strings. [`decode_legacy_scopes`] reads the
//! older layout, which has one `scopes` string for the full map.
//!
//! # Examples
//!
//! ```
//! use srcmap_scopes::{
//!     decode_scopes, encode_scopes, Binding, GeneratedRange, OriginalScope,
//!     Position, ScopeDefinition, ScopeInfo,
//! };
//!
//! // Build scope info
//! let info = ScopeInfo {
//!     scopes: vec![vec![OriginalScope {
//!         start: Position { line: 0, column: 0 },
//!         end: Position { line: 5, column: 0 },
//!         name: None,
//!         kind: Some("global".to_string()),
//!         is_stack_frame: false,
//!         variables: vec!["x".to_string()],
//!         children: vec![],
//!     }]],
//!     ranges: vec![GeneratedRange {
//!         start: Position { line: 0, column: 0 },
//!         end: Position { line: 5, column: 0 },
//!         is_stack_frame: false,
//!         is_hidden: false,
//!         definition: Some(ScopeDefinition { source_index: 0, scope_index: 0 }),
//!         call_site: None,
//!         bindings: vec![Binding::Expression("_x".to_string())],
//!         children: vec![],
//!     }],
//! };
//!
//! // Encode
//! let mut names = vec![];
//! let encoded = encode_scopes(&info, &mut names);
//! assert_eq!(encoded.scopes.len(), 1);
//! assert_eq!(encoded.ranges.len(), 1);
//!
//! // Decode
//! let decoded = decode_scopes(&encoded.scopes, &encoded.ranges, &names, 1).unwrap();
//! assert_eq!(decoded, info);
//! ```

mod decode;
mod encode;

use std::fmt;

pub use decode::{decode_legacy_scopes, decode_scopes};
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
/// Tag `J`: a call site relative to the definition of a parent range.
const TAG_GENERATED_RANGE_RELATIVE_CALL_SITE: u64 = 0x9;

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

/// A reference from a generated range to an original scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeDefinition {
    /// Index of the source in `sources`, and in [`ScopeInfo::scopes`].
    pub source_index: u32,
    /// Index of the scope in the pre-order list of the scopes of that source.
    /// The first root scope of the source has index 0.
    pub scope_index: u32,
}

/// A generated range in the output code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedRange {
    pub start: Position,
    pub end: Position,
    /// Whether this range is a stack frame (function boundary).
    pub is_stack_frame: bool,
    /// Whether this stack frame should be hidden from traces.
    ///
    /// The wire format has this flag only for a range that is a stack frame.
    /// The encoder does not write it when `is_stack_frame` is `false`.
    pub is_hidden: bool,
    /// The original scope that this range belongs to.
    pub definition: Option<ScopeDefinition>,
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
    /// The root scopes of each source file (aligned with `sources`).
    /// An empty list means no scope info for that source file.
    pub scopes: Vec<Vec<OriginalScope>>,
    /// Top-level generated ranges for the output code.
    pub ranges: Vec<GeneratedRange>,
}

/// The `scopes` and `ranges` fields of a source map, as [`encode_scopes`]
/// writes them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedScopes {
    /// One entry for each source. `None` is a source without scopes, and it is
    /// `null` in the JSON.
    pub scopes: Vec<Option<String>>,
    /// The encoded generated ranges. The list is empty when the map has no
    /// generated ranges.
    pub ranges: Vec<String>,
}

impl ScopeInfo {
    /// Get the original scope that the `definition` of a generated range
    /// refers to.
    ///
    /// The scope index counts the scopes of one source in pre-order.
    pub fn original_scope_for_definition(
        &self,
        definition: ScopeDefinition,
    ) -> Option<&OriginalScope> {
        let roots = self.scopes.get(definition.source_index as usize)?;
        // Iterative pre-order traversal to avoid stack overflow on deeply nested scopes
        let mut stack: Vec<&OriginalScope> = roots.iter().rev().collect();
        let mut remaining = definition.scope_index;
        while let Some(node) = stack.pop() {
            if remaining == 0 {
                return Some(node);
            }
            remaining -= 1;
            // Push children in reverse order so leftmost is processed first
            stack.extend(node.children.iter().rev());
        }
        None
    }
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
    /// A relative call site (tag `J`) refers to a parent range that does not
    /// exist, or to a parent whose definition is not a decoded scope.
    UnresolvedCallSite,
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
            Self::UnresolvedCallSite => {
                write!(f, "relative call site has no parent range with a known scope")
            }
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

#[cfg(test)]
mod tests;
