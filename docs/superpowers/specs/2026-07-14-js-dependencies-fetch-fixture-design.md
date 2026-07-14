# JavaScript Dependencies and Fetch Fixture Stability

## Context

The root JavaScript tooling dependencies have newer releases available:

- `fallow` 3.5.0
- `oxfmt` 0.59.0
- `oxlint` 1.74.0

Separately, Windows CI intermittently times out while fetch integration tests wait for a second local HTTP request. The local fixture currently reads only the request line before writing a response and closing the socket. Unread request headers can cause Windows to reset the connection, so the client treats the first request as failed and never sends the expected second request.

## Goals

- Update the three root JavaScript tooling dependencies to their current exact versions.
- Make the local HTTP fixture consume a complete request header block before responding.
- Prove the fixture behavior with a deterministic regression test.
- Keep production fetch behavior unchanged.
- Preserve the intentional `memchr` 2.8.2 lockfile selection because 2.8.3 failed the performance gate.

## Non-goals

- Refactor the production fetch implementation.
- Add an HTTP mock-server dependency.
- Increase timeouts to hide the socket reset.
- Change unrelated test infrastructure.

## Design

### Dependency update

Update the exact versions in the root `package.json`, regenerate `pnpm-lock.yaml`, and run the existing formatting, linting, analysis, and test commands. No package scripts or configuration should change unless a new tool version exposes a real incompatibility.

### HTTP fixture

Change `read_request` in `crates/cli/tests/cli.rs` to read bytes until the HTTP header terminator `\r\n\r\n`. Retain the first request line for path parsing and reject an unexpectedly large header block with a named size limit.

This ensures the server has consumed the client's request data before it writes a response and drops the connection. The fixture remains dependency-free and continues to support only the GET requests needed by these tests.

### Regression test

Add a fixture-level test that opens a loopback connection and sends the request in two stages:

1. Send only the request line and verify `read_request` has not completed.
2. Send headers and the terminating blank line.
3. Verify `read_request` completes and returns the expected path.

The test must fail against the current request-line-only implementation before the helper is changed. It must pass after the complete-header implementation is added.

## Verification

- Run the focused regression test and record the expected red then green result.
- Repeatedly run the fetch integration tests to exercise the fixture.
- Run `pnpm check` and `pnpm test`.
- Confirm the lockfile has no unexpected dependency changes.
- Push a pull request and require Windows CI, security checks, coverage, and CodSpeed to pass.
- After merge, verify every workflow for the exact merge commit on `main`.

## Rollout

Land the dependency update and fixture fix together because the test-only fix removes the CI flake that could otherwise obscure validation of the dependency update. If a tooling update causes a separate failure, isolate that compatibility change in its own commit before merging.
