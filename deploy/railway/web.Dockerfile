FROM node:26.13.0-bookworm-slim@sha256:bc0a2afd3b74ce12222fc8bab99271e66257f021078f5f4a5d687a77d280c1d9
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

ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
WORKDIR /app
ENV NODE_ENV=production HOST=0.0.0.0
COPY .output /app/.output
COPY .scope-deployment-sha /app/.scope-deployment-sha
ENV HOME=/home/node
USER node
CMD ["node", "--import", "/app/.output/server/_instrumentation/instrumentation.mjs", "/app/.output/server/index.mjs"]
