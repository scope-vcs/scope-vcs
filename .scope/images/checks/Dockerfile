# syntax=docker/dockerfile:1.7

FROM --platform=linux/amd64 rust:1.98.0-bookworm@sha256:585c722e62805d1636fee6c80e744cdddfd81a9d32e08cba21ff15b69845e0fd AS scope-runtime-builder

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

FROM --platform=linux/amd64 rust:1.98.0-bookworm@sha256:585c722e62805d1636fee6c80e744cdddfd81a9d32e08cba21ff15b69845e0fd

ARG NODE_VERSION=24.18.0
ARG NODE_SHA256=55aa7153f9d88f28d765fcdad5ae6945b5c0f98a36881703817e4c450fa76742
ARG NPM_VERSION=11.19.1
ARG NPM_SHA512=cedb312b1b7f92421a02cfb68b4194e88f8346651dacd6acc5364a25ef5309d4a32b19616a1ff8aff865e7280c0fa5c0835a99f5cd93368991e2a80ec9da75d2
ARG PNPM_VERSION=11.24.0
ARG POSTGRES_VERSION=15.19-0+deb12u1
ARG PLAYWRIGHT_VERSION=1.61.1
ARG DEBIAN_SNAPSHOT=20260920T000000Z

ENV DEBIAN_FRONTEND=noninteractive \
    PLAYWRIGHT_BROWSERS_PATH=/root/.cache/ms-playwright \
    RUSTUP_TOOLCHAIN=1.98.0-x86_64-unknown-linux-gnu

RUN sed -i \
        -e "s|http://deb.debian.org/debian-security|https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
        -e "s|http://deb.debian.org/debian|https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
    && apt-get update \
    && apt-get upgrade --yes \
    && apt-get install --yes --no-install-recommends \
        ffmpeg \
        iproute2 \
        jq \
        libheif-examples \
        python3-yaml \
        "postgresql-15=${POSTGRES_VERSION}" \
        "postgresql-client-15=${POSTGRES_VERSION}" \
        time \
    && rm -rf /var/lib/apt/lists/*

RUN curl --fail --silent --show-error --retry 3 --retry-all-errors --retry-delay 2 \
        "https://nodejs.org/dist/v${NODE_VERSION}/node-v${NODE_VERSION}-linux-x64.tar.xz" \
        --output /tmp/node.tar.xz \
    && echo "${NODE_SHA256}  /tmp/node.tar.xz" | sha256sum --check --strict \
    && tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1 \
    && rm /tmp/node.tar.xz \
    && curl --fail --silent --show-error --retry 3 --retry-all-errors --retry-delay 2 \
        "https://registry.npmjs.org/npm/-/npm-${NPM_VERSION}.tgz" \
        --output /tmp/npm.tgz \
    && echo "${NPM_SHA512}  /tmp/npm.tgz" | sha512sum --check --strict \
    && npm install --global --ignore-scripts --no-audit --no-fund /tmp/npm.tgz \
    && rm /tmp/npm.tgz \
    && corepack enable \
    && corepack prepare "pnpm@${PNPM_VERSION}" --activate

RUN rustup component add \
        --toolchain "${RUSTUP_TOOLCHAIN}" \
        rustfmt \
        clippy \
    && npm install --global "playwright@${PLAYWRIGHT_VERSION}" \
    && playwright install --with-deps chromium \
    && npm cache clean --force \
    && rm -rf /var/lib/apt/lists/* /tmp/*

ARG PLAYWRIGHT_CHROMIUM_VERSION=149.0.7827.55
ARG PLAYWRIGHT_CHROMIUM_REVISION=1228

RUN test "$(rustc --version | cut -d' ' -f2)" = "1.98.0" \
    && test "$(node --version)" = "v${NODE_VERSION}" \
    && test "$(npm --version)" = "${NPM_VERSION}" \
    && test "$(pnpm --version)" = "${PNPM_VERSION}" \
    && python3 -c 'import yaml; assert yaml.safe_load("enabled: true")["enabled"] is True' \
    && jq --version \
    && ffmpeg -version \
    && ffprobe -version \
    && heif-convert --version \
    && heif-info --version \
    && test "$(psql --version | cut -d' ' -f3)" = "${POSTGRES_VERSION%%-*}" \
    && test "$(playwright --version)" = "Version ${PLAYWRIGHT_VERSION}" \
    && test -x "${PLAYWRIGHT_BROWSERS_PATH}/chromium-${PLAYWRIGHT_CHROMIUM_REVISION}/chrome-linux64/chrome" \
    && "${PLAYWRIGHT_BROWSERS_PATH}/chromium-${PLAYWRIGHT_CHROMIUM_REVISION}/chrome-linux64/chrome" --version \
        | grep --quiet "^Google Chrome for Testing ${PLAYWRIGHT_CHROMIUM_VERSION} *$"

COPY --from=scope-runtime-builder /build/target/release/scope-runner-runtime /scope/bin/scope-runner-runtime
COPY LICENSE NOTICE legal/third-party-rust.txt /scope/licenses/

RUN test -x /scope/bin/scope-runner-runtime

WORKDIR /workspace
