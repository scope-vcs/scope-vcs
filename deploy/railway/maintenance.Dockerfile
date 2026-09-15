# PostgreSQL 18 clients can dump and restore the Railway PostgreSQL 18 database.
# Trixie's glibc and OpenSSL 3 also support the Ubuntu-built maintenance binary.
FROM postgres:18.6@sha256:4ef4dbc939d61acea57712655ddb4b4ab27419c913f94cca0cd57cb3ea3c2280
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates git \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
WORKDIR /app
COPY bin /app/bin
ENV HOME=/home/scope XDG_CACHE_HOME=/home/scope/.cache
USER 65532:65532
# This service runs maintenance commands; disable the database image entrypoint.
ENTRYPOINT []
CMD ["/app/bin/scope-maintenance", "serve"]
