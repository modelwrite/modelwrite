# Security

Modelwrite parses untrusted model files; treat any crash, panic, or memory
unsafety triggered by a crafted OKF or legacy file as a security issue.

## Reporting

Email security@modelwrite.org with a minimal reproducer. Please do not open
a public issue for a suspected vulnerability. We acknowledge within 5
business days and aim to fix within 90 days.

## Scope

- The Rust engine (parsing, validation, gate, C ABI, MCP server).
- The judge harness when it handles untrusted files.

The static site and sample data are out of scope. There is no bounty
program yet.
