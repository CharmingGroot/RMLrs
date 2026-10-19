# Using RMLrs

- [Command line](#command-line)
- [Where sources are read from](#where-sources-are-read-from)
- [Databases](#databases)
- [R2RML and old RML mappings](#r2rml-and-old-rml-mappings)
- [Output and logical targets](#output-and-logical-targets)
- [Rust library](#rust-library)
- [Python and WebAssembly](#python-and-webassembly)
- [Functions (RML-FNML)](#functions-rml-fnml)
- [Cargo features](#cargo-features)
- [Errors](#errors)

## Command line

```bash
cargo install --git https://github.com/CharmingGroot/RMLrs rmlrs-cli
```

```text
rmlrs run [OPTIONS] <MAPPING>

  <MAPPING>                    the mapping document (Turtle)
  -o, --output <FILE>          write N-Quads here instead of standard output
      --base-iri <IRI>         base IRI for relative IRIs [default: http://example.com/]
      --db-url <JDBC>          database for R2RML logical tables
      --db-user <USER>         user for --db-url
      --db-password <PASS>     password for --db-url (or RMLRS_DB_PASSWORD)
```

The exit status is 0 on success and 1 on any error; the error is printed on
standard error. Quads go out as they are generated, so a large output does
not have to fit in memory.

## Where sources are read from

| In the mapping | Read from |
|---|---|
| `rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "x.csv" ]` | the directory of the mapping document |
| `rml:source [ a rml:RelativePathSource ; rml:path "x.csv" ]` (or `rml:root rml:CurrentWorkingDirectory`) | the current working directory |
| `rml:source "x.csv"` (the older string form) | the current working directory |
| `rml:source [ a csvw:Table ; csvw:url "x.csv" ; ... ]` | the directory of the mapping document, see below |
| `rml:source [ a d2rq:Database ; ... ]` | a database, see below |
| `rml:source [ a dcat:Distribution ; dcat:downloadURL <https://...> ]` | an HTTP(S) GET of the URL (`http` feature) |
| `rml:source [ a td:Thing ; td:hasPropertyAffordance [ td:hasForm [ hctl:hasTarget "https://..." ] ] ]` | an HTTP(S) GET of the target (`http` feature) |
| `rml:source [ a sd:Service ; sd:endpoint <https://.../sparql> ]` | the results of the query in `rml:iterator` (`http` feature) |

A compressed file is decompressed when the source says so with
`rml:compression` (`rml:gzip`, `rml:zip`, `rml:targz`, `rml:tarxz`).

A file is read as UTF-8 unless the source gives another `rml:encoding`.
`rml:UTF-16` reads UTF-16: a byte order mark at the start picks the byte order,
and without one the file is big-endian (RFC 2781). Other encodings are
reported as not supported.

Remote sources come from the RML-IO registry's DCAT and Web of Things
descriptions. A Web of Things form may set `hctl:forContentType` (sent as
`Accept`), `htv:headers` and `htv:methodName "GET"`. Each document is
fetched once per logical source, with TLS from rustls and the operating
system's certificates; an error status is a source error. Not supported:
`dcat:accessURL` (a data service), other HTTP methods, and streams
(`kafka:`, `mqtt:`, `tcp:` targets, server-sent events), which a caller
reads and feeds in itself.

A SPARQL endpoint source takes the reference formulation
`formats:SPARQL_Results_CSV`, `formats:SPARQL_Results_TSV` or
`formats:SPARQL_Results_JSON`, and its `rml:iterator` is a SELECT query. The
query is sent with the SPARQL 1.1 Protocol (URL-encoded POST) asking for that
format. Each solution is an iteration and a reference names a variable
(`"name"` for `?name`); an unbound variable gives no value. Values are the
string form of the bound term (an IRI, a literal's lexical form), as in the
CSV results format, so the term map decides the RDF term. Not supported:
XML results, and a SPARQL query over an RDF file (that needs a SPARQL
engine, which RMLrs does not include).

### CSV on the Web (CSVW) table descriptions

A `csvw:Table` source names a CSV file with `csvw:url` and says how to read it
with a `csvw:dialect`. Read it with `rml:referenceFormulation rml:CSV`:

```turtle
<#Students> a rml:LogicalSource ;
  rml:source [ a rml:Source, csvw:Table ;
    csvw:url "students.csv" ;
    csvw:null "NULL" ;
    csvw:dialect [ a csvw:Dialect ;
      csvw:delimiter ";" ;
      csvw:encoding "utf-16" ;
      csvw:trim true ] ] ;
  rml:referenceFormulation rml:CSV .
```

- A relative `csvw:url` is resolved against the directory of the mapping
  document (CSVW resolves it against the metadata document). `file:` URLs
  are read too, and `http:`/`https:` URLs are fetched like other remote
  sources (`http` feature) unless the document is handed over in memory
  under that URL. Other URL schemes are reported as not supported.
- Dialect properties read: `csvw:delimiter`, `csvw:quoteChar` (one character
  each), `csvw:doubleQuote` (`false` means a backslash escapes inside quoted
  cells), `csvw:commentPrefix`, `csvw:header`, `csvw:headerRowCount`,
  `csvw:skipRows`, `csvw:skipColumns`, `csvw:trim` (`true`, `false`, `"start"`,
  `"end"`), `csvw:skipInitialSpace` and `csvw:encoding` (`utf-8`, `utf-16`,
  `utf-16le`, `utf-16be` and their WHATWG aliases; `utf-16` without a byte
  order mark is little-endian, as WHATWG defines it). `csvw:lineTerminators`
  may only list `"\n"` and `"\r\n"`, which are always accepted. Blank rows are
  always skipped, whatever `csvw:skipBlankRows` says. Any other dialect
  property, and `csvw:tableSchema`, is reported as not supported.
- Without a header row (`csvw:header false`), the columns are named `_col.1`,
  `_col.2`, ... as CSVW names them.
- `csvw:null` on the table gives the NULL values, like `rml:null`.
- A property that is not given keeps the plain CSV reading of `rml:CSV`, not
  the CSVW default: cells are not trimmed, no row is a comment, and no value
  is NULL. CSVW does not agree with itself on some defaults (its `trim`
  default is `true` in the property definition and `false` in its example
  default dialect), and RML-IO says CSV has no NULL value unless one is
  declared.
- Properties of the `rml:Source` win over CSVW ones: `rml:encoding` over
  `csvw:encoding`, `rml:null` over `csvw:null`. `rml:compression` applies
  too (to local files only).

Library callers can hand documents over in memory instead
(`Options::in_memory`, keyed by the `rml:path` or `csvw:url` the mapping
uses, or by the URL of a remote source). In-memory documents are already
text, so `rml:encoding` does not apply to them. `Mapping::remote_urls` lists
the URLs a mapping reads, so a caller can fetch them its own way. The Python
and WebAssembly modules work that way; they are built without the `http`
feature.

## Databases

A logical source over a database names it with a `d2rq:Database`:

```turtle
<#db> a d2rq:Database ;
  d2rq:jdbcDSN "jdbc:postgresql://localhost:5432/mydb" ;
  d2rq:username "me" ;
  d2rq:password "secret" .

<#People> rml:logicalSource [
    rml:source <#db> ;
    rml:referenceFormulation rml:SQL2008Table ;   # or rml:SQL2008Query
    rml:iterator "people"                         # the table, or the query
  ] .
```

| Database | `d2rq:jdbcDSN` |
|---|---|
| PostgreSQL | `jdbc:postgresql://host[:port]/database` |
| MySQL, MariaDB | `jdbc:mysql://host[:port]/database` |
| SQL Server | `jdbc:sqlserver://host[:port];databaseName=database` |

Only the `jdbc:` connection string is used; `d2rq:jdbcDriver` is ignored.
Values come with the natural RDF datatypes of their SQL types (integers,
decimals, doubles, booleans, dates, times, timestamps, binary as hex). Rows
are streamed from PostgreSQL (a server cursor) and MySQL; SQL Server results
are read in full first.

## R2RML and old RML mappings

Mappings in the [R2RML](https://www.w3.org/TR/r2rml/) vocabulary (`rr:`) and
in the RML vocabulary that came before the current specifications
(`http://semweb.mmlab.be/ns/rml#` with `ql:` reference formulations) are
translated to the current RML when read. Nothing needs to change in them.

R2RML mappings name tables and queries but not the database; give it to
RMLrs:

```bash
rmlrs run r2rml.ttl --db-url jdbc:postgresql://localhost:5432/mydb --db-user me
```

In the library this is `Options::database`.

Differences of the old vocabulary that RMLrs keeps for those documents:

- JSONPath references may be relative to the iteration (`"name"` for `$.name`).
- JSON values come out as plain strings, as the old RML had no natural
  datatypes for JSON (`30.50`, not `"30.5"^^xsd:double`).
- A logical source with both `rml:query` and `rr:tableName` uses the query.
- A SPARQL endpoint read the old way (`sd:Service` source, `rml:query`,
  `ql:JSONPath` with the iterator `$.results.bindings[*]`) becomes a SPARQL
  endpoint source with `formats:SPARQL_Results_JSON`; the references
  `"name.value"` and `"name"` read the variable `name`. Other old SPARQL
  sources and other paths into a binding (`"name.type"`) are not supported.

## Output and logical targets

The default output is N-Quads: on standard output, in `--output`, or handed
to the caller in the library. Quads that appear twice are written once.
Quoted triples (RML-star) are written as `<< s p o >>`.

Term maps can also send their triples to RML-IO logical targets, files the
mapping describes:

```turtle
@prefix formats: <http://www.w3.org/ns/formats/> .

<#out> a rml:LogicalTarget ;
  rml:target [ a rml:Target ; rml:root rml:MappingDirectory ; rml:path "people.ttl.gz" ] ;
  rml:serialization formats:Turtle ;
  rml:compression rml:gzip .

<#Person> rml:subjectMap [ rml:template "http://example.com/person/{$.id}" ;
                           rml:logicalTarget <#out> ] .
```

Supported serializations (`formats:N-Quads`, `formats:N-Triples`,
`formats:Turtle`, `formats:N3`, `formats:RDF_JSON`, `formats:JSON-LD`,
`formats:RDF_XML`): N-Quads, N-Triples, Turtle, N3, RDF/JSON, JSON-LD and
RDF/XML;
encodings UTF-8 and UTF-16; compression GZip, Zip, Tar+GZip and Tar+Xz. A
target file is created even when no triple goes there.

- **JSON-LD** is written in flattened, expanded form: an array of node
  objects with full IRIs and no `@context`. A named graph is the `@graph` of
  the node named after it. Literals are value objects (`@value` with
  `@language` or `@type`), so lexical forms are kept exactly.
- **RDF/XML** groups triples by subject in `rdf:Description` elements;
  subjects and objects are written as full IRIs (`rdf:about`,
  `rdf:resource`) or `rdf:nodeID`. A predicate needs a qualified name, so its
  IRI must end in an XML name (`http://example.com/p` does,
  `http://example.com/123` does not); otherwise the run fails with a data
  error, as it does for characters XML 1.0 cannot hold.
- **Named graphs** can only go to N-Quads and JSON-LD targets. N-Triples,
  Turtle, N3, RDF/JSON and RDF/XML hold triples only, and RML-IO (section
  7.4) calls a mapping invalid when it sends a named graph to one of them.
  RMLrs refuses such a mapping with a mapping error before it runs: a graph
  map other than `rml:defaultGraph`, in a triples map whose subject,
  predicate or object maps (or that graph map itself) send triples to such a
  target. Triples the check cannot foresee (RML-CC collections) are refused
  the same way when they reach the target. `rml:defaultGraph` is fine
  everywhere.
- JSON-LD, RDF/XML and RDF/JSON targets are written when the run ends, so
  their triples are held in memory until then. Quoted triples (RML-star)
  can only go to N-Quads, N-Triples, Turtle and N3 targets.

## Rust library

The crate is not on crates.io yet; depend on the repository:

```toml
[dependencies]
rmlrs = { git = "https://github.com/CharmingGroot/RMLrs", features = ["xml", "postgres"] }
```

```rust
// Everything at once.
let quads: Vec<rmlrs::Quad> = rmlrs::run_file("mapping.ttl", "http://example.com/")?;

// Quad by quad, with full control over where sources come from.
let mapping = rmlrs::parse_mapping(&std::fs::read_to_string("mapping.ttl")?, None)?;
let options = rmlrs::Options {
    base_iri: "http://example.com/".into(),
    mapping_dir: "mappings".into(),
    working_dir: std::env::current_dir()?,
    in_memory: [("events.json".to_owned(), body)].into(),   // instead of a file (or a
                                                            // remote source's URL)
    database: None,                                         // for R2RML logical tables
    ..Default::default()
};
rmlrs::run_with_targets(&mapping, &options, &mut |quad| {
    println!("{quad}");
    Ok(())
})?;
```

| Function | What it does |
|---|---|
| `run_file` / `run_file_to` | read a mapping file, run it, return the quads / hand them over one by one |
| `run_with_targets` | run a parsed mapping, write logical targets, hand over the default output |
| `run` / `run_to` | the same without writing logical targets |
| `run_in_memory` | mapping text and documents in memory (what the bindings use) |
| `parse_mapping` | parse a mapping document (current, R2RML or old RML vocabulary) |
| `to_nquads` | quads to N-Quads text |

## Python and WebAssembly

Both modules take the mapping as text and the sources as a map from
`rml:path` to document text, and return N-Quads. They give the same output
as the command line on all 76 RML-Core test cases.

- Python: [bindings/python/README.md](../bindings/python/README.md) (build with maturin)
- WebAssembly: [bindings/wasm/README.md](../bindings/wasm/README.md) (browsers and Node)

## Functions (RML-FNML)

Function executions (`rml:functionExecution`) and conditions
(`rml:condition`) run the built-in functions the RML-FNML test suite uses
(for example GREL `toUpperCase`, `string_replace` and `escape`, and IDLab
`equal`). With the
`fnml` feature, a host program can add its own functions through
`Options::functions` (`rmlrs_engine::functions::Function`: an IRI, parameter
and output IRIs, and a Rust function). A function the mapping uses but no
one provides is a mapping error before any data is read.

## Cargo features

The `rmlrs` crate keeps the core small; formats and modules are opt-in.
`rmlrs-cli` enables all of them.

| Feature | Adds |
|---|---|
| (none) | RML-Core, JSON and CSV sources, RML-CC, RML-LV, RML-star, logical targets |
| `xml` | XML sources with XPath 3.1 |
| `fnml` | RML-FNML function executions and conditions |
| `compression` | compressed sources and targets |
| `postgres`, `mysql`, `mssql` | database drivers |
| `http` | remote sources fetched over HTTP(S) (DCAT, Web of Things, `csvw:url`) and SPARQL endpoints; without it, give remote documents in `Options::in_memory` under their URL |

## Errors

| Error | Meaning |
|---|---|
| `invalid mapping` | the document is not a valid RML mapping (missing subject map, a cycle in logical views, an unknown function, ...) |
| `data error` | the data makes the mapping produce something invalid (a value that is not an IRI, an ill-typed literal, a column a query does not return) |
| `source error` | a source cannot be read (missing file, database unreachable) |
| `syntax error` | the mapping is not valid Turtle |
| `not supported` | the mapping uses something RMLrs does not implement yet |

Errors stop the run: RMLrs does not skip records silently.
