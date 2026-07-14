# JavaScript Dependencies and Fetch Fixture Stability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Update the root JavaScript tooling and eliminate intermittent Windows fetch-test socket resets without changing production behavior.

**Architecture:** Keep the dependency refresh isolated to `package.json` and `pnpm-lock.yaml`. Correct the test-only HTTP fixture by reading a complete header block before responding, with a deterministic test that proves the helper does not return after only a request line.

**Tech Stack:** Rust 2024, standard-library TCP sockets and channels, Cargo tests, pnpm, fallow, oxfmt, oxlint, GitHub Actions, CodSpeed.

## Global Constraints

- Keep production fetch behavior unchanged.
- Add no HTTP mock-server dependency.
- Do not increase timeouts to hide the socket reset.
- Preserve `memchr` 2.8.2 because 2.8.3 failed the performance gate.
- Use signed conventional commits.

## File Structure

- Modify `crates/cli/tests/cli.rs`: deterministic fixture regression test and complete-header request reader.
- Modify `package.json`: exact root tooling versions.
- Modify `pnpm-lock.yaml`: regenerated dependency graph.
- Create `docs/superpowers/plans/2026-07-14-js-dependencies-fetch-fixture.md`: execution record.

---

### Task 1: Consume Complete HTTP Fixture Requests

**Files:**
- Modify and test: `crates/cli/tests/cli.rs:950-1015`

**Interfaces:**
- Consumes: `TcpListener`, `TcpStream`, `mpsc`, `Duration`, and the existing `read_request(&mut TcpStream) -> String` helper.
- Produces: the same `read_request(&mut TcpStream) -> String` interface, now returning only after `\r\n\r\n` has been consumed.

- [ ] **Step 1: Add the deterministic failing test after `read_request`**

```rust
#[test]
fn read_request_waits_for_complete_headers() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let (accepted_sender, accepted_receiver) = mpsc::channel();
    let (path_sender, path_receiver) = mpsc::channel();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        accepted_sender.send(()).unwrap();
        path_sender.send(read_request(&mut stream)).unwrap();
    });

    let mut client = TcpStream::connect(address).unwrap();
    accepted_receiver.recv_timeout(Duration::from_secs(2)).unwrap();
    client.write_all(b"GET /complete HTTP/1.1\r\n").unwrap();
    client.flush().unwrap();

    assert!(
        path_receiver.recv_timeout(Duration::from_millis(100)).is_err(),
        "request completed before the header terminator"
    );

    client.write_all(b"Host: localhost\r\nConnection: close\r\n\r\n").unwrap();
    assert_eq!(path_receiver.recv_timeout(Duration::from_secs(2)).unwrap(), "/complete");
    server.join().unwrap();
}
```

- [ ] **Step 2: Run the focused test and verify red**

Run:

```bash
cargo test -p srcmap-cli --test cli read_request_waits_for_complete_headers -- --exact
```

Expected: FAIL with `request completed before the header terminator`.

- [ ] **Step 3: Replace request-line-only reading with bounded complete-header reading**

```rust
const MAX_REQUEST_HEADERS_SIZE: usize = 32 * 1024;

fn read_request(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut request = Vec::with_capacity(512);
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        stream.read_exact(&mut byte).unwrap();
        request.push(byte[0]);
        assert!(
            request.len() <= MAX_REQUEST_HEADERS_SIZE,
            "HTTP request headers are unexpectedly large"
        );
    }

    let request_line = request.split(|byte| *byte == b'\n').next().unwrap_or_default();
    let request_line = String::from_utf8_lossy(request_line);
    request_line.split_whitespace().nth(1).unwrap_or_default().to_string()
}
```

- [ ] **Step 4: Run the focused test and fetch integration tests**

Run:

```bash
cargo test -p srcmap-cli --test cli read_request_waits_for_complete_headers -- --exact
cargo test -p srcmap-cli --test cli fetch_
```

Expected: PASS.

- [ ] **Step 5: Run a local stress loop**

Run:

```bash
for run in {1..25}; do cargo test -q -p srcmap-cli --test cli fetch_ || exit 1; done
```

Expected: every iteration exits successfully.

- [ ] **Step 6: Commit the fixture fix**

```bash
git add crates/cli/tests/cli.rs
git commit -S -m "test: consume complete fixture requests"
```

### Task 2: Update Root JavaScript Tooling

**Files:**
- Modify: `package.json:40-44`
- Modify: `pnpm-lock.yaml`

**Interfaces:**
- Consumes: existing root scripts for fallow, oxfmt, and oxlint.
- Produces: exact versions `fallow` 3.5.0, `oxfmt` 0.59.0, and `oxlint` 1.74.0 with a frozen-install-compatible lockfile.

- [ ] **Step 1: Update exact manifest versions**

```json
"devDependencies": {
  "fallow": "3.5.0",
  "oxfmt": "0.59.0",
  "oxlint": "1.74.0"
}
```

- [ ] **Step 2: Regenerate the lockfile without running package scripts**

Run:

```bash
pnpm install --lockfile-only --ignore-scripts
```

Expected: `package.json` and `pnpm-lock.yaml` resolve the three requested versions.

- [ ] **Step 3: Verify the updated tools**

Run:

```bash
pnpm run fmt:js:check
pnpm run lint:js
pnpm outdated --format list
```

Expected: formatting and linting pass, with no newer direct JavaScript dependency reported.

- [ ] **Step 4: Review dependency scope and preserve the Rust lock selection**

Run:

```bash
git diff -- package.json pnpm-lock.yaml
rg -n 'name = "memchr"|version = "2\.8\.2"' Cargo.lock
```

Expected: only the requested JavaScript tooling graph changes and `memchr` remains at 2.8.2.

- [ ] **Step 5: Commit the dependency update**

```bash
git add package.json pnpm-lock.yaml
git commit -S -m "chore: update JavaScript tooling"
```

### Task 3: Full Verification and Publication

**Files:**
- Verify all files changed since `origin/main`.

**Interfaces:**
- Consumes: completed Tasks 1 and 2.
- Produces: a clean pull request with passing local and remote validation, merged to `main`.

- [ ] **Step 1: Run full local verification with bounded logs**

```bash
pnpm check > /tmp/srcmap-js-fixture-check.log 2>&1
pnpm test > /tmp/srcmap-js-fixture-test.log 2>&1
```

Expected: both commands exit successfully.

- [ ] **Step 2: Review the complete diff and repository state**

```bash
git diff --check origin/main...HEAD
git diff --stat origin/main...HEAD
git status --short --branch
git log --show-signature --format=fuller origin/main..HEAD
```

Expected: only the design, plan, fixture test helper, dependency manifests, lockfiles, and the intentional `.worktrees/` ignore entry are changed; commits have valid signatures.

- [ ] **Step 3: Push and create a ready pull request**

```bash
git push -u origin codex/update-js-stabilize-fetch-tests
gh pr create --base main --head codex/update-js-stabilize-fetch-tests --title "test: stabilize fetch fixtures and update JS tooling" --body-file /tmp/srcmap-js-fixture-pr.md
```

Expected: a ready pull request is created.

- [ ] **Step 4: Require all PR checks**

Run `gh pr checks --watch` and inspect any failure before retrying or fixing it.

Expected: Windows CI, security, coverage, and CodSpeed pass.

- [ ] **Step 5: Squash merge and verify the exact merge commit**

```bash
gh pr merge --squash --delete-branch --subject "test: stabilize fetch fixtures and update JS tooling"
```

Expected: the pull request is merged, local `main` matches `origin/main`, and every workflow for the merge commit completes successfully.
