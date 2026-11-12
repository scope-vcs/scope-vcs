# syntax=docker/dockerfile:1.7

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
ARG APT_VERSION=2.6.2
RUN mv /etc/apt/sources.list.d /tmp/apt-sources.list.d \
    && if [ -f /etc/apt/sources.list ]; then mv /etc/apt/sources.list /tmp/apt-sources.list; fi \
    && mkdir /etc/apt/sources.list.d \
    && printf '%s\n' 'Types: deb' 'URIs: http://deb.debian.org/debian-security' 'Suites: bookworm-security' \
        'Components: main' 'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/apt-security.sources \
    && rm -rf /var/lib/apt/lists/* \
    && timeout --verbose --kill-after=10s 300s sh -ec '\
    apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 update \
    && apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends \
        "apt=${APT_VERSION}" "libapt-pkg6.0=${APT_VERSION}"' \
    && rm -rf /etc/apt/sources.list.d \
    && mv /tmp/apt-sources.list.d /etc/apt/sources.list.d \
    && if [ -f /tmp/apt-sources.list ]; then mv /tmp/apt-sources.list /etc/apt/sources.list; fi \
    && rm -rf /var/lib/apt/lists/*

COPY dev/install-git.sh /tmp/install-git.sh
RUN bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"

FROM --platform=linux/amd64 rust:1.100.0-slim-bookworm@sha256:476f6598b8d14a1f1bc7e58efc67af206ac8a559c37a3401ce512db2582d0ff7 AS chef
RUN cargo install cargo-chef --version 0.1.78 --locked
WORKDIR /build

FROM chef AS planner
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY api api
COPY cli-downloads cli-downloads
COPY media-service media-service
COPY media-worker media-worker
COPY repo-router repo-router
COPY worker worker
COPY runner-runtime runner-runtime
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS runtime-builder
COPY --from=planner /build/recipe.json recipe.json
RUN cargo chef cook --locked --release -p scope-runner-runtime --recipe-path recipe.json
COPY --from=planner /build ./
RUN cargo build --locked --release -p scope-runner-runtime

FROM --platform=linux/amd64 debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867

ARG IMAGE_DEPENDENCY_EPOCH=local
ARG APT_VERSION=2.6.2
ARG GIT_VERSION

ENV DEBIAN_FRONTEND=noninteractive

RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && mv /etc/apt/sources.list.d/debian.sources /tmp/debian.sources \
    && printf '%s\n' 'Types: deb' 'URIs: http://deb.debian.org/debian-security' 'Suites: bookworm-security' \
        'Components: main' 'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/apt-security.sources \
    && timeout --verbose --kill-after=10s 300s sh -ec '\
    apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 update \
    && apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends \
        "apt=${APT_VERSION}" "libapt-pkg6.0=${APT_VERSION}"' \
    && rm /etc/apt/sources.list.d/apt-security.sources \
    && mv /tmp/debian.sources /etc/apt/sources.list.d/debian.sources \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && timeout --verbose --kill-after=10s 300s sh -ec '\
    apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=30 update \
    && apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=30 upgrade --yes \
    && apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=30 install --yes --no-install-recommends \
        ca-certificates \
        libcurl4 \
        libexpat1 \
        libssl3 \
        zlib1g' \
    && rm -rf /var/lib/apt/lists/*

COPY --from=git-builder /opt/git /opt/git
ENV PATH=/opt/git/bin:$PATH

COPY --from=runtime-builder /build/target/release/scope-runner-runtime /scope/bin/scope-runner-runtime
COPY LICENSE NOTICE legal/third-party-rust.txt /scope/licenses/

RUN test "$(git --version)" = "git version ${GIT_VERSION}" \
    && test -x /scope/bin/scope-runner-runtime

WORKDIR /workspace
