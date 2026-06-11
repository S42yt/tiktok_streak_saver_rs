use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

// ── Config Types ──

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub schedule: Schedule,
    #[serde(default)]
    pub auth: Auth,
    #[serde(default)]
    pub targets: Targets,
    #[serde(default)]
    pub browser: Browser,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct General {
    #[serde(default)]
    pub test_mode: bool,
    #[serde(default = "default_message")]
    pub message: String,
    #[serde(default = "default_log_file")]
    pub log_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schedule {
    #[serde(default)]
    pub hour: u32,
    #[serde(default = "default_minute")]
    pub minute: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Auth {
    /// "cookies" to load from a JSON file, "browser" to log in interactively
    #[serde(default = "default_auth_method")]
    pub method: String,
    #[serde(default = "default_cookies_file")]
    pub cookies_file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Targets {
    #[serde(default)]
    pub users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Browser {
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_user_agent")]
    pub user_agent: String,
    #[serde(default = "default_tiktok_url")]
    pub tiktok_url: String,
}

// ── Defaults ──

fn default_message() -> String {
    ".".into()
}
fn default_log_file() -> String {
    "tiktok_bot.log".into()
}
fn default_minute() -> u32 {
    2
}
fn default_auth_method() -> String {
    "cookies".into()
}
fn default_cookies_file() -> String {
    "cookies.json".into()
}
fn default_true() -> bool {
    true
}
fn default_user_agent() -> String {
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
     (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
        .into()
}
fn default_tiktok_url() -> String {
    "https://www.tiktok.com/messages?lang=en".into()
}

impl Default for General {
    fn default() -> Self {
        Self {
            test_mode: false,
            message: default_message(),
            log_file: default_log_file(),
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self {
            hour: 0,
            minute: default_minute(),
        }
    }
}

impl Default for Auth {
    fn default() -> Self {
        Self {
            method: default_auth_method(),
            cookies_file: default_cookies_file(),
        }
    }
}

impl Default for Targets {
    fn default() -> Self {
        Self { users: vec![] }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self {
            headless: default_true(),
            user_agent: default_user_agent(),
            tiktok_url: default_tiktok_url(),
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            general: General::default(),
            schedule: Schedule::default(),
            auth: Auth::default(),
            targets: Targets::default(),
            browser: Browser::default(),
        }
    }
}

// ── Load / Create ──

pub fn load_or_create(path: &Path) -> Result<Config> {
    if path.exists() {
        let content =
            std::fs::read_to_string(path).context("Failed to read config file")?;
        let config: Config =
            toml::from_str(&content).context("Failed to parse config TOML")?;
        Ok(config)
    } else {
        let config = Config::default();
        let content = toml::to_string_pretty(&config)?;
        std::fs::write(path, &content)
            .context("Failed to write default config")?;
        println!(
            "No config found — created default at '{}'.\n\
             Edit it or run `tiktok-streak-saver setup` to configure interactively.",
            path.display()
        );
        Ok(config)
    }
}

// ── Interactive Setup Wizard ──

pub fn setup_wizard(path: &Path) -> Result<()> {
    use console::style;
    use dialoguer::{Confirm, Input, Select};

    println!();
    println!(
        "{}",
        style("  TikTok Streak Saver — Setup Wizard  ")
            .bold()
            .cyan()
    );
    println!();

    let mut config = if path.exists() {
        load_or_create(path)?
    } else {
        Config::default()
    };

    // ── Auth ──
    let auth_opts = &["cookies  (import from browser extension)", "browser  (log in via Chrome window)"];
    let auth_idx = Select::new()
        .with_prompt("Authentication method")
        .items(auth_opts)
        .default(if config.auth.method == "browser" { 1 } else { 0 })
        .interact()?;
    config.auth.method = if auth_idx == 0 {
        "cookies".into()
    } else {
        "browser".into()
    };

    if config.auth.method == "cookies" {
        config.auth.cookies_file = Input::new()
            .with_prompt("  Cookies file path")
            .default(config.auth.cookies_file)
            .interact_text()?;
    }

    // ── Targets ──
    let users_str: String = Input::new()
        .with_prompt("Target usernames (comma-separated)")
        .default(config.targets.users.join(", "))
        .interact_text()?;
    config.targets.users = users_str
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    // ── Message ──
    config.general.message = Input::new()
        .with_prompt("Message to send")
        .default(config.general.message)
        .interact_text()?;

    // ── Schedule ──
    config.schedule.hour = Input::new()
        .with_prompt("Send hour   (0-23)")
        .default(config.schedule.hour)
        .interact_text()?;
    config.schedule.minute = Input::new()
        .with_prompt("Send minute (0-59)")
        .default(config.schedule.minute)
        .interact_text()?;

    // ── Flags ──
    config.general.test_mode = Confirm::new()
        .with_prompt("Enable test mode? (runs immediately, ignores schedule)")
        .default(config.general.test_mode)
        .interact()?;

    config.browser.headless = Confirm::new()
        .with_prompt("Run browser in headless mode?")
        .default(config.browser.headless)
        .interact()?;

    // ── Save ──
    let content = toml::to_string_pretty(&config)?;
    std::fs::write(path, &content).context("Failed to write config")?;

    println!();
    println!(
        " {} Config saved to {}",
        style("✓").green().bold(),
        style(path.display()).underlined()
    );
    println!();
    Ok(())
}
