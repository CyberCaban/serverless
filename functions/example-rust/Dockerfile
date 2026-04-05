FROM golang:1.22-alpine AS builder
RUN apk add --no-cache musl-dev rust cargo
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM alpine:3.20
RUN apk add --no-cache ca-certificates curl
WORKDIR /app
COPY --from=builder /app/target/release/example-rust ./app
EXPOSE 3000
CMD ["./app"]
