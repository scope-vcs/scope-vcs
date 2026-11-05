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
ARG APT_VERSION=2.8.4ubuntu0.1
RUN mv /etc/apt/sources.list.d /tmp/apt-sources.list.d \
    && if [ -f /etc/apt/sources.list ]; then mv /etc/apt/sources.list /tmp/apt-sources.list; fi \
    && mkdir /etc/apt/sources.list.d \
    && printf '%s\n' 'Types: deb' 'URIs: http://security.ubuntu.com/ubuntu' 'Suites: noble-security' \
        'Components: main' 'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/apt-security.sources \
    && rm -rf /var/lib/apt/lists/* \
    && timeout --verbose --kill-after=10s 300s sh -ec '\
    apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 update \
    && apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends \
        "apt=${APT_VERSION}" "libapt-pkg6.0t64=${APT_VERSION}"' \
    && rm -rf /etc/apt/sources.list.d \
    && mv /tmp/apt-sources.list.d /etc/apt/sources.list.d \
    && if [ -f /tmp/apt-sources.list ]; then mv /tmp/apt-sources.list /etc/apt/sources.list; fi \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4t64 libexpat1 libssl3t64 zlib1g \
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
