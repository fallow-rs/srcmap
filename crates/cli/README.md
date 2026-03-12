# srcmap-cli

[![crates.io](https://img.shields.io/crates/v/srcmap-cli.svg)](https://crates.io/crates/srcmap-cli)
[![CI](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml/badge.svg)](https://github.com/BartWaardenburg/srcmap/actions/workflows/ci.yml)

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
| `schema` | Describe all commands as JSON (for agent introspection) |

## Usage

```sh
# Inspect a source map
srcmap info bundle.js.map

# Validate
srcmap validate bundle.js.map

# Look up original position (0-based line:column)
srcmap lookup bundle.js.map 42 12

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
- **Structured errors** — error codes (`IO_ERROR`, `PARSE_ERROR`, `NOT_FOUND`, `VALIDATION_ERROR`, `PATH_TRAVERSAL`, `INVALID_INPUT`) in JSON mode

## Part of [srcmap](https://github.com/BartWaardenburg/srcmap)

See also:
- [`srcmap-sourcemap`](https://crates.io/crates/srcmap-sourcemap) - Parser and consumer
- [`srcmap-generator`](https://crates.io/crates/srcmap-generator) - Source map builder
- [`srcmap-remapping`](https://crates.io/crates/srcmap-remapping) - Concatenation and composition
- [`srcmap-codec`](https://crates.io/crates/srcmap-codec) - VLQ encode/decode

## License

MIT
