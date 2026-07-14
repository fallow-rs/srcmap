# Fetch Fixture Test Hardening Design

## Goal

Make the regression test for complete HTTP request header consumption deterministic, and correct the implementation plan so its expected diff matches the work that was merged.

## Design

Keep the production `TcpStream` wrapper responsible for configuring its read timeout. Extract the bounded request-header reading and path parsing into a helper generic over `Read`.

Test the helper with a staged reader. The reader provides the request line, signals when the helper requests more input, blocks until the test releases the remaining headers, and then provides the header terminator. The test can use channel state instead of elapsed time to prove that parsing does not complete after the request line alone.

The existing real TCP fetch tests continue to exercise the wrapper and network behavior. The new unit-level regression test isolates only the synchronization-sensitive header consumption contract.

## Documentation

Update the existing implementation plan's expected final scope to include the intentional `.gitignore` change for `.worktrees/`.

## Verification

Prove the regression test by temporarily restoring request-line-only behavior and observing the focused test fail, then restore complete-header reading and observe it pass. Run the focused CLI fetch tests, the full repository quality gate, and the full test suite before publishing the change.
