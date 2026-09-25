# syntax=docker/dockerfile:1.7

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
COPY deploy/railway/install-git.sh /tmp/install-git.sh
RUN bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"

FROM --platform=linux/amd64 rust:1.98.1-bookworm@sha256:93ce27a88655056a51dbdd8f5f2d7ddc071c7b0070fb288a37b5a285fc83971e AS scope-runtime-builder

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

FROM --platform=linux/amd64 rust:1.98.1-slim-bookworm@sha256:dacc9e51f252243eb59d2fb4cb4ad8b0d3f607b6a82c398cf8a321e59ff778a7

ARG NODE_VERSION=24.21.0
ARG NODE_SHA256=fd8e59d5a511510f6a298afb548f18c7d2b1be404d8b4a27d94fbe49f56cb2d6
ARG NPM_VERSION=11.19.1
ARG NPM_SHA512=cedb312b1b7f92421a02cfb68b4194e88f8346651dacd6acc5364a25ef5309d4a32b19616a1ff8aff865e7280c0fa5c0835a99f5cd93368991e2a80ec9da75d2
ARG PNPM_VERSION=11.24.0
ARG POSTGRES_VERSION=18.6-1.pgdg12+2
ARG PLAYWRIGHT_VERSION=1.63.0
ARG DEBIAN_SNAPSHOT=20260921T000000Z
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256

ENV DEBIAN_FRONTEND=noninteractive \
    PLAYWRIGHT_BROWSERS_PATH=/root/.cache/ms-playwright \
    RUSTUP_TOOLCHAIN=1.98.1-x86_64-unknown-linux-gnu

RUN sed -i \
        -e "s|http://deb.debian.org/debian-security|https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
        -e "s|http://deb.debian.org/debian|https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
    && apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl \
    && mkdir -p /usr/local/share/keyrings \
    && curl --fail --silent --show-error --location \
        https://www.postgresql.org/media/keys/ACCC4CF8.asc \
        --output /usr/local/share/keyrings/postgresql.asc \
    && echo '0144068502a1eddd2a0280ede10ef607d1ec592ce819940991203941564e8e76  /usr/local/share/keyrings/postgresql.asc' \
        | sha256sum --check --strict \
    && printf '%s\n' 'deb [signed-by=/usr/local/share/keyrings/postgresql.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main 18' \
        > /etc/apt/sources.list.d/postgresql.list \
    && apt-get update \
    && apt-get upgrade --yes \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        cmake \
        ffmpeg \
        iproute2 \
        jq \
        libheif-examples \
        openssh-client \
        pkg-config \
        python3-yaml \
        "postgresql-18=${POSTGRES_VERSION}" \
        "postgresql-client-18=${POSTGRES_VERSION}" \
        time \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

COPY --from=git-builder /opt/git /opt/git
ENV PATH=/opt/git/bin:$PATH

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

# Keep the files used by the checks startup path in small eager SOCI layers.
# The original Rust, Debian, and browser layers remain available for other checks.
RUN set -eu; \
    toolchain_dir="/usr/local/rustup/toolchains/${RUSTUP_TOOLCHAIN}"; \
    for file in "$toolchain_dir"/lib/libLLVM.so.* "$toolchain_dir"/bin/rustc /usr/local/cargo/bin/rustup; do \
        test -f "$file"; \
        cp -p "$file" "$file.hot"; \
        mv "$file.hot" "$file"; \
        touch "$file"; \
    done

RUN set -eu; \
    for file in "/usr/local/rustup/toolchains/${RUSTUP_TOOLCHAIN}"/lib/librustc_driver-*.so; do \
        test -f "$file"; \
        cp -p "$file" "$file.hot"; \
        mv "$file.hot" "$file"; \
        touch "$file"; \
    done

RUN set -eu; \
    { printf '%s\n' /usr/bin/ffmpeg; \
      ldd /usr/bin/ffmpeg | awk '$3 ~ /^\// {print $3} $1 ~ /^\// {print $1}' | xargs -r realpath; \
    } | sort -u | while IFS= read -r file; do \
        test -f "$file"; \
        cp -p "$file" "$file.hot"; \
        mv "$file.hot" "$file"; \
        touch "$file"; \
    done

RUN find /usr/local/lib/node_modules/playwright -type f \
    -exec sh -c 'for file do cp -p "$file" "$file.hot" && mv "$file.hot" "$file" && touch "$file" || exit; done' sh {} +

ARG PLAYWRIGHT_CHROMIUM_VERSION=153.0.8010.12
ARG PLAYWRIGHT_CHROMIUM_REVISION=1243

RUN test "$(rustc --version | cut -d' ' -f2)" = "1.98.1" \
    && test "$(git --version)" = "git version ${GIT_VERSION}" \
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
