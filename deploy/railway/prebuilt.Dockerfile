# Git is built from the version and checked source archive in dev/tool-versions.json.
FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG INSTALL_GIT
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
ARG IMAGE_DEPENDENCY_EPOCH=local
COPY install-git.sh /tmp/install-git.sh
RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && if [ "$INSTALL_GIT" = 1 ]; then bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"; else mkdir -p /opt/git; fi

FROM ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3
ARG INSTALL_GIT=0
ARG IMAGE_DEPENDENCY_EPOCH=local
# jemalloc decay: hand freed pages back to the OS within seconds instead of holding them.
ENV _RJEM_MALLOC_CONF=background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000
RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3t64 \
    && if [ "$INSTALL_GIT" = 1 ]; then apt-get install -y --no-install-recommends libcurl4t64 libexpat1 zlib1g; fi \
    && rm -rf /var/lib/apt/lists/*
ARG GIT_VERSION
ARG BINARY
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
ENV SCOPE_COMPONENT_BINARY=$BINARY
COPY --from=git-builder /opt/git /opt/git
ENV PATH=/opt/git/bin:$PATH
RUN if [ "$INSTALL_GIT" = 1 ]; then test "$(git --version)" = "git version ${GIT_VERSION}"; fi
WORKDIR /app
COPY . /app/
RUN rm /app/install-git.sh
# Git materializations and local object storage use .scope by default. Keep
# release binaries root-owned while allowing the service to write its data.
RUN useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
ENV HOME=/home/scope XDG_CACHE_HOME=/home/scope/.cache
USER 65532:65532
CMD ["sh", "-c", "exec /app/bin/$SCOPE_COMPONENT_BINARY"]
