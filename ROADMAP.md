# Roadmap

**Pre-release (version 0.0.0; October 2026).** There is no tagged release or
published package yet. M0, M1, M1.5, M2, M4, M5 and M6 are done; M3
(performance) has published database numbers, and the file-based scenarios
are being measured again. What remains unsupported is listed in the
[README](README.md#features): Kafka and MQTT streams, SPARQL over RDF files,
CSVW table schemas. Results per suite are in the [README](README.md#conformance)
and [conformance/baseline.csv](conformance/baseline.csv), known test-suite
problems in [conformance/known-issues.csv](conformance/known-issues.csv),
measurements in [docs/benchmarks.md](docs/benchmarks.md).

RMLrs is a Rust engine for RML (RDF Mapping Language). It covers JSON, SQL, CSV and XML sources, with **JSON and SQL first**. The scope rules are in [PHILOSOPHY.md](PHILOSOPHY.md). The reasons behind the main choices are in [docs/adr](docs/adr).

## Target

At least the level of the most complete library engines today, measured on the official KG-Construct implementation report:

| Module | Target |
|---|---|
| RML-Core | 76 / 76 |
| RML-IO | ≥ 61 / 73 |
| RML-FNML | ≥ 18 / 20 |
| RML-CC | 35 / 35 |
| RML-LV | 41 / 41 |
| RML-IO-Registry, SQL cases | 77 / 77 |

Plus published performance numbers on the KROWN benchmark.

## Ecosystem goals

- **Listed in the official [RML implementation report](https://w3id.org/kg-construct/rml-implementation-report).** Submit `data/processors.csv` and `data/results/rmlrs.csv` after the first release, and update them on every release. The conformance runner (M0) writes results in that format.
- **Listed in [awesome-kgc-tools](https://github.com/kg-construct/awesome-kgc-tools)** under KGC Materializers, once the repository is public.

## Test suites

Snapshot of 2026-10-02, counted from each suite's `metadata.csv`:

| Suite | Cases | Notes |
|---|---|---|
| rml-core | 76 | JSON only since 2025-02 |
| rml-io | 73 | sources, logical targets, serialisations, compression |
| rml-io-registry | 102 | 77 SQL cases (MySQL 26, PostgreSQL 26, SQL Server 25) |
| rml-fnml | 20 | functions |
| rml-cc | 35 | collections and containers |
| rml-lv | 41 | logical views |
| rml-star | 18 | RDF-star |
| **Total** | **365** | 290 expect output, 75 expect an error |

There is no official standalone runner. RMLrs ships its own, and its output uses the implementation-report format (`testcase_id,status,notes`).

## Milestones

### M0 · Skeleton
- Cargo workspace (edition 2024): `rmlrs` (public API), `rmlrs-model`, `rmlrs-parser`, `rmlrs-term`, `rmlrs-source`, `rmlrs-engine`, `rmlrs-writer`, `rmlrs-cli`.
- The test suites as git submodules pinned to commits.
- Conformance runner: reads every case, runs it, writes results and a per-module summary, and fails on regressions against `conformance/baseline.csv`. An unimplemented feature must never count as a pass for an expected-error case.
- CI: fmt, clippy, tests, cargo-deny and conformance.
- **Done when** the runner reads all 365 cases and reports them as failed, and CI is green.

### M1 · Core and JSON
- Mapping model and Turtle parser, with shortcut expansion and validation.
- Terms: templates and escapes, IRI encoding and validation, base IRI, term types, deterministic blank nodes, datatypes and language tags.
- JSON sources: JSONPath (RFC 9535), multi-valued references, natural datatypes, null handling.
- Joins, graph maps, duplicate removal, N-Quads output, and `rmlrs run mapping.ttl -o out.nq`.
- **Done when** RML-Core passes 76 / 76.

### M1.5 · Python and WebAssembly
- Python wheel (pyo3) that drops into existing Python RML workflows, with output any Python RDF library can load.
- WebAssembly build for browser editors and mapping-generation loops. In-memory sources only (JSON, and CSV when available); no database or file-system access.
- Both stay thin wrappers over the library API; in-memory input goes through the same API as native callers.
- **Done when** the wheel and the WASM package pass the same Core cases as the native build.

### M2 · SQL
- PostgreSQL, then MySQL, then SQL Server: tables and queries, identifier rules, natural datatypes, NULL handling and error cases.
- The runner starts the databases in containers and loads each case's `resource.sql`.
- **Done when** the io-registry SQL cases pass 77 / 77. *Done:* 63 / 77; the other 14 are listed test-suite problems.

### M3 · Performance
- Streaming execution, hash joins and bounded memory.
- Basic CSV support, because the KROWN generator produces CSV and PostgreSQL inputs.
- A KROWN engine wrapper.
- **Done when** time and memory numbers against other engines are published.

### M4 · CSV, XML and IO
- CSV and CSVW dialects, XPath with namespaces, logical targets, serialisations, compression and encodings.
- **Done when** RML-IO and the remaining io-registry cases pass.
- *Done:* RML-IO 70 / 73, io-registry 79 / 102. The cases left are listed test-suite problems or need what the engine leaves to its caller or to a server: Kafka and MQTT brokers, a live public SPARQL endpoint, a SPARQL engine over RDF files (out of scope, see [PHILOSOPHY.md](PHILOSOPHY.md)).

### M5 · Extension modules
- FNML (functions, including an API for host-registered functions), CC, LV and RDF-star.
- **Done when** all five report modules meet the target, and the implementation-report entry is updated.

### M6 · Compatibility
- There is no Java binding: JVM systems can run RMLrs in a separate process. (Python and WebAssembly moved to M1.5.)
- The W3C R2RML test suite, through a translation layer for the legacy `rr:` vocabulary. *Done:* R2RML and old RML (`semweb.mmlab.be`, `ql:`) mappings are translated when read; R2RML 61 / 62, old RML test cases 262 / 323.
- Optional input of mapping plans produced by an external mapping-algebra planner.

## Outside the engine (TBD)

These stay out of RMLrs by design (see [PHILOSOPHY.md](PHILOSOPHY.md)). They will be planned after the milestones above:

- loading the output into a store, as a graph database or as entity and edge tables in SQL;
- query patterns for consumers of the output.

If code for these is ever needed, it goes in a separate repository or in `examples/`, not in the engine crates.
