FROM docker.io/library/debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a AS heif-builder

ARG LIBHEIF_VERSION=1.23.5
ARG LIBHEIF_SHA256=fd9036064c4432f0550d15072ddf34956a248279ee9aeaff0fba3fa0f77d8f1a
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates cmake curl libaom-dev libde265-dev \
        libjpeg-dev libpng-dev libx265-dev ninja-build pkg-config \
    && rm -rf /var/lib/apt/lists/* \
    && curl --fail --location --silent --show-error --retry 3 \
        "https://github.com/strukturag/libheif/releases/download/v${LIBHEIF_VERSION}/libheif-${LIBHEIF_VERSION}.tar.gz" \
        --output /tmp/libheif.tar.gz \
    && echo "${LIBHEIF_SHA256}  /tmp/libheif.tar.gz" | sha256sum --check --strict \
    && tar --extract --gzip --file /tmp/libheif.tar.gz --directory /tmp \
    && cmake -S "/tmp/libheif-${LIBHEIF_VERSION}" -B /tmp/libheif-build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/libheif \
        -DCMAKE_INSTALL_LIBDIR=lib -DBUILD_TESTING=OFF \
        -DENABLE_PLUGIN_LOADING=OFF -DWITH_EXAMPLES=ON \
        -DWITH_EXAMPLE_HEIF_THUMB=OFF -DWITH_EXAMPLE_HEIF_VIEW=OFF \
        -DWITH_LIBDE265=ON -DWITH_X265=ON \
        -DWITH_AOM_DECODER=ON -DWITH_AOM_ENCODER=ON \
        -DWITH_X264=OFF -DWITH_OpenH264_DECODER=OFF \
        -DWITH_LIBSHARPYUV=OFF \
    && cmake --build /tmp/libheif-build --parallel 2 \
    && cmake --install /tmp/libheif-build
RUN LD_LIBRARY_PATH=/opt/libheif/lib /opt/libheif/bin/heif-info --version \
    | grep --fixed-strings "$LIBHEIF_VERSION"

FROM docker.io/library/rust:1.98.1-trixie@sha256:a8a5f0a1e5fe7dfe1d352591e4a1c7dd2c08fd70475cae872cf3458ba0df0546 AS build

COPY --from=heif-builder /opt/libheif /opt/libheif
ENV PATH=/opt/libheif/bin:$PATH LD_LIBRARY_PATH=/opt/libheif/lib CARGO_BUILD_JOBS=2

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates=20250419 \
        ffmpeg=7:7.1.5-0+deb13u1 \
        libaom3 libde265-0 libjpeg62-turbo libpng16-16t64 libx265-215 \
    && rm -rf /var/lib/apt/lists/* \
    && echo /opt/libheif/lib > /etc/ld.so.conf.d/libheif.conf \
    && ldconfig

ENV SCOPE_MEDIA_HEIF_CONVERT=/opt/libheif/bin/heif-convert \
    SCOPE_MEDIA_HEIF_INFO=/opt/libheif/bin/heif-info

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
    && chmod +x media-worker/tests/*.sh \
    && media-worker/tests/generate-codec-fixtures.sh /tmp/scope-media-fixtures \
    && SCOPE_MEDIA_FFMPEG=/scope/media-worker/tests/ffmpeg-many-cpus.sh \
        target/release/scope-media-worker codec-self-test /tmp/scope-media-fixtures \
    && media-worker/tests/assert-codec-output.sh /tmp/scope-media-fixtures/processed

FROM docker.io/library/debian:trixie-slim@sha256:a99cfc517144bc59b1978475ec53b46ecabec7e43635402ee5b77cc54cd1b20a

LABEL org.opencontainers.image.source="https://github.com/scope-vcs/scope-vcs"

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates=20250419 \
        ffmpeg=7:7.1.5-0+deb13u1 \
        libaom3 libde265-0 libjpeg62-turbo libpng16-16t64 libx265-215 \
    && rm -rf /var/lib/apt/lists/* \
    && echo /opt/libheif/lib > /etc/ld.so.conf.d/libheif.conf \
    && mkdir -p /tmp/scope-media-worker /scope/licenses \
    && chown 65532:65532 /tmp/scope-media-worker

COPY --from=build /scope/target/release/scope-media-worker /usr/local/bin/scope-media-worker
COPY --from=heif-builder /opt/libheif /opt/libheif
COPY --from=heif-builder /tmp/libheif.tar.gz /scope/licenses/libheif-1.23.5.tar.gz
COPY --from=heif-builder /tmp/libheif-1.23.5/COPYING /scope/licenses/libheif-COPYING
COPY --from=heif-builder /tmp/libheif-1.23.5/examples/COPYING /scope/licenses/libheif-examples-COPYING
COPY LICENSE NOTICE legal/third-party-rust.txt /scope/licenses/
COPY media-worker/CODEC-NOTICE.md /scope/licenses/
COPY media-worker/Dockerfile /scope/licenses/libheif-build.Dockerfile

RUN ldconfig

ENV SCOPE_MEDIA_SCRATCH_DIR=/tmp/scope-media-worker \
    SCOPE_MEDIA_HEIF_CONVERT=/opt/libheif/bin/heif-convert \
    SCOPE_MEDIA_HEIF_INFO=/opt/libheif/bin/heif-info \
    PATH=/opt/libheif/bin:$PATH \
    LD_LIBRARY_PATH=/opt/libheif/lib
RUN heif-info --version | grep --fixed-strings '1.23.5' \
    && heif-convert --version | grep --fixed-strings '1.23.5' \
    && ffmpeg -version | grep --fixed-strings '7.1.5' \
    && ! dpkg-query --show libheif1 >/dev/null 2>&1
EXPOSE 8080
VOLUME ["/tmp/scope-media-worker"]
USER 65532:65532
ENTRYPOINT ["/usr/local/bin/scope-media-worker"]
