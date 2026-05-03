// SPDX-License-Identifier: MIT
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Terminal,
};
use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use wayclick_ipc_client::SyncClient;

// Catppuccin Mocha palette
const BASE: Color = Color::Rgb(30, 30, 46);
const SURFACE0: Color = Color::Rgb(49, 50, 68);
const SURFACE1: Color = Color::Rgb(69, 71, 90);
const TEXT: Color = Color::Rgb(205, 214, 244);
const SUBTEXT0: Color = Color::Rgb(166, 173, 200);
const BLUE: Color = Color::Rgb(137, 180, 250);
const GREEN: Color = Color::Rgb(166, 227, 161);
const RED: Color = Color::Rgb(243, 139, 168);
const YELLOW: Color = Color::Rgb(249, 226, 175);
const MAUVE: Color = Color::Rgb(203, 166, 247);
const TEAL: Color = Color::Rgb(148, 226, 213);

#[derive(Debug, Clone)]
struct TriggerInfo {
    id: String,
    mode: String,
    action: String,
    active: bool,
}

#[derive(Debug, Clone)]
struct DeviceStatus {
    path: String,
    name: String,
    connected: bool,
}

struct App {
    socket_path: PathBuf,
    enabled: bool,
    dry_run: bool,
    config_path: String,
    uptime_secs: u64,
    backend: String,
    triggers: Vec<TriggerInfo>,
    devices: Vec<DeviceStatus>,
    logs: Vec<String>,
    trigger_list_state: ListState,
    active_pane: Pane,
    should_quit: bool,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Pane {
    Triggers,
    Devices,
}

impl App {
    fn new(socket_path: PathBuf) -> Self {
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            socket_path,
            enabled: false,
            dry_run: true,
            config_path: String::new(),
            uptime_secs: 0,
            backend: String::new(),
            triggers: Vec::new(),
            devices: Vec::new(),
            logs: Vec::new(),
            trigger_list_state: state,
            active_pane: Pane::Triggers,
            should_quit: false,
            last_error: None,
        }
    }

    fn refresh(&mut self) {
        self.last_error = None;

        // Fetch status
        match SyncClient::request(&self.socket_path, "status_json", None) {
            Ok(resp) => {
                if let Some(result) = resp.get("result") {
                    self.enabled = result
                        .get("enabled")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);
                    self.dry_run = result
                        .get("dry_run")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(true);
                    self.config_path = result
                        .get("config_path")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    self.uptime_secs = result
                        .get("uptime_secs")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    self.backend = result
                        .get("backend")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let active: Vec<String> = result
                        .get("active_triggers")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();

                    self.devices = result
                        .get("active_devices")
                        .and_then(|v| v.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| {
                                    v.as_str().map(|s| DeviceStatus {
                                        path: s.to_string(),
                                        name: String::new(),
                                        connected: true,
                                    })
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    for t in &mut self.triggers {
                        t.active = active.contains(&t.id);
                    }
                }
            }
            Err(e) => {
                self.last_error = Some(format!("Status: {}", e));
            }
        }

        // Fetch trigger list (result is a direct array)
        match SyncClient::request(&self.socket_path, "list_triggers", None) {
            Ok(resp) => {
                if let Some(result) = resp.get("result") {
                    let triggers_arr = result.as_array();
                    if let Some(triggers) = triggers_arr {
                        self.triggers = triggers
                            .iter()
                            .filter_map(|t| {
                                let id = t.get("id")?.as_str()?.to_string();
                                let mode = t
                                    .get("mode")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("toggle")
                                    .to_string();
                                let action = t
                                    .get("action_type")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let active =
                                    t.get("active").and_then(|v| v.as_bool()).unwrap_or(false);
                                Some(TriggerInfo {
                                    id,
                                    mode,
                                    action,
                                    active,
                                })
                            })
                            .collect();
                    }
                }
            }
            Err(e) => {
                self.last_error = Some(format!("Triggers: {}", e));
            }
        }

        // Fetch logs (result is array of {level, message, timestamp} objects)
        let params = serde_json::json!({"n": 50});
        match SyncClient::request(&self.socket_path, "logs_tail", Some(params)) {
            Ok(resp) => {
                if let Some(result) = resp.get("result") {
                    if let Some(logs) = result.as_array() {
                        self.logs = logs
                            .iter()
                            .filter_map(|entry| {
                                let level = entry.get("level")?.as_str()?;
                                let message = entry.get("message")?.as_str()?;
                                Some(format!("[{}] {}", level, message))
                            })
                            .collect();
                    }
                }
            }
            Err(e) => {
                if self.last_error.is_none() {
                    self.last_error = Some(format!("Logs: {}", e));
                }
            }
        }
    }

    fn selected_trigger(&self) -> Option<&TriggerInfo> {
        self.trigger_list_state
            .selected()
            .and_then(|i| self.triggers.get(i))
    }

    fn toggle_enabled(&mut self) {
        let method = if self.enabled { "disable" } else { "enable" };
        match SyncClient::request(&self.socket_path, method, None) {
            Ok(_) => self.enabled = !self.enabled,
            Err(e) => self.last_error = Some(format!("Toggle: {}", e)),
        }
    }

    fn reload_config(&mut self) {
        match SyncClient::request(&self.socket_path, "reload_config", None) {
            Ok(_) => {}
            Err(e) => self.last_error = Some(format!("Reload: {}", e)),
        }
    }

    fn fire_selected(&mut self) {
        if let Some(trigger) = self.selected_trigger().cloned() {
            let params = serde_json::json!({"id": trigger.id, "press": true});
            match SyncClient::request(&self.socket_path, "trigger", Some(params)) {
                Ok(_) => {}
                Err(e) => self.last_error = Some(format!("Fire: {}", e)),
            }
        }
    }

    fn next_trigger(&mut self) {
        let len = self.triggers.len();
        if len == 0 {
            return;
        }
        let i = self
            .trigger_list_state
            .selected()
            .map(|i| if i + 1 >= len { 0 } else { i + 1 })
            .unwrap_or(0);
        self.trigger_list_state.select(Some(i));
    }

    fn prev_trigger(&mut self) {
        let len = self.triggers.len();
        if len == 0 {
            return;
        }
        let i = self
            .trigger_list_state
            .selected()
            .map(|i| if i == 0 { len - 1 } else { i - 1 })
            .unwrap_or(0);
        self.trigger_list_state.select(Some(i));
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = wayclick_ipc_client::socket::default_socket_path();

    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(socket_path);
    app.refresh();

    let tick_rate = Duration::from_millis(500);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        let timeout = tick_rate.saturating_sub(last_tick.elapsed());
        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                handle_key(&mut app, key);
            }
        }

        if last_tick.elapsed() >= tick_rate {
            app.refresh();
            last_tick = Instant::now();
        }

        if app.should_quit {
            break;
        }
    }

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }
        KeyCode::Char('t') => app.toggle_enabled(),
        KeyCode::Char('e') if !app.enabled => {
            app.toggle_enabled();
        }
        KeyCode::Char('d') if app.enabled => {
            app.toggle_enabled();
        }
        KeyCode::Char('r') => app.reload_config(),
        KeyCode::Enter | KeyCode::Char(' ') => app.fire_selected(),
        KeyCode::Down | KeyCode::Char('j') => {
            if let Pane::Triggers = app.active_pane {
                app.next_trigger();
            }
        }
        KeyCode::Up | KeyCode::Char('k') => {
            if let Pane::Triggers = app.active_pane {
                app.prev_trigger();
            }
        }
        KeyCode::Tab => {
            app.active_pane = match app.active_pane {
                Pane::Triggers => Pane::Devices,
                Pane::Devices => Pane::Triggers,
            };
        }
        _ => {}
    }
}

fn ui(f: &mut ratatui::Frame, app: &mut App) {
    let size = f.area();

    // Main background
    let bg = Block::default().style(Style::default().bg(BASE));
    f.render_widget(bg, size);

    // Layout: header, body, footer
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(10),   // Body
            Constraint::Length(1), // Footer
        ])
        .split(size);

    // Header
    render_header(f, app, main_chunks[0]);

    // Body: left/right split
    let body_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(main_chunks[1]);

    // Left: triggers (top) + devices (bottom)
    let left_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(body_chunks[0]);

    // Right: detail (top) + logs (bottom)
    let right_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(body_chunks[1]);

    render_triggers(f, app, left_chunks[0]);
    render_devices(f, app, left_chunks[1]);
    render_detail(f, app, right_chunks[0]);
    render_logs(f, app, right_chunks[1]);

    // Footer
    render_footer(f, app, main_chunks[2]);
}

fn render_header(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let status = if app.enabled { "enabled" } else { "disabled" };
    let status_color = if app.enabled { GREEN } else { RED };
    let mode = if app.dry_run { " dry-run" } else { "" };

    let header = Line::from(vec![
        Span::styled(
            " wayclick ",
            Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
        ),
        Span::styled("── [", Style::default().fg(SURFACE1)),
        Span::styled(
            status,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("]", Style::default().fg(SURFACE1)),
        Span::styled(mode, Style::default().fg(YELLOW)),
        Span::styled(
            format!(" ── config: {} ", app.config_path),
            Style::default().fg(SUBTEXT0),
        ),
        Span::styled(
            format!("── backend: {} ", app.backend),
            Style::default().fg(SUBTEXT0),
        ),
        Span::styled(
            format!("── uptime: {}s ", app.uptime_secs),
            Style::default().fg(SUBTEXT0),
        ),
    ]);

    let widget = Paragraph::new(header).style(Style::default().bg(SURFACE0));
    f.render_widget(widget, area);
}

fn render_triggers(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    let is_active = app.active_pane == Pane::Triggers;
    let border_color = if is_active { BLUE } else { SURFACE1 };

    let items: Vec<ListItem> = app
        .triggers
        .iter()
        .map(|t| {
            let indicator = if t.active { "●" } else { "○" };
            let state = if t.active { "[ACTIVE]" } else { "[idle]" };
            let state_color = if t.active { GREEN } else { SUBTEXT0 };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!(" {} ", indicator),
                    Style::default().fg(if t.active { GREEN } else { SURFACE1 }),
                ),
                Span::styled(format!("{:<16}", t.id), Style::default().fg(TEXT)),
                Span::styled(format!("{:<10}", state), Style::default().fg(state_color)),
                Span::styled(&t.mode, Style::default().fg(MAUVE)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" TRIGGERS ")
        .title_style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BASE));

    let list = List::new(items).block(block).highlight_style(
        Style::default()
            .bg(SURFACE1)
            .fg(TEXT)
            .add_modifier(Modifier::BOLD),
    );

    f.render_stateful_widget(list, area, &mut app.trigger_list_state);
}

fn render_detail(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" TRIGGER DETAIL ")
        .title_style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SURFACE1))
        .style(Style::default().bg(BASE));

    if let Some(trigger) = app.selected_trigger() {
        let lines = vec![
            Line::from(vec![
                Span::styled("  id:      ", Style::default().fg(SUBTEXT0)),
                Span::styled(&trigger.id, Style::default().fg(TEXT)),
            ]),
            Line::from(vec![
                Span::styled("  mode:    ", Style::default().fg(SUBTEXT0)),
                Span::styled(&trigger.mode, Style::default().fg(MAUVE)),
            ]),
            Line::from(vec![
                Span::styled("  action:  ", Style::default().fg(SUBTEXT0)),
                Span::styled(&trigger.action, Style::default().fg(TEAL)),
            ]),
            Line::from(vec![
                Span::styled("  active:  ", Style::default().fg(SUBTEXT0)),
                Span::styled(
                    if trigger.active { "yes" } else { "no" },
                    Style::default().fg(if trigger.active { GREEN } else { RED }),
                ),
            ]),
        ];

        let paragraph = Paragraph::new(lines).block(block);
        f.render_widget(paragraph, area);
    } else {
        let paragraph = Paragraph::new(Line::from(Span::styled(
            "  No trigger selected",
            Style::default().fg(SUBTEXT0),
        )))
        .block(block);
        f.render_widget(paragraph, area);
    }
}

fn render_devices(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let is_active = app.active_pane == Pane::Devices;
    let border_color = if is_active { BLUE } else { SURFACE1 };

    let items: Vec<ListItem> = app
        .devices
        .iter()
        .map(|d| {
            let indicator = if d.connected { "✓" } else { "✗" };
            let color = if d.connected { GREEN } else { RED };
            let display_name = if d.name.is_empty() { &d.path } else { &d.name };

            ListItem::new(Line::from(vec![
                Span::styled(format!(" {} ", indicator), Style::default().fg(color)),
                Span::styled(format!("{:<24}", d.path), Style::default().fg(TEXT)),
                Span::styled(display_name, Style::default().fg(SUBTEXT0)),
            ]))
        })
        .collect();

    let block = Block::default()
        .title(" DEVICES ")
        .title_style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(BASE));

    let list = List::new(items).block(block);
    f.render_widget(list, area);
}

fn render_logs(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let block = Block::default()
        .title(" RECENT LOGS ")
        .title_style(Style::default().fg(BLUE).add_modifier(Modifier::BOLD))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(SURFACE1))
        .style(Style::default().bg(BASE));

    // Show last N logs that fit
    let available_height = area.height.saturating_sub(2) as usize;
    let start = if app.logs.len() > available_height {
        app.logs.len() - available_height
    } else {
        0
    };

    let lines: Vec<Line> = app.logs[start..]
        .iter()
        .map(|log| {
            let color = if log.contains("[ERROR]") || log.contains("[error]") {
                RED
            } else if log.contains("[WARN]") || log.contains("[warn]") {
                YELLOW
            } else if log.contains("[DEBUG]") || log.contains("[debug]") {
                SUBTEXT0
            } else {
                TEXT
            };
            Line::from(Span::styled(
                format!("  {}", log),
                Style::default().fg(color),
            ))
        })
        .collect();

    let paragraph = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn render_footer(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let footer_text = if let Some(ref err) = app.last_error {
        Line::from(Span::styled(
            format!(" ⚠ {} ", err),
            Style::default().fg(RED).bg(SURFACE0),
        ))
    } else {
        Line::from(vec![
            Span::styled(" q", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":quit  ", Style::default().fg(SUBTEXT0)),
            Span::styled("t", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":toggle  ", Style::default().fg(SUBTEXT0)),
            Span::styled("r", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":reload  ", Style::default().fg(SUBTEXT0)),
            Span::styled("e", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":enable  ", Style::default().fg(SUBTEXT0)),
            Span::styled("d", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":disable  ", Style::default().fg(SUBTEXT0)),
            Span::styled("↑↓", Style::default().fg(BLUE).add_modifier(Modifier::BOLD)),
            Span::styled(":select  ", Style::default().fg(SUBTEXT0)),
            Span::styled(
                "enter",
                Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(":fire  ", Style::default().fg(SUBTEXT0)),
            Span::styled(
                "tab",
                Style::default().fg(BLUE).add_modifier(Modifier::BOLD),
            ),
            Span::styled(":switch pane", Style::default().fg(SUBTEXT0)),
        ])
    };

    let widget = Paragraph::new(footer_text).style(Style::default().bg(SURFACE0));
    f.render_widget(widget, area);
}
