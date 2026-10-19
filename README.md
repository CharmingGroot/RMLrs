# RMLrs

[![CI](https://github.com/CharmingGroot/RMLrs/actions/workflows/ci.yml/badge.svg)](https://github.com/CharmingGroot/RMLrs/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV 1.88](https://img.shields.io/badge/rustc-1.88+-orange.svg)](Cargo.toml)

**An RML engine written in Rust.** RMLrs turns JSON, CSV, XML and relational
data into RDF, following the [RML](https://w3id.org/rml/portal/) (RDF Mapping
Language) specifications and their official test suites. It also runs
[R2RML](https://www.w3.org/TR/r2rml/) mappings and mappings in the older RML
vocabulary.

It is a command-line tool and a Rust library. Python and WebAssembly
modules wrap the same library for JSON and CSV documents passed in memory,
and give the same output as the command line on all 76 RML-Core cases.

## Pre-release status

**RMLrs is a pre-release project (version 0.0.0).** There is no tagged release
or published crate, wheel, WebAssembly package or binary yet. Build it from
source using the instructions below. Public APIs and supported features may
change before the first release. Conformance and performance figures in this
README come from this repository's own runner and benchmarks.

## Contents

- [Pre-release status](#pre-release-status)
- [What it does](#what-it-does)
- [Where it fits](#where-it-fits)
- [Conformance](#conformance)
- [Features](#features)
- [Install](#install)
- [Usage](#usage)
- [Performance](#performance)
- [Documentation](#documentation)
- [Contributing](#contributing)
- [License](#license)

## What it does

You describe, in an RML mapping, how records in your sources become RDF
triples. RMLrs reads the sources and writes the triples.

`people.json`:

```json
[
  { "id": 1, "name": "Alice", "knows": [2] },
  { "id": 2, "name": "Bob",   "knows": [] }
]
```

`mapping.ttl`:

```turtle
@prefix rml:  <http://w3id.org/rml/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

<#Person> a rml:TriplesMap ;
  rml:logicalSource [
    rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "people.json" ] ;
    rml:referenceFormulation rml:JSONPath ;
    rml:iterator "$[*]"
  ] ;
  rml:subjectMap [ rml:template "http://example.com/person/{$.id}" ; rml:class foaf:Person ] ;
  rml:predicateObjectMap [ rml:predicate foaf:name ; rml:objectMap [ rml:reference "$.name" ] ] ;
  rml:predicateObjectMap [ rml:predicate foaf:knows ;
                           rml:objectMap [ rml:template "http://example.com/person/{$.knows[*]}" ] ] .
```

```console
$ rmlrs run mapping.ttl
<http://example.com/person/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.com/person/1> <http://xmlns.com/foaf/0.1/name> "Alice" .
<http://example.com/person/1> <http://xmlns.com/foaf/0.1/knows> <http://example.com/person/2> .
<http://example.com/person/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.com/person/2> <http://xmlns.com/foaf/0.1/name> "Bob" .
```

The same mapping language joins sources: records from a JSON API, a SQL
table and a CSV file that describe the same thing end up as the same node.

## Where it fits

RMLrs is intended for batch mappings on one machine, or for applications that
need the same RML implementation through a Rust library, command line, Python
or WebAssembly. It runs mappings over files, databases and HTTP(S) sources.
It is not a continuous Kafka/MQTT stream processor or a SPARQL engine. The
[memory limits](docs/architecture.md#memory-and-streaming) matter when inputs
are large, and this pre-release currently needs a source build.

## Conformance

Results of the official test suites, run by [`rmlrs-conformance`](conformance/)
on every change (SQL cases nightly, against real PostgreSQL, MySQL and SQL
Server containers). Every case that does not pass is either not implemented
or listed with its reason in [`known-issues.csv`](conformance/known-issues.csv).
The counts below include database cases from the nightly run; a run without
`--databases` does not execute those cases.

| Suite | Passed | Notes |
|---|---|---|
| [RML-Core](https://w3id.org/rml/core/spec) | **76 / 76** | |
| [RML-IO](https://w3id.org/rml/io/spec) | 70 / 73 | not supported: a SPARQL query over an RDF file (RMLrs has no SPARQL engine); one database case the runner cannot set up yet; one listed |
| [RML-IO-Registry](https://github.com/kg-construct/rml-io-registry) | 79 / 102 | SQL: 63 / 77, the other 14 are listed test-suite problems; other sources: 16 / 25, 6 listed, Kafka and MQTT streams are unsupported, one case needs a live public SPARQL endpoint |
| [RML-FNML](https://w3id.org/rml/fnml/spec) | 19 / 20 | the other one is a listed test-suite problem |
| [RML-CC](https://w3id.org/rml/cc/spec) (collections and containers) | **35 / 35** | |
| [RML-LV](https://w3id.org/rml/lv/spec) (logical views) | **41 / 41** | |
| [RML-star](https://w3id.org/rml/star/spec) (RDF-star) | 10 / 18 | the other 8 pass once a stray line is removed from their input (listed) |
| [W3C R2RML](https://www.w3.org/2001/sw/rdb2rdf/test-cases/) (PostgreSQL) | 61 / 62 | the other one is listed |
| [Old RML test cases](https://github.com/RMLio/rml-test-cases) | 260 / 323 | 24 need a SPARQL server, which the runner does not start; 39 listed, mostly cases where this suite and the W3C R2RML suite expect opposite results |

How the runner decides pass and fail, and how to run it yourself:
[docs/conformance.md](docs/conformance.md).

## Features

**Sources**

- JSON with JSONPath, CSV, XML with XPath 3.1
- CSVW table descriptions with their dialect (delimiter, quoting, header
  and skipped rows, trimming, null values, encoding)
- PostgreSQL, MySQL and SQL Server (`d2rq:Database`), tables and queries,
  with the natural RDF datatypes of SQL values; PostgreSQL and MySQL rows
  are fetched incrementally, while SQL Server results are buffered
- files relative to the mapping or the working directory, or documents the
  caller passes in memory
- HTTP(S) downloads described with DCAT or Web of Things, and SPARQL
  endpoints with CSV, TSV or JSON results (the `http` feature, on in the CLI)
- UTF-8 and UTF-16 sources
- GZip, Zip, Tar+GZip and Tar+Xz compressed sources

**Mapping language**

- RML-Core, including joins, named graphs, language and datatype maps,
  blank nodes and base IRIs
- RML-FNML function executions and conditions, with the built-in functions
  the test suites use and an API for your own functions
- RML-CC collections and containers (`rdf:List`, `rdf:Bag`, `rdf:Seq`,
  `rdf:Alt`), including named collections that grow across records
- RML-LV logical views: fields, nested fields, index keys, left and inner
  joins, changing the reference formulation inside a field
- RML-star quoted triples, asserted and non-asserted triples maps
- R2RML mappings (the database is given with `--db-url`) and mappings in
  the old RML vocabulary (`semweb.mmlab.be/ns/rml#`, `ql:`)

**Output**

- N-Quads to standard output or a file; quoted triples as `<< s p o >>`
- RML-IO logical targets: N-Quads, N-Triples, Turtle, N3, RDF/JSON, JSON-LD
  and RDF/XML files, UTF-8 or UTF-16, optionally compressed
- duplicate quads removed

**Not supported (yet):** Kafka and MQTT stream sources, SPARQL queries over
RDF files (RMLrs has no SPARQL engine), CSVW table schemas, source encodings
other than UTF-8 and UTF-16, quoted triples in RDF/JSON, JSON-LD and RDF/XML
targets.
Mapping features outside the RML specifications are out of scope
([PHILOSOPHY.md](PHILOSOPHY.md)).

## Install

There is no release yet. With a Rust toolchain (1.88 or later):

```bash
cargo install --git https://github.com/CharmingGroot/RMLrs rmlrs-cli
```

This installs the `rmlrs` command with every source format and database
driver. Python and WebAssembly builds: [bindings/python](bindings/python/README.md),
[bindings/wasm](bindings/wasm/README.md).

## Usage

**Command line**

```bash
rmlrs run mapping.ttl -o out.nq --base-iri http://example.com/

# R2RML: the mapping names tables, the database is given here
RMLRS_DB_PASSWORD=secret rmlrs run r2rml.ttl \
  --db-url jdbc:postgresql://localhost:5432/mydb --db-user me
```

**Rust**

```rust
let quads = rmlrs::run_file("mapping.ttl", "http://example.com/")?;
print!("{}", rmlrs::to_nquads(&quads));
```

`rmlrs::run_to` and `rmlrs::run_with_targets` hand over each quad as soon as
it is generated, so the output does not have to fit in memory.

**Python**

```python
import rmlrs
nquads = rmlrs.materialize(mapping_ttl, base_iri="http://example.com/",
                           sources={"people.json": json_text})
```

**JavaScript (WebAssembly)**

```js
import init, { materialize } from "./pkg/rmlrs_wasm.js";
await init();
const nquads = materialize(mappingTtl, { "people.json": jsonText }, "http://example.com/");
```

More, including in-memory sources, databases, logical targets, your own
functions and Cargo features: [docs/usage.md](docs/usage.md).

## Performance

RMLrs reads CSV and PostgreSQL/MySQL rows incrementally and hands each quad
to the output as soon as it exists. JSON documents and SQL Server query
results are held in memory; XML input is read in full before eligible local
iterations are parsed in chunks. Joins, logical views and some target formats
also keep state. Duplicate removal keeps a 16-byte fingerprint per distinct
quad. See [architecture](docs/architecture.md#memory-and-streaming) for the
limits by source and output.

Measured with [KROWN](https://github.com/kg-construct/KROWN)'s own execution
framework against Morph-KGC 2.10.0 (4 processes), 3 runs each, same output
in every run (Apple Silicon Mac, Docker VM with 10 CPUs and 8 GB):

| Scenario | RMLrs | Morph-KGC |
|---|---|---|
| PostgreSQL, 100K rows × 20 columns | 2.27–2.57 s | 5.07–6.24 s |
| PostgreSQL, 50% join | 1.13–1.56 s | 2.47–2.85 s |
| MySQL, 100K rows × 20 columns | 2.19–2.51 s | 5.56–6.22 s |
| MySQL, 50% join | 0.82–1.14 s | 2.58–3.68 s |

This is not the environment of the KROWN paper. Method, all scenarios
measured so far and how to reproduce them: [docs/benchmarks.md](docs/benchmarks.md).

## Documentation

- [docs/usage.md](docs/usage.md): command line, library, bindings, databases, targets, functions
- [docs/conformance.md](docs/conformance.md): test suites, the runner, known issues
- [docs/benchmarks.md](docs/benchmarks.md): how performance is measured, and results
- [docs/architecture.md](docs/architecture.md): crates and how a mapping runs
- [docs/provenance.md](docs/provenance.md): which specification each crate implements, and what it relies on
- [PHILOSOPHY.md](PHILOSOPHY.md): scope, and the rule for new features
- [ROADMAP.md](ROADMAP.md): milestones and what is next
- [docs/adr](docs/adr): architecture decision records
- [CHANGELOG.md](CHANGELOG.md)

## Contributing

Bug reports, test cases and pull requests are welcome. Read
[CONTRIBUTING.md](CONTRIBUTING.md) first: changes need tests, keep the
conformance baseline green, and commits are signed off (DCO). Please follow
the [Code of Conduct](CODE_OF_CONDUCT.md). Security issues: [SECURITY.md](SECURITY.md).

## License

[Apache License 2.0](LICENSE). Third-party code and the test suites used in
testing are listed in [NOTICE](NOTICE).

## Acknowledgements

The test suites of the [W3C Knowledge Graph Construction Community Group](https://www.w3.org/community/kg-construct/),
the [KROWN](https://github.com/kg-construct/KROWN) benchmark, the
[RML.io](https://rml.io/) test cases and the W3C RDB2RDF Working Group's
R2RML test cases make the figures above possible.
