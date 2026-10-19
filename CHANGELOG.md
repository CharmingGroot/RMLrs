# Changelog

All notable changes to RMLrs are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/) for releases
([RELEASING.md](RELEASING.md)).

## Unreleased

### Documentation

- Add Korean translations of the README, contribution guide, architecture,
  security policy and code of conduct.

## [0.1.0-alpha.1] - 2026-10-19

Initial GitHub source pre-release. No package has been published yet.

### Mapping language

- RML-Core: triples maps, subject, predicate, object and graph maps,
  constants, references and templates, term types, datatype and language
  maps, classes, joins (referencing object maps), base IRIs.
- RML-FNML: function executions and conditions, built-in GREL and IDLab
  functions used by the test suite, and an API for host functions.
- RML-CC: gather maps for `rdf:List`, `rdf:Bag`, `rdf:Seq` and `rdf:Alt`,
  with the append and Cartesian-product strategies, empty collections on
  request, and named collections that grow across iterations.
- RML-LV: logical views with expression and iterable fields, nested fields,
  index keys, left and inner joins, and a change of reference formulation
  inside a field.
- RML-star: quoted triples in subject and object maps, asserted and
  non-asserted triples maps, joined quoted triples.
- R2RML and old RML mappings are translated when read; R2RML logical tables
  read the database given to the processor (`--db-url`, `Options::database`).

### Sources and output

- JSON (JSONPath), CSV and XML (XPath 3.1) sources, read one iteration at a
  time; documents can be passed in memory.
- PostgreSQL, MySQL and SQL Server sources with natural RDF datatypes;
  PostgreSQL through a server cursor, MySQL rows as they arrive.
- CSVW table descriptions (`csvw:Table`) with their dialect: delimiter,
  quote character, escaping, comment prefix, header rows, skipped rows and
  columns, trimming, encoding and null values.
- UTF-16 sources (`rml:encoding`), decoded as they are read.
- HTTP(S) sources described with DCAT (`dcat:downloadURL`) or Web of Things
  (`td:Thing`), and remote CSVW `csvw:url`s, behind the `http` feature (on in
  the CLI, off in the bindings); documents passed in memory under the URL take
  precedence.
- SPARQL endpoint sources (`sd:Service`) with CSV, TSV or JSON results; old
  RML SPARQL sources are translated to them.
- GZip, Zip, Tar+GZip and Tar+Xz compressed sources and targets.
- N-Quads output (quoted triples as `<< s p o >>`); RML-IO logical targets in
  N-Quads, N-Triples, Turtle, N3, RDF/JSON, JSON-LD and RDF/XML, UTF-8 or
  UTF-16.
- A mapping that sends triples in a named graph to a target whose format
  cannot hold named graphs (N-Triples, Turtle, N3, RDF/JSON, RDF/XML) is
  rejected as invalid (RML-IO, section 7.4); such graphs used to be dropped.
- Duplicate removal with a 16-byte fingerprint per distinct quad.

### Tools

- `rmlrs` command-line interface.
- Python module and WebAssembly module with the same output as the CLI.
- Conformance runner for the KG-Construct test suites, the W3C R2RML test
  cases and the old RML test cases, with a regression baseline.
- KROWN benchmark integration (`bench/krown`).
