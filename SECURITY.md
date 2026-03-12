# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.2.x   | Yes                |
| < 0.2   | No                 |

## Reporting a Vulnerability

**Please do not open a public GitHub issue for security vulnerabilities.**

Instead, report vulnerabilities through one of the following channels:

- GitHub: [Private vulnerability reporting](https://github.com/BartWaardenburg/srcmap/security/advisories/new)

Include as much detail as possible: steps to reproduce, affected versions, and potential impact.

## What to Expect

- **Acknowledgment** within 48 hours of your report.
- **Status update** within 7 days with an initial assessment and expected timeline.
- A fix will be developed privately and released as a patch version. A security advisory will be published once the fix is available.

## Scope

The following are considered security issues for this project:

- Memory safety violations triggered by malformed or malicious source map input (buffer overflows, use-after-free, out-of-bounds reads)
- Denial of service through crafted input (e.g., excessive memory allocation, infinite loops)
- WASM sandbox escapes or unexpected host environment access
- Vulnerabilities in dependencies that are exploitable through srcmap's public API

The following are generally **not** in scope:

- Issues that require physical access to the machine running srcmap
- Bugs that do not have a security impact (please use regular GitHub issues for those)

## Recognition

Contributors who report valid security vulnerabilities will be credited in the published advisory, unless they prefer to remain anonymous.
