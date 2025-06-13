FROM rust:1.76 as builder
WORKDIR /usr/src/app
COPY . .
RUN cargo build --release --manifest-path rust/Cargo.toml

FROM debian:buster-slim
WORKDIR /app
COPY --from=builder /usr/src/app/target/release/rust_blockchain .
EXPOSE 8080
ENV BIND_IP=0.0.0.0
ENV BIND_PORT=8080
CMD ["/app/rust_blockchain"]
