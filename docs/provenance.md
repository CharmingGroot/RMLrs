# Provenance

Where each part of RMLrs comes from.

**RMLrs is not a port of another engine.** Its crates are written in Rust
from the specifications listed below and tested against their official test
suites; they contain no code taken from other RML implementations.

The one piece of code adapted from another project is the benchmark engine
resource in `bench/krown/rmlrs.py` (from the KROWN benchmark, MIT), listed in
[NOTICE](../NOTICE).

Specifications and test suites are not copied into the repository's tree;
the test suites are referenced as git submodules. The RMLrs license does not
cover them. Two pinned submodules lack a repository-wide LICENSE file, as
explained in [NOTICE](../NOTICE); check their upstream terms before
redistributing initialized submodules. See [conformance.md](conformance.md)
for how the suites are used.

## Crates

| Crate | Implements | Relies on (crates) |
|---|---|---|
| `rmlrs-model` | RDF terms and quads ([RDF 1.1 Concepts](https://www.w3.org/TR/rdf11-concepts/)), quoted triples ([RDF-star](https://w3c.github.io/rdf-star/cg-spec/editors_draft.html)), the mapping model of [RML-Core](https://w3id.org/rml/core/spec), [RML-IO](https://w3id.org/rml/io/spec), [RML-FNML](https://w3id.org/rml/fnml/spec), [RML-CC](https://w3id.org/rml/cc/spec), [RML-LV](https://w3id.org/rml/lv/spec) and [RML-star](https://w3id.org/rml/star/spec) | `thiserror` |
| `rmlrs-parser` | [Turtle 1.1](https://www.w3.org/TR/turtle/) (the subset mapping documents use), [N-Quads](https://www.w3.org/TR/n-quads/) with quoted triples, reading the RML vocabularies into the model; translation of [R2RML](https://www.w3.org/TR/r2rml/) and the [old RML vocabulary](https://rml.io/specs/rml/) | none |
| `rmlrs-term` | term generation of RML-Core: templates, IRI rules and percent-encoding ([RFC 3987](https://www.rfc-editor.org/rfc/rfc3987)), typed literals with [XSD 1.1](https://www.w3.org/TR/xmlschema11-2/) canonical forms, language tags ([BCP 47](https://www.rfc-editor.org/info/bcp47)) | `percent-encoding` |
| `rmlrs-source` | reference formulations of [RML-IO-Registry](https://github.com/kg-construct/rml-io-registry): JSONPath ([RFC 9535](https://www.rfc-editor.org/rfc/rfc9535), own implementation of the subset RML uses), CSV ([RFC 4180](https://www.rfc-editor.org/rfc/rfc4180), own reader), XPath 3.1, SQL with the natural RDF datatypes of [R2RML §10](https://www.w3.org/TR/r2rml/#natural-mapping); RML-IO compression; logical-view rows | `serde_json`; XML: `xee-xpath`, `xot`, `xmlparser`; databases: `postgres`, `mysql`, `tiberius` (+ `tokio`); compression: `flate2`, `zip`, `tar`, `lzma-rust2` |
| `rmlrs-engine` | the RML-Core generation algorithm and joins, RML-FNML function executions and conditions (built-in [GREL](https://openrefine.org/docs/manual/grelfunctions) and IDLab functions the test suite uses, [ADR 0008](adr/0008-fnml-built-in-functions.md)), RML-CC gather maps, RML-LV logical views, RML-star quoted triples, duplicate removal | `serde_json`, `percent-encoding` |
| `rmlrs-writer` | N-Quads and N-Triples output, RML-IO logical targets (N-Quads, N-Triples, Turtle, N3, [RDF/JSON](https://www.w3.org/TR/rdf-json/)), compression | `serde_json`; compression: `flate2`, `zip`, `tar`, `lzma-rust2` |
| `rmlrs` | the public API over the crates above | none beyond them |
| `rmlrs-cli` | the `rmlrs` command | `clap` |
| `bindings/python` | the Python module | `pyo3` |
| `bindings/wasm` | the WebAssembly module | `wasm-bindgen`, `js-sys` |
| `conformance` | the conformance runner and an RDF dataset isomorphism check | `clap`, the database drivers |

Every dependency is checked by `cargo deny` against the license allow list
in [`deny.toml`](../deny.toml) (Apache-2.0, MIT, BSD, ISC, Unicode-3.0,
Zlib), all compatible with Apache-2.0. Binaries built from RMLrs include
these crates; their licenses apply to that code.
