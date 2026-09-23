FROM ubuntu:24.04
ARG INSTALL_GIT=0
ARG BINARY
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
ENV SCOPE_COMPONENT_BINARY=$BINARY
# jemalloc decay: hand freed pages back to the OS within seconds instead of holding them.
ENV _RJEM_MALLOC_CONF=background_thread:true,dirty_decay_ms:5000,muzzy_decay_ms:5000
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3t64 \
    && if [ "$INSTALL_GIT" = 1 ]; then apt-get install -y --no-install-recommends git; fi \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . /app/
# Git materializations and local object storage use .scope by default. Keep
# release binaries root-owned while allowing the service to write its data.
RUN useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
ENV HOME=/home/scope XDG_CACHE_HOME=/home/scope/.cache
USER 65532:65532
CMD ["sh", "-c", "exec /app/bin/$SCOPE_COMPONENT_BINARY"]
