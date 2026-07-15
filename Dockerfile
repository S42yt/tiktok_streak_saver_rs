# ── Stage 1: build ──
FROM rust:1.88-slim-bookworm AS builder

RUN apt-get update && apt-get install -y pkg-config libssl-dev && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY Cargo.toml Cargo.lock* ./
COPY src/ src/

RUN cargo build --release

# ── Stage 2: runtime ──
FROM debian:bookworm-slim

# Chrome + chromedriver (driver fetched from Chrome for Testing at the exact
# version of the installed Chrome, so the two can never drift apart)
RUN apt-get update && apt-get install -y --no-install-recommends \
        wget gnupg curl ca-certificates unzip fonts-liberation \
        libasound2 libatk-bridge2.0-0 libatk1.0-0 libcups2 \
        libdbus-1-3 libdrm2 libgbm1 libgtk-3-0 libnspr4 \
        libnss3 libxcomposite1 libxdamage1 libxrandr2 xdg-utils \
    && wget -q -O - https://dl.google.com/linux/linux_signing_key.pub \
       | gpg --dearmor -o /usr/share/keyrings/google-chrome.gpg \
    && echo "deb [arch=amd64 signed-by=/usr/share/keyrings/google-chrome.gpg] \
       http://dl.google.com/linux/chrome/deb/ stable main" \
       > /etc/apt/sources.list.d/google-chrome.list \
    && apt-get update \
    && apt-get install -y google-chrome-stable \
    && CHROME_VERSION="$(google-chrome-stable --version | sed -E 's/[^0-9]*([0-9.]+).*/\1/')" \
    && wget -q -O /tmp/chromedriver.zip \
       "https://storage.googleapis.com/chrome-for-testing-public/${CHROME_VERSION}/linux64/chromedriver-linux64.zip" \
    || { CHROME_MAJOR="${CHROME_VERSION%%.*}" \
         && DRIVER_VERSION="$(wget -q -O - "https://googlechromelabs.github.io/chrome-for-testing/LATEST_RELEASE_${CHROME_MAJOR}")" \
         && wget -q -O /tmp/chromedriver.zip \
            "https://storage.googleapis.com/chrome-for-testing-public/${DRIVER_VERSION}/linux64/chromedriver-linux64.zip"; } \
    && unzip -q /tmp/chromedriver.zip -d /tmp \
    && mv /tmp/chromedriver-linux64/chromedriver /usr/local/bin/chromedriver \
    && chmod +x /usr/local/bin/chromedriver \
    && rm -rf /tmp/chromedriver.zip /tmp/chromedriver-linux64 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/target/release/tiktok-streak-saver /usr/local/bin/

RUN mkdir -p /app/data

# Default: headless one-shot mode (schedule handled by the binary)
CMD ["tiktok-streak-saver", "schedule"]
