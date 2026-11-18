//! The `debugId` comment of the ECMA-426 debug ID proposal.

/// The number of lines at the end of a file that can hold the comment.
const TRAILING_LINES: usize = 5;

const JS_PREFIX: &str = "//# debugId=";
const CSS_PREFIX: &str = "/*# debugId=";
const CSS_SUFFIX: &str = "*/";

const UUID_LEN: usize = 36;
const UUID_HYPHENS: [usize; 4] = [8, 13, 18, 23];

/// Extract the debug ID from the `debugId` comment of a generated file.
///
/// The function follows the
/// [debug ID proposal](https://github.com/tc39/ecma426/blob/main/proposals/debug-id.md):
///
/// - The JavaScript form is `//# debugId=<UUID>`.
/// - The comment must be in the last 5 lines of the file. A `sourceMappingURL`
///   comment can be above or below it.
/// - The comment must start the line, and no text can follow the UUID.
/// - The UUID has the format `85314830-023f-4cf1-a267-535f4e37bb17`. The
///   function accepts upper-case and lower-case hex digits.
///
/// The function also accepts the CSS form `/*# debugId=<UUID> */`. The proposal
/// specifies the comment only for JavaScript, thus the CSS form is an extension
/// that mirrors the CSS form of the `sourceMappingURL` comment.
///
/// The function returns the UUID in lower case. It returns `None` when the last
/// 5 lines have no valid comment. When more than one line has a valid comment,
/// the function returns the ID of the last one.
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
/// let css = "body{color:red}\n/*# debugId=85314830-023f-4cf1-a267-535f4e37bb17 */\n";
/// assert_eq!(extract_debug_id(css).as_deref(), Some("85314830-023f-4cf1-a267-535f4e37bb17"));
///
/// assert_eq!(extract_debug_id("console.log(1);\n"), None);
/// ```
pub fn extract_debug_id(source: &str) -> Option<String> {
    source.lines().rev().take(TRAILING_LINES).find_map(debug_id_of_line)
}

fn debug_id_of_line(line: &str) -> Option<String> {
    let id = if let Some(rest) = line.strip_prefix(JS_PREFIX) {
        rest
    } else {
        line.strip_prefix(CSS_PREFIX)?.strip_suffix(CSS_SUFFIX)?.trim_end_matches([' ', '\t'])
    };
    is_canonical_uuid(id).then(|| id.to_ascii_lowercase())
}

fn is_canonical_uuid(id: &str) -> bool {
    id.len() == UUID_LEN
        && id.bytes().enumerate().all(|(index, byte)| {
            if UUID_HYPHENS.contains(&index) { byte == b'-' } else { byte.is_ascii_hexdigit() }
        })
}
