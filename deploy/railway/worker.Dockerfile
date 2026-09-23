FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
COPY install-git.sh /tmp/install-git.sh
RUN bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"

FROM node:24.21.0-bookworm-slim@sha256:0e0ff40c39bc087845bfb27465a0df4ea419520094bc35842ff83dd8cbe6f9b6 AS analyzer-dependencies
WORKDIR /app/dependency-analyzer
COPY dependency-analyzer/package.json dependency-analyzer/package-lock.json ./
# Install the reviewed lockfile before copying application source. Lifecycle
# scripts stay disabled so release preparation never executes repository code.
RUN npm ci --ignore-scripts --omit=dev \
    && npm cache clean --force

FROM ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3
ARG GIT_VERSION
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libcurl4t64 libexpat1 libssl3t64 zlib1g \
    && rm -rf /var/lib/apt/lists/*
COPY --from=git-builder /opt/git /opt/git
COPY --from=analyzer-dependencies /usr/local/ /usr/local/
COPY --from=analyzer-dependencies /app/dependency-analyzer/node_modules /app/dependency-analyzer/node_modules

WORKDIR /app/dependency-analyzer
COPY dependency-analyzer/analyze.mjs ./
COPY dependency-analyzer/src ./src
COPY dependency-analyzer/third-party-dependency-analyzer.txt ./

WORKDIR /app
COPY bin /app/bin
# Repository restoration and dependency snapshots live below .scope. The
# worker can write there and in its home, but cannot replace its executable.
RUN useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
ENV NODE_VERSION=24.21.0 \
    HOME=/home/scope \
    XDG_CACHE_HOME=/home/scope/.cache \
    SCOPE_DEPENDENCY_ANALYZER_PATH=/app/dependency-analyzer/analyze.mjs
ENV PATH=/opt/git/bin:$PATH
RUN test "$(git --version)" = "git version ${GIT_VERSION}" \
    && test "$(node --version)" = "v${NODE_VERSION}"
USER 65532:65532
CMD ["/app/bin/scope-worker"]
