//! Utility functions for source map path resolution, validation, rewriting, and encoding.

use std::path::{Path, PathBuf};

use crate::{GeneratedLocation, Mapping, OriginalLocation, ParseError, SourceMap};

// ── Path utilities (gap #10) ─────────────────────────────────────

/// Find the longest common directory prefix among absolute file paths.
///
/// Only considers absolute paths (starting with `/`). Splits by `/` and finds
/// common path components. Returns the common prefix with a trailing `/`.
/// Returns `None` if no common prefix exists or fewer than 2 paths are provided.
pub fn find_common_prefix<'a>(paths: impl Iterator<Item = &'a str>) -> Option<String> {
    let abs_paths: Vec<&str> = paths.filter(|p| p.starts_with('/')).collect();
    if abs_paths.len() < 2 {
        return None;
    }

    // Split into components and exclude the last component (filename) from each path
    let first_all: Vec<&str> = abs_paths[0].split('/').collect();
    let first_dir = &first_all[..first_all.len().saturating_sub(1)];
    let mut common_len = first_dir.len();

    for path in &abs_paths[1..] {
        let components: Vec<&str> = path.split('/').collect();
        let dir = &components[..components.len().saturating_sub(1)];
        let mut match_len = 0;
        for (a, b) in first_dir.iter().zip(dir.iter()) {
            if a != b {
                break;
            }
            match_len += 1;
        }
        common_len = common_len.min(match_len);
    }

    // Must have at least the root component ("") plus one directory
    if common_len < 2 {
        return None;
    }

    let prefix = first_dir[..common_len].join("/");
    if prefix.is_empty() || prefix == "/" {
        return None;
    }

    Some(format!("{prefix}/"))
}

/// Compute the relative path from `base` to `target`.
///
/// Both paths should be absolute or relative to the same root.
/// Uses `../` for parent directory traversal.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::utils::make_relative_path;
/// assert_eq!(make_relative_path("/a/b/c.js", "/a/d/e.js"), "../d/e.js");
/// ```
pub fn make_relative_path(base: &str, target: &str) -> String {
    if base == target {
        return ".".to_string();
    }

    let base_parts: Vec<&str> = base.split('/').collect();
    let target_parts: Vec<&str> = target.split('/').collect();

    // Remove the filename from the base (last component)
    let base_dir = &base_parts[..base_parts.len().saturating_sub(1)];
    let target_dir = &target_parts[..target_parts.len().saturating_sub(1)];
    let target_file = target_parts.last().unwrap_or(&"");

    // Find common prefix length
    let mut common = 0;
    for (a, b) in base_dir.iter().zip(target_dir.iter()) {
        if a != b {
            break;
        }
        common += 1;
    }

    let ups = base_dir.len() - common;
    let mut result = String::new();

    for _ in 0..ups {
        result.push_str("../");
    }

    for part in &target_dir[common..] {
        result.push_str(part);
        result.push('/');
    }

    result.push_str(target_file);

    if result.is_empty() {
        ".".to_string()
    } else {
        result
    }
}

// ── Source map validation (gap #6) ───────────────────────────────

/// Quick check if a JSON string looks like a valid source map.
///
/// Performs a lightweight structural check without fully parsing the source map.
/// Returns `true` if the JSON contains either:
/// - `version` + `mappings` + at least one of `sources`, `names`, `sourceRoot`, `sourcesContent`
/// - OR a `sections` field (indexed source map)
pub fn is_sourcemap(json: &str) -> bool {
    let Ok(val) = serde_json::from_str::<serde_json::Value>(json) else {
        return false;
    };

    let Some(obj) = val.as_object() else {
        return false;
    };

    // Indexed source map
    if obj.contains_key("sections") {
        return true;
    }

    // Regular source map: needs version + mappings + at least one source-related field
    let has_version = obj.contains_key("version");
    let has_mappings = obj.contains_key("mappings");
    let has_source_field = obj.contains_key("sources")
        || obj.contains_key("names")
        || obj.contains_key("sourceRoot")
        || obj.contains_key("sourcesContent");

    has_version && has_mappings && has_source_field
}

// ── URL resolution (gap #5) ──────────────────────────────────────

/// Resolve a relative `sourceMappingURL` against the minified file's URL.
///
/// - If `source_map_ref` is already absolute (starts with `http://`, `https://`, or `/`),
///   returns it as-is.
/// - If `source_map_ref` starts with `data:`, returns `None` (inline maps).
/// - Otherwise, replaces the filename portion of `minified_url` with `source_map_ref`
///   and handles `../` traversal.
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::utils::resolve_source_map_url;
/// let url = resolve_source_map_url("https://example.com/js/app.js", "app.js.map");
/// assert_eq!(url, Some("https://example.com/js/app.js.map".to_string()));
/// ```
pub fn resolve_source_map_url(minified_url: &str, source_map_ref: &str) -> Option<String> {
    // Inline data URLs don't need resolution
    if source_map_ref.starts_with("data:") {
        return None;
    }

    // Already absolute
    if source_map_ref.starts_with("http://")
        || source_map_ref.starts_with("https://")
        || source_map_ref.starts_with('/')
    {
        return Some(source_map_ref.to_string());
    }

    // Find the base directory of the minified URL
    if let Some(last_slash) = minified_url.rfind('/') {
        let base = &minified_url[..=last_slash];
        let combined = format!("{base}{source_map_ref}");
        Some(normalize_path_components(&combined))
    } else {
        // No directory component in the URL
        Some(source_map_ref.to_string())
    }
}

/// Resolve a source map reference against a filesystem path.
///
/// Uses the parent directory of `minified_path` as the base, joins with `source_map_ref`,
/// and normalizes `..` components.
pub fn resolve_source_map_path(minified_path: &Path, source_map_ref: &str) -> Option<PathBuf> {
    let parent = minified_path.parent()?;
    let joined = parent.join(source_map_ref);

    // Normalize the path (resolve .. components without requiring the path to exist)
    Some(normalize_pathbuf(&joined))
}

/// Normalize `..` and `.` components in a URL path string.
fn normalize_path_components(url: &str) -> String {
    // Split off the protocol+host if present
    let (prefix, path) = if let Some(idx) = url.find("://") {
        let after_proto = &url[idx + 3..];
        if let Some(slash_idx) = after_proto.find('/') {
            let split_at = idx + 3 + slash_idx;
            (&url[..split_at], &url[split_at..])
        } else {
            return url.to_string();
        }
    } else {
        ("", url)
    };

    let mut segments: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            ".." => {
                // Never pop past the root empty segment (leading `/`)
                if segments.len() > 1 {
                    segments.pop();
                }
            }
            "." | "" if !segments.is_empty() => {
                // skip `.` and empty segments (from double slashes), except the leading empty
            }
            _ => {
                segments.push(segment);
            }
        }
    }

    let normalized = segments.join("/");
    format!("{prefix}{normalized}")
}

/// Normalize a `PathBuf` by resolving `..` and `.` without filesystem access.
fn normalize_pathbuf(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                components.pop();
            }
            std::path::Component::CurDir => {}
            _ => {
                components.push(component);
            }
        }
    }
    components.iter().collect()
}

// ── Data URL encoding (gap #4) ───────────────────────────────────

const BASE64_CHARS: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Convert a source map JSON string to a `data:` URL.
///
/// Format: `data:application/json;base64,<base64-encoded-json>`
///
/// # Examples
///
/// ```
/// use srcmap_sourcemap::utils::to_data_url;
/// let url = to_data_url(r#"{"version":3}"#);
/// assert!(url.starts_with("data:application/json;base64,"));
/// ```
pub fn to_data_url(json: &str) -> String {
    let encoded = base64_encode(json.as_bytes());
    format!("data:application/json;base64,{encoded}")
}

/// Encode bytes to base64 (no external dependency).
fn base64_encode(input: &[u8]) -> String {
    let mut result = String::with_capacity(input.len().div_ceil(3) * 4);
    let chunks = input.chunks(3);

    for chunk in chunks {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };

        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(BASE64_CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(BASE64_CHARS[((triple >> 12) & 0x3F) as usize] as char);

        if chunk.len() > 1 {
            result.push(BASE64_CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }

        if chunk.len() > 2 {
            result.push(BASE64_CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}

// ── RewriteOptions (gap #8) ─────────────────────────────────────

/// Options for rewriting source map paths and content.
pub struct RewriteOptions<'a> {
    /// Whether to include names in the output (default: true).
    pub with_names: bool,
    /// Whether to include sourcesContent in the output (default: true).
    pub with_source_contents: bool,
    /// Prefixes to strip from source paths.
    /// Use `"~"` to auto-detect and strip the common prefix.
    pub strip_prefixes: &'a [&'a str],
}

impl Default for RewriteOptions<'_> {
    fn default() -> Self {
        Self {
            with_names: true,
            with_source_contents: true,
            strip_prefixes: &[],
        }
    }
}

/// Create a new `SourceMap` with rewritten source paths.
///
/// - If `strip_prefixes` contains `"~"`, auto-detects the common prefix via
///   [`find_common_prefix`].
/// - Strips matching prefixes from all source paths.
/// - If `!with_names`, sets all name indices in mappings to `u32::MAX`.
/// - If `!with_source_contents`, sets all `sourcesContent` entries to `None`.
///
/// Preserves all mappings, `ignore_list`, `extensions`, `debug_id`, and `scopes`.
pub fn rewrite_sources(sm: &SourceMap, options: &RewriteOptions<'_>) -> SourceMap {
    // Determine prefixes to strip
    let auto_prefix = if options.strip_prefixes.contains(&"~") {
        find_common_prefix(sm.sources.iter().map(|s| s.as_str()))
    } else {
        None
    };

    let explicit_prefixes: Vec<&str> = options
        .strip_prefixes
        .iter()
        .filter(|&&p| p != "~")
        .copied()
        .collect();

    // Rewrite sources
    let sources: Vec<String> = sm
        .sources
        .iter()
        .map(|s| {
            let mut result = s.as_str();

            // Try auto-detected prefix first
            if let Some(ref prefix) = auto_prefix
                && let Some(stripped) = result.strip_prefix(prefix.as_str())
            {
                result = stripped;
            }

            // Try explicit prefixes
            for prefix in &explicit_prefixes {
                if let Some(stripped) = result.strip_prefix(prefix) {
                    result = stripped;
                    break;
                }
            }

            result.to_string()
        })
        .collect();

    // Handle sources_content
    let sources_content = if options.with_source_contents {
        sm.sources_content.clone()
    } else {
        vec![None; sm.sources_content.len()]
    };

    // Handle names and mappings
    let (names, mappings) = if options.with_names {
        (sm.names.clone(), sm.all_mappings().to_vec())
    } else {
        let cleared_mappings: Vec<Mapping> = sm
            .all_mappings()
            .iter()
            .map(|m| Mapping {
                name: u32::MAX,
                ..*m
            })
            .collect();
        (Vec::new(), cleared_mappings)
    };

    let mut result = SourceMap::from_parts(
        sm.file.clone(),
        sm.source_root.clone(),
        sources,
        sources_content,
        names,
        mappings,
        sm.ignore_list.clone(),
        sm.debug_id.clone(),
        sm.scopes.clone(),
    );

    // Preserve extension fields (x_* keys like x_facebook_sources)
    result.extensions = sm.extensions.clone();

    result
}

// ── DecodedMap (gap #9) ──────────────────────────────────────────

/// A unified type that can hold any decoded source map variant.
///
/// Dispatches lookups to the underlying type. Currently only supports
/// regular source maps; the `Hermes` variant will be added when the
/// hermes crate is integrated.
pub enum DecodedMap {
    /// A regular source map.
    Regular(SourceMap),
}

impl DecodedMap {
    /// Parse a JSON string and auto-detect the source map type.
    pub fn from_json(json: &str) -> Result<Self, ParseError> {
        let sm = SourceMap::from_json(json)?;
        Ok(Self::Regular(sm))
    }

    /// Look up the original source position for a generated position (0-based).
    pub fn original_position_for(&self, line: u32, column: u32) -> Option<OriginalLocation> {
        match self {
            DecodedMap::Regular(sm) => sm.original_position_for(line, column),
        }
    }

    /// Look up the generated position for an original source position (0-based).
    pub fn generated_position_for(
        &self,
        source: &str,
        line: u32,
        column: u32,
    ) -> Option<GeneratedLocation> {
        match self {
            DecodedMap::Regular(sm) => sm.generated_position_for(source, line, column),
        }
    }

    /// All source filenames.
    pub fn sources(&self) -> &[String] {
        match self {
            DecodedMap::Regular(sm) => &sm.sources,
        }
    }

    /// All name strings.
    pub fn names(&self) -> &[String] {
        match self {
            DecodedMap::Regular(sm) => &sm.names,
        }
    }

    /// Resolve a source index to its filename.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    pub fn source(&self, idx: u32) -> &str {
        match self {
            DecodedMap::Regular(sm) => sm.source(idx),
        }
    }

    /// Resolve a name index to its string.
    ///
    /// # Panics
    ///
    /// Panics if `idx` is out of bounds.
    pub fn name(&self, idx: u32) -> &str {
        match self {
            DecodedMap::Regular(sm) => sm.name(idx),
        }
    }

    /// The debug ID, if present.
    pub fn debug_id(&self) -> Option<&str> {
        match self {
            DecodedMap::Regular(sm) => sm.debug_id.as_deref(),
        }
    }

    /// Set the debug ID.
    pub fn set_debug_id(&mut self, id: impl Into<String>) {
        match self {
            DecodedMap::Regular(sm) => sm.debug_id = Some(id.into()),
        }
    }

    /// Serialize to JSON.
    pub fn to_json(&self) -> String {
        match self {
            DecodedMap::Regular(sm) => sm.to_json(),
        }
    }

    /// Extract the inner `SourceMap` if this is the `Regular` variant.
    pub fn into_source_map(self) -> Option<SourceMap> {
        match self {
            DecodedMap::Regular(sm) => Some(sm),
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── find_common_prefix ──────────────────────────────────────

    #[test]
    fn common_prefix_basic() {
        let paths = vec!["/a/b/c/file1.js", "/a/b/c/file2.js", "/a/b/c/file3.js"];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, Some("/a/b/c/".to_string()));
    }

    #[test]
    fn common_prefix_different_depths() {
        let paths = vec!["/a/b/c/file1.js", "/a/b/d/file2.js"];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, Some("/a/b/".to_string()));
    }

    #[test]
    fn common_prefix_only_root() {
        let paths = vec!["/a/file1.js", "/b/file2.js"];
        let result = find_common_prefix(paths.into_iter());
        // Only the root `/` is common, which is less than 2 components
        assert_eq!(result, None);
    }

    #[test]
    fn common_prefix_single_path() {
        let paths = vec!["/a/b/c.js"];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, None);
    }

    #[test]
    fn common_prefix_no_absolute_paths() {
        let paths = vec!["a/b/c.js", "a/b/d.js"];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, None);
    }

    #[test]
    fn common_prefix_mixed_absolute_relative() {
        let paths = vec!["/a/b/c.js", "a/b/d.js", "/a/b/e.js"];
        let result = find_common_prefix(paths.into_iter());
        // Only absolute paths are considered, so /a/b/ is common
        assert_eq!(result, Some("/a/b/".to_string()));
    }

    #[test]
    fn common_prefix_empty_iterator() {
        let paths: Vec<&str> = vec![];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, None);
    }

    #[test]
    fn common_prefix_identical_paths() {
        let paths = vec!["/a/b/c.js", "/a/b/c.js"];
        let result = find_common_prefix(paths.into_iter());
        assert_eq!(result, Some("/a/b/".to_string()));
    }

    // ── make_relative_path ──────────────────────────────────────

    #[test]
    fn relative_path_sibling_dirs() {
        assert_eq!(make_relative_path("/a/b/c.js", "/a/d/e.js"), "../d/e.js");
    }

    #[test]
    fn relative_path_same_dir() {
        assert_eq!(make_relative_path("/a/b/c.js", "/a/b/d.js"), "d.js");
    }

    #[test]
    fn relative_path_same_file() {
        assert_eq!(make_relative_path("/a/b/c.js", "/a/b/c.js"), ".");
    }

    #[test]
    fn relative_path_deeper_target() {
        assert_eq!(make_relative_path("/a/b/c.js", "/a/b/d/e/f.js"), "d/e/f.js");
    }

    #[test]
    fn relative_path_multiple_ups() {
        assert_eq!(make_relative_path("/a/b/c/d.js", "/a/e.js"), "../../e.js");
    }

    #[test]
    fn relative_path_completely_different() {
        assert_eq!(
            make_relative_path("/a/b/c.js", "/x/y/z.js"),
            "../../x/y/z.js"
        );
    }

    // ── is_sourcemap ────────────────────────────────────────────

    #[test]
    fn is_sourcemap_regular() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        assert!(is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_indexed() {
        let json = r#"{"version":3,"sections":[{"offset":{"line":0,"column":0},"map":{"version":3,"sources":[],"names":[],"mappings":""}}]}"#;
        assert!(is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_with_source_root() {
        let json = r#"{"version":3,"sourceRoot":"/src/","mappings":"AAAA"}"#;
        assert!(is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_with_sources_content() {
        let json = r#"{"version":3,"sourcesContent":["var x;"],"mappings":"AAAA"}"#;
        assert!(is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_invalid_json() {
        assert!(!is_sourcemap("not json"));
    }

    #[test]
    fn is_sourcemap_missing_version() {
        let json = r#"{"sources":["a.js"],"mappings":"AAAA"}"#;
        assert!(!is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_missing_mappings() {
        let json = r#"{"version":3,"sources":["a.js"]}"#;
        assert!(!is_sourcemap(json));
    }

    #[test]
    fn is_sourcemap_empty_object() {
        assert!(!is_sourcemap("{}"));
    }

    #[test]
    fn is_sourcemap_array() {
        assert!(!is_sourcemap("[]"));
    }

    // ── resolve_source_map_url ──────────────────────────────────

    #[test]
    fn resolve_url_relative() {
        let result = resolve_source_map_url("https://example.com/js/app.js", "app.js.map");
        assert_eq!(
            result,
            Some("https://example.com/js/app.js.map".to_string())
        );
    }

    #[test]
    fn resolve_url_parent_traversal() {
        let result = resolve_source_map_url("https://example.com/js/app.js", "../maps/app.js.map");
        assert_eq!(
            result,
            Some("https://example.com/maps/app.js.map".to_string())
        );
    }

    #[test]
    fn resolve_url_absolute_http() {
        let result = resolve_source_map_url(
            "https://example.com/js/app.js",
            "https://cdn.example.com/maps/app.js.map",
        );
        assert_eq!(
            result,
            Some("https://cdn.example.com/maps/app.js.map".to_string())
        );
    }

    #[test]
    fn resolve_url_absolute_slash() {
        let result = resolve_source_map_url("https://example.com/js/app.js", "/maps/app.js.map");
        assert_eq!(result, Some("/maps/app.js.map".to_string()));
    }

    #[test]
    fn resolve_url_data_url() {
        let result = resolve_source_map_url(
            "https://example.com/js/app.js",
            "data:application/json;base64,abc",
        );
        assert_eq!(result, None);
    }

    #[test]
    fn resolve_url_filesystem_path() {
        let result = resolve_source_map_url("/js/app.js", "app.js.map");
        assert_eq!(result, Some("/js/app.js.map".to_string()));
    }

    #[test]
    fn resolve_url_no_directory() {
        let result = resolve_source_map_url("app.js", "app.js.map");
        assert_eq!(result, Some("app.js.map".to_string()));
    }

    #[test]
    fn resolve_url_excessive_traversal() {
        // `..` should not traverse past the URL root
        let result =
            resolve_source_map_url("https://example.com/js/app.js", "../../../maps/app.js.map");
        assert_eq!(
            result,
            Some("https://example.com/maps/app.js.map".to_string())
        );
    }

    // ── resolve_source_map_path ─────────────────────────────────

    #[test]
    fn resolve_path_simple() {
        let result = resolve_source_map_path(Path::new("/js/app.js"), "app.js.map");
        assert_eq!(result, Some(PathBuf::from("/js/app.js.map")));
    }

    #[test]
    fn resolve_path_parent_traversal() {
        let result = resolve_source_map_path(Path::new("/js/app.js"), "../maps/app.js.map");
        assert_eq!(result, Some(PathBuf::from("/maps/app.js.map")));
    }

    #[test]
    fn resolve_path_subdirectory() {
        let result = resolve_source_map_path(Path::new("/src/app.js"), "maps/app.js.map");
        assert_eq!(result, Some(PathBuf::from("/src/maps/app.js.map")));
    }

    // ── to_data_url ─────────────────────────────────────────────

    #[test]
    fn data_url_prefix() {
        let url = to_data_url(r#"{"version":3}"#);
        assert!(url.starts_with("data:application/json;base64,"));
    }

    #[test]
    fn data_url_roundtrip() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let url = to_data_url(json);
        let encoded = url.strip_prefix("data:application/json;base64,").unwrap();
        let decoded = base64_decode(encoded);
        assert_eq!(decoded, json);
    }

    #[test]
    fn data_url_empty_json() {
        let url = to_data_url("{}");
        let encoded = url.strip_prefix("data:application/json;base64,").unwrap();
        let decoded = base64_decode(encoded);
        assert_eq!(decoded, "{}");
    }

    #[test]
    fn base64_encode_padding_1() {
        // 1 byte input -> 4 chars with 2 padding
        let encoded = base64_encode(b"A");
        assert_eq!(encoded, "QQ==");
    }

    #[test]
    fn base64_encode_padding_2() {
        // 2 byte input -> 4 chars with 1 padding
        let encoded = base64_encode(b"AB");
        assert_eq!(encoded, "QUI=");
    }

    #[test]
    fn base64_encode_no_padding() {
        // 3 byte input -> 4 chars with no padding
        let encoded = base64_encode(b"ABC");
        assert_eq!(encoded, "QUJD");
    }

    #[test]
    fn base64_encode_empty() {
        assert_eq!(base64_encode(b""), "");
    }

    /// Test helper: decode base64 (only used in tests).
    fn base64_decode(input: &str) -> String {
        let mut lookup = [0u8; 128];
        for (i, &c) in BASE64_CHARS.iter().enumerate() {
            lookup[c as usize] = i as u8;
        }

        let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'=').collect();
        let mut result = Vec::with_capacity(bytes.len() * 3 / 4);

        for chunk in bytes.chunks(4) {
            let vals: Vec<u8> = chunk.iter().map(|&b| lookup[b as usize]).collect();
            if vals.len() >= 2 {
                result.push((vals[0] << 2) | (vals[1] >> 4));
            }
            if vals.len() >= 3 {
                result.push((vals[1] << 4) | (vals[2] >> 2));
            }
            if vals.len() >= 4 {
                result.push((vals[2] << 6) | vals[3]);
            }
        }

        String::from_utf8(result).unwrap()
    }

    // ── RewriteOptions / rewrite_sources ────────────────────────

    #[test]
    fn rewrite_options_default() {
        let opts = RewriteOptions::default();
        assert!(opts.with_names);
        assert!(opts.with_source_contents);
        assert!(opts.strip_prefixes.is_empty());
    }

    fn make_test_sourcemap() -> SourceMap {
        let json = r#"{
            "version": 3,
            "sources": ["/src/app/main.js", "/src/app/utils.js"],
            "names": ["foo", "bar"],
            "mappings": "AACA,SCCA",
            "sourcesContent": ["var foo;", "var bar;"]
        }"#;
        SourceMap::from_json(json).unwrap()
    }

    #[test]
    fn rewrite_strip_explicit_prefix() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            strip_prefixes: &["/src/app/"],
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.sources, vec!["main.js", "utils.js"]);
    }

    #[test]
    fn rewrite_strip_auto_prefix() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            strip_prefixes: &["~"],
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.sources, vec!["main.js", "utils.js"]);
    }

    #[test]
    fn rewrite_without_names() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            with_names: false,
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        // All mappings should have name = u32::MAX
        for m in rewritten.all_mappings() {
            assert_eq!(m.name, u32::MAX);
        }
    }

    #[test]
    fn rewrite_without_sources_content() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            with_source_contents: false,
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        for content in &rewritten.sources_content {
            assert!(content.is_none());
        }
    }

    #[test]
    fn rewrite_preserves_mappings() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions::default();
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.all_mappings().len(), sm.all_mappings().len());
        // Position lookups should still work
        let loc = rewritten.original_position_for(0, 0);
        assert!(loc.is_some());
    }

    #[test]
    fn rewrite_preserves_debug_id() {
        let json = r#"{
            "version": 3,
            "sources": ["a.js"],
            "names": [],
            "mappings": "AAAA",
            "debugId": "test-id-123"
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        let opts = RewriteOptions::default();
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.debug_id.as_deref(), Some("test-id-123"));
    }

    #[test]
    fn rewrite_preserves_extensions() {
        let json = r#"{
            "version": 3,
            "sources": ["a.js"],
            "names": [],
            "mappings": "AAAA",
            "x_facebook_sources": [[{"names": ["<global>"], "mappings": "AAA"}]]
        }"#;
        let sm = SourceMap::from_json(json).unwrap();
        assert!(sm.extensions.contains_key("x_facebook_sources"));

        let opts = RewriteOptions::default();
        let rewritten = rewrite_sources(&sm, &opts);
        assert!(rewritten.extensions.contains_key("x_facebook_sources"));
        assert_eq!(
            sm.extensions["x_facebook_sources"],
            rewritten.extensions["x_facebook_sources"]
        );
    }

    #[test]
    fn rewrite_without_names_clears_names_vec() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            with_names: false,
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        assert!(rewritten.names.is_empty());
    }

    #[test]
    fn rewrite_strip_no_match() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            strip_prefixes: &["/other/"],
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.sources, sm.sources);
    }

    // ── DecodedMap ──────────────────────────────────────────────

    #[test]
    fn decoded_map_from_json() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AACAA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        assert_eq!(dm.sources(), &["a.js"]);
        assert_eq!(dm.names(), &["foo"]);
    }

    #[test]
    fn decoded_map_original_position() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let loc = dm.original_position_for(0, 0).unwrap();
        assert_eq!(dm.source(loc.source), "a.js");
        assert_eq!(loc.line, 0);
        assert_eq!(loc.column, 0);
    }

    #[test]
    fn decoded_map_generated_position() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let pos = dm.generated_position_for("a.js", 0, 0).unwrap();
        assert_eq!(pos.line, 0);
        assert_eq!(pos.column, 0);
    }

    #[test]
    fn decoded_map_source_and_name() {
        let json =
            r#"{"version":3,"sources":["a.js","b.js"],"names":["x","y"],"mappings":"AACAA,GCCA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        assert_eq!(dm.source(0), "a.js");
        assert_eq!(dm.source(1), "b.js");
        assert_eq!(dm.name(0), "x");
        assert_eq!(dm.name(1), "y");
    }

    #[test]
    fn decoded_map_debug_id() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":"","debugId":"abc-123"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        assert_eq!(dm.debug_id(), Some("abc-123"));
    }

    #[test]
    fn decoded_map_set_debug_id() {
        let json = r#"{"version":3,"sources":[],"names":[],"mappings":""}"#;
        let mut dm = DecodedMap::from_json(json).unwrap();
        assert_eq!(dm.debug_id(), None);
        dm.set_debug_id("new-id");
        assert_eq!(dm.debug_id(), Some("new-id"));
    }

    #[test]
    fn decoded_map_to_json() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let output = dm.to_json();
        // Should be valid JSON containing the same data
        assert!(output.contains("\"version\":3"));
        assert!(output.contains("\"sources\":[\"a.js\"]"));
    }

    #[test]
    fn decoded_map_into_source_map() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let sm = dm.into_source_map().unwrap();
        assert_eq!(sm.sources, vec!["a.js"]);
    }

    #[test]
    fn decoded_map_invalid_json() {
        let result = DecodedMap::from_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn decoded_map_roundtrip() {
        let json = r#"{"version":3,"sources":["a.js"],"names":["foo"],"mappings":"AACAA","sourcesContent":["var foo;"]}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let output = dm.to_json();
        let dm2 = DecodedMap::from_json(&output).unwrap();
        assert_eq!(dm2.sources(), &["a.js"]);
        assert_eq!(dm2.names(), &["foo"]);
    }

    // ── Integration tests ───────────────────────────────────────

    #[test]
    fn data_url_with_is_sourcemap() {
        let json = r#"{"version":3,"sources":["a.js"],"names":[],"mappings":"AAAA"}"#;
        assert!(is_sourcemap(json));
        let url = to_data_url(json);
        assert!(url.starts_with("data:application/json;base64,"));
    }

    #[test]
    fn rewrite_then_serialize() {
        let sm = make_test_sourcemap();
        let opts = RewriteOptions {
            strip_prefixes: &["~"],
            with_source_contents: false,
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        let json = rewritten.to_json();
        assert!(is_sourcemap(&json));

        // Parse back and verify
        let parsed = SourceMap::from_json(&json).unwrap();
        assert_eq!(parsed.sources, vec!["main.js", "utils.js"]);
    }

    #[test]
    fn decoded_map_rewrite_roundtrip() {
        let json = r#"{"version":3,"sources":["/src/a.js","/src/b.js"],"names":["x"],"mappings":"AACAA,GCAA","sourcesContent":["var x;","var y;"]}"#;
        let dm = DecodedMap::from_json(json).unwrap();
        let sm = dm.into_source_map().unwrap();

        let opts = RewriteOptions {
            strip_prefixes: &["~"],
            with_source_contents: true,
            ..Default::default()
        };
        let rewritten = rewrite_sources(&sm, &opts);
        assert_eq!(rewritten.sources, vec!["a.js", "b.js"]);

        let dm2 = DecodedMap::Regular(rewritten);
        let output = dm2.to_json();
        assert!(is_sourcemap(&output));
    }
}
