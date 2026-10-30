# Builds one node. docker-compose.yml runs three of them as a network.

# The web UI: built from web/ into public/, the pages, their scripts and their styles.
FROM node:24-slim AS web
WORKDIR /app
COPY package.json package-lock.json ./
RUN npm ci
COPY web web
RUN npm run build

FROM rust:1.88-slim AS build
WORKDIR /app
# There's no .git inside the build context, so the commit comes in as an argument.
ARG GIT_COMMIT=unknown
ENV GIT_COMMIT=$GIT_COMMIT
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY --from=web /app/public public
RUN cargo build --release --locked

FROM debian:bookworm-slim
# The node serves the UI from the path it was built at.
COPY --from=build /app/public /app/public
COPY --from=build /app/target/release/blockchain /usr/local/bin/blockchain
COPY --from=build /app/target/release/loadgen /usr/local/bin/loadgen
ENV BIND_ADDR=0.0.0.0:3000 \
    CHAIN_FILE=/data/chain.json
VOLUME /data
EXPOSE 3000
CMD ["blockchain"]
