#!/bin/bash
# ─────────────────────────────────────────────────────────
# TikTok Streak Bot — VPS Linux Setup Script
# Tested on Ubuntu 20.04 / 22.04 / Debian 11+
# ─────────────────────────────────────────────────────────

set -e

echo "=== [1/5] Updating package list ==="
sudo apt-get update -qq

echo "=== [2/5] Installing Chromium + dependencies ==="
sudo apt-get install -y -qq \
    chromium-browser \
    chromium-chromedriver \
    python3 \
    python3-pip \
    python3-venv \
    fonts-liberation \
    libasound2 \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libcups2 \
    libdbus-1-3 \
    libgdk-pixbuf2.0-0 \
    libnspr4 \
    libnss3 \
    libx11-xcb1 \
    libxcomposite1 \
    libxdamage1 \
    libxrandr2 \
    xdg-utils \
    --no-install-recommends

echo "=== [3/5] Creating Python virtual environment ==="
python3 -m venv venv
source venv/bin/activate

echo "=== [4/5] Installing Python dependencies ==="
pip install --quiet --upgrade pip
pip install --quiet -r requirements.txt

echo "=== [5/5] Done! ==="
echo ""
echo "Next steps:"
echo "  1. Put your cookies.json in this folder"
echo "  2. Edit config.json with your TARGET_USERS"
echo "  3. Test run:  source venv/bin/activate && python main.py"
echo "  4. For background (persistent): use screen or systemd (see README)"
echo ""
