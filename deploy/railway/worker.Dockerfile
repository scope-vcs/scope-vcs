FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG INSTALL_GIT=1
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
ARG IMAGE_DEPENDENCY_EPOCH=local
COPY install-git.sh /tmp/install-git.sh
RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && if [ "$INSTALL_GIT" = 1 ]; then bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"; else mkdir -p /opt/git; fi

FROM ubuntu:24.04@sha256:008173c23f95b170204355c12626cb5a965d779a7e1283b09e9cffbb1bf33ca3
ARG IMAGE_DEPENDENCY_EPOCH=local
RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libcurl4t64 libexpat1 libssl3t64 zlib1g \
    && rm -rf /var/lib/apt/lists/*
ARG GIT_VERSION
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
COPY --from=git-builder /opt/git /opt/git
WORKDIR /app
COPY bin /app/bin
RUN useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
ENV HOME=/home/scope \
    XDG_CACHE_HOME=/home/scope/.cache
ENV PATH=/opt/git/bin:$PATH
RUN test "$(git --version)" = "git version ${GIT_VERSION}"
USER 65532:65532
CMD ["/app/bin/scope-worker"]
