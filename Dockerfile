FROM rust:1.98.1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home blossom && mkdir /data && chown blossom /data
COPY --from=build /src/target/release/blossom /usr/local/bin/blossom
USER blossom
ENV TOON_BLOB_STORAGE_DIR=/data \
    TOON_BLOB_PUBLIC_URL=http://localhost:3000
VOLUME /data
# 3000: public port (blob reads). 3001: write port, internal only: never publish it.
EXPOSE 3000 3001
ENTRYPOINT ["blossom"]
