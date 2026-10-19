# Conformance

RMLrs is measured against the official test suites of the RML specifications
and two suites for the legacy vocabularies. The suites are git submodules
under [`conformance/suites`](../conformance/suites), pinned to a commit; they
are not copied into this repository.

| Suite | Source | Cases | Passed |
|---|---|---|---|
| RML-Core | [kg-construct/rml-core](https://github.com/kg-construct/rml-core) | 76 | 76 |
| RML-IO | [kg-construct/rml-io](https://github.com/kg-construct/rml-io) | 73 | 70 |
| RML-IO-Registry | [kg-construct/rml-io-registry](https://github.com/kg-construct/rml-io-registry) | 102 | 79 |
| RML-FNML | [kg-construct/rml-fnml](https://github.com/kg-construct/rml-fnml) | 20 | 19 |
| RML-CC | [kg-construct/rml-cc](https://github.com/kg-construct/rml-cc) | 35 | 35 |
| RML-LV | [kg-construct/rml-lv](https://github.com/kg-construct/rml-lv) | 41 | 41 |
| RML-star | [kg-construct/rml-star](https://github.com/kg-construct/rml-star) | 18 | 10 |
| W3C R2RML | [kg-construct/r2rml-test-cases-support](https://github.com/kg-construct/r2rml-test-cases-support) | 62 | 61 |
| Old RML test cases | [RMLio/rml-test-cases](https://github.com/RMLio/rml-test-cases) | 323 | 260 |

The current result of every case is in
[`conformance/baseline.csv`](../conformance/baseline.csv).
The table includes SQL cases from runs with `--databases`. A run without that
flag reports them in its totals but does not execute them; use `--databases`
to reproduce the full pass counts above.

## Running it

```bash
git submodule update --init
cargo run --release -p rmlrs-conformance -- --check conformance/baseline.csv
```

With `--databases` the runner also runs the SQL cases. It starts PostgreSQL,
MySQL and SQL Server in Docker containers on free local ports, loads each
case's script, and removes the containers at the end:

```bash
cargo run --release -p rmlrs-conformance -- --databases --check conformance/baseline.csv
```

Other options: `--only rml-core,rml-io` runs some suites, `--verbose` prints
every case, `--write-baseline <file>` writes the results as a new baseline.
Results are written to `conformance/results/<suite>.csv` in the format of the
[RML implementation report](https://w3id.org/kg-construct/rml-implementation-report)
(`testcase_id,status,notes`). Setting `RMLRS_DUMP=<dir>` keeps the output of
failing cases there.

CI runs the suites without databases on every pull request and with
databases every night.

## How a case passes

- A case with an expected output passes when RMLrs's output is the same RDF
  dataset: equal up to the renaming of blank nodes, including blank nodes
  inside quoted triples.
- A case that expects an error passes only when RMLrs reports a mapping or
  data error. An error for something RMLrs does not implement ("not
  supported") is a failure, so a missing feature can never pass as the
  "right" error.
- `--check` fails the run when a case that passes in the baseline no longer
  passes. A change that makes new cases pass updates the baseline in the same
  pull request.

## Known issues

Some cases fail because the test case itself is in question: two cases that
expect opposite things, an input file with a stray line, a database script
written for another database. Each of them is listed in
[`conformance/known-issues.csv`](../conformance/known-issues.csv) with the
reason, and the runner adds that reason to the case's notes. RMLrs does not
special-case a test to make it pass.

Examples:

- The old RML test cases expect values that do not form an IRI to be
  skipped; the W3C R2RML test cases expect a data error for the same data.
  RMLrs follows the W3C suite.
- Eight RML-IO-Registry cases expect `xsd:double` values in Java's
  `Double.toString` form (`30.0`); RMLrs writes the XSD canonical form
  (`3.0E1`).
- Eight RML-star cases have a line copied from their README at the top of a
  CSV input. With that line removed, they pass.

## Remaining limits

RMLrs supports CSVW table descriptions, Web of Things HTTP sources, SPARQL
endpoint results in CSV/TSV/JSON, UTF-16 inputs, and RDF/XML and JSON-LD
targets. These are no longer general missing features. Individual failures
remain in the pinned suites. Fixture and expectation disagreements are
recorded in [`known-issues.csv`](../conformance/known-issues.csv).

- A SPARQL query over an RDF file needs a SPARQL engine, which RMLrs does not
  include. Kafka and MQTT stream sources are also not implemented.
- The old RML suite includes 24 cases that need a SPARQL server; the runner
  does not start one. An RML-IO-Registry case needs a live public SPARQL
  endpoint. These results do not establish that SPARQL endpoint sources are
  unsupported: the HTTP client and CSV/TSV/JSON result readers are implemented.
- Some remaining failures are fixture or expectation disagreements documented
  in `known-issues.csv`. A passing baseline check means no previously passing
  case regressed; it does not mean every case passed.
