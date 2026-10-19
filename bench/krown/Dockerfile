# SPDX-License-Identifier: Apache-2.0
# RMLrs CLI image for the KROWN execution framework.
# Build from the repository root:
#   docker build -f bench/krown/Dockerfile -t rmlrs-krown/rmlrs:dev .
FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release -p rmlrs-cli

FROM debian:bookworm-slim
COPY --from=build /src/target/release/rmlrs /usr/local/bin/rmlrs
RUN mkdir -p /data/shared
WORKDIR /data/shared
CMD ["rmlrs", "--help"]
