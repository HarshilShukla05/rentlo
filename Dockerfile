# One image for every Rentlo service. The service picks its role with
# RENTLO_COMMAND (bot, api, migrate).

FROM node:24-bookworm-slim AS web
WORKDIR /web
COPY web/package.json web/package-lock.json ./
RUN npm ci --no-audit --no-fund
COPY web/index.html ./
COPY web/src ./src
RUN npm run build

FROM rust:1.97-trixie AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml ./
COPY crates ./crates
RUN cargo build --release --locked -p rentlo \
 && cp target/release/rentlo /rentlo

FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --uid 1000 --create-home rentlo \
 && mkdir /data && chown rentlo /data
COPY --from=build /rentlo /usr/local/bin/rentlo
COPY --from=web /web/dist /usr/local/share/rentlo/web
ENV RENTLO_WEB_ROOT=/usr/local/share/rentlo/web
EXPOSE 8080
USER rentlo
# The WhatsApp session file. Mounted from a disk in production.
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/rentlo"]
