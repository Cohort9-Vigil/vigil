FROM rust:alpine AS builder

WORKDIR /app
RUN apk add --no-cache musl-dev

COPY services/Cargo.toml services/Cargo.lock* ./
COPY services/common ./common
COPY services/indexer ./indexer
COPY services/risk-engine ./risk-engine
COPY services/liquidator ./liquidator

RUN cargo build --release -p indexer

FROM alpine:3.20

RUN apk add --no-cache ca-certificates
WORKDIR /app

COPY --from=builder /app/target/release/indexer /usr/local/bin/indexer

ENTRYPOINT ["indexer"]
