//! The `debugId` comment of the ECMA-426 debug ID proposal.

/// The number of lines at the end of a file that can hold the comment.
const TRAILING_LINES: usize = 5;

const JS_PREFIX: &str = "//# debugId=";
const CSS_PREFIX: &str = "/*# debugId=";
const CSS_SUFFIX: &str = "*/";

const UUID_LEN: usize = 36;
const UUID_HYPHENS: [usize; 4] = [8, 13, 18, 23];

/// Extract the debug ID from the `debugId` comment of a generated JavaScript file.
///
/// The function follows the
/// [debug ID proposal](https://github.com/tc39/ecma426/blob/main/proposals/debug-id.md):
///
/// - The comment has the form `//# debugId=<UUID>`.
/// - The comment must be in the last 5 lines of the file. A `sourceMappingURL`
///   comment can be above or below it.
/// - The comment must start the line, and no text can follow the UUID.
/// - The UUID has the format `85314830-023f-4cf1-a267-535f4e37bb17`. The
///   function accepts upper-case and lower-case hex digits.
///
/// A line ends at each JavaScript line terminator: LF, CR, CRLF, U+2028 and
/// U+2029. One line terminator at the end of the file does not start a line.
///
/// The function returns the UUID in lower case. It returns `None` when the last
/// 5 lines have no valid comment. When more than one line has a valid comment,
/// the function returns the ID of the last one.
///
/// For a CSS file, use [`extract_css_debug_id`].
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::extract_debug_id;
///
/// let js = "console.log(1);\n\
///           //# debugId=85314830-023F-4CF1-A267-535F4E37BB17\n\
///           //# sourceMappingURL=app.js.map\n";
/// assert_eq!(extract_debug_id(js).as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
///
/// assert_eq!(extract_debug_id("console.log(1);\n"), None);
/// ```
pub fn extract_debug_id(source: &str) -> Option<String> {
    find_in_trailing_lines(source, |line| line.strip_prefix(JS_PREFIX))
}

/// Extract the debug ID from the `debugId` comment of a generated CSS file.
///
/// The comment has the form `/*# debugId=<UUID> */`. The debug ID proposal
/// specifies the comment only for JavaScript, thus this form is an extension
/// that mirrors the CSS form of the `sourceMappingURL` comment. Spaces and
/// tabs between the UUID and `*/` are optional.
///
/// All other rules are the rules of [`extract_debug_id`]. A line ends at LF,
/// CR, CRLF, U+2028 and U+2029 here too.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::extract_css_debug_id;
///
/// let css = "body{color:red}\n/*# debugId=85314830-023f-4cf1-a267-535f4e37bb17 */\n";
/// assert_eq!(extract_css_debug_id(css).as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
/// ```
pub fn extract_css_debug_id(source: &str) -> Option<String> {
    find_in_trailing_lines(source, |line| {
        Some(line.strip_prefix(CSS_PREFIX)?.strip_suffix(CSS_SUFFIX)?.trim_end_matches([' ', '\t']))
    })
}

/// Returns the debug ID of the last line in the last `TRAILING_LINES` lines
/// for which `id_of_line` gives a canonical UUID.
fn find_in_trailing_lines<'a>(
    source: &'a str,
    id_of_line: impl Fn(&'a str) -> Option<&'a str>,
) -> Option<String> {
    let mut rest = strip_line_terminator(source).unwrap_or(source);
    for _ in 0..TRAILING_LINES {
        let (before, line) = split_last_line(rest);
        if let Some(id) = id_of_line(line).filter(|id| is_canonical_uuid(id)) {
            return Some(id.to_ascii_lowercase());
        }
        rest = before?;
    }
    None
}

const fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Removes one line terminator from the end of `text`. CRLF is one terminator.
fn strip_line_terminator(text: &str) -> Option<&str> {
    text.strip_suffix("\r\n").or_else(|| text.strip_suffix(is_line_terminator))
}

/// Splits `text` into the text before its last line, and that line. The first
/// part is `None` when `text` is one line.
fn split_last_line(text: &str) -> (Option<&str>, &str) {
    match text.rfind(is_line_terminator) {
        Some(index) => {
            let line_start = index + text[index..].chars().next().map_or(1, char::len_utf8);
            (strip_line_terminator(&text[..line_start]), &text[line_start..])
        }
        None => (None, text),
    }
}

fn is_canonical_uuid(id: &str) -> bool {
    id.len() == UUID_LEN
        && id.bytes().enumerate().all(|(index, byte)| {
            if UUID_HYPHENS.contains(&index) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
}
