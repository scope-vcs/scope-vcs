# syntax=docker/dockerfile:1.7

# The runner base image holds only what the native runner needs inside a job
# container: the runtime at /scope/bin/scope-runner-runtime, Git, and a shell.
# Workflows that need toolchains build their own image from this one.

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
COPY deploy/railway/install-git.sh /tmp/install-git.sh
RUN bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"

FROM --platform=linux/amd64 rust:1.98.1-slim-bookworm@sha256:dacc9e51f252243eb59d2fb4cb4ad8b0d3f607b6a82c398cf8a321e59ff778a7 AS runtime-builder

WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY crates crates
COPY api api
COPY cache-service cache-service
COPY media-service media-service
COPY media-worker media-worker
COPY repo-router repo-router
COPY worker worker
COPY runner-runtime runner-runtime
RUN cargo build --locked --release -p scope-runner-runtime

FROM --platform=linux/amd64 debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867

ARG DEBIAN_SNAPSHOT=20261005T090935Z
ARG GIT_VERSION

ENV DEBIAN_FRONTEND=noninteractive

# Git needs these libraries at run time. The snapshot keeps upgrades
# reproducible. Plain HTTP is used because this base has no CA certificates yet;
# apt still verifies the signed package indexes.
RUN sed -i \
        -e "s|http://deb.debian.org/debian-security|http://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
        -e "s|http://deb.debian.org/debian|http://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
    && apt-get update \
    && apt-get upgrade --yes \
    && apt-get install --yes --no-install-recommends \
        ca-certificates \
        libcurl4 \
        libexpat1 \
        libssl3 \
        zlib1g \
    && rm -rf /var/lib/apt/lists/*

COPY --from=git-builder /opt/git /opt/git
ENV PATH=/opt/git/bin:$PATH

COPY --from=runtime-builder /build/target/release/scope-runner-runtime /scope/bin/scope-runner-runtime
COPY LICENSE NOTICE legal/third-party-rust.txt /scope/licenses/

RUN test "$(git --version)" = "git version ${GIT_VERSION}" \
    && test -x /scope/bin/scope-runner-runtime

WORKDIR /workspace
