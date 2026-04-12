//! JavaScript identifier validation utilities.
//!
//! Provides functions to check whether a string is a valid JavaScript
//! identifier and to extract the first identifier token from a source line.

/// Check if a string is a valid JavaScript identifier.
///
/// Follows the ECMAScript specification for `IdentifierName`:
/// - First character: `$`, `_`, ASCII letter, or Unicode ID_Start
/// - Subsequent characters: `$`, `_`, ASCII alphanumeric, `\u{200c}` (ZWNJ),
///   `\u{200d}` (ZWJ), or Unicode ID_Continue
pub fn is_valid_javascript_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !is_id_start(first) {
        return false;
    }
    chars.all(is_id_continue)
}

/// Extract the first valid JavaScript identifier from a source line.
///
/// Skips leading whitespace, then collects characters that form a valid
/// identifier. Returns `None` if no identifier is found.
pub fn get_javascript_token(source_line: &str) -> Option<&str> {
    let trimmed = source_line.trim_start();
    if trimmed.is_empty() {
        return None;
    }

    let mut chars = trimmed.chars();
    let first = chars.next()?;
    if !is_id_start(first) {
        return None;
    }

    let start = source_line.len() - trimmed.len();
    let mut end = start + first.len_utf8();

    for ch in chars {
        if !is_id_continue(ch) {
            break;
        }
        end += ch.len_utf8();
    }

    let token = &source_line[start..end];
    if token.is_empty() { None } else { Some(token) }
}

/// Check if a character can start a JavaScript identifier.
fn is_id_start(c: char) -> bool {
    c == '$' || c == '_' || c.is_ascii_alphabetic() || (!c.is_ascii() && c.is_alphabetic())
}

/// Check if a character can continue a JavaScript identifier.
fn is_id_continue(c: char) -> bool {
    c == '$'
        || c == '_'
        || c == '\u{200c}'
        || c == '\u{200d}'
        || c.is_ascii_alphanumeric()
        || (!c.is_ascii() && c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_identifiers() {
        assert!(is_valid_javascript_identifier("foo"));
        assert!(is_valid_javascript_identifier("_bar"));
        assert!(is_valid_javascript_identifier("$baz"));
        assert!(is_valid_javascript_identifier("camelCase"));
        assert!(is_valid_javascript_identifier("PascalCase"));
        assert!(is_valid_javascript_identifier("snake_case"));
        assert!(is_valid_javascript_identifier("x"));
        assert!(is_valid_javascript_identifier("_"));
        assert!(is_valid_javascript_identifier("$"));
        assert!(is_valid_javascript_identifier("_$mixed123"));
        assert!(is_valid_javascript_identifier("a1"));
    }

    #[test]
    fn test_invalid_identifiers() {
        assert!(!is_valid_javascript_identifier(""));
        assert!(!is_valid_javascript_identifier("123"));
        assert!(!is_valid_javascript_identifier("1abc"));
        assert!(!is_valid_javascript_identifier("-foo"));
        assert!(!is_valid_javascript_identifier("foo bar"));
        assert!(!is_valid_javascript_identifier("foo-bar"));
        assert!(!is_valid_javascript_identifier(".foo"));
    }

    #[test]
    fn test_unicode_identifiers() {
        // CJK characters
        assert!(is_valid_javascript_identifier("\u{4e16}\u{754c}"));
        // Accented characters
        assert!(is_valid_javascript_identifier("\u{00e9}l\u{00e8}ve"));
        // Cyrillic
        assert!(is_valid_javascript_identifier("\u{0442}\u{0435}\u{0441}\u{0442}"));
    }

    #[test]
    fn test_zwnj_zwj() {
        // ZWNJ and ZWJ are valid as continuation characters
        assert!(is_valid_javascript_identifier("a\u{200c}b"));
        assert!(is_valid_javascript_identifier("a\u{200d}b"));
        // But not as start characters
        assert!(!is_valid_javascript_identifier("\u{200c}abc"));
        assert!(!is_valid_javascript_identifier("\u{200d}abc"));
    }

    #[test]
    fn test_get_javascript_token_basic() {
        assert_eq!(get_javascript_token("hello world"), Some("hello"));
        assert_eq!(get_javascript_token("  foo(bar)"), Some("foo"));
        assert_eq!(get_javascript_token("  _private"), Some("_private"));
        assert_eq!(get_javascript_token("  $jquery"), Some("$jquery"));
    }

    #[test]
    fn test_get_javascript_token_whitespace() {
        assert_eq!(get_javascript_token("   abc123"), Some("abc123"));
        assert_eq!(get_javascript_token("\t\ttab"), Some("tab"));
        assert_eq!(get_javascript_token("noSpace"), Some("noSpace"));
    }

    #[test]
    fn test_get_javascript_token_none() {
        assert_eq!(get_javascript_token(""), None);
        assert_eq!(get_javascript_token("   "), None);
        assert_eq!(get_javascript_token("  123abc"), None);
        assert_eq!(get_javascript_token("  .foo"), None);
        assert_eq!(get_javascript_token("  (bar)"), None);
    }

    #[test]
    fn test_get_javascript_token_stops_at_non_ident() {
        assert_eq!(get_javascript_token("foo.bar"), Some("foo"));
        assert_eq!(get_javascript_token("func("), Some("func"));
        assert_eq!(get_javascript_token("x = 5"), Some("x"));
        assert_eq!(get_javascript_token("arr[0]"), Some("arr"));
    }

    #[test]
    fn test_get_javascript_token_unicode() {
        assert_eq!(get_javascript_token("  \u{4e16}\u{754c}!"), Some("\u{4e16}\u{754c}"));
    }
}
