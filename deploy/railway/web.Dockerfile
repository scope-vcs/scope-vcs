FROM node:24-bookworm-slim
ARG SCOPE_ANALYTICS_RELEASE
ENV SCOPE_ANALYTICS_RELEASE=$SCOPE_ANALYTICS_RELEASE
WORKDIR /app
ENV NODE_ENV=production HOST=0.0.0.0
COPY .output /app/.output
COPY .scope-deployment-sha /app/.scope-deployment-sha
CMD ["node", "/app/.output/server/index.mjs"]
