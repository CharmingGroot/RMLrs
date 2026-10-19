# 0007. XPath 3.1 through xee-xpath, behind the `xml` feature

Status: accepted · 2026-10-06

## Context

RML-IO-Registry says XPath iterators SHOULD follow XPath 3.1. The official test cases use namespaces, attributes, `text()` and the parent step (`../@id`), so an iteration must be able to look outside the node it is on.

## Options

| Option | For | Against |
|---|---|---|
| `sxd-xpath` (XPath 1.0) | Small (7 crates), widely used | No release since 2018 |
| `xrust` (XPath 3.1 data model, XSLT) | Maintained, 79 crates | Its own documentation says it is not extensively tested; mostly XPath 1.0 so far |
| Own XPath subset over `roxmltree` | Small, could stream simple paths | Rejects valid XPath 3.1 mappings; a second language to maintain next to JSONPath |
| **`xee-xpath` (XPath 3.1)** | The most complete XPath 3.1 in Rust; maintained | About 200 crates (ICU among them); CLI binary 6.0 MB → 11.7 MB |

## Decision

`xee-xpath`, as the optional `xml` feature (on in the CLI and the conformance runner, off by default in the library and the bindings). The document is parsed into a tree; each node selected by the iterator is one iteration; a reference returns the string value of each selected item, as `xsd:string`.

## Consequences

- XML memory grows with the document (about 50 times the file size on a 100,000-element file; see `bench/README.md`), unlike CSV, which streams.
- Update 2026-10-09: when the iterator is a plain child path and every reference stays inside the iterated element, the document is cut into elements and parsed a chunk at a time (345 MB → 44 MB on the same file); other mappings still build the whole tree.
- Every XPath case of rml-io passes; RMLIOREGTC0003b, which expects an error for a missing element, is listed as a test-suite problem because the registry text says NULL.

## Revisit when

- XML memory becomes the bottleneck on a real workload: then consider a streaming path for iterators that are plain child paths, with xee for the rest.
- `xee-xpath` stops being maintained.
