FROM rust:1.87-bookworm AS build

WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim AS runtime

RUN apt-get update && apt-get install -y --no-install-recommends \
  ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=build /app/target/release/crypto-dashboard-rust /app/crypto-dashboard-rust
COPY --from=build /app/static /app/static
COPY --from=build /app/db.json /app/db.json

ENV PORT=8080
EXPOSE 8080

CMD ["/app/crypto-dashboard-rust"]
