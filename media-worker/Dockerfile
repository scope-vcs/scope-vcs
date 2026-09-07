FROM docker.io/library/rust:1.98-bookworm@sha256:4e4a7e7939c17991ab35f2b8c2e67593980f771d28f6b1254b1850f860fd0c7f AS build

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates=20250419~deb12u1 \
        ffmpeg=7:5.1.9-0+deb12u1 \
        libheif-examples=1.15.1-1+deb12u1 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /scope
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY api ./api
COPY cache-service ./cache-service
COPY crates ./crates
COPY media-service ./media-service
COPY media-worker/Cargo.toml ./media-worker/Cargo.toml
COPY media-worker/src ./media-worker/src
COPY media-worker/tests ./media-worker/tests
COPY repo-router ./repo-router
COPY runner-runtime ./runner-runtime
COPY worker ./worker
RUN cargo test --locked --release -p scope-media-worker \
    && cargo build --locked --release -p scope-media-worker \
    && chmod +x media-worker/tests/generate-codec-fixtures.sh media-worker/tests/assert-codec-output.sh \
    && media-worker/tests/generate-codec-fixtures.sh /tmp/scope-media-fixtures \
    && target/release/scope-media-worker codec-self-test /tmp/scope-media-fixtures \
    && media-worker/tests/assert-codec-output.sh /tmp/scope-media-fixtures/processed

FROM docker.io/library/debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867

LABEL org.opencontainers.image.source="https://github.com/scope-vcs/scope-vcs"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates=20250419~deb12u1 \
        ffmpeg=7:5.1.9-0+deb12u1 \
        libheif-examples=1.15.1-1+deb12u1 \
    && rm -rf /var/lib/apt/lists/* \
    && mkdir -p /tmp/scope-media-worker /scope/licenses \
    && chown 65532:65532 /tmp/scope-media-worker

COPY --from=build /scope/target/release/scope-media-worker /usr/local/bin/scope-media-worker
COPY LICENSE NOTICE legal/third-party-rust.txt /scope/licenses/
COPY media-worker/CODEC-NOTICE.md /scope/licenses/

ENV SCOPE_MEDIA_SCRATCH_DIR=/tmp/scope-media-worker
EXPOSE 8080
VOLUME ["/tmp/scope-media-worker"]
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/scope-media-worker"]
