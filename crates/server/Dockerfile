# syntax=docker/dockerfile:1
# Build from the repo root: docker build -f crates/server/Dockerfile .
FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY . .
# ponytail: BuildKit cache mounts keep the cargo registry + target dir across builds (no cargo-chef layer dance).
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p glot-server \
    && cp target/release/glot-server /glot-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /glot-server /usr/local/bin/glot-server
ENV GLOT_BIND=0.0.0.0:8790
EXPOSE 8790
USER nobody
CMD ["glot-server"]
