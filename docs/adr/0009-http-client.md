# 0009. HTTP client: ureq on rustls, behind the `http` feature

Status: accepted · 2026-10-16

## Context

RML-IO and its registry describe sources that live on the web: the `dcat:downloadURL` of a `dcat:Distribution`, a Web of Things `td:Thing` form with an `http`/`https` target, a `csvw:url`, and a SPARQL endpoint (`sd:Service`) queried with the SPARQL 1.1 Protocol. Reading them is part of the specifications, so it is in scope (ADR 0001). What stays with the caller is everything around a single request: polling, retrying, scheduling, and streams (Kafka, MQTT, server-sent events).

The client has to build without a C TLS library, pass `cargo deny` with the current license list, and stay out of the WebAssembly binding, where the browser does the fetching.

## Options

| Option | For | Against |
|---|---|---|
| No client; documents only through `Options::in_memory` | No new dependency | The CLI cannot run a mapping with a remote source on its own |
| `reqwest` (blocking) | Widely used | Pulls in an async runtime and many crates for a few blocking GETs |
| `ureq` with its default features | Small, blocking, maintained | Bundles Mozilla's roots through `webpki-roots`, licensed CDLA-Permissive-2.0, which is not in the `deny.toml` allow list |
| `ureq` with `native-tls` | OS TLS everywhere | OpenSSL on Linux: a C library and its headers at build time |
| **`ureq` on rustls (ring provider) with the OS certificates** | Pure Rust TLS; no bundled certificate list; rustls and `rustls-native-certs` are already in the tree (SQL Server driver) | The trust store is the machine's, so a machine without CA certificates cannot reach HTTPS servers |

## Decision

`ureq` 3 with `default-features = false` and `rustls-no-provider`; `rustls` with the `ring` provider passed explicitly; root certificates loaded once per process from the operating system with `rustls-native-certs`. All of it sits behind the optional `http` feature: on in the CLI, off by default in the library and in the Python and WebAssembly bindings.

New crates in the lock file: `ureq`, `ureq-proto`, `http`, `httparse`, `utf8-zero`, all MIT OR Apache-2.0. `cargo deny check` passes without a change to the license list.

One request per logical source: a GET for a document (`hctl:forContentType` as `Accept`, `htv:headers`, only `htv:methodName "GET"`), a URL-encoded POST for a SPARQL query. The response is read as a stream. A status other than 2xx is a source error; there are no retries.

Without the feature, or when the caller prefers to fetch, `Options::in_memory` takes a remote document under its URL, and `Mapping::remote_urls` lists the URLs a mapping reads.

## Consequences

- The conformance runner never uses the network. A remote source under `http://w3id.org/rml/resources/` is read from the test case's declared input file of the same name; any other remote source or SPARQL endpoint is reported as not supported, whatever features the build has. The cases that need a live service (a public SPARQL endpoint, Kafka, MQTT) stay failed.
- The WebAssembly binding does not grow; browser callers fetch remote documents themselves and pass them in memory.
- A SPARQL query over a local RDF file still needs a SPARQL engine inside RMLrs, which this decision does not add.

## Revisit when

- `ureq` stops being maintained, or rustls changes its provider API in a way `ureq` does not follow.
- Users need proxies, authentication schemes or timeouts that the current configuration does not expose.
- The project decides to accept CDLA-Permissive-2.0, which would allow a bundled root list for machines without a CA store.
