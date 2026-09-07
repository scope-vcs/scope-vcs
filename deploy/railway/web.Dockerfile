FROM node:24-bookworm-slim
WORKDIR /app
ENV NODE_ENV=production HOST=0.0.0.0
COPY .output /app/.output
COPY .scope-deployment-sha /app/.scope-deployment-sha
CMD ["node", "/app/.output/server/index.mjs"]
