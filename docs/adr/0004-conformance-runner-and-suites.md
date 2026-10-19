# 0004. Conformance runner and test suites

Status: accepted · 2026-10-02

## Context

The KG-Construct suites have no official standalone runner. The reference processor's JUnit tests are the de-facto harness, and SQL cases ship only a `resource.sql` fixture with no container setup.

## Decision

- The suites are git submodules pinned to commits. They are licensed CC BY 4.0 and stay unmodified in their own repositories.
- RMLrs has its own runner. It writes results in the implementation-report format (`testcase_id,status,notes`), so they can be submitted as they are.
- `conformance/baseline.csv` is the gate: CI fails if a case that passed now fails.
- SQL cases run against containerised PostgreSQL, MySQL and SQL Server.
- While the repository is private, SQL cases run nightly rather than on every pull request, to stay within the CI minutes budget.

## Watch out

An expected-error case must only pass on the *right* error. "Not implemented" is never a pass.
