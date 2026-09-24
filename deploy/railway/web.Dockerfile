FROM node:24.21.0-bookworm-slim@sha256:0e0ff40c39bc087845bfb27465a0df4ea419520094bc35842ff83dd8cbe6f9b6
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
WORKDIR /app
ENV NODE_ENV=production HOST=0.0.0.0
COPY .output /app/.output
COPY .scope-deployment-sha /app/.scope-deployment-sha
ENV HOME=/home/node
USER node
CMD ["node", "/app/.output/server/index.mjs"]
