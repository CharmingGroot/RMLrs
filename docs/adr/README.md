# Architecture decision records

| ADR | Decision | Status |
|---|---|---|
| [0001](0001-pure-and-minimal-scope.md) | Scope is mapping + sources → RDF, specifications only | Accepted |
| [0002](0002-own-mapping-parser.md) | Write our own mapping parser instead of depending on an existing front end | Accepted |
| [0003](0003-json-and-sql-first.md) | JSON and SQL sources first, then CSV and XML | Accepted |
| [0004](0004-conformance-runner-and-suites.md) | Own conformance runner, suites as pinned submodules, no-regression gate | Accepted |
| [0005](0005-no-java-binding.md) | No Java binding; conformance at the level of the most complete engines | Accepted |
| [0006](0006-bindings-right-after-core.md) | Python and WebAssembly right after Core (M1.5) | Accepted |
| [0007](0007-xpath-with-xee.md) | XPath 3.1 through xee-xpath, behind the `xml` feature | Accepted |
| [0008](0008-fnml-built-in-functions.md) | RML-FNML functions: descriptions and code in Rust, plus host functions | Accepted |
| [0009](0009-http-client.md) | HTTP client: ureq on rustls with OS certificates, behind the `http` feature | Accepted |

Format: context, options, decision, consequences, and when to revisit.
