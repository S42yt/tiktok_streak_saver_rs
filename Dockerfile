# ── Stage 1: build ──
FROM rust:1.82-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN cargo build --release

# ── Stage 2: runtime ──
FROM debian:bookworm-slim

# Chrome + chromedriver
RUN apt-get update && apt-get install -y --no-install-recommends \
        wget gnupg curl ca-certificates fonts-liberation \
        libasound2 libatk-bridge2.0-0 libatk1.0-0 libcups2 \
        libdbus-1-3 libdrm2 libgbm1 libgtk-3-0 libnspr4 \
        libnss3 libxcomposite1 libxdamage1 libxrandr2 xdg-utils \
    && wget -q -O - https://dl.google.com/linux/linux_signing_key.pub \
       | gpg --dearmor -o /usr/share/keyrings/google-chrome.gpg \
    && echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] \
       http://dl.google.com/linux/chrome/deb/ stable main" \
       > /etc/apt/sources.list.d/google-chrome.list \
    && apt-get update \
    && apt-get install -y google-chrome-stable chromium-driver \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/tiktok-streak-saver /usr/local/bin/

RUN mkdir -p /app/data

# Default: headless one-shot mode (schedule handled by the binary)
CMD ["tiktok-streak-saver", "once", "--config", "/app/config.toml"]
