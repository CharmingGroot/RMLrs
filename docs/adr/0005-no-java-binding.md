# 0005. No Java binding; conformance at the level of the most complete engines

Status: accepted · 2026-10-02

## Context

JVM systems are a natural audience for an RML engine. A JNI binding would serve them, at the cost of a second build and release pipeline.

## Decision

- No Java binding. JVM systems can run RMLrs in a separate process. The planned bindings are Python and WebAssembly.
- The conformance target is at least the level of the most complete library engine on the official implementation report of 2026-05: Core 76, IO 61, FNML 18, CC 35 and LV 41. That engine has no SQL source support; RMLrs adds the 77 io-registry SQL cases.

## Consequences

Common engine designs (pluggable source resolvers, join storage, streaming) are a reference. Vocabulary extensions that some engines add for in-memory input are not adopted; in-memory input is a library API instead (see [0001](0001-pure-and-minimal-scope.md)).

## Revisit when

A user needs RMLrs inside a JVM process and cannot run a separate one.
