# TikTok Streak Saver

A fast, low-memory bot that sends a daily message to your TikTok contacts so your streaks never die. Written in Rust with a real-time TUI dashboard.

```
╭─ TikTok Streak Saver  v1.0 ─────────────────────╮
│                                                   │
│  ╭─ Status ─────────────────────────────────────╮ │
│  │  State:     ● Sending messages...            │ │
│  │  Next run:  00:02 daily  (in 8h 41m)        │ │
│  │  Auth:      Cookies  (cookies.json)          │ │
│  │  Message:   "."                              │ │
│  ╰──────────────────────────────────────────────╯ │
│  ╭─ Targets ────────────────────────────────────╮ │
│  │  alex                ● Sent                  │ │
│  │  jordan              ◐ Sending...            │ │
│  ╰──────────────────────────────────────────────╯ │
│  ╭─ Activity Log ───────────────────────────────╮ │
│  │  00:02:01  Bot started                       │ │
│  │  00:02:03  Loaded 23 cookies                 │ │
│  │  00:02:14  Message confirmed to 'alex'       │ │
│  ╰──────────────────────────────────────────────╯ │
│                                                   │
│  q Quit   r Run Now   ↑↓ Scroll Log              │
╰───────────────────────────────────────────────────╯
```

## Features

- **TUI dashboard** — live status, per-user progress, scrollable log.
- **Two auth methods** — import cookies from a browser extension *or* log in via a Chrome window.
- **Smart verification** — checks that the input field actually cleared after pressing Enter; retries on silent failures.
- **Iframe bypass** — detects when TikTok hides the DM interface inside iframes and switches context automatically.
- **Low-RAM friendly** — single-process Chrome, images disabled, aggressive flag tuning for 1 GB VPS boxes.
- **Scheduling** — runs once per day at the configured time with a built-in scheduler (no external cron needed).
- **TOML config** — clean, commented `config.toml` instead of JSON.

## Prerequisites

| Dependency | Install |
|---|---|
| **Rust** (build only) | [rustup.rs](https://rustup.rs) |
| **Chrome / Chromium** | Your package manager or [google.com/chrome](https://www.google.com/chrome/) |
| **ChromeDriver** | `apt install chromium-driver` / `choco install chromedriver` / [chromedriver.chromium.org](https://chromedriver.chromium.org/downloads) |

> ChromeDriver's major version must match your Chrome version.

## Quick Start

```bash
# 1. Clone & build
git clone https://github.com/thetrekir/tiktok-streak-bot.git
cd tiktok-streak-bot
cargo build --release

# 2. Configure
cp config.example.toml config.toml   # then edit
#   — or —
./target/release/tiktok-streak-saver setup   # interactive wizard

# 3. Authenticate (choose one)

#  Option A: Cookie import
#   Log in to tiktok.com in your browser, export cookies and save
#   as cookies.txt (or cookies.json). Both formats are auto-detected:
#
#   Netscape format (tabs, from "Get cookies.txt" or curl):
#     .tiktok.com	TRUE	/	FALSE	1791819519	name	value
#
#   JSON format (from Cookie-Editor https://cookie-editor.com/):
#     [{ "name": "...", "value": "...", "domain": "..." }]

#  Option B: Browser login
./target/release/tiktok-streak-saver auth
#   A Chrome window opens → log in → press Enter in the terminal.
#   Cookies are saved automatically.

# 4. Run
./target/release/tiktok-streak-saver          # TUI mode (default)
./target/release/tiktok-streak-saver once     # headless one-shot
```

## Commands

| Command | Description |
|---|---|
| `tiktok-streak-saver` | Start the TUI dashboard (same as `run`) |
| `tiktok-streak-saver run` | Start the TUI dashboard |
| `tiktok-streak-saver setup` | Interactive config wizard |
| `tiktok-streak-saver auth` | Log in to TikTok via browser, save cookies |
| `tiktok-streak-saver once` | Single headless run (Docker / cron / systemd) |

All commands accept `-c <path>` to use a different config file (default: `config.toml`).

## Configuration

Copy `config.example.toml` to `config.toml` and edit it, or run the setup wizard. Here is a minimal example:

```toml
[general]
test_mode = false
message = "."

[schedule]
hour = 0
minute = 2

[auth]
method = "cookies"        # or "browser"
cookies_file = "cookies.json"

[targets]
users = ["alice", "bob"]

[browser]
headless = true
```

See [`config.example.toml`](config.example.toml) for the full reference with comments.

## Deployment

### Docker (recommended for servers)

1. Create a `data/` directory with your `config.toml` and `cookies.json`.
2. Set the `TZ` variable in `docker-compose.yml` to your timezone.
3. Build & run:

```bash
docker compose up -d --build
docker compose logs -f
```

### systemd (Linux)

```ini
# /etc/systemd/system/tiktok-bot.service
[Unit]
Description=TikTok Streak Saver
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/tiktok-bot
ExecStart=/opt/tiktok-bot/tiktok-streak-saver once
Restart=always
RestartSec=60

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now tiktok-bot
```

### Windows background service

1. Build the release binary (`cargo build --release`).
2. Place the `.exe`, `config.toml`, and `cookies.json` in a folder (e.g. `C:\TikTokBot`).
3. Use [NSSM](http://nssm.cc/) to install it as a service:

```cmd
nssm install TikTokStreakBot C:\TikTokBot\tiktok-streak-saver.exe once
sc start TikTokStreakBot
```

## TUI Keybindings

| Key | Action |
|---|---|
| `q` | Quit (when bot is idle) |
| `Ctrl+q` | Force quit |
| `r` | Run the bot now (overrides schedule) |
| `↑` / `k` | Scroll log up |
| `↓` / `j` | Scroll log down |

## Logs

The bot writes to `tiktok_bot.log` (configurable) on every run. If a message fails the log tells you whether the element was not found, the cookie expired, or the send was silently dropped by TikTok.

## Migrating from the Python version

1. Replace `config.json` with `config.toml` (see example above).
2. Your existing `cookies.json` works as-is — no changes needed.
3. The Python `main.py` is no longer used and can be removed.
