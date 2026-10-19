# 0002. Own mapping parser

Status: accepted · 2026-10-02

## Context

An existing open-source RML front end translates RML documents into mapping-algebra plans and covers all specification modules with regression tests. Its executors are not written in Rust, and it is not published on crates.io. Using it as our front end would leave only the executor to write.

## Options

| Option | For | Against |
|---|---|---|
| Depend on the existing front end | Parsing maintained by others | Not published on crates.io, so RMLrs could not be published either; plan format changes would ripple into us |
| Vendor it | Publishable | Ongoing sync burden |
| **Own parser** | Publishable; we own the boundaries | More work |

## Decision

Write our own parser and internal plan from the specifications. Anything copied from another project would be credited in [NOTICE](../../NOTICE). Accepting mapping-algebra plans produced by an external planner as an alternative input is on the roadmap (M6).

## Revisit when

A Rust RML front end is published on crates.io with a stable plan format.
