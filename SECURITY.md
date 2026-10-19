# Security Policy

[English](SECURITY.md) | [한국어](SECURITY.ko.md)

## Supported versions

The first GitHub pre-release is `v0.1.0-alpha.1`. Security fixes go to the
`main` branch; pre-release APIs and behavior may change.

## Reporting a vulnerability

Please report vulnerabilities privately through GitHub's
[private vulnerability reporting](https://github.com/CharmingGroot/RMLrs/security/advisories/new)
rather than in a public issue.

Include the affected version or commit, a mapping and input that reproduce the
problem, and what you expected to happen.

## Running mappings

Treat a mapping as a program with access to the process's permissions. A
mapping may read local files, fetch HTTP(S) URLs, connect to databases, and
write logical target files. The CLI does not sandbox these operations or
restrict paths and network destinations. The usage guide describes
[source paths](docs/usage.md#where-sources-are-read-from) and
[logical targets](docs/usage.md#output-and-logical-targets).

Review mappings before running them, especially when they come from someone
else. For untrusted mappings, run RMLrs in an environment with only the files,
database credentials and network access needed for that job, and choose an
output directory where overwriting a file is acceptable. Avoid storing
database passwords in mappings that will be committed or shared; the CLI can
read `RMLRS_DB_PASSWORD` instead. Compressed inputs and large JSON/XML
documents may consume substantial memory or disk resources.

`run_in_memory` uses supplied documents when their keys match the mapping,
but missing documents can fall back to local files in `mapping_dir`. It is
not a sandbox for untrusted mappings.

Areas that handle untrusted input and deserve extra care:

- database connections and the credentials in a mapping's `d2rq:Database`
- compressed and archive sources (gzip, xz, zip, tar) and the paths inside them
- XPath and JSONPath evaluation on input documents
- file paths in logical sources and outputs
