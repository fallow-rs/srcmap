# Contributing to srcmap

Thanks for your interest in contributing to srcmap! Whether it's a bug fix, new feature, documentation improvement, or benchmark, all contributions are welcome.

## Quick start

**Prerequisites:**

- [Rust](https://rustup.rs/) 1.88 or newer (edition 2024)
- [Node.js](https://nodejs.org/) 22 with Corepack available (for running JS tests and benchmarks)
- [wasm-pack](https://rustwasm.github.io/wasm-pack/installer/) (for building WASM packages)
- [cargo-deny](https://github.com/EmbarkStudios/cargo-deny) and [typos](https://github.com/crate-ci/typos) (for the root checks)
- Optional: [mise](https://mise.jdx.dev) to install the pinned tools in `mise.toml`, and [hk](https://hk.jdx.dev) for the git hooks

**Setup:**

```bash
git clone https://github.com/fallow-rs/srcmap.git
cd srcmap
corepack enable
corepack pnpm install --frozen-lockfile

# Optional: install the pinned tools and enable the git hooks
mise install
hk install --mise

# Build the Rust workspace
cargo build --workspace

# Build all generated NAPI and WASM artifacts used by the JavaScript tests
corepack pnpm run build:test-artifacts

# Run all repository checks
corepack pnpm run check

# Run the Rust and JavaScript test suites
corepack pnpm test
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
  scopes-wasm/       Experimental WASM bindings for scopes (unpublished)
  symbolicate-wasm/  WASM bindings for symbolicate
  codec/             NAPI bindings for codec
  sourcemap/         NAPI bindings for sourcemap
  generator/         Experimental NAPI bindings for generator (unpublished)
  remapping/         JavaScript remapping wrapper plus experimental NAPI bindings
  trace-mapping/     Drop-in @jridgewell/trace-mapping replacement
  gen-mapping/       Drop-in @jridgewell/gen-mapping replacement
  source-map/        Drop-in Mozilla source-map v0.6 replacement

benchmarks/       JS benchmarks comparing against existing libraries
```

The experimental `generator`, `remapping` NAPI, and `scopes-wasm` binding crates remain workspace members so they compile with the rest of the repository. They use `publish = false` and are not released as npm packages. The published `@srcmap/remapping` JavaScript wrapper shares the `packages/remapping` directory but does not publish the experimental NAPI binary.

## Development workflow

### Building

```bash
cargo build                     # Debug build
cargo build --release           # Optimized build

# Build NAPI test artifacts without rewriting tracked loaders
corepack pnpm run build:test-artifacts:napi

# Build a specific WASM package for Node.js and browsers
corepack pnpm --filter @srcmap/sourcemap-wasm build:all
```

### Testing

```bash
cargo test                      # All Rust tests
cargo test -p srcmap-sourcemap  # Single crate
corepack pnpm run test:js       # JS/WASM tests (run build:test-artifacts first)
```

### Benchmarks

```bash
# Rust benchmarks (criterion)
cargo bench -p srcmap-sourcemap

# JS benchmarks (comparison with other libraries)
corepack pnpm --dir benchmarks run bench:wasm
```

### Coverage

```bash
corepack pnpm run coverage:rust # Rust coverage (requires cargo-llvm-cov)
corepack pnpm run coverage:js   # JS coverage
```

## Git hooks

[hk](https://hk.jdx.dev) runs the git hooks. `hk.pkl` defines them, and each
step runs a command that CI also runs:

- `pre-commit`: `cargo fmt --check` and Clippy when a Rust file or Cargo file
  is staged, `typos`, and oxfmt and oxlint when a JavaScript package file is
  staged.
- `pre-push`: `cargo fmt --check`, Clippy, `cargo doc`, `cargo shear`,
  `cargo deny` (bans, licenses and sources; CI also checks advisories) and
  `fallow dead-code`.

Without mise, install `hk` and `pkl` at the versions in `mise.toml` and run
`hk install`. The typos, cargo-shear and cargo-deny steps skip with a hint when
the tool is missing. The JavaScript steps skip when `node_modules` is missing.
The hooks check the working tree, not only the staged content. The
pre-commit hook only checks. Run `hk fix` to apply `cargo fmt` and oxfmt.
`HK=0 git commit` skips the hooks for one command.

If you set `core.hooksPath` to `.githooks` in an earlier clone, remove it
before `hk install`: `git config --unset core.hooksPath`.

## Code standards

- **Formatting:** `cargo fmt` is enforced by CI. Run it before committing.
- **Linting:** `cargo clippy` must pass without warnings.
- **Tests:** Add or update tests for any changed behavior. All crates should maintain good test coverage.
- **Documentation:** Public APIs should have doc comments. Use `cargo doc --open` to preview.

## Commit conventions

We use [conventional commits](https://www.conventionalcommits.org/):

- `feat:`: new feature
- `fix:`: bug fix
- `refactor:`: code change that neither fixes a bug nor adds a feature
- `test:`: adding or updating tests
- `docs:`: documentation only
- `chore:`: maintenance, CI, dependencies
- `perf:`: performance improvement

Example: `feat: add name resolution to scopes decoder`

## Pull request process

1. Fork the repo and create a branch from `main`.
2. Make your changes, ensuring `corepack pnpm run check` and `corepack pnpm test` both pass.
3. Write a clear PR description. The repo has a [PR template](.github/PULL_REQUEST_TEMPLATE.md) to guide you.
4. For performance-sensitive changes, include benchmark results.
5. Keep PRs focused. Prefer smaller, reviewable changes over large ones.

## Reporting bugs and suggesting features

- **Bugs:** Open an issue using the [bug report template](https://github.com/fallow-rs/srcmap/issues/new?template=bug_report.yml). Include a minimal reproducing source map when possible.
- **Features:** Open an issue using the [feature request template](https://github.com/fallow-rs/srcmap/issues/new?template=feature_request.yml). Discussion before implementation is encouraged for larger changes.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
