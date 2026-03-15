use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use srcmap_codec::{decode, encode};
use srcmap_remapping::{ConcatBuilder, remap};
use srcmap_sourcemap::utils::resolve_source_map_url;
use srcmap_sourcemap::{Bias, SourceMap, SourceMappingUrl, parse_source_mapping_url};

// ── CLI definition ───────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "srcmap",
    about = "Inspect, validate, compose, and manipulate source maps",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show source map metadata and statistics
    Info {
        /// Source map file (use `-` for stdin)
        file: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Validate a source map file
    Validate {
        /// Source map file (use `-` for stdin)
        file: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Look up the original position for a generated position
    Lookup {
        /// Source map file
        file: PathBuf,

        /// Generated line (0-based)
        line: u32,

        /// Generated column (0-based)
        column: u32,

        /// Search bias: "glb" (default, greatest lower bound) or "lub" (least upper bound)
        #[arg(long, default_value = "glb")]
        bias: String,

        /// Number of context lines to show around the matched position
        #[arg(long, default_value = "0")]
        context: u32,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Reverse lookup: find the generated position for an original position
    Resolve {
        /// Source map file
        file: PathBuf,

        /// Source filename to look up
        #[arg(long)]
        source: String,

        /// Original line (0-based)
        line: u32,

        /// Original column (0-based)
        column: u32,

        /// Search bias: "lub" (default, least upper bound) or "glb" (greatest lower bound)
        #[arg(long, default_value = "lub")]
        bias: String,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Decode a VLQ mappings string to JSON
    Decode {
        /// VLQ-encoded mappings string (omit to read from stdin)
        mappings: Option<String>,

        /// Output as compact single-line JSON
        #[arg(long)]
        compact: bool,
    },

    /// Encode decoded mappings JSON back to a VLQ string
    Encode {
        /// JSON file with decoded mappings (omit to read from stdin)
        file: Option<PathBuf>,

        /// Output as JSON (wraps result in {"vlq": "..."})
        #[arg(long)]
        json: bool,
    },

    /// List all mappings in a source map
    Mappings {
        /// Source map file
        file: PathBuf,

        /// Filter by source filename
        #[arg(long)]
        source: Option<String>,

        /// Maximum number of mappings to show
        #[arg(long, default_value = "50")]
        limit: usize,

        /// Skip first N mappings
        #[arg(long, default_value = "0")]
        offset: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Concatenate multiple source maps into one
    Concat {
        /// Source map files to concatenate (in order)
        files: Vec<PathBuf>,

        /// Output file (stdout if omitted)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output filename to embed in the map
        #[arg(long)]
        file_name: Option<String>,

        /// Output as JSON with metadata (instead of raw source map)
        #[arg(long)]
        json: bool,

        /// Validate without writing output
        #[arg(long)]
        dry_run: bool,
    },

    /// Compose/remap source maps through a transform chain
    Remap {
        /// Outer (final transform) source map
        file: PathBuf,

        /// Directory to search for upstream source maps
        #[arg(long)]
        dir: Option<PathBuf>,

        /// Explicit upstream source map files (source=path pairs)
        #[arg(long = "upstream", value_name = "SOURCE=PATH")]
        upstreams: Vec<String>,

        /// Output file (stdout if omitted)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output as JSON with metadata (instead of raw source map)
        #[arg(long)]
        json: bool,

        /// Validate without writing output
        #[arg(long)]
        dry_run: bool,
    },

    /// Symbolicate a stack trace using source maps
    Symbolicate {
        /// File containing the stack trace (use `-` for stdin)
        file: PathBuf,

        /// Directory to search for source maps
        #[arg(long)]
        dir: Option<PathBuf>,

        /// Explicit source map files (source=path pairs)
        #[arg(long = "map", value_name = "SOURCE=PATH")]
        maps: Vec<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Inspect ECMA-426 scopes and variable bindings in a source map
    Scopes {
        /// Source map file
        file: PathBuf,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Fetch a JavaScript/CSS bundle and its source map from a URL
    Fetch {
        /// URL of the JavaScript or CSS file
        url: String,

        /// Output directory (default: current directory)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// List or extract original sources embedded in a source map
    Sources {
        /// Source map file
        file: PathBuf,

        /// Extract sourcesContent to files on disk
        #[arg(long)]
        extract: bool,

        /// Output directory for extracted files (default: current directory)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Describe all commands and their arguments as JSON (for agent introspection)
    Schema,
}

// ── Structured error ─────────────────────────────────────────────

struct CliError {
    message: String,
    code: &'static str,
}

impl CliError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }

    fn io(message: impl Into<String>) -> Self {
        Self::new("IO_ERROR", message)
    }

    fn parse(message: impl Into<String>) -> Self {
        Self::new("PARSE_ERROR", message)
    }

    fn not_found(message: impl Into<String>) -> Self {
        Self::new("NOT_FOUND", message)
    }

    fn validation(message: impl Into<String>) -> Self {
        Self::new("VALIDATION_ERROR", message)
    }

    fn path_traversal(message: impl Into<String>) -> Self {
        Self::new("PATH_TRAVERSAL", message)
    }

    fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("INVALID_INPUT", message)
    }

    fn fetch_error(message: impl Into<String>) -> Self {
        Self::new("FETCH_ERROR", message)
    }

    fn to_json(&self) -> String {
        serde_json::to_string_pretty(&serde_json::json!({
            "error": self.message,
            "code": self.code,
        }))
        .unwrap()
    }
}

// ── Input validation ─────────────────────────────────────────────

/// Reject strings containing ASCII control characters (below 0x20, except newline/tab)
fn reject_control_chars(input: &str, label: &str) -> Result<(), CliError> {
    for (i, byte) in input.bytes().enumerate() {
        if byte < 0x20 && byte != b'\n' && byte != b'\r' && byte != b'\t' {
            return Err(CliError::invalid_input(format!(
                "{label} contains control character 0x{byte:02x} at offset {i}"
            )));
        }
    }
    Ok(())
}

/// Validate that a path does not escape the sandbox directory via traversal
fn validate_safe_path(path: &Path, sandbox: &Path) -> Result<PathBuf, CliError> {
    let canonical = path
        .canonicalize()
        .map_err(|e| CliError::io(format!("failed to resolve path {}: {e}", path.display())))?;
    let sandbox_canonical = sandbox.canonicalize().map_err(|e| {
        CliError::io(format!(
            "failed to resolve sandbox {}: {e}",
            sandbox.display()
        ))
    })?;
    if !canonical.starts_with(&sandbox_canonical) {
        return Err(CliError::path_traversal(format!(
            "path {} escapes sandbox directory {}",
            path.display(),
            sandbox.display()
        )));
    }
    Ok(canonical)
}

/// Validate an output path: must be within CWD
fn validate_output_path(path: &Path) -> Result<(), CliError> {
    let cwd = std::env::current_dir().map_err(|e| CliError::io(format!("cannot get cwd: {e}")))?;

    // For output files that don't exist yet, validate the parent directory
    if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() {
            // Relative path with no parent dir component — within CWD
            return Ok(());
        }
        let parent_canonical = parent.canonicalize().map_err(|e| {
            CliError::io(format!(
                "output parent directory {} does not exist: {e}",
                parent.display()
            ))
        })?;
        let cwd_canonical = cwd
            .canonicalize()
            .map_err(|e| CliError::io(format!("failed to resolve cwd: {e}")))?;
        if !parent_canonical.starts_with(&cwd_canonical) {
            return Err(CliError::path_traversal(format!(
                "output path {} escapes current working directory",
                path.display()
            )));
        }
    }
    Ok(())
}

/// Validate a source name from a source map (used in remap directory search)
fn validate_source_name(source: &str) -> Result<(), CliError> {
    reject_control_chars(source, "source name")?;
    if source.contains("..") {
        return Err(CliError::path_traversal(format!(
            "source name contains path traversal: {source}"
        )));
    }
    if source.contains('?') || source.contains('#') {
        return Err(CliError::invalid_input(format!(
            "source name contains invalid characters (? or #): {source}"
        )));
    }
    if source.contains('%') {
        return Err(CliError::invalid_input(format!(
            "source name contains percent-encoding: {source}"
        )));
    }
    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────

fn read_file_or_stdin(path: &PathBuf) -> Result<String, CliError> {
    if path.as_os_str() == "-" {
        let mut buf = String::new();
        io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| CliError::io(format!("failed to read stdin: {e}")))?;
        Ok(buf)
    } else {
        fs::read_to_string(path)
            .map_err(|e| CliError::io(format!("failed to read {}: {e}", path.display())))
    }
}

fn parse_source_map(path: &PathBuf) -> Result<(SourceMap, String), CliError> {
    let json = read_file_or_stdin(path)?;
    let sm = SourceMap::from_json(&json)
        .map_err(|e| CliError::parse(format!("invalid source map: {e}")))?;
    Ok((sm, json))
}

fn write_output(output: &Option<PathBuf>, content: &str) -> Result<(), CliError> {
    match output {
        Some(path) => {
            validate_output_path(path)?;
            fs::write(path, content)
                .map_err(|e| CliError::io(format!("failed to write {}: {e}", path.display())))
        }
        None => {
            println!("{content}");
            Ok(())
        }
    }
}

fn http_get(url: &str) -> Result<String, CliError> {
    let response = ureq::get(url)
        .set(
            "User-Agent",
            concat!("srcmap-cli/", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .map_err(|e| CliError::fetch_error(format!("failed to fetch {url}: {e}")))?;

    let status = response.status();
    if status != 200 {
        return Err(CliError::fetch_error(format!("HTTP {status} for {url}")));
    }

    response
        .into_string()
        .map_err(|e| CliError::fetch_error(format!("failed to read response body from {url}: {e}")))
}

fn url_filename(url: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    let path = path.split('#').next().unwrap_or(path);
    let name = path.rsplit('/').next().unwrap_or("bundle.js");
    if name.is_empty() {
        "bundle.js".to_string()
    } else {
        name.to_string()
    }
}

fn sanitize_source_path(source: &str) -> Result<PathBuf, CliError> {
    let stripped = source
        .strip_prefix("webpack:///")
        .or_else(|| source.strip_prefix("webpack://"))
        .or_else(|| source.strip_prefix("file:///"))
        .or_else(|| source.strip_prefix("file://"))
        .unwrap_or(source);

    let stripped = stripped.trim_start_matches('/');

    if stripped.is_empty() {
        return Err(CliError::invalid_input(
            "empty source name after stripping prefixes",
        ));
    }

    // Normalize: resolve . and .., stripping leading ../ (relative to map file, safe to drop)
    let mut components = Vec::new();
    for component in Path::new(stripped).components() {
        match component {
            std::path::Component::ParentDir => {
                if !components.is_empty() {
                    components.pop();
                }
                // Leading .. silently dropped — it's relative to the map file location
            }
            std::path::Component::CurDir => {}
            _ => {
                components.push(component);
            }
        }
    }

    if components.is_empty() {
        return Err(CliError::invalid_input(format!(
            "source name resolves to empty path: {source}"
        )));
    }

    let path: PathBuf = components.iter().collect();
    Ok(path)
}

fn format_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

// ── Commands ─────────────────────────────────────────────────────

fn cmd_info(file: &PathBuf, json: bool) -> Result<(), CliError> {
    let (sm, raw) = parse_source_map(file)?;

    if json {
        let has_content = sm.sources_content.iter().filter(|c| c.is_some()).count();
        let content_size: usize = sm
            .sources_content
            .iter()
            .filter_map(|c| c.as_ref())
            .map(|s| s.len())
            .sum();

        let obj = serde_json::json!({
            "file": sm.file,
            "sourceRoot": sm.source_root,
            "sources": sm.sources.len(),
            "names": sm.names.len(),
            "mappings": sm.mapping_count(),
            "rangeMappings": sm.range_mapping_count(),
            "lines": sm.line_count(),
            "sourcesWithContent": has_content,
            "totalContentSize": content_size,
            "fileSize": raw.len(),
            "ignoreList": sm.ignore_list,
            "debugId": sm.debug_id,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        if let Some(ref f) = sm.file {
            println!("File:         {f}");
        }
        if let Some(ref root) = sm.source_root {
            println!("Source root:   {root}");
        }
        println!("Sources:      {}", sm.sources.len());
        println!("Names:        {}", sm.names.len());
        println!("Mappings:     {}", sm.mapping_count());
        if sm.has_range_mappings() {
            println!("  Range:      {} range mappings", sm.range_mapping_count());
        }
        println!("Lines:        {}", sm.line_count());
        println!("File size:    {}", format_size(raw.len()));

        let has_content = sm.sources_content.iter().filter(|c| c.is_some()).count();
        if has_content > 0 {
            let content_size: usize = sm
                .sources_content
                .iter()
                .filter_map(|c| c.as_ref())
                .map(|s| s.len())
                .sum();
            println!(
                "Content:      {has_content}/{} sources ({})",
                sm.sources.len(),
                format_size(content_size)
            );
        }

        if let Some(ref id) = sm.debug_id {
            println!("Debug ID:     {id}");
        }

        if !sm.ignore_list.is_empty() {
            println!("Ignore list:  {} sources", sm.ignore_list.len());
        }

        println!();
        println!("Sources:");
        for (i, s) in sm.sources.iter().enumerate() {
            let ignored = if sm.ignore_list.contains(&(i as u32)) {
                " (ignored)"
            } else {
                ""
            };
            let content = match sm.sources_content.get(i) {
                Some(Some(c)) => format!(" [{}]", format_size(c.len())),
                _ => String::new(),
            };
            println!("  {i}: {s}{content}{ignored}");
        }

        if !sm.names.is_empty() {
            println!();
            println!("Names ({}):", sm.names.len());
            let display_count = sm.names.len().min(20);
            for name in &sm.names[..display_count] {
                println!("  {name}");
            }
            if sm.names.len() > 20 {
                println!("  ... and {} more", sm.names.len() - 20);
            }
        }
    }

    Ok(())
}

fn cmd_validate(file: &PathBuf, json: bool) -> Result<bool, CliError> {
    let raw = read_file_or_stdin(file)?;
    match SourceMap::from_json(&raw) {
        Ok(sm) => {
            if json {
                let obj = serde_json::json!({
                    "valid": true,
                    "version": 3,
                    "sources": sm.sources.len(),
                    "names": sm.names.len(),
                    "mappings": sm.mapping_count(),
                    "lines": sm.line_count(),
                });
                println!("{}", serde_json::to_string_pretty(&obj).unwrap());
            } else {
                println!("Valid source map v3");
                println!(
                    "  {} sources, {} names, {} mappings across {} lines",
                    sm.sources.len(),
                    sm.names.len(),
                    sm.mapping_count(),
                    sm.line_count()
                );
            }
            Ok(true)
        }
        Err(e) => {
            if json {
                let obj = serde_json::json!({
                    "valid": false,
                    "error": format!("{e}"),
                });
                println!("{}", serde_json::to_string_pretty(&obj).unwrap());
            } else {
                eprintln!("Invalid source map: {e}");
            }
            Ok(false)
        }
    }
}

fn parse_bias(s: &str) -> Result<Bias, CliError> {
    match s {
        "glb" | "greatest-lower-bound" => Ok(Bias::GreatestLowerBound),
        "lub" | "least-upper-bound" => Ok(Bias::LeastUpperBound),
        _ => Err(CliError::invalid_input(format!(
            "invalid bias: {s} (expected \"glb\" or \"lub\")"
        ))),
    }
}

fn cmd_lookup(
    file: &PathBuf,
    line: u32,
    column: u32,
    bias: &str,
    context: u32,
    json: bool,
) -> Result<(), CliError> {
    let b = parse_bias(bias)?;
    let (sm, _) = parse_source_map(file)?;

    match sm.original_position_for_with_bias(line, column, b) {
        Some(loc) => {
            let source = sm.source(loc.source);
            let name = loc.name.map(|n| sm.name(n).to_string());

            // Extract context lines from sourcesContent if requested
            let context_lines = if context > 0 {
                sm.sources_content
                    .get(loc.source as usize)
                    .and_then(|c| c.as_ref())
                    .map(|content| {
                        let lines: Vec<&str> = content.lines().collect();
                        let target = loc.line as usize;
                        let start = target.saturating_sub(context as usize);
                        let end = (target + context as usize + 1).min(lines.len());
                        (start, target, lines[start..end].to_vec())
                    })
            } else {
                None
            };

            if json {
                let mut obj = serde_json::json!({
                    "source": source,
                    "line": loc.line,
                    "column": loc.column,
                    "name": name,
                });

                if let Some((start, target, ref lines)) = context_lines {
                    let ctx: Vec<serde_json::Value> = lines
                        .iter()
                        .enumerate()
                        .map(|(i, text)| {
                            let line_num = start + i;
                            serde_json::json!({
                                "line": line_num,
                                "text": text,
                                "highlight": line_num == target,
                            })
                        })
                        .collect();
                    obj["context"] = serde_json::json!(ctx);
                }

                println!("{}", serde_json::to_string_pretty(&obj).unwrap());
            } else {
                println!("{source}:{line}:{col}", line = loc.line, col = loc.column);
                if let Some(name) = name {
                    println!("  name: {name}");
                }

                if let Some((start, target, ref lines)) = context_lines {
                    println!();
                    let gutter_width = format!("{}", start + lines.len()).len();
                    for (i, text) in lines.iter().enumerate() {
                        let line_num = start + i;
                        let marker = if line_num == target { ">" } else { " " };
                        println!(
                            "{marker} {line_num:>gutter_width$} | {text}",
                            gutter_width = gutter_width
                        );
                    }
                }
            }
        }
        None => {
            return Err(CliError::not_found(format!(
                "no mapping found for {line}:{column}"
            )));
        }
    }

    Ok(())
}

fn cmd_resolve(
    file: &PathBuf,
    source: &str,
    line: u32,
    column: u32,
    bias: &str,
    json: bool,
) -> Result<(), CliError> {
    let b = parse_bias(bias)?;
    reject_control_chars(source, "source")?;
    let (sm, _) = parse_source_map(file)?;

    match sm.generated_position_for_with_bias(source, line, column, b) {
        Some(loc) => {
            if json {
                let obj = serde_json::json!({
                    "line": loc.line,
                    "column": loc.column,
                });
                println!("{}", serde_json::to_string_pretty(&obj).unwrap());
            } else {
                println!("{}:{}", loc.line, loc.column);
            }
        }
        None => {
            return Err(CliError::not_found(format!(
                "no mapping found for {source}:{line}:{column}"
            )));
        }
    }

    Ok(())
}

fn cmd_decode(mappings: Option<String>, compact: bool) -> Result<(), CliError> {
    let input = match mappings {
        Some(m) => m,
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CliError::io(format!("failed to read stdin: {e}")))?;
            buf.trim().to_string()
        }
    };

    reject_control_chars(&input, "mappings input")?;
    let decoded = decode(&input).map_err(|e| CliError::parse(format!("decode error: {e}")))?;
    let as_vecs: Vec<Vec<Vec<i64>>> = decoded
        .into_iter()
        .map(|line| line.into_iter().map(|seg| seg.to_vec()).collect())
        .collect();
    let json = if compact {
        serde_json::to_string(&as_vecs)
    } else {
        serde_json::to_string_pretty(&as_vecs)
    }
    .map_err(|e| CliError::parse(format!("JSON serialization error: {e}")))?;
    println!("{json}");
    Ok(())
}

fn cmd_encode(file: Option<PathBuf>, json: bool) -> Result<(), CliError> {
    let input = match file {
        Some(path) => read_file_or_stdin(&path)?,
        None => {
            let mut buf = String::new();
            io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| CliError::io(format!("failed to read stdin: {e}")))?;
            buf
        }
    };

    let raw: Vec<Vec<Vec<i64>>> =
        serde_json::from_str(&input).map_err(|e| CliError::parse(format!("invalid JSON: {e}")))?;
    let decoded: srcmap_codec::SourceMapMappings = raw
        .into_iter()
        .map(|line| line.into_iter().map(srcmap_codec::Segment::from).collect())
        .collect();
    let encoded = encode(&decoded);

    if json {
        let obj = serde_json::json!({ "vlq": encoded });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        println!("{encoded}");
    }
    Ok(())
}

fn cmd_mappings(
    file: &PathBuf,
    source_filter: &Option<String>,
    limit: usize,
    offset: usize,
    json: bool,
) -> Result<(), CliError> {
    if let Some(name) = source_filter {
        reject_control_chars(name, "source filter")?;
    }
    let (sm, _) = parse_source_map(file)?;

    let all = sm.all_mappings();

    let total = if let Some(source_name) = source_filter {
        let source_idx = sm
            .source_index(source_name)
            .ok_or_else(|| CliError::not_found(format!("source not found: {source_name}")))?;
        all.iter().filter(|m| m.source == source_idx).count()
    } else {
        sm.mapping_count()
    };

    let filtered: Vec<_> = if let Some(source_name) = source_filter {
        let source_idx = sm
            .source_index(source_name)
            .ok_or_else(|| CliError::not_found(format!("source not found: {source_name}")))?;
        all.iter()
            .filter(|m| m.source == source_idx)
            .skip(offset)
            .take(limit)
            .collect()
    } else {
        all.iter().skip(offset).take(limit).collect()
    };

    if json {
        let entries: Vec<serde_json::Value> = filtered
            .iter()
            .map(|m| {
                let source = if m.source != u32::MAX {
                    Some(sm.source(m.source).to_string())
                } else {
                    None
                };
                let name = if m.name != u32::MAX {
                    Some(sm.name(m.name).to_string())
                } else {
                    None
                };
                serde_json::json!({
                    "generatedLine": m.generated_line,
                    "generatedColumn": m.generated_column,
                    "source": source,
                    "originalLine": m.original_line,
                    "originalColumn": m.original_column,
                    "name": name,
                    "isRangeMapping": m.is_range_mapping,
                })
            })
            .collect();

        let obj = serde_json::json!({
            "mappings": entries,
            "total": total,
            "offset": offset,
            "limit": limit,
            "hasMore": offset + filtered.len() < total,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        println!(
            "{:<8} {:<8} {:<30} {:<8} {:<8} {:<6} name",
            "gen.ln", "gen.col", "source", "orig.ln", "orig.col", "range"
        );
        println!("{:-<86}", "");
        for m in &filtered {
            let source = if m.source != u32::MAX {
                sm.source(m.source)
            } else {
                "-"
            };
            let name = if m.name != u32::MAX {
                sm.name(m.name)
            } else {
                ""
            };
            let range_marker = if m.is_range_mapping { "R" } else { "" };
            println!(
                "{:<8} {:<8} {:<30} {:<8} {:<8} {:<6} {}",
                m.generated_line,
                m.generated_column,
                source,
                m.original_line,
                m.original_column,
                range_marker,
                name
            );
        }

        if offset + limit < total {
            println!();
            println!(
                "Showing {}-{} of {total}. Use --offset and --limit to paginate.",
                offset,
                offset + filtered.len()
            );
        }
    }

    Ok(())
}

fn cmd_concat(
    files: &[PathBuf],
    output: &Option<PathBuf>,
    file_name: Option<String>,
    json: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    if files.is_empty() {
        return Err(CliError::validation(
            "at least one source map file is required",
        ));
    }

    if let Some(path) = output {
        validate_output_path(path)?;
    }

    let mut builder = ConcatBuilder::new(file_name);
    let mut line_offset: u32 = 0;
    let mut file_stats: Vec<serde_json::Value> = Vec::new();

    for path in files {
        let (sm, _) = parse_source_map(path)?;
        let lines = sm.line_count() as u32;
        let sources = sm.sources.len();
        let mappings = sm.mapping_count();
        builder.add_map(&sm, line_offset);
        file_stats.push(serde_json::json!({
            "file": path.display().to_string(),
            "sources": sources,
            "mappings": mappings,
            "lines": lines,
            "lineOffset": line_offset,
        }));
        line_offset += lines;
    }

    let map_json = builder.to_json();
    let result = SourceMap::from_json(&map_json)
        .map_err(|e| CliError::parse(format!("failed to parse generated map: {e}")))?;

    if dry_run {
        if json {
            let obj = serde_json::json!({
                "dryRun": true,
                "inputFiles": file_stats,
                "result": {
                    "sources": result.sources.len(),
                    "mappings": result.mapping_count(),
                    "lines": result.line_count(),
                    "fileSize": map_json.len(),
                },
            });
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        } else {
            eprintln!(
                "Dry run: would concatenate {} files → {} sources, {} mappings, {} lines ({})",
                files.len(),
                result.sources.len(),
                result.mapping_count(),
                result.line_count(),
                format_size(map_json.len()),
            );
        }
        return Ok(());
    }

    if json {
        let obj = serde_json::json!({
            "inputFiles": file_stats,
            "result": {
                "sources": result.sources.len(),
                "mappings": result.mapping_count(),
                "lines": result.line_count(),
                "fileSize": map_json.len(),
            },
            "sourceMap": serde_json::from_str::<serde_json::Value>(&map_json).unwrap(),
        });
        write_output(output, &serde_json::to_string_pretty(&obj).unwrap())?;
    } else {
        write_output(output, &map_json)?;
        if output.is_some() {
            eprintln!(
                "Concatenated {} files: {} sources, {} mappings, {} lines",
                files.len(),
                result.sources.len(),
                result.mapping_count(),
                result.line_count()
            );
        }
    }

    Ok(())
}

fn cmd_remap(
    file: &PathBuf,
    dir: &Option<PathBuf>,
    upstreams: &[String],
    output: &Option<PathBuf>,
    json: bool,
    dry_run: bool,
) -> Result<(), CliError> {
    if let Some(path) = output {
        validate_output_path(path)?;
    }

    let (outer, _) = parse_source_map(file)?;

    // Validate and resolve search directory
    let cwd = std::env::current_dir().map_err(|e| CliError::io(format!("cannot get cwd: {e}")))?;
    let safe_dir = if let Some(d) = dir {
        Some(validate_safe_path(d, &cwd)?)
    } else {
        None
    };

    // Build explicit upstream map: source name → file path
    let mut upstream_paths: std::collections::HashMap<String, PathBuf> =
        std::collections::HashMap::new();

    for entry in upstreams {
        let (source, path) = entry.split_once('=').ok_or_else(|| {
            CliError::validation(format!(
                "invalid upstream format: {entry} (expected SOURCE=PATH)"
            ))
        })?;
        reject_control_chars(source, "upstream source")?;
        upstream_paths.insert(source.to_string(), PathBuf::from(path));
    }

    // Track which upstream maps were found
    let found_upstreams: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());
    let skipped_sources: std::cell::RefCell<Vec<String>> = std::cell::RefCell::new(Vec::new());

    let result = remap(&outer, |source| {
        // Try explicit upstream first
        if let Some(path) = upstream_paths.get(source) {
            let content = fs::read_to_string(path).ok()?;
            let sm = SourceMap::from_json(&content).ok()?;
            found_upstreams.borrow_mut().push(source.to_string());
            return Some(sm);
        }

        // Validate source name before using it in path construction
        if validate_source_name(source).is_err() {
            skipped_sources.borrow_mut().push(source.to_string());
            return None;
        }

        // Try directory search: look for source.map next to the source
        if let Some(ref search_dir) = safe_dir {
            // Try <source>.map
            let map_path = search_dir.join(format!("{source}.map"));
            if let Ok(canonical) = map_path.canonicalize()
                && canonical.starts_with(search_dir)
                && let Ok(content) = fs::read_to_string(&canonical)
                && let Ok(sm) = SourceMap::from_json(&content)
            {
                found_upstreams.borrow_mut().push(source.to_string());
                return Some(sm);
            }

            // Try replacing extension with .map
            let source_path = Path::new(source);
            if let Some(stem) = source_path.file_stem() {
                let map_name = format!("{}.map", stem.to_string_lossy());
                let map_path = search_dir.join(map_name);
                if let Ok(canonical) = map_path.canonicalize()
                    && canonical.starts_with(search_dir)
                    && let Ok(content) = fs::read_to_string(&canonical)
                    && let Ok(sm) = SourceMap::from_json(&content)
                {
                    found_upstreams.borrow_mut().push(source.to_string());
                    return Some(sm);
                }
            }
        }

        None
    });

    let found = found_upstreams.into_inner();
    let skipped = skipped_sources.into_inner();

    if dry_run {
        if json {
            let obj = serde_json::json!({
                "dryRun": true,
                "outerSources": outer.sources.len(),
                "upstreamMapsFound": found,
                "skippedSources": skipped,
                "result": {
                    "sources": result.sources.len(),
                    "mappings": result.mapping_count(),
                    "lines": result.line_count(),
                },
            });
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        } else {
            eprintln!(
                "Dry run: would remap {} sources → {} upstream maps found",
                outer.sources.len(),
                found.len(),
            );
            if !skipped.is_empty() {
                eprintln!("  Skipped (invalid source names): {}", skipped.join(", "));
            }
            eprintln!(
                "  Result: {} sources, {} mappings, {} lines",
                result.sources.len(),
                result.mapping_count(),
                result.line_count(),
            );
        }
        return Ok(());
    }

    let map_json = result.to_json();

    if json {
        let obj = serde_json::json!({
            "upstreamMapsFound": found,
            "skippedSources": skipped,
            "result": {
                "sources": result.sources.len(),
                "mappings": result.mapping_count(),
                "lines": result.line_count(),
                "fileSize": map_json.len(),
            },
            "sourceMap": serde_json::from_str::<serde_json::Value>(&map_json).unwrap(),
        });
        write_output(output, &serde_json::to_string_pretty(&obj).unwrap())?;
    } else {
        write_output(output, &map_json)?;
        if output.is_some() {
            eprintln!(
                "Remapped: {} sources, {} mappings, {} lines",
                result.sources.len(),
                result.mapping_count(),
                result.line_count()
            );
        }
    }

    Ok(())
}

fn cmd_symbolicate(
    file: &PathBuf,
    dir: &Option<PathBuf>,
    maps: &[String],
    json: bool,
) -> Result<(), CliError> {
    let stack_input = read_file_or_stdin(file)?;

    let cwd = std::env::current_dir().map_err(|e| CliError::io(format!("cannot get cwd: {e}")))?;
    let safe_dir = if let Some(d) = dir {
        Some(validate_safe_path(d, &cwd)?)
    } else {
        None
    };

    // Build explicit map: source → SourceMap
    let mut explicit_maps: std::collections::HashMap<String, SourceMap> =
        std::collections::HashMap::new();
    for entry in maps {
        let (source, path_str) = entry.split_once('=').ok_or_else(|| {
            CliError::validation(format!(
                "invalid map format: {entry} (expected SOURCE=PATH)"
            ))
        })?;
        let path = PathBuf::from(path_str);
        let content = fs::read_to_string(&path)
            .map_err(|e| CliError::io(format!("failed to read {}: {e}", path.display())))?;
        let sm = SourceMap::from_json(&content)
            .map_err(|e| CliError::parse(format!("invalid source map {}: {e}", path.display())))?;
        explicit_maps.insert(source.to_string(), sm);
    }

    let result = srcmap_symbolicate::symbolicate(&stack_input, |source| {
        // Try explicit maps first
        if let Some(sm) = explicit_maps.get(source) {
            return Some(sm.clone());
        }

        // Try directory search
        if let Some(ref search_dir) = safe_dir {
            let map_path = search_dir.join(format!("{source}.map"));
            if let Ok(canonical) = map_path.canonicalize()
                && canonical.starts_with(search_dir)
                && let Ok(content) = fs::read_to_string(&canonical)
                && let Ok(sm) = SourceMap::from_json(&content)
            {
                return Some(sm);
            }
        }

        None
    });

    if json {
        println!("{}", srcmap_symbolicate::to_json(&result));
    } else {
        print!("{result}");
    }

    Ok(())
}

fn format_scope_tree(scope: &srcmap_scopes::OriginalScope, indent: usize) {
    let pad = "  ".repeat(indent);
    let kind = scope.kind.as_deref().unwrap_or("?");
    let name = scope
        .name
        .as_deref()
        .map(|n| format!(" \"{n}\""))
        .unwrap_or_default();
    let frame = if scope.is_stack_frame { " [frame]" } else { "" };
    println!(
        "{pad}{kind}{name}{frame}  {}:{}-{}:{}",
        scope.start.line, scope.start.column, scope.end.line, scope.end.column
    );
    if !scope.variables.is_empty() {
        println!("{pad}  vars: {}", scope.variables.join(", "));
    }
    for child in &scope.children {
        format_scope_tree(child, indent + 1);
    }
}

fn format_range_tree(range: &srcmap_scopes::GeneratedRange, sources: &[String], indent: usize) {
    let pad = "  ".repeat(indent);
    let frame = if range.is_stack_frame { " [frame]" } else { "" };
    let hidden = if range.is_hidden { " [hidden]" } else { "" };
    println!(
        "{pad}{}:{}-{}:{}{frame}{hidden}",
        range.start.line, range.start.column, range.end.line, range.end.column
    );
    if let Some(def) = range.definition {
        println!("{pad}  definition: scope #{def}");
    }
    if let Some(ref cs) = range.call_site {
        let source = sources
            .get(cs.source_index as usize)
            .map(|s| s.as_str())
            .unwrap_or("?");
        println!("{pad}  call site: {source}:{}:{}", cs.line, cs.column);
    }
    for binding in &range.bindings {
        match binding {
            srcmap_scopes::Binding::Expression(expr) => {
                println!("{pad}  binding: {expr}");
            }
            srcmap_scopes::Binding::Unavailable => {
                println!("{pad}  binding: <unavailable>");
            }
            srcmap_scopes::Binding::SubRanges(subs) => {
                for sub in subs {
                    let expr = sub.expression.as_deref().unwrap_or("<unavailable>");
                    println!(
                        "{pad}  binding: {expr} (from {}:{})",
                        sub.from.line, sub.from.column
                    );
                }
            }
        }
    }
    for child in &range.children {
        format_range_tree(child, sources, indent + 1);
    }
}

fn scope_to_json(scope: &srcmap_scopes::OriginalScope) -> serde_json::Value {
    serde_json::json!({
        "start": { "line": scope.start.line, "column": scope.start.column },
        "end": { "line": scope.end.line, "column": scope.end.column },
        "kind": scope.kind,
        "name": scope.name,
        "isStackFrame": scope.is_stack_frame,
        "variables": scope.variables,
        "children": scope.children.iter().map(scope_to_json).collect::<Vec<_>>(),
    })
}

fn range_to_json(range: &srcmap_scopes::GeneratedRange, sources: &[String]) -> serde_json::Value {
    let bindings: Vec<serde_json::Value> = range
        .bindings
        .iter()
        .map(|b| match b {
            srcmap_scopes::Binding::Expression(expr) => {
                serde_json::json!({ "expression": expr })
            }
            srcmap_scopes::Binding::Unavailable => {
                serde_json::json!({ "unavailable": true })
            }
            srcmap_scopes::Binding::SubRanges(subs) => {
                let entries: Vec<serde_json::Value> = subs
                    .iter()
                    .map(|s| {
                        serde_json::json!({
                            "expression": s.expression,
                            "from": { "line": s.from.line, "column": s.from.column },
                        })
                    })
                    .collect();
                serde_json::json!({ "subRanges": entries })
            }
        })
        .collect();

    let call_site = range.call_site.as_ref().map(|cs| {
        let source = sources
            .get(cs.source_index as usize)
            .map(|s| s.as_str())
            .unwrap_or("?");
        serde_json::json!({
            "source": source,
            "line": cs.line,
            "column": cs.column,
        })
    });

    serde_json::json!({
        "start": { "line": range.start.line, "column": range.start.column },
        "end": { "line": range.end.line, "column": range.end.column },
        "isStackFrame": range.is_stack_frame,
        "isHidden": range.is_hidden,
        "definition": range.definition,
        "callSite": call_site,
        "bindings": bindings,
        "children": range.children.iter().map(|c| range_to_json(c, sources)).collect::<Vec<_>>(),
    })
}

fn cmd_scopes(file: &PathBuf, json: bool) -> Result<(), CliError> {
    let (sm, _) = parse_source_map(file)?;

    let scopes = sm
        .scopes
        .as_ref()
        .ok_or_else(|| CliError::not_found("source map does not contain scopes data"))?;

    if json {
        let original: Vec<serde_json::Value> = scopes
            .scopes
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                s.as_ref().map(|scope| {
                    let source = sm.sources.get(i).map(|s| s.as_str()).unwrap_or("?");
                    serde_json::json!({
                        "source": source,
                        "scope": scope_to_json(scope),
                    })
                })
            })
            .collect();

        let ranges: Vec<serde_json::Value> = scopes
            .ranges
            .iter()
            .map(|r| range_to_json(r, &sm.sources))
            .collect();

        let obj = serde_json::json!({
            "originalScopes": original,
            "generatedRanges": ranges,
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else {
        // Original scopes
        let scope_count: usize = scopes.scopes.iter().filter(|s| s.is_some()).count();
        println!("Original scopes ({scope_count} sources with scopes):");
        for (i, scope) in scopes.scopes.iter().enumerate() {
            if let Some(scope) = scope {
                let source = sm.sources.get(i).map(|s| s.as_str()).unwrap_or("?");
                println!();
                println!("  [{i}] {source}:");
                format_scope_tree(scope, 2);
            }
        }

        // Generated ranges
        println!();
        println!("Generated ranges ({}):", scopes.ranges.len());
        for range in &scopes.ranges {
            println!();
            format_range_tree(range, &sm.sources, 1);
        }
    }

    Ok(())
}

fn cmd_fetch(url: &str, output: &Option<PathBuf>, json: bool) -> Result<(), CliError> {
    // Validate URL
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err(CliError::invalid_input(
            "URL must start with http:// or https://",
        ));
    }

    let output_dir = match output {
        Some(dir) => dir.clone(),
        None => {
            std::env::current_dir().map_err(|e| CliError::io(format!("cannot get cwd: {e}")))?
        }
    };

    if !output_dir.exists() {
        fs::create_dir_all(&output_dir).map_err(|e| {
            CliError::io(format!(
                "failed to create output directory {}: {e}",
                output_dir.display()
            ))
        })?;
    }

    // Fetch the bundle
    eprintln!("Fetching {url}...");
    let bundle_body = http_get(url)?;
    let bundle_filename = url_filename(url);
    let bundle_path = output_dir.join(&bundle_filename);

    fs::write(&bundle_path, &bundle_body)
        .map_err(|e| CliError::io(format!("failed to write {}: {e}", bundle_path.display())))?;
    eprintln!(
        "  Saved {} ({})",
        bundle_path.display(),
        format_size(bundle_body.len())
    );

    // Extract sourceMappingURL
    let map_result = match parse_source_mapping_url(&bundle_body) {
        Some(SourceMappingUrl::Inline(decoded_json)) => {
            let map_filename = format!("{bundle_filename}.map");
            let map_path = output_dir.join(&map_filename);
            fs::write(&map_path, &decoded_json).map_err(|e| {
                CliError::io(format!("failed to write {}: {e}", map_path.display()))
            })?;
            eprintln!(
                "  Saved {} (inline, {})",
                map_path.display(),
                format_size(decoded_json.len())
            );
            Some((
                map_path.display().to_string(),
                decoded_json.len(),
                "inline".to_string(),
            ))
        }
        Some(SourceMappingUrl::External(ref map_ref)) => {
            let map_url = resolve_source_map_url(url, map_ref).ok_or_else(|| {
                CliError::fetch_error(format!("could not resolve source map URL: {map_ref}"))
            })?;
            eprintln!("Fetching {map_url}...");
            let map_body = http_get(&map_url)?;
            let map_filename = url_filename(&map_url);
            let map_path = output_dir.join(&map_filename);
            fs::write(&map_path, &map_body).map_err(|e| {
                CliError::io(format!("failed to write {}: {e}", map_path.display()))
            })?;
            eprintln!(
                "  Saved {} ({})",
                map_path.display(),
                format_size(map_body.len())
            );
            Some((map_path.display().to_string(), map_body.len(), map_url))
        }
        None => {
            // Try conventional .map suffix
            let map_url = format!("{url}.map");
            eprintln!("No sourceMappingURL found, trying {map_url}...");
            match http_get(&map_url) {
                Ok(map_body) => {
                    let map_filename = format!("{bundle_filename}.map");
                    let map_path = output_dir.join(&map_filename);
                    fs::write(&map_path, &map_body).map_err(|e| {
                        CliError::io(format!("failed to write {}: {e}", map_path.display()))
                    })?;
                    eprintln!(
                        "  Saved {} (convention, {})",
                        map_path.display(),
                        format_size(map_body.len())
                    );
                    Some((map_path.display().to_string(), map_body.len(), map_url))
                }
                Err(_) => {
                    eprintln!("  No source map found");
                    None
                }
            }
        }
    };

    if json {
        let obj = serde_json::json!({
            "bundle": {
                "url": url,
                "file": bundle_path.display().to_string(),
                "size": bundle_body.len(),
            },
            "sourceMap": map_result.as_ref().map(|(path, size, source_url)| {
                serde_json::json!({
                    "url": source_url,
                    "file": path,
                    "size": size,
                })
            }),
        });
        println!("{}", serde_json::to_string_pretty(&obj).unwrap());
    } else if map_result.is_none() {
        println!("Fetched {} (no source map found)", bundle_path.display());
    } else {
        println!("Done");
    }

    Ok(())
}

fn cmd_sources(
    file: &PathBuf,
    extract: bool,
    output: &Option<PathBuf>,
    json: bool,
) -> Result<(), CliError> {
    let (sm, _) = parse_source_map(file)?;

    if extract {
        let output_dir = match output {
            Some(dir) => dir.clone(),
            None => {
                std::env::current_dir().map_err(|e| CliError::io(format!("cannot get cwd: {e}")))?
            }
        };

        let mut extracted = Vec::new();
        let mut skipped = Vec::new();

        for (i, source_name) in sm.sources.iter().enumerate() {
            let content = match sm.sources_content.get(i).and_then(|c| c.as_ref()) {
                Some(c) => c,
                None => {
                    skipped.push(source_name.clone());
                    continue;
                }
            };

            let dest_path = match sanitize_source_path(source_name) {
                Ok(p) => output_dir.join(p),
                Err(_) => {
                    skipped.push(source_name.clone());
                    continue;
                }
            };

            if let Some(parent) = dest_path.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    CliError::io(format!(
                        "failed to create directory {}: {e}",
                        parent.display()
                    ))
                })?;
            }

            fs::write(&dest_path, content).map_err(|e| {
                CliError::io(format!("failed to write {}: {e}", dest_path.display()))
            })?;

            extracted.push(serde_json::json!({
                "source": source_name,
                "file": dest_path.display().to_string(),
                "size": content.len(),
            }));
        }

        if json {
            let obj = serde_json::json!({
                "extracted": extracted,
                "skipped": skipped,
                "total": sm.sources.len(),
            });
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        } else {
            println!(
                "Extracted {}/{} sources to {}",
                extracted.len(),
                sm.sources.len(),
                output_dir.display()
            );
            for entry in &extracted {
                let source = entry["source"].as_str().unwrap();
                let size = entry["size"].as_u64().unwrap() as usize;
                println!("  {source} [{}]", format_size(size));
            }
            if !skipped.is_empty() {
                println!("Skipped {} sources without content", skipped.len());
            }
        }
    } else {
        // List mode
        let entries: Vec<serde_json::Value> = sm
            .sources
            .iter()
            .enumerate()
            .map(|(i, source)| {
                let content_size = sm
                    .sources_content
                    .get(i)
                    .and_then(|c| c.as_ref())
                    .map(|c| c.len());
                let ignored = sm.ignore_list.contains(&(i as u32));

                serde_json::json!({
                    "index": i,
                    "source": source,
                    "hasContent": content_size.is_some(),
                    "contentSize": content_size,
                    "ignored": ignored,
                })
            })
            .collect();

        if json {
            let obj = serde_json::json!({
                "sources": entries,
                "total": sm.sources.len(),
                "withContent": entries.iter().filter(|e| e["hasContent"].as_bool() == Some(true)).count(),
            });
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        } else {
            let with_content = entries
                .iter()
                .filter(|e| e["hasContent"].as_bool() == Some(true))
                .count();
            println!(
                "Sources ({}, {} with content):",
                sm.sources.len(),
                with_content
            );
            for entry in &entries {
                let idx = entry["index"].as_u64().unwrap();
                let source = entry["source"].as_str().unwrap();
                let size_str = match entry["contentSize"].as_u64() {
                    Some(size) => format!(" [{}]", format_size(size as usize)),
                    None => " [no content]".to_string(),
                };
                let ignored = if entry["ignored"].as_bool() == Some(true) {
                    " (ignored)"
                } else {
                    ""
                };
                println!("  {idx}: {source}{size_str}{ignored}");
            }
        }
    }

    Ok(())
}

fn cmd_schema() -> Result<(), CliError> {
    let schema = serde_json::json!({
        "name": "srcmap",
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Inspect, validate, compose, and manipulate source maps",
        "note": "All line and column values are 0-based (ECMA-426 spec)",
        "globalFlags": {
            "--json": "Available on most commands. Outputs structured JSON instead of human-readable text. Errors are also returned as JSON when this flag is active.",
        },
        "commands": [
            {
                "name": "info",
                "description": "Show source map metadata and statistics",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file (use `-` for stdin)"}
                ],
                "flags": {
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "validate",
                "description": "Validate a source map file and report structure",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file (use `-` for stdin)"}
                ],
                "flags": {
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                },
                "exitCodes": {"0": "valid", "1": "invalid or error"}
            },
            {
                "name": "lookup",
                "description": "Find original position for a generated position (forward mapping)",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file"},
                    {"name": "line", "type": "u32", "required": true, "description": "Generated line (0-based)"},
                    {"name": "column", "type": "u32", "required": true, "description": "Generated column (0-based)"}
                ],
                "flags": {
                    "--bias": {"type": "string", "default": "glb", "description": "Search bias: glb (greatest lower bound) or lub (least upper bound)"},
                    "--context": {"type": "u32", "default": 0, "description": "Number of context lines to show around the matched position"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "resolve",
                "description": "Find generated position for an original position (reverse mapping)",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file"},
                    {"name": "line", "type": "u32", "required": true, "description": "Original line (0-based)"},
                    {"name": "column", "type": "u32", "required": true, "description": "Original column (0-based)"}
                ],
                "flags": {
                    "--source": {"type": "string", "required": true, "description": "Source filename to look up"},
                    "--bias": {"type": "string", "default": "lub", "description": "Search bias: lub (least upper bound) or glb (greatest lower bound)"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "decode",
                "description": "Decode a VLQ mappings string to JSON array",
                "args": [
                    {"name": "mappings", "type": "string", "required": false, "description": "VLQ-encoded mappings string (reads stdin if omitted)"}
                ],
                "flags": {
                    "--compact": {"type": "bool", "default": false, "description": "Output as compact single-line JSON"}
                }
            },
            {
                "name": "encode",
                "description": "Encode decoded mappings JSON back to a VLQ string",
                "args": [
                    {"name": "file", "type": "path", "required": false, "description": "JSON file with decoded mappings (reads stdin if omitted)"}
                ],
                "flags": {
                    "--json": {"type": "bool", "default": false, "description": "Wrap result in JSON object {\"vlq\": \"...\"}"}
                }
            },
            {
                "name": "mappings",
                "description": "List all mappings in a source map with pagination",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file"}
                ],
                "flags": {
                    "--source": {"type": "string", "required": false, "description": "Filter by source filename"},
                    "--limit": {"type": "usize", "default": 50, "description": "Maximum number of mappings to show"},
                    "--offset": {"type": "usize", "default": 0, "description": "Skip first N mappings"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON with pagination metadata"}
                }
            },
            {
                "name": "concat",
                "description": "Concatenate multiple source maps into one (mutating)",
                "args": [
                    {"name": "files", "type": "path[]", "required": true, "description": "Source map files to concatenate (in order)"}
                ],
                "flags": {
                    "-o, --output": {"type": "path", "required": false, "description": "Output file (stdout if omitted)"},
                    "--file_name": {"type": "string", "required": false, "description": "Output filename to embed in the map"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON with metadata"},
                    "--dry-run": {"type": "bool", "default": false, "description": "Validate and preview result without writing"}
                }
            },
            {
                "name": "remap",
                "description": "Compose/remap source maps through a transform chain (mutating)",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Outer (final transform) source map"}
                ],
                "flags": {
                    "--dir": {"type": "path", "required": false, "description": "Directory to search for upstream source maps"},
                    "--upstream": {"type": "string[]", "required": false, "description": "Explicit upstream mappings (SOURCE=PATH pairs, repeatable)"},
                    "-o, --output": {"type": "path", "required": false, "description": "Output file (stdout if omitted)"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON with metadata"},
                    "--dry-run": {"type": "bool", "default": false, "description": "Validate and preview result without writing"}
                }
            },
            {
                "name": "symbolicate",
                "description": "Symbolicate a stack trace using source maps",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "File containing the stack trace (use `-` for stdin)"}
                ],
                "flags": {
                    "--dir": {"type": "path", "required": false, "description": "Directory to search for source maps"},
                    "--map": {"type": "string[]", "required": false, "description": "Explicit source map files (SOURCE=PATH pairs, repeatable)"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "scopes",
                "description": "Inspect ECMA-426 scopes and variable bindings in a source map",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file"}
                ],
                "flags": {
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "fetch",
                "description": "Fetch a JavaScript/CSS bundle and its source map from a URL",
                "args": [
                    {"name": "url", "type": "string", "required": true, "description": "URL of the JavaScript or CSS file"}
                ],
                "flags": {
                    "-o, --output": {"type": "path", "required": false, "description": "Output directory (default: current directory)"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "sources",
                "description": "List or extract original sources embedded in a source map",
                "args": [
                    {"name": "file", "type": "path", "required": true, "description": "Source map file"}
                ],
                "flags": {
                    "--extract": {"type": "bool", "default": false, "description": "Extract sourcesContent to files on disk"},
                    "-o, --output": {"type": "path", "required": false, "description": "Output directory for extracted files (default: current directory)"},
                    "--json": {"type": "bool", "default": false, "description": "Output as JSON"}
                }
            },
            {
                "name": "schema",
                "description": "Describe all commands and their arguments as JSON (this output)",
                "args": [],
                "flags": {}
            }
        ]
    });
    println!("{}", serde_json::to_string_pretty(&schema).unwrap());
    Ok(())
}

// ── Main ─────────────────────────────────────────────────────────

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Determine if --json is active for structured error output
    let json_mode = matches!(
        &cli.command,
        Command::Info { json: true, .. }
            | Command::Validate { json: true, .. }
            | Command::Lookup { json: true, .. }
            | Command::Resolve { json: true, .. }
            | Command::Encode { json: true, .. }
            | Command::Mappings { json: true, .. }
            | Command::Concat { json: true, .. }
            | Command::Remap { json: true, .. }
            | Command::Symbolicate { json: true, .. }
            | Command::Scopes { json: true, .. }
            | Command::Fetch { json: true, .. }
            | Command::Sources { json: true, .. }
    );

    let result = match &cli.command {
        Command::Info { file, json } => cmd_info(file, *json),
        Command::Validate { file, json } => match cmd_validate(file, *json) {
            Ok(true) => Ok(()),
            Ok(false) => return ExitCode::FAILURE,
            Err(e) => Err(e),
        },
        Command::Lookup {
            file,
            line,
            column,
            bias,
            context,
            json,
        } => cmd_lookup(file, *line, *column, bias, *context, *json),
        Command::Resolve {
            file,
            source,
            line,
            column,
            bias,
            json,
        } => cmd_resolve(file, source, *line, *column, bias, *json),
        Command::Decode { mappings, compact } => cmd_decode(mappings.clone(), *compact),
        Command::Encode { file, json } => cmd_encode(file.clone(), *json),
        Command::Mappings {
            file,
            source,
            limit,
            offset,
            json,
        } => cmd_mappings(file, source, *limit, *offset, *json),
        Command::Concat {
            files,
            output,
            file_name,
            json,
            dry_run,
        } => cmd_concat(files, output, file_name.clone(), *json, *dry_run),
        Command::Remap {
            file,
            dir,
            upstreams,
            output,
            json,
            dry_run,
        } => cmd_remap(file, dir, upstreams, output, *json, *dry_run),
        Command::Symbolicate {
            file,
            dir,
            maps,
            json,
        } => cmd_symbolicate(file, dir, maps, *json),
        Command::Scopes { file, json } => cmd_scopes(file, *json),
        Command::Fetch { url, output, json } => cmd_fetch(url, output, *json),
        Command::Sources {
            file,
            extract,
            output,
            json,
        } => cmd_sources(file, *extract, output, *json),
        Command::Schema => cmd_schema(),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            if json_mode {
                eprintln!("{}", e.to_json());
            } else {
                eprintln!("error: {}", e.message);
            }
            ExitCode::FAILURE
        }
    }
}
