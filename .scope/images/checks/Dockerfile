# syntax=docker/dockerfile:1.7

FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
COPY deploy/railway/install-git.sh /tmp/install-git.sh
RUN bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"

FROM --platform=linux/amd64 rust:1.98.1-slim-bookworm@sha256:dacc9e51f252243eb59d2fb4cb4ad8b0d3f607b6a82c398cf8a321e59ff778a7 AS scope-runtime-builder

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

ADD --chmod=644 --checksum=sha256:0144068502a1eddd2a0280ede10ef607d1ec592ce819940991203941564e8e76 \
    https://www.postgresql.org/media/keys/ACCC4CF8.asc /etc/apt/keyrings/postgresql.asc

RUN sed -i \
        -e "s|http://deb.debian.org/debian-security|https://snapshot.debian.org/archive/debian-security/${DEBIAN_SNAPSHOT}|" \
        -e "s|http://deb.debian.org/debian|https://snapshot.debian.org/archive/debian/${DEBIAN_SNAPSHOT}|" \
        /etc/apt/sources.list.d/debian.sources \
    && printf '%s\n' 'Acquire::Check-Valid-Until "false";' > /etc/apt/apt.conf.d/99snapshot \
    && printf '%s\n' 'deb [signed-by=/etc/apt/keyrings/postgresql.asc] https://apt.postgresql.org/pub/repos/apt bookworm-pgdg main 18' \
        > /etc/apt/sources.list.d/postgresql.list \
    && apt-get update \
    && apt-get upgrade --yes \
    && apt-get install --yes --no-install-recommends \
        build-essential \
        cmake \
        curl \
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

# SOCI fetches layers under the publisher's 150 MB cutoff before the task
# starts and fetches larger layers lazily. Each RUN below rewrites files most
# jobs read into its own layer under that cutoff. Chromium and PostgreSQL stay
# lazy because only some jobs use them.
RUN --mount=type=bind,source=.scope/images/checks/promote-eager-files.sh,target=/tmp/promote-eager-files \
    toolchain="/usr/local/rustup/toolchains/${RUSTUP_TOOLCHAIN}" \
    && bash /tmp/promote-eager-files \
        "$toolchain"/lib/libLLVM.so.* \
        "$toolchain"/bin/rustc \
        /usr/local/cargo/bin/rustup

RUN --mount=type=bind,source=.scope/images/checks/promote-eager-files.sh,target=/tmp/promote-eager-files \
    bash /tmp/promote-eager-files "/usr/local/rustup/toolchains/${RUSTUP_TOOLCHAIN}"/lib/librustc_driver-*.so

RUN --mount=type=bind,source=.scope/images/checks/promote-eager-files.sh,target=/tmp/promote-eager-files \
    toolchain="/usr/local/rustup/toolchains/${RUSTUP_TOOLCHAIN}" \
    && bash /tmp/promote-eager-files \
        "$toolchain"/bin/cargo \
        "$toolchain"/bin/cargo-clippy \
        "$toolchain"/bin/cargo-fmt \
        "$toolchain"/bin/clippy-driver \
        "$toolchain"/bin/rustdoc \
        "$toolchain"/bin/rustfmt \
        "$toolchain"/lib/rustlib/x86_64-unknown-linux-gnu \
        /usr/bin/x86_64-linux-gnu-gcc-12 \
        /usr/lib/gcc/x86_64-linux-gnu/12/collect2 \
        /usr/lib/gcc/x86_64-linux-gnu/12/crt*.o \
        /usr/lib/gcc/x86_64-linux-gnu/12/libgcc* \
        /usr/lib/x86_64-linux-gnu/Scrt1.o \
        /usr/lib/x86_64-linux-gnu/crti.o \
        /usr/lib/x86_64-linux-gnu/crtn.o \
        /usr/lib/x86_64-linux-gnu/libc.so \
        /usr/lib/x86_64-linux-gnu/libc_nonshared.a \
        /usr/local/rustup/settings.toml \
        /etc/ssl/certs/ca-certificates.crt

RUN --mount=type=bind,source=.scope/images/checks/promote-eager-files.sh,target=/tmp/promote-eager-files \
    bash /tmp/promote-eager-files --libraries-of /usr/bin/ffmpeg

RUN --mount=type=bind,source=.scope/images/checks/promote-eager-files.sh,target=/tmp/promote-eager-files \
    bash /tmp/promote-eager-files /usr/local/lib/node_modules/playwright

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
