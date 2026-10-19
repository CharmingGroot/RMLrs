# 0003. JSON and SQL first

Status: accepted · 2026-10-02

## Context

RMLrs will support JSON, SQL, CSV and XML. The order matters because each format brings its own dependencies and test cases.

## Decision

JSON first, then SQL (PostgreSQL, MySQL, SQL Server), then CSV and XML.

## Why it fits the test suites

- Since February 2025 the RML-Core suite is JSON only (76 cases), so Core can be completed with JSON alone.
- SQL cases live in rml-io-registry: 77 of its 102 cases.
- JSON and SQL together are 161 cases: Core 76, io-registry SQL 77 and io-registry JSON 8.

## Consequences

Basic CSV support moves forward to M3, because the KROWN benchmark generator produces CSV and PostgreSQL inputs.
