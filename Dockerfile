FROM debian:bookworm-slim

ARG SYMBIO_BUILD_ID
LABEL symbio.build-id=${SYMBIO_BUILD_ID}

COPY dist/symbio /usr/local/bin/symbio
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && chmod 0555 /usr/local/bin/symbio
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/symbio"]
