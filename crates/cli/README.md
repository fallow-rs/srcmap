# srcmap-cli

[![crates.io](https://img.shields.io/crates/v/srcmap-cli.svg)](https://crates.io/crates/srcmap-cli)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

CLI tool to inspect, validate, compose, and manipulate source maps.

Agent-friendly with `--json` structured output on all commands and `srcmap schema` for runtime introspection.

## Install

```sh
cargo install srcmap-cli
```

## Commands

| Command | Description |
|---------|-------------|
| `info` | Show source map metadata and statistics |
| `validate` | Validate a source map file |
| `lookup` | Find original position for a generated position |
| `resolve` | Find generated position for an original position |
| `decode` | Decode a VLQ mappings string to JSON |
| `encode` | Encode decoded mappings JSON back to a VLQ string |
| `mappings` | List all mappings with pagination |
| `concat` | Concatenate multiple source maps into one |
| `remap` | Compose/remap source maps through a transform chain |
| `symbolicate` | Symbolicate a stack trace using source maps |
| `scopes` | Inspect ECMA-426 scopes and variable bindings |
| `fetch` | Fetch a JS/CSS bundle and its source map from a URL |
| `sources` | List or extract original sources from a source map |
| `schema` | Describe all commands as JSON (for agent introspection) |

## Usage

```sh
# Inspect a source map
srcmap info bundle.js.map

# Validate
srcmap validate bundle.js.map

# Look up original position (0-based line:column)
srcmap lookup bundle.js.map 42 12

# Look up with surrounding source context
srcmap lookup bundle.js.map 42 12 --context 5

# Reverse lookup
srcmap resolve bundle.js.map --source src/app.ts 10 0

# Decode VLQ mappings
srcmap decode "AAAA;AACA"

# List mappings with pagination
srcmap mappings bundle.js.map --limit 100 --offset 0

# Concatenate source maps
srcmap concat a.js.map b.js.map -o bundle.js.map

# Remap through upstream maps
srcmap remap minified.js.map --dir ./sourcemaps -o original.js.map

# Symbolicate a stack trace
srcmap symbolicate stacktrace.txt --dir ./sourcemaps

# Inspect ECMA-426 scopes
srcmap scopes bundle.js.map

# Fetch a bundle and its source map from a URL
srcmap fetch https://cdn.example.com/app.min.js -o ./debug

# List embedded original sources
srcmap sources bundle.js.map

# Extract all original sources to disk
srcmap sources bundle.js.map --extract -o ./src

# All commands support --json for structured output
srcmap info bundle.js.map --json

# Introspect all commands and their arguments
srcmap schema
```

## Features

- **Structured JSON output** — `--json` flag on all commands for machine-readable output
- **Agent introspection** — `srcmap schema` describes all commands, args, types, and flags as JSON
- **stdin support** — use `-` as file argument to read from stdin
- **Dry run** — `--dry-run` on mutating commands (`concat`, `remap`) to validate without writing
- **Input hardening** — rejects control characters, path traversals, and percent-encoding in source names
- **Output sandboxing** — all file writes validated against the current working directory
- **Remote fetching** — `srcmap fetch` downloads bundles and source maps from URLs, resolving `sourceMappingURL` automatically
- **Source extraction** — `srcmap sources --extract` writes embedded `sourcesContent` to disk with directory structure
- **Context lines** — `srcmap lookup --context N` shows surrounding original source around a mapped position
- **Structured errors** — error codes (`IO_ERROR`, `PARSE_ERROR`, `NOT_FOUND`, `VALIDATION_ERROR`, `PATH_TRAVERSAL`, `INVALID_INPUT`, `FETCH_ERROR`) in JSON mode

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See also:
- [`srcmap-sourcemap`](https://crates.io/crates/srcmap-sourcemap) - Parser and consumer
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode

## License

MIT
