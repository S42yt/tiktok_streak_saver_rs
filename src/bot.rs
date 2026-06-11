use anyhow::{bail, Context, Result};
use chrono::Local;
use rand::Rng;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;
use thirtyfour::prelude::*;
use tokio::sync::mpsc;

use crate::config::Config;

// ── TikTok XPath selectors ──

const MSG_LIST_XPATH: &str = "//*[@data-e2e='dm-new-conversation-list']";
const CONV_ITEM_XPATH: &str = "//*[@data-e2e='dm-new-conversation-item']";
const NICK_CLASS: &str = "PInfoNickname";
const CLICK_TARGET_XPATH: &str =
    r#"//*[@id="main-content-messages"]/div/div[3]/div[4]/div"#;
const WRITE_TARGET_XPATH: &str = r#"//*[@id="main-content-messages"]/div/div[3]/div[4]/div/div[1]/div/div[2]/div[2]/div/div/div/div"#;
const TOAST_XPATH: &str = "//li[@data-sonner-toast]";
const PASSKEY_XPATH: &str =
    "//div[starts-with(@id, 'floating-ui-')]/div/div[2]/button[1]";

const MAX_RETRIES: u32 = 3;
const RETRY_DELAY: Duration = Duration::from_secs(12);

// ── Log channel types ──

#[derive(Debug, Clone)]
pub enum LogEntry {
    Info(String),
    Warn(String),
    Error(String),
    Success(String),
    UserStatus { user: String, status: UserStatus },
    BotState(BotState),
}

#[derive(Debug, Clone, PartialEq)]
pub enum UserStatus {
    Pending,
    Sending,
    Sent,
    Failed,
    Retrying(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum BotState {
    Idle,
    Starting,
    LoadingCookies,
    Navigating,
    SendingMessages,
    Done,
    Error,
}

pub type LogTx = mpsc::UnboundedSender<LogEntry>;

fn emit(tx: &LogTx, e: LogEntry) {
    let _ = tx.send(e);
}
fn info(tx: &LogTx, m: impl Into<String>) {
    emit(tx, LogEntry::Info(m.into()));
}
fn warn(tx: &LogTx, m: impl Into<String>) {
    emit(tx, LogEntry::Warn(m.into()));
}
fn error(tx: &LogTx, m: impl Into<String>) {
    emit(tx, LogEntry::Error(m.into()));
}
fn success(tx: &LogTx, m: impl Into<String>) {
    emit(tx, LogEntry::Success(m.into()));
}

fn file_log(path: &str, msg: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{} {}", Local::now().format("%Y-%m-%d %H:%M:%S"), msg);
    }
}

fn rand_delay(lo: f64, hi: f64) -> Duration {
    Duration::from_secs_f64(rand::thread_rng().gen_range(lo..hi))
}

// ═══════════════════════════════════════════════
//  ChromeDriver management
// ═══════════════════════════════════════════════

struct DriverHandle {
    _child: Child,
    port: u16,
}

impl Drop for DriverHandle {
    fn drop(&mut self) {
        let _ = self._child.kill();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Find chromedriver binary. Checks well-known locations before falling back
/// to bare PATH lookup, so a user-local copy beats a stale system-wide one.
fn find_chromedriver() -> String {
    let candidates: Vec<std::path::PathBuf> = {
        let mut v = Vec::new();
        // User-local installs
        if let Ok(home) = std::env::var("USERPROFILE")
            .or_else(|_| std::env::var("HOME"))
        {
            v.push(std::path::PathBuf::from(&home).join(".local/bin/chromedriver.exe"));
            v.push(std::path::PathBuf::from(&home).join(".local/bin/chromedriver"));
        }
        // Common system locations (Linux)
        v.push("/usr/bin/chromedriver".into());
        v.push("/usr/local/bin/chromedriver".into());
        v
    };
    for p in &candidates {
        if p.exists() {
            return p.to_string_lossy().into_owned();
        }
    }
    // Fall back to whatever PATH resolves
    "chromedriver".into()
}

fn spawn_chromedriver() -> Result<DriverHandle> {
    let bin = find_chromedriver();
    let port = free_port();
    let child = Command::new(&bin)
        .arg(format!("--port={}", port))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| {
            format!(
                "Failed to start chromedriver (tried: {bin}).\n\
                 Make sure it is installed and in your PATH.\n\
                 Install:  https://googlechromelabs.github.io/chrome-for-testing/\n\
                 Or:       apt install chromium-driver   (Linux)\n\
                 Or:       choco install chromedriver     (Windows)"
            )
        })?;

    // Poll until chromedriver is ready (up to 10 s)
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if std::net::TcpStream::connect(format!("127.0.0.1:{}", port)).is_ok() {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    // One extra beat to let it finish init
    std::thread::sleep(Duration::from_millis(500));

    Ok(DriverHandle {
        _child: child,
        port,
    })
}

async fn build_driver(cfg: &Config, port: u16) -> Result<WebDriver> {
    let mut caps = DesiredCapabilities::chrome();

    // Session directory (isolated profile, cleaned up after each run)
    let session_dir = std::env::current_dir()
        .unwrap_or_default()
        .join("session_data");
    caps.add_arg(&format!("--user-data-dir={}", session_dir.display()))?;

    // Headless
    if cfg.browser.headless || cfg!(target_os = "linux") {
        caps.add_arg("--headless=new")?;
    }

    // Anti-detection
    caps.add_arg(&format!("user-agent={}", cfg.browser.user_agent))?;
    caps.add_arg("--disable-blink-features=AutomationControlled")?;

    // Suppress Chrome console noise (DevTools, USB, GPU, etc.)
    caps.add_arg("--log-level=3")?;
    caps.add_arg("--silent")?;
    caps.add_arg("--disable-logging")?;
    caps.add_exclude_switch("enable-logging")?;
    caps.add_exclude_switch("enable-automation")?;

    // Container / VPS flags
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--disable-gpu")?;
    caps.add_arg("--disable-software-rasterizer")?;

    // RAM optimisation — single-process and no-zygote are Linux-only
    // (they crash Chrome on Windows)
    if cfg!(target_os = "linux") {
        caps.add_arg("--single-process")?;
        caps.add_arg("--no-zygote")?;
    }
    caps.add_arg("--renderer-process-limit=1")?;
    caps.add_arg("--disable-extensions")?;
    caps.add_arg("--disable-plugins")?;
    caps.add_arg("--disable-images")?;
    caps.add_arg("--blink-settings=imagesEnabled=false")?;
    caps.add_arg("--disable-background-networking")?;
    caps.add_arg("--disable-default-apps")?;
    caps.add_arg("--disable-sync")?;
    caps.add_arg("--disable-translate")?;
    caps.add_arg("--mute-audio")?;
    caps.add_arg("--no-first-run")?;
    caps.add_arg("--window-size=1280,800")?;

    let url = format!("http://localhost:{}", port);
    let driver = WebDriver::new(&url, caps).await.map_err(|e| {
        anyhow::anyhow!(
            "Could not create WebDriver session: {e}\n\
             \n\
             Common causes:\n\
             - Chrome/ChromeDriver version mismatch (must share the same major version)\n\
             - Chrome is not installed or not in the default location\n\
             \n\
             Your ChromeDriver is at: chromedriver --version"
        )
    })?;

    driver.set_page_load_timeout(Duration::from_secs(60)).await?;
    driver.set_script_timeout(Duration::from_secs(30)).await?;
    Ok(driver)
}

fn kill_browsers() {
    if cfg!(target_os = "windows") {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "chromedriver.exe", "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", "chrome.exe", "/T"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    } else {
        for name in &["chromedriver", "chromium", "chrome"] {
            let _ = Command::new("pkill")
                .args(["-f", name])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
}

// ═══════════════════════════════════════════════
//  Cookie loading  (auto-detects JSON vs Netscape format)
// ═══════════════════════════════════════════════

/// Netscape cookie-jar format (curl / wget / browser "Export Cookies" addons):
///   domain  include_subdomains  path  secure  expiry  name  value
/// Fields are separated by TABs.  Lines starting with # are comments.
fn parse_netscape(raw: &str) -> Vec<serde_json::Value> {
    raw.lines()
        .filter(|l| {
            let l = l.trim();
            !l.is_empty() && !l.starts_with('#')
        })
        .filter_map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            if cols.len() < 7 {
                return None;
            }
            let domain = cols[0].trim();
            let _include_sub = cols[1].trim(); // not used by WebDriver
            let path = cols[2].trim();
            let secure = cols[3].trim().eq_ignore_ascii_case("TRUE");
            let expiry: i64 = cols[4].trim().parse().unwrap_or(0);
            let name = cols[5].trim();
            let value = cols[6].trim();

            if name.is_empty() {
                return None;
            }

            let mut obj = serde_json::json!({
                "name": name,
                "value": value,
                "domain": domain,
                "path": path,
                "secure": secure,
            });
            if expiry > 0 {
                obj["expiry"] = serde_json::json!(expiry);
            }
            if secure {
                obj["sameSite"] = serde_json::json!("None");
            }
            Some(obj)
        })
        .collect()
}

/// JSON cookie format (Cookie-Editor browser extension):
///   [ { "name": "...", "value": "...", "domain": "...", ... }, ... ]
fn parse_json(raw: &str) -> Result<Vec<serde_json::Value>> {
    let cookies: Vec<serde_json::Value> =
        serde_json::from_str(raw).context("Cookie file is not valid JSON")?;

    Ok(cookies
        .into_iter()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?;
            let value = c.get("value")?.as_str()?;
            if name.is_empty() {
                return None;
            }

            let domain = c
                .get("domain")
                .and_then(|v| v.as_str())
                .unwrap_or(".tiktok.com");
            let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(false);

            let mut obj = serde_json::json!({
                "name": name,
                "value": value,
                "domain": domain,
                "path": path,
                "secure": secure,
            });

            // sameSite
            match c.get("sameSite").and_then(|v| v.as_str()) {
                Some(s) if s.eq_ignore_ascii_case("lax") => {
                    obj["sameSite"] = serde_json::json!("Lax");
                }
                Some(s) if s.eq_ignore_ascii_case("strict") => {
                    obj["sameSite"] = serde_json::json!("Strict");
                }
                _ if secure => {
                    obj["sameSite"] = serde_json::json!("None");
                }
                _ => {}
            }

            // expiry (Cookie-Editor uses "expirationDate")
            if let Some(exp) = c
                .get("expirationDate")
                .or_else(|| c.get("expiry"))
                .and_then(|v| v.as_f64())
            {
                obj["expiry"] = serde_json::json!(exp as i64);
            }

            Some(obj)
        })
        .collect())
}

async fn load_cookies(
    driver: &WebDriver,
    cookie_file: &str,
    tx: &LogTx,
) -> Result<bool> {
    info(tx, format!("Loading cookies from '{cookie_file}'..."));

    let raw = std::fs::read_to_string(cookie_file)
        .context("Could not read cookie file")?;

    // Auto-detect format: if it parses as JSON array, use JSON; otherwise Netscape
    let cookies = match parse_json(&raw) {
        Ok(c) if !c.is_empty() => {
            info(tx, format!("Detected JSON format — {} cookies", c.len()));
            c
        }
        _ => {
            let c = parse_netscape(&raw);
            if c.is_empty() {
                error(tx, "Could not parse cookie file as JSON or Netscape format");
                return Ok(false);
            }
            info(tx, format!("Detected Netscape format — {} cookies", c.len()));
            c
        }
    };

    // Navigate to TikTok so the domain is set for cookie injection
    driver.goto("https://www.tiktok.com/explore").await?;
    info(tx, "Navigated to tiktok.com, injecting cookies...");
    tokio::time::sleep(rand_delay(3.0, 5.0)).await;

    let (mut ok, mut fail) = (0u32, 0u32);

    for (i, cookie_json) in cookies.iter().enumerate() {
        let name = cookie_json["name"].as_str().unwrap_or("?");

        let cookie_obj = match serde_json::from_value::<Cookie>(cookie_json.clone()) {
            Ok(co) => co,
            Err(_) => {
                fail += 1;
                continue;
            }
        };

        match driver.add_cookie(cookie_obj).await {
            Ok(_) => ok += 1,
            Err(e) => {
                fail += 1;
                if fail <= 5 {
                    warn(tx, format!("Cookie #{} ('{name}') failed: {e}", i + 1));
                }
            }
        }
    }

    if fail > 0 {
        warn(tx, format!("{fail} cookie(s) failed to load"));
    }
    if ok > 0 {
        success(tx, format!("Loaded {ok} cookies"));
        Ok(true)
    } else {
        error(tx, "No cookies were loaded!");
        Ok(false)
    }
}

// ═══════════════════════════════════════════════
//  Iframe helpers
// ═══════════════════════════════════════════════

async fn iframe_with_element(
    driver: &WebDriver,
    xpath: &str,
    tx: &LogTx,
) -> Result<bool> {
    driver.enter_default_frame().await?;

    let iframes = match driver.find_all(By::Tag("iframe")).await {
        Ok(f) if !f.is_empty() => f,
        _ => return Ok(false),
    };
    info(tx, format!("Scanning {} iframe(s)...", iframes.len()));

    for (idx, _frame) in iframes.iter().enumerate() {
        let _ = driver.enter_default_frame().await;
        if driver.enter_frame(idx as u16).await.is_err() {
            continue;
        }
        if let Ok(els) = driver.find_all(By::XPath(xpath)).await {
            if !els.is_empty() {
                info(tx, format!("Target found in iframe #{idx}"));
                return Ok(true);
            }
        }
    }

    driver.enter_default_frame().await?;
    Ok(false)
}

async fn wait_for(
    driver: &WebDriver,
    xpath: &str,
    timeout: Duration,
    tx: &LogTx,
) -> Result<bool> {
    // Try main document first (fast path)
    if driver
        .query(By::XPath(xpath))
        .wait(Duration::from_secs(5), Duration::from_millis(500))
        .first()
        .await
        .is_ok()
    {
        info(tx, "Element found in main document");
        return Ok(true);
    }

    info(tx, "Not in main document — checking iframes...");
    if iframe_with_element(driver, xpath, tx).await? {
        if driver
            .query(By::XPath(xpath))
            .wait(timeout, Duration::from_millis(500))
            .first()
            .await
            .is_ok()
        {
            info(tx, "Element confirmed inside iframe");
            return Ok(true);
        }
    }

    error(tx, "Timed out waiting for element (main + iframes)");
    Ok(false)
}

// ═══════════════════════════════════════════════
//  Conversation & messaging
// ═══════════════════════════════════════════════

async fn find_conversation(
    driver: &WebDriver,
    user: &str,
    tx: &LogTx,
) -> Result<bool> {
    info(tx, format!("Looking for '{user}'..."));

    if driver
        .query(By::XPath(CONV_ITEM_XPATH))
        .wait(Duration::from_secs(30), Duration::from_millis(500))
        .first()
        .await
        .is_err()
    {
        error(tx, "Timeout waiting for conversation list");
        return Ok(false);
    }
    tokio::time::sleep(rand_delay(2.0, 4.0)).await;

    let items = driver.find_all(By::XPath(CONV_ITEM_XPATH)).await?;
    info(tx, format!("Found {} conversations", items.len()));

    let nick_xpath = format!(".//p[contains(@class, '{NICK_CLASS}')]");

    for (i, item) in items.iter().enumerate() {
        let nick_el = match item.find(By::XPath(&nick_xpath)).await {
            Ok(el) => el,
            Err(_) => continue,
        };
        let nick = nick_el.text().await.unwrap_or_default();
        if !nick.trim().eq_ignore_ascii_case(user) {
            continue;
        }

        info(tx, format!("Matched '{user}' at item #{}", i + 1));
        let _ = driver
            .execute("arguments[0].scrollIntoView(true);", vec![item.to_json()?])
            .await;
        tokio::time::sleep(Duration::from_millis(500)).await;
        item.click().await?;
        tokio::time::sleep(rand_delay(3.0, 5.0)).await;
        return Ok(true);
    }

    warn(tx, format!("'{user}' not found in list"));
    Ok(false)
}

async fn send_message(
    driver: &WebDriver,
    message: &str,
    tx: &LogTx,
) -> Result<bool> {
    info(tx, "Sending message...");

    // Wait for any toast notification to disappear
    let _ = tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            if driver.find(By::XPath(TOAST_XPATH)).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    })
    .await;

    // Click the chat area to focus it
    let click_el = match driver
        .query(By::XPath(CLICK_TARGET_XPATH))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
    {
        Ok(el) => el,
        Err(_) => {
            error(tx, "Click target not found");
            return Ok(false);
        }
    };

    let _ = driver
        .execute(
            "arguments[0].scrollIntoView(true);",
            vec![click_el.to_json()?],
        )
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    if click_el.click().await.is_err() {
        warn(tx, "Normal click failed, trying JS click...");
        if driver
            .execute("arguments[0].click();", vec![click_el.to_json()?])
            .await
            .is_err()
        {
            error(tx, "JS click also failed");
            return Ok(false);
        }
    }
    tokio::time::sleep(rand_delay(1.5, 2.5)).await;

    // Locate the write target (input area)
    let write_el = match driver
        .query(By::XPath(WRITE_TARGET_XPATH))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
    {
        Ok(el) => el,
        Err(_) => {
            error(tx, "Write target not found");
            return Ok(false);
        }
    };

    let _ = driver
        .execute("arguments[0].focus();", vec![write_el.to_json()?])
        .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Type the message and press Enter
    match write_el.send_keys(message).await {
        Ok(_) => {
            tokio::time::sleep(rand_delay(0.8, 1.5)).await;
            write_el.send_keys(Key::Enter).await?;
            info(tx, "Keystrokes sent, verifying...");
        }
        Err(_) => {
            warn(tx, "send_keys failed, injecting via JS...");
            driver
                .execute(
                    "arguments[0].textContent = arguments[1];",
                    vec![write_el.to_json()?, serde_json::json!(message)],
                )
                .await?;
            tokio::time::sleep(rand_delay(0.5, 1.0)).await;
            write_el.send_keys(Key::Enter).await?;
            info(tx, "JS-injected message sent, verifying...");
        }
    }

    tokio::time::sleep(rand_delay(2.0, 4.0)).await;
    verify_sent(driver, tx).await
}

async fn verify_sent(driver: &WebDriver, tx: &LogTx) -> Result<bool> {
    let el = match driver
        .query(By::XPath(WRITE_TARGET_XPATH))
        .wait(Duration::from_secs(6), Duration::from_millis(500))
        .first()
        .await
    {
        Ok(e) => e,
        Err(_) => {
            warn(tx, "Cannot locate input after send — verification failed");
            return Ok(false);
        }
    };

    let text = el.text().await.unwrap_or_default();
    let content = if text.trim().is_empty() {
        el.attr("textContent")
            .await
            .unwrap_or(None)
            .unwrap_or_default()
    } else {
        text
    };

    if content.trim().is_empty() {
        success(tx, "Verified: input cleared — message sent");
        Ok(true)
    } else {
        warn(
            tx,
            format!(
                "Verification FAILED: field still has '{}'",
                content.trim()
            ),
        );
        Ok(false)
    }
}

async fn dismiss_passkey(driver: &WebDriver, tx: &LogTx) {
    info(tx, "Checking for passkey popup...");
    match driver
        .query(By::XPath(PASSKEY_XPATH))
        .wait(Duration::from_secs(15), Duration::from_millis(500))
        .first()
        .await
    {
        Ok(btn) => {
            info(tx, "Passkey popup found — dismissing");
            let _ = btn.click().await;
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
        Err(_) => {
            info(tx, "No passkey popup");
        }
    }
}

// ═══════════════════════════════════════════════
//  Main bot run
// ═══════════════════════════════════════════════

pub async fn run_bot(cfg: &Config, tx: &LogTx) -> Result<u32> {
    let log_file = &cfg.general.log_file;

    emit(tx, LogEntry::BotState(BotState::Starting));
    info(tx, "Starting bot run...");
    file_log(log_file, "--- Run started ---");

    kill_browsers();
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Clean old session directory
    let session_dir = std::env::current_dir()?.join("session_data");
    if session_dir.exists() {
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    let handle = spawn_chromedriver()?;
    info(tx, format!("ChromeDriver on port {}", handle.port));

    // Wrap the driver work so cleanup always runs
    let result = async {
        let driver = build_driver(cfg, handle.port).await?;
        info(tx, "Browser session ready");

        // ── Auth ──
        emit(tx, LogEntry::BotState(BotState::LoadingCookies));
        match cfg.auth.method.as_str() {
            "cookies" => {
                if !load_cookies(&driver, &cfg.auth.cookies_file, tx).await? {
                    bail!("Failed to load cookies");
                }
            }
            "browser" => {
                if !Path::new(&cfg.auth.cookies_file).exists() {
                    bail!(
                        "No saved cookies. Run `tiktok-streak-saver auth` first \
                         to log in via the browser."
                    );
                }
                if !load_cookies(&driver, &cfg.auth.cookies_file, tx).await? {
                    bail!("Saved cookies are invalid — re-run the auth command.");
                }
            }
            other => bail!("Unknown auth method: {other}"),
        }

        // ── Navigate ──
        emit(tx, LogEntry::BotState(BotState::Navigating));
        info(tx, format!("Opening {}", cfg.browser.tiktok_url));
        driver.goto(&cfg.browser.tiktok_url).await?;

        dismiss_passkey(&driver, tx).await;

        info(tx, "Waiting for message list...");
        if wait_for(&driver, MSG_LIST_XPATH, Duration::from_secs(35), tx).await? {
            info(tx, "Message list loaded");
        } else {
            warn(tx, "Message list not found — continuing anyway");
        }
        tokio::time::sleep(rand_delay(3.0, 6.0)).await;

        // ── Send to each target ──
        if cfg.targets.users.is_empty() {
            error(tx, "No target users configured!");
            let _ = driver.quit().await;
            return Ok(0u32);
        }

        emit(tx, LogEntry::BotState(BotState::SendingMessages));
        let total = cfg.targets.users.len();
        info(
            tx,
            format!("Targeting {} user(s): {}", total, cfg.targets.users.join(", ")),
        );

        let mut sent_count = 0u32;

        for (idx, user) in cfg.targets.users.iter().enumerate() {
            let safe: String = user.chars().filter(|c| !c.is_control()).collect();
            let mut sent = false;

            emit(
                tx,
                LogEntry::UserStatus {
                    user: safe.clone(),
                    status: UserStatus::Sending,
                },
            );

            for attempt in 1..=MAX_RETRIES {
                info(
                    tx,
                    format!("'{safe}' attempt {attempt}/{MAX_RETRIES}"),
                );

                if attempt > 1 {
                    emit(
                        tx,
                        LogEntry::UserStatus {
                            user: safe.clone(),
                            status: UserStatus::Retrying(attempt),
                        },
                    );
                    let _ = driver.goto(&cfg.browser.tiktok_url).await;
                    let _ = wait_for(
                        &driver,
                        MSG_LIST_XPATH,
                        Duration::from_secs(35),
                        tx,
                    )
                    .await;
                    tokio::time::sleep(rand_delay(3.0, 5.0)).await;
                }

                if !find_conversation(&driver, user, tx).await? {
                    warn(tx, format!("Conversation not found for '{safe}'"));
                    if attempt < MAX_RETRIES {
                        info(tx, format!("Retrying in {}s...", RETRY_DELAY.as_secs()));
                        tokio::time::sleep(RETRY_DELAY).await;
                    }
                    continue;
                }

                if send_message(&driver, &cfg.general.message, tx).await? {
                    sent = true;
                    break;
                }

                warn(
                    tx,
                    format!("Send failed for '{safe}' on attempt {attempt}"),
                );
                if attempt < MAX_RETRIES {
                    info(tx, format!("Retrying in {}s...", RETRY_DELAY.as_secs()));
                    tokio::time::sleep(RETRY_DELAY).await;
                }
            }

            if sent {
                sent_count += 1;
                success(tx, format!("Message confirmed to '{safe}'"));
                emit(
                    tx,
                    LogEntry::UserStatus {
                        user: safe.clone(),
                        status: UserStatus::Sent,
                    },
                );
                file_log(log_file, &format!("SENT to '{safe}'"));
            } else {
                error(
                    tx,
                    format!("All {MAX_RETRIES} attempts failed for '{safe}'"),
                );
                emit(
                    tx,
                    LogEntry::UserStatus {
                        user: safe.clone(),
                        status: UserStatus::Failed,
                    },
                );
                file_log(log_file, &format!("FAILED for '{safe}'"));
            }

            if total > 1 && idx < total - 1 {
                let d = rand_delay(5.0, 10.0);
                info(tx, format!("Waiting {:.1}s...", d.as_secs_f64()));
                tokio::time::sleep(d).await;
            }
        }

        info(tx, format!("Done: {sent_count}/{total} messages sent"));
        file_log(
            log_file,
            &format!("Run complete: {sent_count}/{total} sent"),
        );

        let _ = driver.quit().await;
        Ok(sent_count)
    }
    .await;

    // Always clean up
    drop(handle);
    kill_browsers();
    let session_dir = std::env::current_dir()?.join("session_data");
    if session_dir.exists() {
        let _ = std::fs::remove_dir_all(&session_dir);
    }

    emit(
        tx,
        LogEntry::BotState(if result.is_ok() {
            BotState::Done
        } else {
            BotState::Error
        }),
    );
    result
}

// ═══════════════════════════════════════════════
//  Browser-based auth (interactive login)
// ═══════════════════════════════════════════════

pub async fn browser_auth(config_path: &Path) -> Result<()> {
    use console::style;

    let cfg = crate::config::load_or_create(config_path)?;

    println!();
    println!(
        "{}",
        style("  TikTok Browser Authentication  ").bold().cyan()
    );
    println!();
    println!("  A Chrome window will open to the TikTok login page.");
    println!("  Log in normally, then come back here and press Enter.");
    println!("  Your session cookies will be saved for future runs.");
    println!();

    kill_browsers();
    tokio::time::sleep(Duration::from_secs(1)).await;

    let handle = spawn_chromedriver()?;

    // Non-headless driver so the user can interact
    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg(&format!("user-agent={}", cfg.browser.user_agent))?;
    caps.add_arg("--disable-blink-features=AutomationControlled")?;
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--window-size=1280,900")?;

    let url = format!("http://localhost:{}", handle.port);
    let driver = WebDriver::new(&url, caps).await?;

    driver
        .goto("https://www.tiktok.com/login")
        .await
        .context("Could not navigate to TikTok login")?;

    println!(
        "  {} Browser opened — log in to TikTok now.",
        style("→").cyan().bold()
    );
    println!();
    println!("  Press Enter after you are logged in...");
    let _ = std::io::stdin().read_line(&mut String::new());

    // Grab all cookies from the browser session
    let cookies = driver.get_all_cookies().await?;

    let cookie_json: Vec<serde_json::Value> = cookies
        .iter()
        .map(|c| {
            // thirtyfour Cookie has public fields, not getter methods
            let mut obj = serde_json::json!({
                "name": &c.name,
                "value": &c.value,
            });
            if let Some(d) = &c.domain {
                obj["domain"] = serde_json::json!(d);
            }
            if let Some(p) = &c.path {
                obj["path"] = serde_json::json!(p);
            }
            if let Some(s) = c.secure {
                obj["secure"] = serde_json::json!(s);
            }
            if let Some(e) = c.expiry {
                obj["expirationDate"] = serde_json::json!(e);
            }
            obj
        })
        .collect();

    let path = &cfg.auth.cookies_file;
    std::fs::write(path, serde_json::to_string_pretty(&cookie_json)?)
        .context("Could not write cookies file")?;

    println!();
    println!(
        "  {} {} cookies saved to '{}'",
        style("✓").green().bold(),
        cookie_json.len(),
        path
    );

    let _ = driver.quit().await;
    drop(handle);
    kill_browsers();

    Ok(())
}

// ═══════════════════════════════════════════════
//  Headless one-shot (for Docker / cron)
// ═══════════════════════════════════════════════

pub async fn run_once(cfg: &Config) -> Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel();

    let printer = tokio::spawn(async move {
        while let Some(entry) = rx.recv().await {
            let now = Local::now().format("%H:%M:%S");
            match entry {
                LogEntry::Info(m) => println!("{now}  [INFO]  {m}"),
                LogEntry::Warn(m) => println!("{now}  [WARN]  {m}"),
                LogEntry::Error(m) => eprintln!("{now}  [ERR]   {m}"),
                LogEntry::Success(m) => println!("{now}  [ OK ]  {m}"),
                LogEntry::UserStatus { user, status } => {
                    println!("{now}  [USER]  {user} → {status:?}");
                }
                LogEntry::BotState(s) => {
                    println!("{now}  [STATE] {s:?}");
                }
            }
        }
    });

    let result = run_bot(cfg, &tx).await;
    drop(tx);
    let _ = printer.await;

    match result {
        Ok(n) => println!("\nFinished — {n} message(s) sent."),
        Err(e) => eprintln!("\nBot error: {e}"),
    }
    Ok(())
}
