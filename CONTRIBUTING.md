# Contributing to srcmap

Thanks for your interest in contributing to srcmap! Whether it's a bug fix, new feature, documentation improvement, or benchmark — all contributions are welcome.

## Quick start

**Prerequisites:**

- [Rust](https://rustup.rs/) (latest stable, edition 2024)
- [Node.js](https://nodejs.org/) (for running JS tests and benchmarks)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) (for building WASM packages)

**Setup:**

```bash
git clone https://github.com/BartWaardenburg/srcmap.git
cd srcmap

# Build all Rust crates
cargo build

# Run tests
cargo test

# Run JS tests (requires building WASM packages first)
npm test
```

## Project structure

```
crates/
  codec/          VLQ encode/decode primitives
  sourcemap/      Source map parser + consumer (O(log n) lookups)
  generator/      Incremental source map builder
  remapping/      Concatenation + composition
  scopes/         ECMA-426 scopes & variables
  symbolicate/    Stack trace symbolication
  hermes/         Hermes bytecode source map support
  ram-bundle/     React Native RAM bundle support
  cli/            CLI with structured JSON output

packages/
  sourcemap-wasm/    WASM bindings for sourcemap
  generator-wasm/    WASM bindings for generator
  remapping-wasm/    WASM bindings for remapping
  scopes-wasm/       WASM bindings for scopes
  symbolicate-wasm/  WASM bindings for symbolicate
  codec/             NAPI bindings for codec
  sourcemap/         NAPI bindings for sourcemap
  generator/         NAPI bindings for generator
  remapping/         NAPI bindings for remapping
  trace-mapping/     Drop-in @jridgewell/trace-mapping replacement

benchmarks/       JS benchmarks comparing against existing libraries
```

## Development workflow

### Building

```bash
cargo build                     # Debug build
cargo build --release           # Optimized build

# Build a specific WASM package
cd packages/sourcemap-wasm && wasm-pack build --target web
```

### Testing

```bash
cargo test                      # All Rust tests
cargo test -p srcmap-sourcemap  # Single crate
npm run test:js                 # JS/WASM tests
```

### Benchmarks

```bash
# Rust benchmarks (criterion)
cargo bench -p srcmap-sourcemap

# JS benchmarks (comparison with other libraries)
cd benchmarks && npm install && node sourcemap-wasm.mjs
```

### Coverage

```bash
npm run coverage:rust           # Rust coverage (requires cargo-llvm-cov)
npm run coverage:js             # JS coverage
```

## Code standards

- **Formatting:** `cargo fmt` is enforced by a pre-commit hook and CI. Run it before committing.
- **Linting:** `cargo clippy` must pass without warnings.
- **Tests:** Add or update tests for any changed behavior. All crates should maintain good test coverage.
- **Documentation:** Public APIs should have doc comments. Use `cargo doc --open` to preview.

## Commit conventions

We use [conventional commits](https://www.conventionalcommits.org/):

- `feat:` — new feature
- `fix:` — bug fix
- `refactor:` — code change that neither fixes a bug nor adds a feature
- `test:` — adding or updating tests
- `docs:` — documentation only
- `chore:` — maintenance, CI, dependencies
- `perf:` — performance improvement

Example: `feat: add name resolution to scopes decoder`

## Pull request process

1. Fork the repo and create a branch from `main`.
2. Make your changes, ensuring `cargo fmt`, `cargo clippy`, and `cargo test` all pass.
3. Write a clear PR description — the repo has a [PR template](.github/PULL_REQUEST_TEMPLATE.md) to guide you.
4. For performance-sensitive changes, include benchmark results.
5. Keep PRs focused. Prefer smaller, reviewable changes over large ones.

## Reporting bugs and suggesting features

- **Bugs:** Open an issue using the [bug report template](https://github.com/BartWaardenburg/srcmap/issues/new?template=bug_report.yml). Include a minimal reproducing source map when possible.
- **Features:** Open an issue using the [feature request template](https://github.com/BartWaardenburg/srcmap/issues/new?template=feature_request.yml). Discussion before implementation is encouraged for larger changes.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
