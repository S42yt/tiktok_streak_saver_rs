use crate::bot::{run_bot, BotState, LogEntry, UserStatus};
use crate::config::Config;
use anyhow::Result;
use chrono::{Duration as ChronoDuration, Local, NaiveTime};
use crossterm::{
    event::{Event, EventStream, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{
    prelude::*,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::*,
};
use std::io;
use std::time::Duration;
use tokio::sync::mpsc;

// ── App state ──

struct App {
    config: Config,
    logs: Vec<(String, LogEntry)>,
    log_offset: usize,
    users: Vec<(String, UserStatus)>,
    state: BotState,
    running: bool,
    last_run: Option<String>,
    trigger: bool,
}

impl App {
    fn new(config: &Config) -> Self {
        let users = config
            .targets
            .users
            .iter()
            .map(|u| (u.clone(), UserStatus::Pending))
            .collect();
        Self {
            config: config.clone(),
            logs: Vec::new(),
            log_offset: 0,
            users,
            state: BotState::Idle,
            running: false,
            last_run: None,
            trigger: config.general.test_mode,
        }
    }

    fn push_log(&mut self, entry: LogEntry) {
        let ts = Local::now().format("%H:%M:%S").to_string();
        match &entry {
            LogEntry::UserStatus { user, status } => {
                if let Some(u) = self.users.iter_mut().find(|(n, _)| n == user) {
                    u.1 = status.clone();
                }
            }
            LogEntry::BotState(s) => {
                self.state = s.clone();
                if matches!(s, BotState::Done | BotState::Error) {
                    self.running = false;
                    self.last_run =
                        Some(Local::now().format("%Y-%m-%d %H:%M").to_string());
                }
            }
            _ => {}
        }
        self.logs.push((ts, entry));
        // Keep a reasonable buffer
        if self.logs.len() > 500 {
            self.logs.drain(0..self.logs.len() - 500);
        }
        // Auto-scroll to bottom
        self.log_offset = self.logs.len().saturating_sub(1);
    }

    fn next_run_display(&self) -> String {
        if self.config.general.test_mode {
            return "Test mode (immediate)".into();
        }
        let now = Local::now();
        let target = NaiveTime::from_hms_opt(
            self.config.schedule.hour,
            self.config.schedule.minute,
            0,
        )
        .unwrap_or_default();

        let today = now.date_naive().and_time(target);
        let next = if today > now.naive_local() {
            today
        } else {
            today + ChronoDuration::days(1)
        };
        let diff = next - now.naive_local();
        let h = diff.num_hours();
        let m = diff.num_minutes() % 60;
        format!(
            "{:02}:{:02} daily  (in {}h {:02}m)",
            self.config.schedule.hour, self.config.schedule.minute, h, m
        )
    }

    fn reset_users(&mut self) {
        for u in &mut self.users {
            u.1 = UserStatus::Pending;
        }
    }
}

// ── Drawing ──

fn draw(f: &mut Frame, app: &App) {
    let area = f.area();

    let target_height = (app.users.len() as u16 + 2).max(3);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // title
            Constraint::Length(6),            // status
            Constraint::Length(target_height), // targets
            Constraint::Min(6),              // log
            Constraint::Length(1),            // hotkeys
        ])
        .split(area);

    // ── Title bar ──
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            " TikTok Streak Saver ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(" v1.0 ", Style::default().fg(Color::DarkGray)),
    ]))
    .alignment(Alignment::Center)
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Cyan)),
    );
    f.render_widget(title, rows[0]);

    // ── Status panel ──
    let (state_label, state_color) = match app.state {
        BotState::Idle => ("Waiting", Color::Yellow),
        BotState::Starting => ("Starting...", Color::Blue),
        BotState::LoadingCookies => ("Loading cookies...", Color::Blue),
        BotState::Navigating => ("Navigating...", Color::Blue),
        BotState::SendingMessages => ("Sending messages...", Color::Cyan),
        BotState::Done => ("Completed", Color::Green),
        BotState::Error => ("Error", Color::Red),
    };

    let auth_label = match app.config.auth.method.as_str() {
        "browser" => format!("Browser auth  ({})", app.config.auth.cookies_file),
        _ => format!("Cookies  ({})", app.config.auth.cookies_file),
    };

    let status = Paragraph::new(vec![
        Line::from(vec![
            Span::raw("  State:     "),
            Span::styled(
                format!("● {state_label}"),
                Style::default()
                    .fg(state_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::raw("  Next run:  "),
            Span::styled(app.next_run_display(), Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::raw("  Auth:      "),
            Span::styled(auth_label, Style::default().fg(Color::White)),
        ]),
        Line::from(vec![
            Span::raw("  Message:   "),
            Span::styled(
                format!("\"{}\"", app.config.general.message),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ])
    .block(
        Block::default()
            .title(" Status ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(status, rows[1]);

    // ── Targets ──
    let target_lines: Vec<Line> = app
        .users
        .iter()
        .map(|(name, st)| {
            let (icon, col, label) = match st {
                UserStatus::Pending => ("○", Color::DarkGray, "Pending".to_string()),
                UserStatus::Sending => ("◐", Color::Blue, "Sending...".to_string()),
                UserStatus::Sent => ("●", Color::Green, "Sent".to_string()),
                UserStatus::Failed => ("✗", Color::Red, "Failed".to_string()),
                UserStatus::Retrying(n) => ("↻", Color::Yellow, format!("Retry #{n}")),
            };
            Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{:20}", name),
                    Style::default().fg(Color::White),
                ),
                Span::styled(
                    format!("{icon} {label}"),
                    Style::default().fg(col),
                ),
            ])
        })
        .collect();

    let targets_widget = Paragraph::new(target_lines).block(
        Block::default()
            .title(" Targets ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(targets_widget, rows[2]);

    // ── Activity log ──
    let log_area_height = rows[3].height.saturating_sub(2) as usize; // minus borders
    let total = app.logs.len();
    let start = if total > log_area_height {
        // Keep the view pinned to the bottom unless the user scrolled up
        let max_start = total.saturating_sub(log_area_height);
        app.log_offset.min(max_start)
    } else {
        0
    };

    let visible: Vec<Line> = app.logs[start..]
        .iter()
        .take(log_area_height)
        .map(|(ts, entry)| {
            let (msg, col) = match entry {
                LogEntry::Info(m) => (m.clone(), Color::White),
                LogEntry::Warn(m) => (m.clone(), Color::Yellow),
                LogEntry::Error(m) => (m.clone(), Color::Red),
                LogEntry::Success(m) => (m.clone(), Color::Green),
                LogEntry::UserStatus { user, status } => {
                    (format!("{user} → {status:?}"), Color::Cyan)
                }
                LogEntry::BotState(s) => (format!("State → {s:?}"), Color::Blue),
            };
            Line::from(vec![
                Span::styled(
                    format!(" {ts} "),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(msg, Style::default().fg(col)),
            ])
        })
        .collect();

    let log_widget = Paragraph::new(visible).block(
        Block::default()
            .title(" Activity Log ")
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(log_widget, rows[3]);

    // ── Hotkeys ──
    let hotkeys = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            "q",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Quit   "),
        Span::styled(
            "r",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Run Now   "),
        Span::styled(
            "↑↓",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" Scroll Log"),
    ]);
    f.render_widget(Paragraph::new(hotkeys), rows[4]);
}

// ── TUI event loop ──

pub async fn run(config: Config) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(&config);

    app.push_log(LogEntry::Info("Bot started".into()));
    app.push_log(LogEntry::Info(format!(
        "Loaded config — {} target(s)",
        config.targets.users.len()
    )));

    if config.general.test_mode {
        app.push_log(LogEntry::Warn(
            "TEST MODE — running immediately".into(),
        ));
    } else {
        app.push_log(LogEntry::Info(format!(
            "Scheduled: daily at {:02}:{:02}",
            config.schedule.hour, config.schedule.minute
        )));
    }

    let (log_tx, mut log_rx) = mpsc::unbounded_channel::<LogEntry>();
    let mut events = EventStream::new();
    let mut last_run_date: Option<chrono::NaiveDate> = None;

    loop {
        // Render
        terminal.draw(|f| draw(f, &app))?;

        // Await events — tick every 250 ms to update the clock
        let tick = tokio::time::sleep(Duration::from_millis(250));
        tokio::pin!(tick);

        tokio::select! {
            maybe_ev = events.next() => {
                if let Some(Ok(Event::Key(key))) = maybe_ev {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Char('Q') => {
                            if !app.running || key.modifiers.contains(KeyModifiers::CONTROL) {
                                break;
                            }
                        }
                        KeyCode::Char('r') | KeyCode::Char('R') if !app.running => {
                            app.trigger = true;
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            app.log_offset = app.log_offset.saturating_sub(1);
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            app.log_offset = app.log_offset.saturating_add(1);
                        }
                        KeyCode::Esc if app.running => {
                            // ignore — can't stop mid-run safely
                        }
                        _ => {}
                    }
                }
            }
            Some(entry) = log_rx.recv() => {
                app.push_log(entry);
            }
            _ = &mut tick => {
                // Schedule check
                if !app.running && !config.general.test_mode {
                    let now = Local::now();
                    let target = NaiveTime::from_hms_opt(
                        config.schedule.hour,
                        config.schedule.minute,
                        0,
                    )
                    .unwrap_or_default();

                    let cur = now.time();
                    // 1-minute window
                    let end_min = (config.schedule.minute + 1) % 60;
                    let end_hr = if config.schedule.minute == 59 {
                        (config.schedule.hour + 1) % 24
                    } else {
                        config.schedule.hour
                    };
                    let end = NaiveTime::from_hms_opt(end_hr, end_min, 0).unwrap_or_default();

                    let in_window = cur >= target && cur < end;
                    let today = now.date_naive();

                    if in_window && last_run_date != Some(today) {
                        last_run_date = Some(today);
                        app.trigger = true;
                    }
                }
            }
        }

        // Launch a bot run if triggered
        if app.trigger && !app.running {
            app.trigger = false;
            app.running = true;
            app.reset_users();
            app.state = BotState::Idle;

            let cfg = config.clone();
            let tx = log_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = run_bot(&cfg, &tx).await {
                    let _ = tx.send(LogEntry::Error(format!("Bot error: {e}")));
                    let _ = tx.send(LogEntry::BotState(BotState::Error));
                }
            });
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}
