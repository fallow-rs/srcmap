# Fetch Fixture Test Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the scheduler-sensitive HTTP fixture regression test with deterministic staged input and correct the prior plan's expected file scope.

**Architecture:** Keep `read_request` responsible for the `TcpStream` timeout and delegate header consumption to a helper generic over `Read`. Exercise that helper with a staged reader that signals and blocks exactly when more bytes are requested after the request line.

**Tech Stack:** Rust 2024, standard library I/O and channels, Cargo integration tests, pnpm repository scripts.

## Global Constraints

- Preserve the existing 32 KiB request-header bound and two-second socket timeout.
- Add no dependencies.
- Keep production HTTP fixture behavior unchanged.
- Use signed conventional commits without attribution.

---

### Task 1: Deterministic complete-header regression test

**Files:**
- Modify: `crates/cli/tests/cli.rs:1-10,955-999`

**Interfaces:**
- Consumes: `std::io::Read`, `std::sync::mpsc`, and the existing `MAX_REQUEST_HEADERS_SIZE` constant.
- Produces: `read_request_from<R: Read>(reader: &mut R) -> String` and a staged-reader regression test.

- [ ] **Step 1: Extract the reader-generic helper without changing behavior**

Keep socket configuration in `read_request` and move the existing complete-header loop and path extraction into:

```rust
fn read_request(stream: &mut TcpStream) -> String {
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    read_request_from(stream)
}

fn read_request_from<R: Read>(reader: &mut R) -> String {
    let mut request = Vec::with_capacity(512);
    while !request.ends_with(b"\r\n\r\n") {
        let mut byte = [0_u8; 1];
        reader.read_exact(&mut byte).unwrap();
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

- [ ] **Step 2: Add a staged `Read` implementation and replace the timeout-based test**

Use `Cursor<Vec<u8>>` for the two stages. On the first read after the request line is exhausted, signal the test and wait for explicit release before exposing the remaining headers:

```rust
struct StagedRequest {
    request_line: Cursor<Vec<u8>>,
    headers: Cursor<Vec<u8>>,
    waiting_sender: Option<mpsc::Sender<()>>,
    release_receiver: Receiver<()>,
}

impl Read for StagedRequest {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.request_line.read(buffer)?;
        if read > 0 {
            return Ok(read);
        }

        if let Some(sender) = self.waiting_sender.take() {
            sender.send(()).map_err(std::io::Error::other)?;
            self.release_receiver.recv().map_err(std::io::Error::other)?;
        }

        self.headers.read(buffer)
    }
}
```

Spawn `read_request_from` on a worker thread. Wait for the staged reader's signal, assert `path_receiver.try_recv()` returns `mpsc::TryRecvError::Empty`, release the remaining headers, and assert the parsed path is `/complete`.

- [ ] **Step 3: Prove the regression test detects request-line-only behavior**

Temporarily change the helper loop terminator to stop after the first newline. Run:

```bash
cargo test -p srcmap-cli --test cli read_request_waits_for_complete_headers -- --exact
```

Expected: FAIL because the worker returns before the staged reader requests the remaining headers. Restore complete-header reading immediately afterward.

- [ ] **Step 4: Verify the focused behavior**

Run:

```bash
cargo test -p srcmap-cli --test cli read_request_waits_for_complete_headers -- --exact
cargo test -p srcmap-cli --test cli fetch
```

Expected: both commands exit successfully.

### Task 2: Documentation scope and repository verification

**Files:**
- Modify: `docs/superpowers/plans/2026-07-14-js-dependencies-fetch-fixture.md:217`

**Interfaces:**
- Consumes: the merged file list from PR #75.
- Produces: an accurate expected-scope statement naming the intentional worktree ignore entry.

- [ ] **Step 1: Correct the expected final scope**

Replace the stale sentence with:

```markdown
Expected: only the design, plan, fixture test helper, dependency manifests, lockfiles, and the intentional `.worktrees/` ignore entry are changed; commits have valid signatures.
```

- [ ] **Step 2: Run full verification with bounded logs**

Run:

```bash
pnpm check > /tmp/srcmap-fetch-hardening-check.log 2>&1
pnpm test > /tmp/srcmap-fetch-hardening-test.log 2>&1
git diff --check origin/main...HEAD
```

Expected: all commands exit successfully.

- [ ] **Step 3: Review and commit the intended scope**

Review `git status --short`, `git diff`, and staged files. Commit the test and documentation changes with:

```bash
git commit -S -m "test: harden fetch fixture synchronization"
```

- [ ] **Step 4: Push and create the pull request**

Push `codex/harden-fetch-fixture-test`, create a ready pull request targeting `main`, require all checks, and verify the exact merge commit after merging.
