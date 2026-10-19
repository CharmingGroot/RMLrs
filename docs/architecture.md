# Architecture

## Crates

```text
rmlrs-cli ──┐
bindings ───┼─> rmlrs (facade) ─> rmlrs-parser ─> rmlrs-model
            │                  ─> rmlrs-engine ─> rmlrs-source
            │                                  ─> rmlrs-term
            │                  ─> rmlrs-writer
conformance ┘
```

| Crate | Responsibility |
|---|---|
| `rmlrs-model` | RDF terms and quads (quoted triples included), the mapping model, errors |
| `rmlrs-parser` | Turtle and N-Quads parsers; reading a mapping graph into the model; translation of the R2RML and old RML vocabularies (`legacy.rs`) |
| `rmlrs-term` | term generation: templates, IRI rules and encoding, typed literals, language tags |
| `rmlrs-source` | iterating sources and evaluating references: JSONPath, CSV and CSVW dialects, XPath 3.1, SQL (PostgreSQL, MySQL, SQL Server), SPARQL results, logical-view rows; UTF-16 decoding, decompression, HTTP(S) downloads (`http` feature) |
| `rmlrs-engine` | running a mapping: triples maps, joins, functions, gather maps, logical views, quoted triples, duplicate removal |
| `rmlrs-writer` | N-Quads output and RML-IO logical targets (N-Quads, N-Triples, Turtle, N3, RDF/JSON, JSON-LD, RDF/XML) |
| `rmlrs` | the public API that ties them together |
| `rmlrs-cli` | the `rmlrs` command |
| `bindings/python`, `bindings/wasm` | thin wrappers over `rmlrs::run_in_memory` |
| `conformance` | the conformance runner (`rmlrs-conformance`) |

## How a mapping runs

1. **Parse.** The mapping document is read as Turtle. Legacy vocabularies
   are translated to the current RML first. The mapping graph becomes the
   model; structural rules (one subject map, valid term types, no cycles in
   views or quoted triples maps, ...) are checked here.
2. **Compile.** Before any data is read, the engine checks every reference
   formulation, compiles every reference expression (JSONPath, XPath, column
   names, view keys) and binds every function call to its implementation. A
   mapping that cannot run fails here.
3. **Group.** Triples maps with the same logical source are grouped, so each
   source is read once for all the triples maps that use it.
4. **Iterate.** Each triples map of a group generates triples for each
   iteration. CSV rows, PostgreSQL cursor batches and MySQL rows are fetched
   incrementally. JSON and SQL Server results are buffered first; XML input
   is read in full before its nodes are iterated.
5. **Join.** A referencing object map over another source looks up an index
   of the parent's iterations, keyed by the parent side of the join
   conditions. Indexes are built on first use, or before a database child
   source is opened so that no connection waits.
6. **Output.** Each quad is handed to the caller as soon as it exists. A
   128-bit fingerprint per distinct quad (16 bytes) removes duplicates;
   quads themselves are not kept. The exception is logical targets whose
   format is a single document (RDF/JSON, JSON-LD, RDF/XML): their quads are
   held until the run ends and written then.

Some features need state beyond one iteration:

- **Logical views** (RML-LV) are computed in full before their first
  iteration is mapped; a join needs all of the parent view anyway.
- **Named collections** (RML-CC) that grow across iterations keep their last
  list cell (or the next container number) open until the run ends.
- **Quoted triples** (RML-star) run the quoted triples map for the same
  iteration with its output captured instead of emitted.

## Memory and streaming

| Input | How it is read |
|---|---|
| CSV and CSVW | Rows are read as needed. |
| JSON | The whole document is parsed and selected iterations are collected. |
| XML | The whole input text is read. Eligible local XPath iterations are then parsed in chunks; other expressions use a whole-document tree. |
| PostgreSQL | A server cursor fetches rows in batches. |
| MySQL | Rows are consumed as the server sends them. |
| SQL Server | The query result is collected before iteration. |
| SPARQL results | CSV and TSV are read by row; JSON is parsed as a whole document. |

Remote and in-memory JSON/XML sources use the whole-document path. For output,
the default sink and line-based targets receive quads as they are generated.
RDF/JSON, JSON-LD and RDF/XML targets hold data until the run finishes. Joins
may build an index of parent iterations, logical views are materialized, and
duplicate removal retains a 16-byte fingerprint for each distinct quad.

Mappings can name local paths, HTTP(S) sources and database connections, and
logical targets can write files. Run mappings from untrusted sources only with
the filesystem and network access you intend to grant them; see
[SECURITY.md](../SECURITY.md#running-mappings).

## Design decisions

Recorded as ADRs in [docs/adr](adr): the pure and minimal scope, an own
mapping parser, the order of formats, the conformance runner, bindings, the
XPath engine, the built-in functions and the HTTP client.
