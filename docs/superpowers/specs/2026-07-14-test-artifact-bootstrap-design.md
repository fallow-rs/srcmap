# Test Artifact Bootstrap Design

## Goal

Provide one repository command that prepares every generated NAPI and WASM artifact required by the JavaScript test suite in a clean checkout or isolated worktree.

## Command contract

Add `pnpm build:test-artifacts` at the workspace root. It runs two internal scripts in sequence:

- `build:test-artifacts:napi` builds the codec and sourcemap NAPI binaries with `--no-js`, preserving the tracked JavaScript loaders while still generating the platform binary and declaration outputs.
- `build:test-artifacts:wasm` runs the existing `build:all` scripts for sourcemap, generator, remapping, and symbolicate WASM packages.

The command requires the existing project prerequisites, including `wasm-pack`. It stops on the first failed build and does not install dependencies or run tests implicitly.

## CI ownership

Replace the duplicated NAPI and WASM build commands in the JavaScript runtime workflow with `corepack pnpm run build:test-artifacts`. CI then validates the same public command contributors use locally. Keep the declaration check after the bootstrap command and before the JavaScript tests.

Extend the workflow policy test to require this root command in the JavaScript runtime job and reject reintroduced package-specific build commands in that job.

## Documentation

Update `CONTRIBUTING.md` so quick start uses the single bootstrap command. Keep the existing focused package build examples for contributors working on one binding package. Update the JavaScript testing note to name the bootstrap command explicitly.

## Verification

Use a clean isolated worktree with dependencies installed but no ignored binding artifacts. Run `pnpm build:test-artifacts`, confirm all required NAPI and WASM outputs exist, and confirm tracked loaders remain unchanged. Then run the workflow policy test, JavaScript tests, the full quality gate, and the full repository test suite.
