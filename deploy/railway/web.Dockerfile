FROM node:26.13.0-bookworm-slim@sha256:bc0a2afd3b74ce12222fc8bab99271e66257f021078f5f4a5d687a77d280c1d9
ARG APT_VERSION=2.6.2
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        "apt=${APT_VERSION}" "libapt-pkg6.0=${APT_VERSION}" \
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
