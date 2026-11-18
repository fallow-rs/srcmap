# srcmap-cli

[![crates.io](https://img.shields.io/crates/v/srcmap-cli.svg)](https://crates.io/crates/srcmap-cli)
[![CI](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/fallow-rs/srcmap/actions/workflows/ci.yml)

CLI tool to inspect, validate, compose, and manipulate source maps.

Agent-friendly with `--json` structured output on most commands and `srcmap schema` for runtime introspection.

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
| `scopes` | Inspect scopes-proposal data and variable bindings |
| `fetch` | Fetch a JS/CSS bundle and its source map from a URL |
| `sources` | List or extract original sources from a source map |
| `schema` | Describe all commands as JSON (for agent introspection) |

## Usage

```sh
# Inspect a source map
srcmap info bundle.js.map

# Validate
srcmap validate bundle.js.map

# Validate, and compare the debug ID of the generated file with the debug ID of the map
srcmap validate bundle.js.map --generated bundle.js

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

# Inspect scopes-proposal data
srcmap scopes bundle.js.map

# Fetch a bundle and its source map from a URL
srcmap fetch https://cdn.example.com/app.min.js -o ./debug

# List embedded original sources
srcmap sources bundle.js.map

# Extract all original sources to disk
srcmap sources bundle.js.map --extract -o ./src

# Most commands support --json for structured output
srcmap info bundle.js.map --json

# Introspect all commands and their arguments
srcmap schema
```

## Features

- **Structured JSON output.** Most commands take `--json`.
- **Agent introspection.** `srcmap schema` describes all commands, arguments, types and flags as JSON.
- **stdin support.** Pass `-` as the file argument to read from stdin.
- **Debug ID check.** `srcmap validate --generated <PATH>` compares the `debugId` comment of the generated file with the `debugId` of the source map. The command exits with 1 when the IDs differ, or when only one side has an ID. A file with the extension `.css` has the CSS comment form.
- **Dry run.** `concat` and `remap` take `--dry-run`, which validates the input and writes nothing.
- **Input hardening.** The CLI rejects control characters, path traversals and percent-encoding in source names.
- **Output sandboxing.** The CLI writes only inside the current working directory.
- **Remote fetching.** `srcmap fetch` downloads a bundle and its source map from a URL. It reads the `sourceMappingURL` comment to find the map.
- **Source extraction.** `srcmap sources --extract` writes the embedded `sourcesContent` to disk and keeps the directory structure.
- **Context lines.** `srcmap lookup --context N` prints N lines of original source around the mapped position.
- **Structured errors.** JSON mode prints an error code (`IO_ERROR`, `PARSE_ERROR`, `NOT_FOUND`, `VALIDATION_ERROR`, `PATH_TRAVERSAL`, `INVALID_INPUT`, `FETCH_ERROR`).

## Part of [srcmap](https://github.com/fallow-rs/srcmap)

See also:
- [`srcmap-sourcemap`](https://crates.io/crates/srcmap-sourcemap) - Parser and consumer
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode

## License

MIT
