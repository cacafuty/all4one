FROM rust:1.94-bookworm AS builder
WORKDIR /workspace

COPY Cargo.toml ./
COPY common/Cargo.toml common/Cargo.toml
COPY agent/Cargo.toml agent/Cargo.toml
COPY agent/build.rs agent/build.rs
COPY agent/proto agent/proto
COPY common/src common/src
COPY agent/src agent/src

RUN cargo build --release -p all4one-agent

FROM docker:29.2.0-cli AS dockercli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=dockercli /usr/local/bin/docker /usr/local/bin/docker
COPY --from=builder /workspace/target/release/all4one-agent /usr/local/bin/all4one-agent
ENTRYPOINT ["all4one-agent"]
