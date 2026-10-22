FROM rust:alpine AS builder

WORKDIR /app
RUN apk add --no-cache musl-dev

COPY Cargo.toml Cargo.lock* ./
COPY common ./common
COPY indexer ./indexer
COPY risk-engine ./risk-engine
COPY liquidator ./liquidator

RUN cargo build --release -p indexer

FROM alpine:3.20

RUN apk add --no-cache ca-certificates
WORKDIR /app

COPY --from=builder /app/target/release/indexer /usr/local/bin/indexer

ENTRYPOINT ["indexer"]
