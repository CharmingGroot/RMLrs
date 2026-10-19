# Releasing

## Versioning

- [Semantic Versioning](https://semver.org/). The project stays at `0.x` until at least M2 in the [roadmap](ROADMAP.md).
- All crates in the workspace share one version and are released together.

## How a release will happen

1. [release-plz](https://release-plz.dev/) opens a release pull request with the version bump and the changelog.
2. Merging it tags `vX.Y.Z` and publishes the crates to crates.io.
3. [cargo-dist](https://opensource.axo.dev/cargo-dist/) builds the `rmlrs` binary for Linux, macOS and Windows and attaches it to the GitHub release.

There has been no release yet. The release-plz and cargo-dist configurations
will be added with the first release; until then `publish = false` keeps the
crates from being published by accident.

## Release notes

Every release lists the conformance pass counts per module, in the same form as the [targets](ROADMAP.md#target). Performance claims in release notes link to the benchmark run that produced them.
