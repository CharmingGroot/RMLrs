# Philosophy

RMLrs does one thing:

```
mapping + source data  →  RDF quads
```

It reads the sources a mapping points at, applies the [RML](https://w3id.org/rml/portal/) mapping, and emits RDF. What happens to that RDF is the caller's business.

## Pure

The engine is a function from *(mapping, source data)* to a set of quads. Given the same inputs it produces the same output.

RMLrs does **not**:

| Out of scope | Where it belongs |
|---|---|
| Storing triples | a triple store or graph database |
| Querying | a SPARQL engine or the consumer's own query layer |
| Reasoning | a reasoner |
| LLM or agent integration | the application that consumes the RDF |
| Polling, scheduling, retrying, incremental sync of upstream systems | the caller that feeds data in |
| Deciding what counts as "the same thing" | the mapping author, through IRI templates |
| Editors and UIs | separate tools |

## Minimal

- **Specifications only.** RMLrs implements the RML specification modules published by the W3C KG-Construct Community Group, and nothing beyond them. No proprietary vocabulary. If something is missing from the specs, propose it there first.
- **Opt-in features.** Each source format and specification module sits behind a Cargo feature. The default is RML-Core with JSON.
- **Few dependencies.** Every dependency needs a reason.
- **Library first.** The library is the product. The CLI and language bindings are thin wrappers around it.

Feeding in-memory data (for example an HTTP response a caller already fetched) is a **library API** concern, not a mapping-language extension. Mapping documents stay standard.

## Measured

- **Correctness** is the official conformance test suites. A change that lowers the pass count does not merge.
- **Performance** is the KROWN benchmark plus our own benches. Performance claims come with numbers.

## Should this feature go in?

Ask in order:

1. Is it defined by an RML specification module? If not, it does not go in.
2. Does it change which quads come out for given inputs, or how fast and with how little memory they come out? If not, it belongs to the caller.
3. Can it be off by default without weakening the core? If not, redesign it.
