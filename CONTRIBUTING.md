# Contributing to RMLrs

[English](CONTRIBUTING.md) | [한국어](CONTRIBUTING.ko.md)

Thanks for your interest. Bug reports, failing mappings, test cases,
documentation and code are all welcome. This document explains how changes
get into RMLrs.

Please follow the [Code of Conduct](CODE_OF_CONDUCT.md). Security issues go
through [SECURITY.md](SECURITY.md), not public issues.

## Scope

RMLrs implements the RML specifications and nothing beyond them
([PHILOSOPHY.md](PHILOSOPHY.md)). A feature that no RML specification module
defines is out of scope, however useful it is; propose it to the
[KG-Construct Community Group](https://www.w3.org/community/kg-construct/)
first. Within the specifications, everything is in scope: missing features,
wrong output, errors that should or should not happen, speed and memory.

For anything bigger than a small fix, open an issue first and agree on the
approach.

## Reporting a bug

Include what is needed to reproduce it:

- the mapping (as small as you can make it) and the input data
- the command or code you ran, and the RMLrs commit
- the output you expected, the output you got, and the error if any

A mapping that another RML engine runs differently is a good report too;
say which engine and version.

## Development setup

- Rust 1.88 or later (`rustup`), with `rustfmt` and `clippy`
- Docker, for the SQL conformance cases and database tests
- the test suites, as git submodules:

```bash
git clone --recurse-submodules https://github.com/CharmingGroot/RMLrs
# or, in an existing clone:
git submodule update --init
```

## Building and testing

Run these checks before submitting code; CI runs the formatting, lint, test,
license, and conformance checks again:

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo deny check                      # licenses and advisories (cargo install cargo-deny)
```

The conformance runner executes the official test suites
([docs/conformance.md](docs/conformance.md)):

```bash
cargo run --release -p rmlrs-conformance -- --check conformance/baseline.csv
cargo run --release -p rmlrs-conformance -- --databases --check conformance/baseline.csv   # SQL cases too
```

Tests that need a real database are `#[ignore]`d and read the server from an
environment variable:

```bash
docker run -d --rm -p 127.0.0.1:55432:5432 -e POSTGRES_HOST_AUTH_METHOD=trust postgres:16-alpine
RMLRS_TEST_POSTGRES=jdbc:postgresql://127.0.0.1:55432/postgres \
  cargo test -p rmlrs-source --features postgres -- --ignored
```

(`RMLRS_TEST_MYSQL` works the same way for MySQL, user and password `root`.)

## Pull requests

- **One concern per pull request**, with a title of the form
  `area: summary`, for example `source/json: support bracket-quoted names` or
  `engine: hash join for referencing object maps`. The area is a crate or
  module (`engine`, `parser`, `source/sql`, `cc`, `lv`, `conformance`, `docs`,
  ...).
- **Tests come with the change.** Name the conformance test cases it makes
  pass, or add unit tests when no official case covers it. A test should fail
  without the change; check that by reverting the fix locally. For a bug fix,
  include a regression test. For a new feature, cover its normal behavior and
  relevant errors. If an automated test is impractical, explain why in the
  pull request. Documentation-only changes do not need a new test.
- **No regressions.** Pull request CI checks the non-database conformance
  cases against `conformance/baseline.csv`; SQL cases run nightly with Docker.
  A pull request that turns a passing checked case into a failing one fails
  CI. When your change makes new cases pass, regenerate the baseline in
  the same pull request and explain the changed case IDs:

  ```bash
  cargo run --release -p rmlrs-conformance -- --databases --write-baseline conformance/baseline.csv
  ```

- **Checks pass:** formatting, clippy with warnings denied, tests and
  `cargo deny`. The PR's `check`, `msrv`, `dco`, `deny`, and `bindings` CI jobs
  must pass before merging. Database conformance runs in the nightly workflow
  and is not a pull request gate.
- **Documentation follows behaviour.** If the change is visible to users,
  update [docs/usage.md](docs/usage.md), the README tables or
  [CHANGELOG.md](CHANGELOG.md) (under "Unreleased"). Update an available
  Korean translation of the affected document too.

### When a test case looks wrong

Test suites have mistakes too. If you believe a case expects the wrong thing,
do not make RMLrs special-case it. Add it to
[`conformance/known-issues.csv`](conformance/known-issues.csv) with a reason
that someone can check (a specification section, a contradicting case, a
broken input file), and consider reporting it upstream.

## Commits

Every commit is signed off under the
[Developer Certificate of Origin](https://developercertificate.org/), which
certifies that you wrote it or otherwise have the right to submit it:

```bash
git commit -s
```

Write the commit message like the pull request title (`area: summary`),
with a body that explains why when it is not obvious.

## Code

- Every source file starts with `// SPDX-License-Identifier: Apache-2.0`.
- Return errors of the right kind: `Mapping` for an invalid mapping, `Data`
  for data that cannot produce valid RDF, `Source` for unreadable input,
  `Unsupported` for features not implemented. The conformance runner relies
  on the difference.
- Prefer the standard library and existing dependencies. A new dependency
  needs a reason in the pull request and must pass `cargo deny`.
- Code taken from another project needs a compatible license and an entry in
  [NOTICE](NOTICE).
- Larger design choices are recorded as an ADR in [docs/adr](docs/adr).

## Repository guidance

This file, the public documentation, and CI define the contribution rules for
everyone, regardless of editor or coding assistant. Contributors are
responsible for understanding and testing any code they submit, including
AI-assisted changes.

Do not add tool-specific instruction files (such as `AGENTS.md`, `CLAUDE.md`,
`GEMINI.md`, or `.cursorrules`) to the repository. CI rejects common agent
configuration paths even if they are force-added despite `.gitignore`. Keep
personal assistant configuration outside the repository. If a shared rule is
missing, propose a focused change to this file or the relevant public
document in a separate pull request, with a rationale. Maintainers review
changes to contribution rules and CI.

## License

By contributing you agree that your contributions are licensed under the
[Apache License 2.0](LICENSE).
