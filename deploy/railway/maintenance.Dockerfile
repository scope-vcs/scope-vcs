FROM debian:bookworm-slim@sha256:5ae3c39ebd15e229dcedd5cee596b2497182493d41ff162e824ba13fc1b2b867 AS git-builder
ARG INSTALL_GIT=1
ARG GIT_VERSION
ARG GIT_SOURCE_SHA256
ARG IMAGE_DEPENDENCY_EPOCH=local
COPY install-git.sh /tmp/install-git.sh
RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && if [ "$INSTALL_GIT" = 1 ]; then bash /tmp/install-git.sh "$GIT_VERSION" "$GIT_SOURCE_SHA256"; else mkdir -p /opt/git; fi

FROM postgres:18.6@sha256:86c951e05bf56c93d95d397747fb8820ac76cc3bedb78f43abd83eedbe3666ae
ARG IMAGE_DEPENDENCY_EPOCH=local
ARG APT_VERSION=3.0.4
RUN mv /etc/apt/sources.list.d /tmp/apt-sources.list.d \
    && if [ -f /etc/apt/sources.list ]; then mv /etc/apt/sources.list /tmp/apt-sources.list; fi \
    && mkdir /etc/apt/sources.list.d \
    && printf '%s\n' 'Types: deb' 'URIs: http://deb.debian.org/debian-security' 'Suites: trixie-security' \
        'Components: main' 'Signed-By: /usr/share/keyrings/debian-archive-keyring.gpg' \
        > /etc/apt/sources.list.d/apt-security.sources \
    && rm -rf /var/lib/apt/lists/* \
    && timeout --verbose --kill-after=10s 300s sh -ec '\
    apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 update \
    && apt-get -o Acquire::Retries=0 -o Acquire::http::Timeout=30 install --yes --no-install-recommends \
        "apt=${APT_VERSION}" "libapt-pkg7.0=${APT_VERSION}"' \
    && rm -rf /etc/apt/sources.list.d \
    && mv /tmp/apt-sources.list.d /etc/apt/sources.list.d \
    && if [ -f /tmp/apt-sources.list ]; then mv /tmp/apt-sources.list /etc/apt/sources.list; fi \
    && rm -rf /var/lib/apt/lists/*

RUN test -n "$IMAGE_DEPENDENCY_EPOCH" \
    && apt-get update \
    && apt-get install -y --no-install-recommends \
        ca-certificates libcurl4t64 libexpat1 zlib1g \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 65532 --user-group --create-home --shell /usr/sbin/nologin scope \
    && mkdir -p /app/.scope /home/scope/.cache \
    && chown 65532:65532 /app/.scope /home/scope/.cache
ARG GIT_VERSION
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
COPY --from=git-builder /opt/git /opt/git
ENV PATH=/opt/git/bin:$PATH
RUN test "$(git --version)" = "git version ${GIT_VERSION}"
WORKDIR /app
COPY bin /app/bin
ENV HOME=/home/scope XDG_CACHE_HOME=/home/scope/.cache
USER 65532:65532
ENTRYPOINT []
CMD ["/app/bin/scope-maintenance", "serve"]
