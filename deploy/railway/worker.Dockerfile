FROM node:24.18.0-bookworm-slim@sha256:d45d78e7929b46875bbd4e29bea672d5bc48186c6c3588306521c815e78352d6 AS analyzer-dependencies
WORKDIR /app/dependency-analyzer
COPY dependency-analyzer/package.json dependency-analyzer/package-lock.json ./
# Install the reviewed lockfile before copying application source. Lifecycle
# scripts stay disabled so release preparation never executes repository code.
RUN npm ci --ignore-scripts --omit=dev \
    && npm cache clean --force

FROM ubuntu:24.04
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git libssl3t64 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=analyzer-dependencies /usr/local/ /usr/local/
COPY --from=analyzer-dependencies /app/dependency-analyzer/node_modules /app/dependency-analyzer/node_modules

WORKDIR /app/dependency-analyzer
COPY dependency-analyzer/analyze.mjs ./
COPY dependency-analyzer/src ./src
COPY dependency-analyzer/third-party-dependency-analyzer.txt ./

WORKDIR /app
COPY bin /app/bin
ENV NODE_VERSION=24.18.0 \
    SCOPE_DEPENDENCY_ANALYZER_PATH=/app/dependency-analyzer/analyze.mjs
CMD ["/app/bin/scope-worker"]
