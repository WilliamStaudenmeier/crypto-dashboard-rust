# Crypto Dashboard Rust

A lightweight cryptocurrency market dashboard built with Rust.

## Overview

- Rust HTTP server using Axum
- JSON handling with serde_json
- Server-side CoinGecko API integration
- Lightweight static dashboard UI served by the Rust app
- Render free-tier deployment via Docker

## Project Structure

- src/main.rs: Rust server and API proxy routes
- static/index.html: Dashboard layout
- static/styles.css: Styling
- static/app.js: Client-side rendering logic
- Cargo.toml: Rust dependencies and package config
- Dockerfile: Containerized deploy/runtime
- render.yaml: Render Blueprint

## Local Development

### 1. Configure environment

```bash
cp .env.example .env
```

Optional variables:

- COINGECKO_BASE_URL (default: https://api.coingecko.com/api/v3)
- COINGECKO_API_KEY (optional)
- PORT (default: 8080)
- FRONTEND_ORIGIN (default: *)
- SNAPSHOT_PATH (default: ./db.json)
- STATIC_DIR (default: ./static)

### 2. Build and run

```bash
cargo run
```

Open http://localhost:8080

## API Endpoints

- GET /health
- GET /api/global
- GET /api/bootstrap
- GET /api/trending
- GET /api/markets?vs_currency=usd&per_page=20&page=1
- GET /api/history?coin_id=bitcoin&days=365&vs_currency=usd
