# SPDX-License-Identifier: Apache-2.0
# KROWN's execution framework is Linux-only (its metrics collector refuses
# other systems). On macOS it runs in this container and starts the engine
# containers through the host's Docker socket.
FROM docker:29-cli AS cli

FROM python:3.12-slim-bookworm
COPY --from=cli /usr/local/bin/docker /usr/local/bin/docker
COPY requirements.txt /tmp/requirements.txt
RUN pip install --no-cache-dir -r /tmp/requirements.txt
