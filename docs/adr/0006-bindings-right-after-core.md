# 0006. Python and WebAssembly right after Core

Status: accepted · 2026-10-04

## Context

The first plan put language bindings last (M6). A survey of RML usage changed the order.

- The largest active RML user base is in Python: the most used Python RML engine alone is downloaded about 31,000 times a month (PyPI, September 2026), mostly from Linux CI and containers.
- Rust-backed Python packages that keep the existing usage pattern get adopted (a Rust-backed RDF store: about 250,000 downloads a month). Fast native engines that ask users to change tools have not been.
- Browser mapping editors run their mappings on a remote engine. No specification-conformant engine runs inside the browser.

## Decision

Add milestone M1.5 right after RML-Core: a Python wheel and a WebAssembly build, both thin wrappers over the library API. The WebAssembly build supports in-memory sources only.

## Consequences

- SQL (M2) starts later.
- The library's in-memory input API must be settled in M1, since both bindings depend on it.

## Not adopted (for now)

From the same review, these stay as in the roadmap: legacy RML vocabulary stays in M6, the publication date is not fixed, no outreach plan to RMLio yet, and structured error reports and an MCP server are not planned.
