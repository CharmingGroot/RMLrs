# 0001. Pure and minimal scope

Status: accepted · 2026-10-02

## Context

An RML engine sits next to many neighbouring concerns: storage, querying, reasoning, scheduling of upstream fetches, editors, and lately LLM integration. Each of them is tempting to bundle and each of them grows the engine.

## Decision

RMLrs only turns *(mapping, source data)* into RDF quads. It implements the RML specification modules and nothing beyond them. The full rule and the out-of-scope list are in [PHILOSOPHY.md](../../PHILOSOPHY.md).

## Consequences

- State across runs (incremental sync, idempotency, quarantine of bad records) belongs to the caller.
- In-memory input is a library API, not a vocabulary extension, so mapping documents stay portable to other engines.
- Some convenient features other engines ship as extensions will not be here.

## Revisit when

A specification requirement cannot be met within these limits. The specification wins.
