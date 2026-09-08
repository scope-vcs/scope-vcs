FROM ubuntu:24.04
ARG INSTALL_GIT=0
ARG BINARY
ENV SCOPE_COMPONENT_BINARY=$BINARY
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3t64 \
    && if [ "$INSTALL_GIT" = 1 ]; then apt-get install -y --no-install-recommends git; fi \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY . /app/
CMD ["sh", "-c", "exec /app/bin/$SCOPE_COMPONENT_BINARY"]
